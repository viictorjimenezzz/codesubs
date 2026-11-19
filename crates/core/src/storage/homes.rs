use std::{
    fs::{self, DirBuilder, File, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::Path,
};

use crate::{Error, Result};

pub(super) fn private_directory(path: &Path) -> Result<()> {
    match DirBuilder::new().recursive(true).mode(0o700).create(path) {
        Ok(()) => require_directory(path)?,
        Err(error) => return Err(error.into()),
    }
    fs::set_permissions(path, Permissions::from_mode(0o700))?;
    Ok(())
}

pub(crate) fn require_directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "expected a directory at {}",
            path.display()
        )))
    }
}

pub(crate) fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn create_account_directory(path: &Path) -> Result<()> {
    DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                Error::SlugTaken(path.display().to_string())
            } else {
                error.into()
            }
        })?;
    sync_parent(path);
    Ok(())
}

pub(crate) fn move_directory(source: &Path, destination: &Path) -> Result<()> {
    require_directory(source)?;
    move_without_replacing(source, destination)
}

pub(super) fn move_without_replacing(source: &Path, destination: &Path) -> Result<()> {
    // Make collision detection atomic on the supported platform, including
    // against a provider helper operating outside the manager's locks.
    rename_exclusive(source, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            Error::SlugTaken(destination.display().to_string())
        } else {
            error.into()
        }
    })?;
    sync_parent(source);
    if source.parent() != destination.parent() {
        sync_parent(destination);
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;

    // The two NUL-terminated paths outlive the call, and neither is modified.
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(source, destination)
}

pub(crate) fn remove_directory(path: &Path) -> Result<()> {
    if exists(path)? {
        require_directory(path)?;
        fs::remove_dir_all(path)?;
        sync_parent(path);
    }
    Ok(())
}

pub(super) fn sync_parent(path: &Path) {
    // Once rename succeeds, the operation is committed from the point of view
    // of all readers. A subsequent directory sync failure must not trigger a
    // rollback of that visible commit.
    if let Some(parent) = path.parent() {
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
    }
}
