use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::Duration,
};

use fs2::FileExt;

use crate::Result;

pub(super) fn lock(path: &Path) -> Result<File> {
    let file = open_lock(path)?;
    FileExt::lock_exclusive(&file)?;
    Ok(file)
}

pub(super) async fn lock_async(path: &Path) -> Result<File> {
    let file = open_lock(path)?;
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(super) fn try_lock(path: &Path) -> Result<Option<File>> {
    let file = open_lock(path)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn open_lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "expected a regular lock file").into(),
        );
    }
    Ok(file)
}
