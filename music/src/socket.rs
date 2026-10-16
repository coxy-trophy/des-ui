//! Private endpoint setup. Never remove a non-socket while recovering a stale service.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub fn prepare_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().filter(|p| p.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "music socket requires an absolute path"))?;
    match std::fs::DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => {},
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "music socket directory must be owned by this user with mode 0700"));
    }
    Ok(())
}

/// Keep this file open for the entire service lifetime. A second starter cannot
/// unlink the first service's endpoint between its stale check and bind.
pub fn lock(path: &Path) -> io::Result<std::fs::File> {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(lock_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "music socket lock must be a private owned file"));
    }
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    Ok(file)
}

pub fn remove_stale(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refusing to replace a non-owned music socket or a non-socket file"));
    }
    std::fs::remove_file(path)
}

pub fn restrict(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn temporary() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("des-music-permissions-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn creates_private_parent_and_rejects_shared_directories_and_symlinks() {
        let root = temporary();
        let private = root.join("private");
        prepare_parent(&private.join("music.sock")).unwrap();
        assert_eq!(std::fs::metadata(&private).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_parent(&private.join("music.sock")).is_err());
        symlink(&private, root.join("link")).unwrap();
        assert!(prepare_parent(&root.join("link/music.sock")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_one_service_can_own_an_endpoint_even_before_it_listens() {
        let root = temporary();
        let path = root.join("music.sock");
        let first = lock(&path).unwrap();
        assert!(lock(&path).is_err());
        drop(first);
        assert!(lock(&path).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_cleanup_never_deletes_a_regular_file_or_symlink() {
        let root = temporary();
        let path = root.join("music.sock");
        std::fs::write(&path, "keep me").unwrap();
        assert!(remove_stale(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
        let link = root.join("link");
        symlink(&path, &link).unwrap();
        assert!(remove_stale(&link).is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }
}
