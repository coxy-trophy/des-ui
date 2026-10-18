//! One-shot readiness acknowledgement for the session startup watchdog.
use std::{io, path::{Path, PathBuf}};

pub struct Readiness(Option<PathBuf>);

impl Readiness {
    pub fn from_env() -> Self {
        Self(std::env::var_os("DES_UI_SESSION_READY").map(PathBuf::from))
    }

    pub fn notify(&mut self) {
        if let Some(path) = self.0.take() {
            if let Err(error) = publish(&path) {
                eprintln!("des-ui: cannot acknowledge startup readiness: {error}");
            }
        }
    }
}

fn publish(path: &Path) -> io::Result<()> {
    use std::{fs::{self, OpenOptions}, io::Write, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    if !path.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "readiness path must be absolute"));
    }
    let parent = path.parent().ok_or_else(|| io::Error::other("no readiness directory"))?;
    let metadata = fs::symlink_metadata(parent)?;
    let uid = fs::metadata("/proc/self")?.uid();
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "readiness directory must be private and user-owned"));
    }
    // create_new refuses existing paths, including symlinks. Partial markers
    // cannot acknowledge readiness: the watchdog requires exact contents.
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(b"ready\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let name = format!("des-ui-readiness-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    #[test]
    fn publishes_private_one_shot_signal_without_overwriting() {
        let dir = Directory::new();
        let path = dir.0.join("ready");
        publish(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"ready\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(publish(&path).is_err());
    }
    #[test]
    fn rejects_symlinks_and_nonprivate_parent() {
        let dir = Directory::new();
        let target = dir.0.join("target");
        let link = dir.0.join("ready");
        std::fs::write(&target, "unchanged").unwrap();
        symlink(&target, &link).unwrap();
        assert!(publish(&link).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(publish(&dir.0.join("new")).is_err());
    }
}
