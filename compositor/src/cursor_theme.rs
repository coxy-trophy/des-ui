//! Static Xcursor fallback for clients that still draw their own cursor surfaces.
//! The on-disk format is the X.org Xcursor format, with one image per nominal size.
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write}, path::{Component, Path, PathBuf},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    sync::{Arc, Mutex, Condvar}};
use smithay::input::pointer::CursorIcon;
use crate::handlers::cursor_shape::Preferences;

const SIZES: [u32; 3] = [24, 32, 48];
const IMAGE_TYPE: u32 = 0xfffd0002;

// CSS names used by Wayland toolkits, plus conventional X11/libXcursor names.
// Each alias is a complete regular file, so packaging does not depend on symlinks.
fn names() -> Vec<(CursorIcon, &'static [&'static str])> {
    use CursorIcon::*;
    vec![
        (Default, &["default", "left_ptr", "arrow", "top_left_arrow", "left-arrow"]),
        (Pointer, &["pointer", "hand", "hand2", "pointing_hand", "e29285e634086352946a0e7090d73106", "9d800788f1b08800ae810202380a0822"]),
        (Text, &["text", "xterm", "ibeam"]),
        (VerticalText, &["vertical-text"]),
        (Grab, &["grab", "hand1", "openhand"]),
        (Grabbing, &["grabbing", "closedhand"]),
        (Wait, &["wait", "watch"]),
        (Progress, &["progress", "left_ptr_watch", "half-busy", "08e8e1c95fe2fc01f976f1e063a24ccd", "3ecb610c1bf2410f44200f48c40d3599"]),
        (NotAllowed, &["not-allowed", "forbidden", "crossed_circle", "circle", "X_cursor", "pirate", "03b6e0fcb3499374a867c041f52298f0"]),
        (NoDrop, &["no-drop", "dnd-no-drop"]),
        (Copy, &["copy", "dnd-copy", "1081e37283d90000800003c07f3ef6bf", "6407b0e94181790501fd1e167b474872"]),
        (Move, &["move", "fleur", "all-scroll", "all-resize", "size_all", "dnd-move", "4498f0e0c1937ffe01fd06f973665830", "9081237383d90e509aa00f00170e968f"]),
        (Alias, &["alias", "dnd-link", "link", "3085a0e285430894940527032f8b26df", "640fb0e74195791501fd1ed57b41487f"]),
        (Crosshair, &["crosshair", "cross", "cross_reverse", "diamond_cross", "tcross"]),
        (Cell, &["cell", "plus"]),
        (ZoomIn, &["zoom-in"]),
        (ZoomOut, &["zoom-out"]),
        (Help, &["help", "question_arrow", "whats_this", "left_ptr_help", "d9ce0ab605698f320427677b458ad60b", "5c6cd98b3f3ebcb1f9c7f1c204630408"]),
        (ContextMenu, &["context-menu"]),
        (DndAsk, &["dnd-ask"]),
        (EwResize, &["ew-resize", "sb_h_double_arrow", "h_double_arrow", "size_hor", "028006030e0e7ebffc7f7070c0600140", "14fef782d02440884392942c11205230"]),
        (NsResize, &["ns-resize", "sb_v_double_arrow", "v_double_arrow", "size_ver", "double_arrow", "00008160000006810000408080010102", "2870a09082c103050810ffdffffe0204"]),
        (ColResize, &["col-resize", "split_h"]),
        (RowResize, &["row-resize", "split_v"]),
        (NwseResize, &["nwse-resize", "bd_double_arrow", "size_fdiag", "c7088f0f3e6c8088236ef8e1e3e70000"]),
        (NeswResize, &["nesw-resize", "fd_double_arrow", "size_bdiag", "fcf1c3c7cd4491d801f1e1c78f100000"]),
        (NResize, &["n-resize", "top_side"]),
        (NeResize, &["ne-resize", "top_right_corner"]),
        (EResize, &["e-resize", "right_side"]),
        (SeResize, &["se-resize", "bottom_right_corner"]),
        (SResize, &["s-resize", "bottom_side"]),
        (SwResize, &["sw-resize", "bottom_left_corner"]),
        (WResize, &["w-resize", "left_side"]),
        (NwResize, &["nw-resize", "top_left_corner"]),
    ]
}

/// Smithay clears Xwayland's environment. Pass only cursor lookup inputs;
/// unrelated session variables and credentials must not enter that server.
pub fn client_environment()->Vec<(std::ffi::OsString,std::ffi::OsString)> {
    cursor_environment_from(std::env::vars_os())
}
fn cursor_environment_from(variables:impl IntoIterator<Item=(std::ffi::OsString,std::ffi::OsString)>)->Vec<(std::ffi::OsString,std::ffi::OsString)> {
    variables.into_iter().filter(|(name,_)|matches!(name.to_str(),Some("XCURSOR_THEME"|"XCURSOR_SIZE"|"XCURSOR_PATH"|"HOME"|"XDG_DATA_HOME"|"XDG_DATA_DIRS"))).collect()
}

