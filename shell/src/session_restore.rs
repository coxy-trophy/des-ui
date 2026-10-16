//! Explicit, privacy-limited desktop session snapshots and bounded restoration.
use crate::apps::AppEntry;
use des_common::ipc::{DesktopState, Place, WORKSPACES};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs::{self, File}, io::{self, Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}, time::{Duration, Instant}};

const MAX_BYTES: usize = 256 * 1024;
const MAX_APPS: usize = 128;
const MAX_TERMINALS: usize = 32;
const MAX_SCREENS: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session { Session { version: 1, workspace: 0, apps: vec![], terminals: TerminalLayout::default() } }
    fn entry() -> AppEntry { AppEntry { desktop_id: "editor.desktop".into(), name: "Editor".into(), exec: vec!["editor".into()], startup_wm_class: None, icon: None, terminal: false } }
    #[test] fn malformed_and_bounded() {
        for bytes in [b"{}".as_slice(), br#"{"version":2,"workspace":0,"apps":[],"terminals":{"directories":[],"active":0,"side":null,"stacked":false}}"#] { assert!(decode(bytes).is_err()); }
        assert!(decode(&vec![b' '; MAX_BYTES + 1]).is_err());
        let mut s = session(); s.workspace = WORKSPACES; assert!(s.validate().is_err());
        let mut s = session(); s.terminals.directories = vec![PathBuf::from("relative")]; assert!(s.validate().is_err());
        let mut s = session(); s.terminals.directories = vec![PathBuf::from("/tmp"); MAX_TERMINALS + 1]; assert!(s.validate().is_err());
        let mut s = session(); s.terminals.side = Some(0); assert!(s.validate().is_err());
        let mut value = serde_json::to_value(session()).unwrap(); value["exec"] = serde_json::json!(["evil"]); assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut s = session(); s.apps = vec![SavedApp { desktop_id: "../evil.desktop".into(), screen: 0, workspace: 0, place: SavedPlace::Main }]; assert!(s.validate().is_err());
    }
    #[test] fn atomic_private_storage_and_symlinks() {
        let temp = tempfile::tempdir().unwrap(); fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap(); let dir = temp.path().join("session");
        save_in(&dir, &session()).unwrap_or_else(|e| panic!("{}: {e}", dir.display())); assert_eq!(load_in(&dir).unwrap(), session());
        assert_eq!(fs::metadata(dir.join("session.json")).unwrap().mode() & 0o777, 0o600);
        fs::set_permissions(dir.join("session.json"), fs::Permissions::from_mode(0o644)).unwrap(); assert!(load_in(&dir).is_err());
        fs::set_permissions(dir.join("session.json"), fs::Permissions::from_mode(0o600)).unwrap();
        save_in(&dir, &session()).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let logs = temp.path().join("des-ui"); fs::create_dir(&logs).unwrap();
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o775)).unwrap();
        let sibling = temp.path().join("des-ui-sessions");
        save_in(&sibling, &session()).unwrap(); assert_eq!(load_in(&sibling).unwrap(), session());
        assert_eq!(fs::metadata(&logs).unwrap().mode() & 0o777, 0o775);
        assert_eq!(fs::read_dir(&logs).unwrap().count(), 0);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap(); assert!(load_in(&dir).is_err());
        let link = temp.path().join("link"); std::os::unix::fs::symlink(&dir, &link).unwrap(); assert!(save_in(&link, &session()).is_err());
    }
    #[test] fn capture_retains_inactive_workspace_layout() {
        let entries = vec![entry()]; let mut state = DesktopState::default();
        for workspace in [1, 2] { state.apps.push(des_common::ipc::AppInfo { app_id: "editor".into(), title: "private".into(), screen: 0, workspace, place: Place::Hidden, pinned: false }); }
        state.saved_places = vec![Place::Side, Place::Main];
        let session = Session::capture(&state, &entries, TerminalLayout::default());
        assert_eq!(session.apps.iter().map(|a| a.place).collect::<Vec<_>>(), vec![SavedPlace::Side, SavedPlace::Main]);
        state.saved_places.clear();
        assert_eq!(Session::capture(&state, &entries, TerminalLayout::default()).apps[0].place, SavedPlace::Hidden);
    }
    #[test] fn privacy_and_restore_lifecycle() {
        let entries = vec![entry()]; let mut state = DesktopState::default();
        state.apps.push(des_common::ipc::AppInfo { app_id: "editor".into(), title: "SECRET document contents".into(), screen: 0, workspace: 0, place: Place::Main, pinned: false });
        let mut s = Session::capture(&state, &entries, TerminalLayout::default());
        let json = serde_json::to_string(&s).unwrap(); assert!(!json.contains("SECRET")); assert!(!json.contains("exec"));
        let now = Instant::now(); let r = Restore::new(&s, &entries, &state, now); assert!(r.launch.is_empty()); assert_eq!(r.placements.len(), 1);
        s.apps.push(s.apps[0].clone()); s.apps.push(SavedApp { desktop_id: "missing.desktop".into(), ..s.apps[0].clone() });
        let mut r = Restore::new(&s, &entries, &DesktopState::default(), now); assert_eq!(r.launch, vec![0, 0]); assert_eq!(r.missing, vec!["missing.desktop"]);
        assert_eq!(r.poll(&state, &entries, now).len(), 1); assert!(r.poll(&state, &entries, now).is_empty());
        assert!(r.poll(&state, &entries, now + Duration::from_secs(31)).is_empty()); assert!(!r.pending());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct TerminalLayout { pub directories: Vec<PathBuf>, pub active: usize, pub side: Option<usize>, pub stacked: bool }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SavedPlace { Hidden, Main, Side }
