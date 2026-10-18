//! Private layout metadata for running terminal IDs; never commands or output.
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub tabs: Vec<String>,
    pub active: usize,
    pub side: Option<usize>,
    pub side_focused: bool,
    pub stacked: bool,
    pub scratchpad: Option<String>,
}

const MAX_BYTES: u64 = 32 * 1024;
const MAX_TABS: usize = 32;

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Layout {
    fn validate(&self) -> io::Result<()> {
        let unique: std::collections::HashSet<_> = self.tabs.iter().collect();
        if self.tabs.len() > MAX_TABS || unique.len() != self.tabs.len() || !self.tabs.iter().all(|s| id_ok(s))
            || (self.tabs.is_empty() && (self.active != 0 || self.side.is_some()))
            || (!self.tabs.is_empty() && self.active >= self.tabs.len())
            || self.side.is_some_and(|side| side >= self.tabs.len() || side == self.active)
            || self.scratchpad.as_ref().is_some_and(|id| !id_ok(id) || self.tabs.contains(id)) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid live terminal layout"));
        }
        Ok(())
    }
    pub fn order(&self, live: &[String], limit: usize) -> Vec<String> {
        let mut order = Vec::new();
        for id in self.tabs.iter().chain(live) {
            if order.len() >= limit.min(MAX_TABS) { break; }
            if id_ok(id) && live.contains(id) && !order.contains(id) { order.push(id.clone()); }
        }
        order
    }
    pub fn selected(&self, order: &[String]) -> (usize, Option<usize>) {
        let index = |old: usize| self.tabs.get(old).and_then(|id| order.iter().position(|live| live == id));
        let active = index(self.active).unwrap_or(0);
        let side = self.side.and_then(index).filter(|side| *side != active);
        (active, side)
    }
}

fn private_directory(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(directory)?;
    if !directory.is_absolute() || !meta.is_dir() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o7777 != 0o700 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "live terminal directory must be private and user-owned"));
    }
    Ok(())
}

fn private_file(meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o7777 != 0o600 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "live terminal metadata must be private and user-owned"));
    }
    Ok(())
}

pub fn load_in(directory: &Path) -> io::Result<Option<Layout>> {
    use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt};
    private_directory(directory)?;
    let path = directory.join("layout.json");
    let file = match OpenOptions::new().read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    private_file(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES { return Err(io::Error::other("live terminal metadata is too large")); }
    let layout: Layout = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    layout.validate()?;
    Ok(Some(layout))
}

pub fn save_in(directory: &Path, layout: &Layout) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::PermissionsExt};
    private_directory(directory)?;
    layout.validate()?;
    let path = directory.join("layout.json");
    match std::fs::symlink_metadata(&path) {
        Ok(meta) => private_file(&meta)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    let bytes = serde_json::to_vec(layout).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_BYTES { return Err(io::Error::other("live terminal metadata is too large")); }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(directory)?.sync_all()
}

/// Only the compositor's first UI can reconnect its terminals. A nested/manual UI
/// inherits the same environment but must not steal the original attach clients.
pub struct Lease { directory: std::path::PathBuf, _file: std::fs::File }
impl Lease {
    pub fn acquire() -> io::Result<Option<Self>> {
        des_common::terminal::directory()?.map(Self::acquire_in).transpose()
    }
    fn acquire_in(directory: std::path::PathBuf) -> io::Result<Self> {
        use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};
        private_directory(&directory)?;
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
            .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
            .open(directory.join("ui.lock"))?;
        private_file(&file.metadata()?)?;
        file.try_lock().map_err(io::Error::other)?;
        Ok(Self { directory, _file: file })
    }
    pub fn directory(&self) -> &Path { &self.directory }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn example() -> Layout {
        Layout { tabs: vec!["t-one".into(), "t-two".into()], active: 1, side: Some(0),
            side_focused: true, stacked: true, scratchpad: Some("s-one".into()) }
    }
    #[test]
    fn restores_order_and_selection_while_recovering_uncheckpointed_tabs() {
        let saved = example();
        let live = vec!["t-two".into(), "t-three".into(), "t-one".into()];
        let order = saved.order(&live, 9);
        assert_eq!(order, ["t-one", "t-two", "t-three"]);
        assert_eq!(saved.selected(&order), (1, Some(0)));
        let order = saved.order(&["t-two".into()], 9);
        assert_eq!(order, ["t-two"]);
        assert_eq!(saved.selected(&order), (0, None));
        assert_eq!(saved.order(&live, 1), ["t-one"]);
    }
    #[test]
    fn a_second_gui_cannot_steal_live_sessions_and_crash_releases_lease() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let first = Lease::acquire_in(dir.path().into()).unwrap();
        assert!(Lease::acquire_in(dir.path().into()).is_err());
        drop(first);
        assert!(Lease::acquire_in(dir.path().into()).is_ok());
    }
    #[test]
    fn private_metadata_round_trip_and_symlink_rejection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        save_in(dir.path(), &example()).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Some(example()));
        let file = dir.path().join("layout.json");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_file(&file).unwrap();
        let target = dir.path().join("other");
        std::fs::write(&target, "unchanged").unwrap();
        symlink(&target, &file).unwrap();
        assert!(save_in(dir.path(), &example()).is_err());
        assert!(load_in(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
    #[test]
    fn malformed_oversize_or_nonprivate_metadata_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.path().join("layout.json");
        std::fs::write(&file, b"{}").unwrap();
        assert!(load_in(dir.path()).is_err());
        std::fs::write(&file, vec![b' '; 32769]).unwrap();
        assert!(load_in(dir.path()).is_err());
        let mut bad = example(); bad.tabs.push("../other".into());
        assert!(save_in(dir.path(), &bad).is_err());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(save_in(dir.path(), &example()).is_err());
    }
}