fn word(bytes: &mut Vec<u8>, value: u32) { bytes.extend_from_slice(&value.to_le_bytes()); }

#[cfg(test)]
fn cursor_file(icon: CursorIcon) -> io::Result<Vec<u8>> {
    cursor_file_with_preferences(icon, &SIZES, Preferences::default())
}

fn cursor_file_with_preferences(icon: CursorIcon, sizes: &[u32], preferences: Preferences) -> io::Result<Vec<u8>> {
    encode_images(sizes.iter().map(|&nominal| (nominal, crate::handlers::cursor_shape::theme_image_with_preferences(icon, nominal, preferences))).collect())
}

fn encode_images(images: Vec<(u32, Image)>) -> io::Result<Vec<u8>> {
    let count = images.len();
    let mut chunks = Vec::new();
    let mut positions = Vec::new();
    for (nominal, (width, height, xhot, yhot, pixels)) in images {
        if width == 0 || height == 0 || width > 0x7fff || height > 0x7fff
            || xhot >= width || yhot >= height || pixels.len() != (width as usize * height as usize * 4) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid cursor image dimensions or hotspot"));
        }
        positions.push((nominal, 16 + count as u32 * 12 + chunks.len() as u32));
        for value in [36, IMAGE_TYPE, nominal, 1, width, height, xhot, yhot, 0] { word(&mut chunks, value); }
        // Xcursor stores packed ARGB CARD32s little endian: exactly the premultiplied
        // BGRA bytes produced by the compositor's shared static rasterizer.
        chunks.extend_from_slice(&pixels);
    }
    let mut bytes = Vec::with_capacity(16 + count * 12 + chunks.len());
    bytes.extend_from_slice(b"Xcur");
    for value in [16, 0x10000, count as u32] { word(&mut bytes, value); }
    for (nominal, position) in positions {
        for value in [IMAGE_TYPE, nominal, position] { word(&mut bytes, value); }
    }
    bytes.extend(chunks);
    Ok(bytes)
}

pub fn write_theme(path: &Path) -> io::Result<()> {
    write_theme_at_sizes(path, &SIZES, Preferences::default())
}

fn write_theme_at_sizes(path: &Path, sizes: &[u32], preferences: Preferences) -> io::Result<()> {
    let cursors = path.join("cursors");
    fs::create_dir_all(&cursors)?;
    fs::write(path.join("index.theme"), "[Icon Theme]\nName=DEs-UI\nComment=Solid geometric cursors for DEs-UI\nInherits=Adwaita\n")?;
    for (icon, aliases) in names() {
        let bytes = cursor_file_with_preferences(icon, sizes, preferences)?;
        for name in aliases { fs::write(cursors.join(name), &bytes)?; }
    }
    Ok(())
}

/// Toolkits request their inherited baseline times the output density. Tag each
/// configured image with that request size, while its physical artwork follows
/// the chosen logical size times density. This also works after a live size edit.
pub fn write_theme_with_preferences(path: &Path, preferences: Preferences) -> io::Result<()> {
    let baseline = std::env::var("XCURSOR_SIZE").ok().and_then(|value| value.parse::<u32>().ok())
        .filter(|size| (16..=64).contains(size)).unwrap_or(24);
    write_theme_with_baseline(path, preferences, baseline)
}

type Image = (u32, u32, u32, u32, Vec<u8>);
fn crop_image(image: Image) -> Image {
    let (width, height, xhot, yhot, pixels) = image;
    if width == 0 || height == 0 || xhot >= width || yhot >= height || pixels.len() != width as usize * height as usize * 4 {
        return (width, height, xhot, yhot, pixels);
    }
    let (mut left, mut right, mut top, mut bottom) = (xhot, xhot, yhot, yhot);
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        if pixel[3] == 0 { continue; }
        let (x,y) = (index as u32 % width, index as u32 / width);
        left = left.min(x); right = right.max(x); top = top.min(y); bottom = bottom.max(y);
    }
    left = left.saturating_sub(1); top = top.saturating_sub(1);
    right = (right + 1).min(width - 1); bottom = (bottom + 1).min(height - 1);
    let (cropped_width, cropped_height) = (right - left + 1, bottom - top + 1);
    let mut cropped = Vec::with_capacity((cropped_width * cropped_height * 4) as usize);
    for y in top..=bottom {
        let begin = ((y * width + left) * 4) as usize;
        cropped.extend_from_slice(&pixels[begin..begin + cropped_width as usize * 4]);
    }
    (cropped_width, cropped_height, xhot - left, yhot - top, cropped)
}

fn write_theme_with_baseline(path: &Path, preferences: Preferences, baseline: u32) -> io::Result<()> {
    let cursors = path.join("cursors");
    fs::create_dir_all(&cursors)?;
    fs::write(path.join("index.theme"), "[Icon Theme]\nName=DEs-UI\nComment=Configured geometric cursors for DEs-UI\nInherits=Adwaita\n")?;
    for (icon, aliases) in names() {
        write_linked_aliases(&cursors, aliases, &configured_file(icon, preferences, baseline)?)?;
    }
    Ok(())
}