impl From<Place> for SavedPlace { fn from(p: Place) -> Self { match p { Place::Hidden => Self::Hidden, Place::Main => Self::Main, Place::Side => Self::Side } } }
impl From<SavedPlace> for Place { fn from(p: SavedPlace) -> Self { match p { SavedPlace::Hidden => Self::Hidden, SavedPlace::Main => Self::Main, SavedPlace::Side => Self::Side } } }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SavedApp { pub desktop_id: String, pub screen: usize, pub workspace: usize, pub place: SavedPlace }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Session { pub version: u32, pub workspace: usize, pub apps: Vec<SavedApp>, pub terminals: TerminalLayout }
fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid or insecure session snapshot") }
fn valid_id(id: &str) -> bool { id.ends_with(".desktop") && id.len() <= 255 && !id.starts_with('.') && !id.chars().any(|c| c.is_control() || c == '/' || c == '\\') }
impl Session {
    pub fn validate(&self) -> io::Result<()> {
        let t = &self.terminals;
        if self.version != 1 || self.workspace >= WORKSPACES || self.apps.len() > MAX_APPS || t.directories.len() > MAX_TERMINALS
            || (!t.directories.is_empty() && t.active >= t.directories.len()) || (t.directories.is_empty() && t.active != 0)
            || t.side.is_some_and(|i| i >= t.directories.len() || i == t.active)
            || self.apps.iter().any(|a| !valid_id(&a.desktop_id) || a.screen >= MAX_SCREENS || a.workspace >= WORKSPACES)
            || t.directories.iter().any(|p| !p.is_absolute() || p.to_str().is_none_or(|s| s.len() > 4096 || s.chars().any(char::is_control))) { return Err(invalid()); }
        Ok(())
    }
    pub fn capture(state: &DesktopState, entries: &[AppEntry], terminals: TerminalLayout) -> Self {
        Self { version: 1, workspace: state.workspace, terminals, apps: state.apps.iter().enumerate().filter_map(|(index, a)| {
            let entry = &entries[entry_for(&a.app_id, entries)?];
            valid_id(&entry.desktop_id).then(|| SavedApp { desktop_id: entry.desktop_id.clone(), screen: a.screen, workspace: a.workspace, place: state.saved_places.get(index).copied().unwrap_or(a.place).into() })
        }).take(MAX_APPS).collect() }
    }
}
pub fn entry_for(id: &str, entries: &[AppEntry]) -> Option<usize> {
    let normalize = |s: &str| s.trim().to_lowercase().trim_end_matches(".desktop").to_owned();
    let key = normalize(id); if key.is_empty() { return None; }
    entries.iter().position(|e| e.startup_wm_class.as_deref().is_some_and(|s| normalize(s) == key))
        .or_else(|| entries.iter().position(|e| normalize(&e.desktop_id) == key))
        .or_else(|| entries.iter().position(|e| e.exec.first().and_then(|s| Path::new(s).file_name()).is_some_and(|s| normalize(&s.to_string_lossy()) == key)))
}
fn decode(bytes: &[u8]) -> io::Result<Session> { if bytes.len() > MAX_BYTES { return Err(invalid()); } let s: Session = serde_json::from_slice(bytes).map_err(|_| invalid())?; s.validate()?; Ok(s) }
fn directory() -> io::Result<PathBuf> {
    let root = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state"))).ok_or_else(invalid)?;
    Ok(root.join("des-ui-sessions"))
}
fn secure_dir(dir: &Path, create: bool) -> io::Result<()> {
    if !dir.is_absolute() { return Err(invalid()); }
    // Reject symbolic links in every ancestor, including the state home.
    let mut path = PathBuf::new();
    for part in dir.components() {
        if matches!(part, std::path::Component::ParentDir) { return Err(invalid()); }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                // A sticky system temporary root is safe; other writable ancestors
                // allow another user to replace the snapshot directory.
                if m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0 { return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("insecure session ancestor: {}", path.display()))); }
            },
            Ok(_) => return Err(invalid()),
            Err(e) if e.kind() == io::ErrorKind::NotFound && create => { std::fs::DirBuilder::new().mode(0o700).create(&path)?; },
            Err(e) => return Err(e),
        }
    }
    let m = fs::symlink_metadata(dir)?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o077 != 0 { return Err(invalid()); } Ok(())
}
use std::os::unix::fs::DirBuilderExt;
fn secure_file(path: &Path) -> io::Result<()> { let m = fs::symlink_metadata(path)?; if !m.is_file() || m.file_type().is_symlink() || m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o077 != 0 || m.len() > MAX_BYTES as u64 { return Err(invalid()); } Ok(()) }
pub fn save(s: &Session) -> io::Result<()> { save_in(&directory()?, s) }
pub fn load() -> io::Result<Session> { load_in(&directory()?) }
fn save_in(dir: &Path, s: &Session) -> io::Result<()> {
    s.validate()?; let bytes = serde_json::to_vec(s).map_err(|_| invalid())?; if bytes.len() > MAX_BYTES { return Err(invalid()); }
    secure_dir(dir, true)?; let target = dir.join("session.json");
    if fs::symlink_metadata(&target).is_ok() { secure_file(&target)?; }
    let mut temp = tempfile::NamedTempFile::new_in(dir)?; temp.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
    temp.write_all(&bytes)?; temp.as_file().sync_all()?; temp.persist(&target).map_err(|e| e.error)?; File::open(dir)?.sync_all()?; Ok(())
}
fn load_in(dir: &Path) -> io::Result<Session> {
    secure_dir(dir, false)?; let target = dir.join("session.json"); secure_file(&target)?;
    let file = fs::OpenOptions::new().read(true).custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32).open(target)?;
    let m = file.metadata()?; if !m.is_file() || m.len() > MAX_BYTES as u64 || m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o077 != 0 { return Err(invalid()); }
    let mut bytes = Vec::new(); file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?; decode(&bytes)
}
#[derive(Debug, Clone)]
pub struct Placement { pub index: usize, pub screen: usize, pub workspace: usize, pub place: Place }
fn placement(index: usize, app: &SavedApp, screens: usize) -> Placement { Placement { index, screen: app.screen.min(screens.saturating_sub(1)), workspace: app.workspace, place: app.place.into() } }
pub struct Restore { pub launch: Vec<usize>, pub missing: Vec<String>, pub placements: Vec<Placement>, waiting: Vec<SavedApp>, seen: Vec<String>, deadline: Instant, polls: usize }
impl Restore {
    pub fn new(session: &Session, entries: &[AppEntry], state: &DesktopState, now: Instant) -> Self {
        let mut result = Self { launch: vec![], missing: vec![], placements: vec![], waiting: vec![], seen: state.apps.iter().map(|a| a.app_id.clone()).collect(), deadline: now + Duration::from_secs(30), polls: 0 };
        let mut used = HashSet::new();
        if session.validate().is_err() { return result; }
        for app in &session.apps {
            let Some(entry) = entries.iter().position(|e| e.desktop_id == app.desktop_id) else { result.missing.push(app.desktop_id.clone()); continue; };
            if let Some((index, _)) = state.apps.iter().enumerate().find(|(i, a)| !used.contains(i) && entry_for(&a.app_id, entries) == Some(entry)) {
                used.insert(index); result.placements.push(placement(index, app, state.screens));
            } else { result.launch.push(entry); result.waiting.push(app.clone()); }
        }
        result
    }
    pub fn pending(&self) -> bool { !self.waiting.is_empty() }
    pub fn poll(&mut self, state: &DesktopState, entries: &[AppEntry], now: Instant) -> Vec<Placement> {
        self.polls += 1;
        if now >= self.deadline || self.polls > 1800 { self.waiting.clear(); return vec![]; }
        let mut output = vec![];
        // app_id multiplicities prevent pre-existing windows and previous arrivals being reused.
        let mut old = self.seen.clone();
        for (index, app) in state.apps.iter().enumerate() {
            if let Some(i) = old.iter().position(|id| id == &app.app_id) { old.remove(i); continue; }
            let Some(entry) = entry_for(&app.app_id, entries) else { continue; };
            if let Some(i) = self.waiting.iter().position(|s| s.desktop_id == entries[entry].desktop_id) {
                let saved = self.waiting.remove(i); output.push(placement(index, &saved, state.screens));
            }
        }
        self.seen = state.apps.iter().map(|a| a.app_id.clone()).collect(); output
    }
}