fn configured_file(icon: CursorIcon, preferences: Preferences, baseline: u32) -> io::Result<Vec<u8>> {
    let preferences = Preferences { size: preferences.size.clamp(16,64), ..preferences };
    let baseline = baseline.clamp(16,64);
    let images = (1..=4).map(|density| (baseline * density,
        crop_image(crate::handlers::cursor_shape::theme_image_at_density(icon, preferences, density)))).collect();
    encode_images(images)
}

fn write_linked_aliases(cursors: &Path, aliases: &[&str], bytes: &[u8]) -> io::Result<()> {
    // Check the whole alias group before replacing any entry. Replacing an
    // inode also preserves unrelated user files hardlinked to an old export.
    for name in aliases {
        match fs::symlink_metadata(cursors.join(name)) {
            Ok(metadata) if metadata.is_file() && metadata.uid() == uid() => {},
            Ok(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "cursor destinations must be owned regular files")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    let canonical = cursors.join(aliases[0]);
    replace_cursor_file(&canonical, None, bytes)?;
    for alias in &aliases[1..] {
        replace_cursor_file(&cursors.join(alias), Some(&canonical), bytes)?;
    }
    Ok(())
}

struct StagedCursor(PathBuf);
impl Drop for StagedCursor {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

fn replace_cursor_file(path: &Path, source: Option<&Path>, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing cursor parent"))?;
    for _ in 0..8 {
        let staging = parent.join(format!(".des-ui-cursor-{}", random_suffix()?));
        if let Some(source) = source {
            match fs::hard_link(source, &staging) {
                Ok(()) => {
                    let staged = StagedCursor(staging);
                    fs::rename(&staged.0, path)?;
                    return Ok(());
                },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {}, // Filesystems without hardlinks use a private copy.
            }
        }
        match OpenOptions::new().write(true).create_new(true).mode(0o644).open(&staging) {
            Ok(mut file) => {
                let staged = StagedCursor(staging);
                file.write_all(bytes)?;
                drop(file);
                fs::rename(&staged.0, path)?;
                return Ok(());
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "cannot stage cursor file"))
}

fn random_suffix() -> io::Result<String> {
    let mut random = [0u8;16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn configured_theme_root() -> Option<PathBuf> {
    configured_root_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

fn configured_root_from(cache: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    cache.map(PathBuf::from).filter(|path| path.is_absolute())
        .or_else(|| home.map(PathBuf::from).filter(|path| path.is_absolute()).map(|home| home.join(".cache")))
        .map(|cache| cache.join("des-ui/cursor-theme"))
}

fn unsafe_cache(why: &str) -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, why) }
fn uid() -> u32 { smithay::reexports::rustix::process::geteuid().as_raw() }

fn private_root(root: &Path) -> io::Result<()> {
    if !root.is_absolute() || root.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(unsafe_cache("cursor cache must be an absolute path without parent components"));
    }
    let mut path = PathBuf::new();
    let mut private_ancestor = false;
    // Managed filesystems can map system root ownership to an unmapped UID.
    // Trust the owner of the filesystem root for immutable system ancestors.
    let system_owner = fs::symlink_metadata("/")?.uid();
    for component in root.components() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::DirBuilder::new().mode(0o700).create(&path)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                fs::symlink_metadata(&path)?
            },
            Err(error) => return Err(error),
        };
        // A normal cache may be group-writable beneath a private home: others
        // cannot reach or replace it through the mode 0700 ancestor. Do not
        // change permissions on existing user cache directories.
        let system_owned = metadata.uid() == 0 || metadata.uid() == system_owner;
        let sticky_system = system_owned && metadata.mode() & 0o1000 != 0;
        if !metadata.is_dir() || metadata.file_type().is_symlink()
            || (!system_owned && metadata.uid() != uid())
            || (metadata.mode() & 0o022 != 0 && !sticky_system && !private_ancestor) {
            return Err(unsafe_cache("cursor cache ancestors must be trusted directories without symlinks"));
        }
        private_ancestor |= metadata.uid() == uid() && metadata.mode() & 0o077 == 0 && metadata.mode() & 0o100 != 0;
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o700 {
        return Err(unsafe_cache("cursor cache must be a user-owned mode 0700 directory"));
    }
    Ok(())
}

fn lock_cache(root: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .custom_flags((smithay::reexports::rustix::fs::OFlags::NOFOLLOW | smithay::reexports::rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(root.join("publish.lock"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err(unsafe_cache("cursor cache lock must be a private owned regular file"));
    }
    file.lock()?;
    Ok(file)
}

const GENERATION_PREFIX: &str = ".generation-";
const GENERATION_MARKER: &str = ".des-ui-generated";
const MARKER_CONTENT: &[u8] = b"DEs-UI cursor theme generation v1\n";

fn generation_name(name: &str) -> bool {
    name.strip_prefix(GENERATION_PREFIX).is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn owned_generation(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else { return false; };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o700 { return false; }
    let Ok(marker) = OpenOptions::new().read(true)
        .custom_flags((smithay::reexports::rustix::fs::OFlags::NOFOLLOW | smithay::reexports::rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path.join(GENERATION_MARKER)) else { return false; };
    let Ok(metadata) = marker.metadata() else { return false; };
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 { return false; }
    let mut bytes = Vec::new();
    marker.take((MARKER_CONTENT.len() + 1) as u64).read_to_end(&mut bytes).is_ok() && bytes == MARKER_CONTENT
}

fn check_stable(root: &Path) -> io::Result<()> {
    let stable = root.join("DEs-UI");
    match fs::symlink_metadata(&stable) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(metadata) => {
            if !metadata.file_type().is_symlink() || metadata.uid() != uid() { return Err(unsafe_cache("configured theme path is not an owned generated link")); }
            let target = fs::read_link(stable)?;
            if !target.to_str().is_some_and(generation_name) || !owned_generation(&root.join(target)) {
                return Err(unsafe_cache("configured theme points outside owned generated storage"));
            }
            Ok(())
        }
    }
}

struct Generation { path: PathBuf, keep: bool }
impl Drop for Generation {
    fn drop(&mut self) { if !self.keep { let _ = fs::remove_dir_all(&self.path); } }
}

fn new_generation(root: &Path) -> io::Result<Generation> {
    for _ in 0..8 {
        let suffix = random_suffix()?;
        let path = root.join(format!("{GENERATION_PREFIX}{suffix}"));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {
                let generation = Generation { path, keep: false };
                fs::set_permissions(&generation.path, fs::Permissions::from_mode(0o700))?;
                // Mark allocation before writing artwork, so a later successful
                // publication can safely reclaim our interrupted generations too.
                let mut marker = OpenOptions::new().write(true).create_new(true).mode(0o600)
                    .open(generation.path.join(GENERATION_MARKER))?;
                marker.set_permissions(fs::Permissions::from_mode(0o600))?;
                marker.write_all(MARKER_CONTENT)?;
                return Ok(generation);
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "cannot allocate cursor theme generation"))
}

fn clean_old_generations(root: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(root) else { return; };
    let mut old: Vec<_> = entries.filter_map(Result::ok).filter_map(|entry| {
        let path = entry.path();
        if path == current || !entry.file_name().to_str().is_some_and(generation_name) || !owned_generation(&path) { return None; }
        let changed = entry.metadata().ok()?.modified().ok()?;
        Some((changed, path))
    }).collect();
    old.sort_by(|a,b| b.0.cmp(&a.0));
    // Keep two previous versions: clients can finish reads through a resolved link.
    for (_, path) in old.into_iter().skip(2) {
        if let Err(error) = fs::remove_dir_all(path) { tracing::warn!("cannot remove old cursor theme: {error}"); }
    }
}

fn publish_in(root: &Path, preferences: Preferences, writer: impl FnOnce(&Path, Preferences) -> io::Result<()>) -> io::Result<()> {
    private_root(root)?;
    let _lock = lock_cache(root)?;
    check_stable(root)?;
    let mut generation = new_generation(root)?;
    writer(&generation.path, preferences)?;
    fs::set_permissions(generation.path.join("cursors"), fs::Permissions::from_mode(0o700))?;
    let name = generation.path.file_name().unwrap();
    let link = root.join(format!(".publish-{}", name.to_string_lossy()));
    symlink(name, &link)?;
    if let Err(error) = fs::rename(&link, root.join("DEs-UI")) {
        let _ = fs::remove_file(link);
        return Err(error);
    }
    generation.keep = true;
    clean_old_generations(root, &generation.path);
    Ok(())
}

pub fn publish_configured(preferences: Preferences) -> io::Result<()> {
    let root = configured_theme_root().ok_or_else(|| io::Error::other("cursor cache is unavailable"))?;
    publish_in(&root, preferences, write_theme_with_preferences)
}

#[derive(Default)]
struct Pending { preferences: Option<Preferences>, last: Option<Preferences>, stopped: bool }

pub struct ThemeWorker { shared: Arc<(Mutex<Pending>, Condvar)> }
impl ThemeWorker {
    pub fn new() -> Self { Self::with_publisher(publish_configured) }
    fn with_publisher(mut publish: impl FnMut(Preferences) -> io::Result<()> + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let thread_state = shared.clone();
        if let Err(error) = std::thread::Builder::new().name("des-ui-cursor-theme".into()).spawn(move || {
            let (mutex, changed) = &*thread_state;
            loop {
                let mut pending = mutex.lock().unwrap_or_else(|error| error.into_inner());
                while !pending.stopped && pending.preferences.is_none() {
                    pending = changed.wait(pending).unwrap_or_else(|error| error.into_inner());
                }
                if pending.stopped { return; }
                let preferences = pending.preferences.take().unwrap();
                drop(pending);
                match publish(preferences) {
                    Ok(()) => {
                        let mut pending = mutex.lock().unwrap_or_else(|error| error.into_inner());
                        // A slider may return to the in-flight appearance while
                        // intermediate choices were queued. It is already ready.
                        if pending.preferences == Some(preferences) { pending.preferences = None; }
                    },
                    Err(error) => {
                        let mut pending = mutex.lock().unwrap_or_else(|error| error.into_inner());
                        if pending.last == Some(preferences) { pending.last = None; }
                        if pending.preferences == Some(preferences) { pending.preferences = None; }
                        drop(pending);
                        tracing::warn!("configured cursor theme was not updated; keeping the previous theme: {error}");
                    },
                }
            }
        }) {
            tracing::warn!("cannot start cursor theme worker: {error}");
            shared.0.lock().unwrap_or_else(|error| error.into_inner()).stopped = true;
        }
        Self { shared }
    }
    pub fn request(&self, preferences: Preferences) {
        let preferences = Preferences { size: preferences.size.clamp(16,64), ..preferences };
        let (mutex, changed) = &*self.shared;
        let mut pending = mutex.lock().unwrap_or_else(|error| error.into_inner());
        if pending.stopped || pending.last == Some(preferences) { return; }
        pending.last = Some(preferences);
        pending.preferences = Some(preferences);
        changed.notify_one();
    }
    pub fn note_published(&self, preferences: Preferences) {
        let preferences = Preferences { size: preferences.size.clamp(16,64), ..preferences };
        let mut pending = self.shared.0.lock().unwrap_or_else(|error| error.into_inner());
        if !pending.stopped { pending.last = Some(preferences); }
    }
}
impl Drop for ThemeWorker {
    fn drop(&mut self) {
        let (mutex, changed) = &*self.shared;
        let mut pending = mutex.lock().unwrap_or_else(|error| error.into_inner());
        pending.stopped = true;
        pending.preferences = None;
        changed.notify_one();
        // Detached by design: a filesystem stall must never block compositor exit.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

    struct Export(PathBuf);
    impl Export {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("des-ui-cursors-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())))
        }
    }
    impl Drop for Export {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn exports_valid_little_endian_images_at_three_sizes() {
        let export = Export::new();
        write_theme(&export.0).unwrap();
        let bytes = fs::read(export.0.join("cursors/default")).unwrap();
        assert_eq!(&bytes[..4], b"Xcur");
        assert_eq!(word(&bytes, 4), 16);
        assert_eq!(word(&bytes, 8), 0x10000);
        assert_eq!(word(&bytes, 12), 3);
        let mut expected_position = 16 + 12 * 3;
        for (i, nominal) in [24, 32, 48].into_iter().enumerate() {
            let toc = 16 + i * 12;
            assert_eq!(word(&bytes, toc), 0xfffd0002);
            assert_eq!(word(&bytes, toc + 4), nominal);
            assert_eq!(word(&bytes, toc + 8) as usize, expected_position);
            let chunk = expected_position;
            assert_eq!(word(&bytes, chunk), 36);
            assert_eq!(word(&bytes, chunk + 4), 0xfffd0002);
            assert_eq!(word(&bytes, chunk + 8), nominal);
            assert_eq!(word(&bytes, chunk + 12), 1);
            let (width, height) = (word(&bytes, chunk + 16), word(&bytes, chunk + 20));
            assert!(width > 0 && height > 0 && width <= 0x7fff && height <= 0x7fff);
            assert!(word(&bytes, chunk + 24) < width);
            assert!(word(&bytes, chunk + 28) < height);
            assert_eq!(word(&bytes, chunk + 32), 0);
            expected_position += 36 + (width * height * 4) as usize;
            let pixels = &bytes[chunk + 36..expected_position];
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
            assert!(pixels.chunks_exact(4).all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        }
        assert_eq!(bytes.len(), expected_position);
    }

    #[test]
    fn xwayland_environment_keeps_cursor_preferences_and_lookup_roots_only() {
        let variables=[("XCURSOR_THEME","UserTheme"),("XCURSOR_SIZE","40"),("XCURSOR_PATH","/private/icons"),("HOME","/home/person"),("XDG_DATA_HOME","/data"),("XDG_DATA_DIRS","/system/data:/other/data"),("DISPLAY",":42"),("SESSION_SECRET","private"),("GTK_THEME","unrelated")];
        let passed=cursor_environment_from(variables.into_iter().map(|(key,value)|(key.into(),value.into())));
        assert_eq!(passed.len(),6);
        for (key,value) in &variables[..6] {assert!(passed.contains(&((*key).into(),(*value).into())),"missing {key}");}
        assert!(passed.iter().all(|(key,_)|key!="DISPLAY" && key!="SESSION_SECRET" && key!="GTK_THEME"));
    }
    #[test]
    fn legacy_hashed_aliases_resolve_to_the_correct_semantic_artwork() {
        let mappings=names();
        for (icon,alias) in [(CursorIcon::Pointer,"e29285e634086352946a0e7090d73106"),(CursorIcon::Progress,"08e8e1c95fe2fc01f976f1e063a24ccd"),(CursorIcon::Copy,"1081e37283d90000800003c07f3ef6bf"),(CursorIcon::Alias,"3085a0e285430894940527032f8b26df"),(CursorIcon::EwResize,"14fef782d02440884392942c11205230"),(CursorIcon::NsResize,"2870a09082c103050810ffdffffe0204"),(CursorIcon::NwseResize,"c7088f0f3e6c8088236ef8e1e3e70000"),(CursorIcon::NeswResize,"fcf1c3c7cd4491d801f1e1c78f100000")] {
            assert!(mappings.iter().any(|(mapped,aliases)|*mapped==icon && aliases.contains(&alias)),"incorrect semantic alias {alias}");
        }
        let all:Vec<_>=mappings.iter().flat_map(|(_,aliases)|aliases.iter()).collect();
        let unique:std::collections::HashSet<_>=all.iter().copied().collect();assert_eq!(all.len(),unique.len(),"aliases cannot silently replace another cursor state");
    }
    #[test]
    fn aliases_are_regular_files_with_matching_bytes_and_named_theme() {
        let export = Export::new();
        write_theme(&export.0).unwrap();
        let index = fs::read_to_string(export.0.join("index.theme")).unwrap();
        assert!(index.contains("Name=DEs-UI\n"));
        for (canonical, aliases) in [("default", &["left_ptr", "arrow", "top_left_arrow"][..]),
            ("text", &["xterm"][..]), ("pointer", &["hand2", "hand"]),
            ("grab", &["hand1", "openhand"]), ("wait", &["watch"]),
            ("ew-resize", &["sb_h_double_arrow", "h_double_arrow"]),
            ("nwse-resize", &["bd_double_arrow"])] {
            let expected = fs::read(export.0.join("cursors").join(canonical)).unwrap();
            for alias in aliases {
                let path = export.0.join("cursors").join(alias);
                assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
                assert_eq!(fs::read(path).unwrap(), expected);
            }
        }
    }

    #[test]
    fn every_state_stores_the_shared_raster_and_hotspot_without_byte_conversion() {
        for (icon, _) in names() {
            let bytes = cursor_file(icon).unwrap();
            for (index, nominal) in SIZES.into_iter().enumerate() {
                let chunk = word(&bytes, 16 + index * 12 + 8) as usize;
                let (width, height, xhot, yhot, pixels) = crate::handlers::cursor_shape::theme_image(icon, nominal);
                assert_eq!(word(&bytes, chunk + 16), width);
                assert_eq!(word(&bytes, chunk + 20), height);
                assert_eq!(word(&bytes, chunk + 24), xhot);
                assert_eq!(word(&bytes, chunk + 28), yhot);
                assert_eq!(&bytes[chunk + 36..chunk + 36 + pixels.len()], pixels);
            }
        }
    }

    #[test]
    fn configured_theme_tracks_logical_size_and_color_at_every_client_density() {
        let export = Export::new();
        let preferences = Preferences { size: 40, color: [17, 101, 203] };
        let cursors = export.0.join("cursors");
        fs::create_dir_all(&cursors).unwrap();
        write_linked_aliases(&cursors, &["default", "left_ptr"], &configured_file(CursorIcon::Default, preferences, 24).unwrap()).unwrap();
        let bytes = fs::read(export.0.join("cursors/default")).unwrap();
        assert_eq!(word(&bytes, 12), 4);
        let mut reference: Option<[f64;4]> = None;
        for index in 0..4 {
            let density = index as f64 + 1.0;
            let toc = 16 + index * 12;
            assert_eq!(word(&bytes, toc + 4), 24 * (index as u32 + 1));
            let chunk = word(&bytes, toc + 8) as usize;
            let (width, height, xhot, yhot) = (word(&bytes, chunk + 16), word(&bytes, chunk + 20), word(&bytes, chunk + 24), word(&bytes, chunk + 28));
            assert!(xhot < width && yhot < height);
            let pixels = &bytes[chunk + 36..chunk + 36 + (width * height * 4) as usize];
            let colored = pixels.chunks_exact(4).filter(|pixel| pixel[3] == 255).any(|pixel| pixel[..3] == [203, 101, 17]);
            assert!(colored, "chosen RGB missing at density {density}");
            let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for (at, _) in pixels.chunks_exact(4).enumerate().filter(|(_, pixel)| pixel[3] > 0) {
                let x = (at as u32 % width) as f64 - xhot as f64;
                let y = (at as u32 / width) as f64 - yhot as f64;
                bounds[0] = bounds[0].min(x / density); bounds[1] = bounds[1].min(y / density);
                bounds[2] = bounds[2].max(x / density); bounds[3] = bounds[3].max(y / density);
            }
            if let Some(previous) = reference {
                for (a,b) in bounds.into_iter().zip(previous) { assert!((a-b).abs() <= 1.5, "cursor logical bounds changed with buffer scale: {bounds:?} vs {previous:?}"); }
            } else { reference = Some(bounds); }
        }
        let canonical = fs::metadata(export.0.join("cursors/default")).unwrap();
        assert_eq!(canonical.ino(), fs::metadata(export.0.join("cursors/left_ptr")).unwrap().ino());
    }

    #[test]
    fn cropping_removes_padding_but_keeps_hotspot_including_empty_images() {
        let mut pixels = vec![0; 10 * 10 * 4];
        pixels[(2 * 10 + 2) * 4..(2 * 10 + 2) * 4 + 4].copy_from_slice(&[3, 2, 1, 255]);
        let (w,h,x,y,pixels) = crop_image((10,10,8,8,pixels));
        assert_eq!((w,h,x,y), (9,9,7,7));
        assert_eq!(&pixels[(1 * w as usize + 1) * 4..(1 * w as usize + 1) * 4 + 4], &[3,2,1,255]);
        let (w,h,x,y,_) = crop_image((10,10,5,5,vec![0;400]));
        assert_eq!((w,h,x,y), (3,3,1,1));
    }

    #[test]
    fn repeated_configured_export_replaces_owned_aliases_without_touching_other_links() {
        let export = Export::new();
        let cursors = export.0.join("cursors");
        fs::create_dir_all(&cursors).unwrap();
        write_linked_aliases(&cursors, &["default", "left_ptr"], b"first").unwrap();
        let precious = export.0.join("precious");
        fs::hard_link(cursors.join("default"), &precious).unwrap();
        write_linked_aliases(&cursors, &["default", "left_ptr"], b"second").unwrap();
        for name in ["default", "left_ptr"] { assert_eq!(fs::read(cursors.join(name)).unwrap(), b"second"); }
        assert_eq!(fs::read(&precious).unwrap(), b"first");
        fs::remove_file(cursors.join("left_ptr")).unwrap();
        symlink(&precious, cursors.join("left_ptr")).unwrap();
        assert!(write_linked_aliases(&cursors, &["default", "left_ptr"], b"third").is_err());
        assert_eq!(fs::read(cursors.join("default")).unwrap(), b"second");
        assert_eq!(fs::read(precious).unwrap(), b"first");
    }

    #[test]
    fn note_published_prevents_startup_duplicate_and_failed_preferences_can_retry() {
        use std::{sync::mpsc, time::Duration};
        let (sent, received) = mpsc::channel();
        let worker = ThemeWorker::with_publisher(move |preferences| {
            sent.send(preferences).unwrap();
            Err(io::Error::other("simulated write failure"))
        });
        worker.note_published(Preferences::default());
        worker.request(Preferences::default());
        assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
        let changed = Preferences { size: 40, ..Preferences::default() };
        worker.request(changed);
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), changed);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while worker.shared.0.lock().unwrap().last.is_some() && std::time::Instant::now() < deadline { std::thread::yield_now(); }
        worker.request(changed);
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), changed);
    }

    #[test]
    fn atomic_publication_keeps_last_good_theme_when_generation_fails() {
        let export = Export::new();
        let root = export.0.join("cursor-theme");
        publish_in(&root, Preferences::default(), tiny_theme).unwrap();
        let stable = root.join("DEs-UI");
        let previous = fs::read_link(&stable).unwrap();
        let bytes = fs::read(stable.join("cursors/default")).unwrap();
        let result = publish_in(&root, Preferences { size: 40, color: [1, 2, 3] }, |path, _| {
            fs::write(path.join("partial"), b"not a theme")?;
            Err(io::Error::other("simulated disk failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read_link(&stable).unwrap(), previous);
        assert_eq!(fs::read(stable.join("cursors/default")).unwrap(), bytes);
        assert_eq!(fs::read_dir(&root).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with(".generation-")).count(), 1);
    }

    #[test]
    fn cache_rejects_symlink_parents_and_external_stable_targets() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let export = Export::new();
        fs::create_dir(&export.0).unwrap();
        fs::set_permissions(&export.0, fs::Permissions::from_mode(0o700)).unwrap();
        let outside = export.0.join("outside");
        fs::create_dir(&outside).unwrap();
        let root = export.0.join("cursor-theme");
        symlink(&outside, &root).unwrap();
        assert!(publish_in(&root, Preferences::default(), write_theme_with_preferences).is_err());
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(outside.join("precious"), b"keep").unwrap();
        symlink(&outside, root.join("DEs-UI")).unwrap();
        assert!(publish_in(&root, Preferences::default(), write_theme_with_preferences).is_err());
        assert_eq!(fs::read(outside.join("precious")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
    }

    #[test]
    fn worker_coalesces_pending_preferences_and_drop_never_waits_for_io() {
        use std::{sync::mpsc, time::{Duration, Instant}};
        let (sent, received) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let mut first = true;
        let worker = ThemeWorker::with_publisher(move |prefs| {
            sent.send(prefs).unwrap();
            if first { first = false; blocked.recv().unwrap(); }
            Ok(())
        });
        let initial = Preferences::default();
        worker.request(initial);
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), initial);
        worker.request(Preferences { size: 32, ..initial });
        let latest = Preferences { size: 40, ..initial };
        worker.request(latest);
        worker.request(latest);
        release.send(()).unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), latest);
        assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
        let began = Instant::now();
        drop(worker);
        assert!(began.elapsed() < Duration::from_millis(100));
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Disconnected)));
    }

    fn tiny_theme(path: &Path, preferences: Preferences) -> io::Result<()> {
        fs::create_dir(path.join("cursors"))?;
        fs::write(path.join("index.theme"), b"[Icon Theme]\nName=DEs-UI\n")?;
        fs::write(path.join("cursors/default"), preferences.size.to_string())
    }

    #[test]
    fn private_home_allows_normal_owned_cache_without_changing_its_permissions() {
        let export = Export::new();
        fs::create_dir(&export.0).unwrap();
        fs::set_permissions(&export.0, fs::Permissions::from_mode(0o700)).unwrap();
        let cache = export.0.join(".cache");
        let app = cache.join("des-ui");
        fs::create_dir_all(&app).unwrap();
        for path in [&cache, &app] { fs::set_permissions(path, fs::Permissions::from_mode(0o775)).unwrap(); }
        let root = app.join("cursor-theme");
        publish_in(&root, Preferences::default(), tiny_theme).unwrap();
        for path in [&cache, &app] { assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o775); }
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(root.join("DEs-UI")).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(root.join("DEs-UI/cursors")).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn cache_lock_symlink_and_nonprivate_root_are_rejected_without_touching_targets() {
        let export = Export::new();
        fs::create_dir(&export.0).unwrap();
        fs::set_permissions(&export.0, fs::Permissions::from_mode(0o700)).unwrap();
        let root = export.0.join("cursor-theme");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(publish_in(&root, Preferences::default(), tiny_theme).is_err());
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o755);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let precious = export.0.join("precious");
        fs::write(&precious, b"keep").unwrap();
        symlink(&precious, root.join("publish.lock")).unwrap();
        assert!(publish_in(&root, Preferences::default(), tiny_theme).is_err());
        assert_eq!(fs::read(precious).unwrap(), b"keep");
    }

    #[test]
    fn publication_retains_two_old_generations_and_never_removes_unmarked_user_files() {
        let export = Export::new();
        let root = export.0.join("cursor-theme");
        publish_in(&root, Preferences::default(), tiny_theme).unwrap();
        let unrelated = root.join(format!("{GENERATION_PREFIX}{}", "a".repeat(32)));
        fs::create_dir(&unrelated).unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(unrelated.join("precious"), b"keep").unwrap();
        let outside = export.0.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("precious"), b"outside").unwrap();
        let alias = root.join(format!("{GENERATION_PREFIX}{}", "b".repeat(32)));
        symlink(&outside, &alias).unwrap();
        for size in [28, 32, 40, 48] { publish_in(&root, Preferences { size, ..Preferences::default() }, tiny_theme).unwrap(); }
        assert_eq!(fs::read_to_string(root.join("DEs-UI/cursors/default")).unwrap(), "48");
        assert_eq!(fs::read_dir(&root).unwrap().filter_map(Result::ok).filter(|entry| owned_generation(&entry.path())).count(), 3);
        assert_eq!(fs::read(unrelated.join("precious")).unwrap(), b"keep");
        assert_eq!(fs::read(outside.join("precious")).unwrap(), b"outside");
        assert!(fs::symlink_metadata(alias).unwrap().file_type().is_symlink());
    }

    #[test]
    fn configured_path_requires_absolute_cache_or_home_without_changing_environment() {
        assert_eq!(configured_root_from(Some("/cache".into()), Some("/home/me".into())), Some(PathBuf::from("/cache/des-ui/cursor-theme")));
        assert_eq!(configured_root_from(Some("relative".into()), Some("/home/me".into())), Some(PathBuf::from("/home/me/.cache/des-ui/cursor-theme")));
        assert_eq!(configured_root_from(None, Some("relative".into())), None);
        assert_eq!(configured_root_from(None, None), None);
    }

    #[test]
    fn dropping_worker_during_blocked_io_does_not_join_or_publish_pending_work() {
        use std::{sync::mpsc, time::{Duration, Instant}};
        let (sent, received) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let worker = ThemeWorker::with_publisher(move |preferences| {
            sent.send(preferences).unwrap();
            blocked.recv().unwrap();
            Ok(())
        });
        worker.request(Preferences::default());
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.request(Preferences { size: 40, ..Preferences::default() });
        let began = Instant::now();
        drop(worker);
        assert!(began.elapsed() < Duration::from_millis(100));
        release.send(()).unwrap();
        assert!(matches!(received.recv_timeout(Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Disconnected)));
    }
}
