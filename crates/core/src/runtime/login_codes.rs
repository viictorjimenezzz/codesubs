use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use tokio::io::unix::AsyncFd;

use crate::{Error, Result};

use super::create_private_dir;

// POSIX guarantees atomic pipe writes of up to 512 bytes.
const MAX_CODE_LENGTH: usize = 511;

pub(crate) struct LoginCodes {
    path: PathBuf,
    device: u64,
    inode: u64,
    fifo: AsyncFd<File>,
    fifo_buffer: Vec<u8>,
    terminal: Option<AsyncFd<File>>,
    terminal_buffer: Vec<u8>,
}

impl LoginCodes {
    pub fn open(pending: &Path, slug: &str, accept_terminal: bool) -> Result<Self> {
        if !valid_slug_component(slug) {
            return Err(Error::Login("Invalid sign-in account slug.".into()));
        }
        create_private_dir(pending)?;
        let path = pending.join(format!("claude-{slug}.code"));
        match open_writer(&path) {
            Ok(_) => {
                return Err(Error::Login(
                    "A sign-in code reader already exists for this account.".into(),
                ));
            }
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) => fs::remove_file(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        mkfifo(&path)?;
        let result = Self::open_created(path.clone(), accept_terminal);
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    fn open_created(path: PathBuf, accept_terminal: bool) -> Result<Self> {
        // Keep the FIFO readable between individual add-code invocations.
        let fifo = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(&path)?;
        let metadata = fifo.metadata()?;
        if !metadata.file_type().is_fifo() {
            return Err(Error::Login(
                "The sign-in code reader is not a FIFO.".into(),
            ));
        }
        let terminal = if accept_terminal && std::io::IsTerminal::is_terminal(&io::stdin()) {
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
                .open("/dev/tty");
            // The owned terminal descriptor is opened with O_NONBLOCK.
            file.ok()
                .and_then(|file| unsafe { AsyncFd::register(file) }.ok())
        } else {
            None
        };
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            // The owned FIFO descriptor is opened with O_NONBLOCK.
            fifo: unsafe { AsyncFd::register(fifo) }.map_err(io::Error::from)?,
            fifo_buffer: Vec::new(),
            terminal,
            terminal_buffer: Vec::new(),
        })
    }

    pub async fn next(&mut self) -> Result<String> {
        loop {
            let line = tokio::select! {
                line = read_line(&self.fifo, &mut self.fifo_buffer) => line?,
                line = read_optional_line(self.terminal.as_ref(), &mut self.terminal_buffer) => {
                    match line? {
                        Some(line) => Some(line),
                        None => {
                            self.terminal = None;
                            continue;
                        }
                    }
                }
            };
            if let Some(line) = line
                && let Ok(text) = std::str::from_utf8(&line)
                && let Ok(code) = validate_code(text)
            {
                return Ok(code.to_owned());
            }
        }
    }
}

impl Drop for LoginCodes {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

async fn read_optional_line(
    stream: Option<&AsyncFd<File>>,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<Vec<u8>>> {
    match stream {
        Some(stream) => read_line(stream, buffer).await,
        None => std::future::pending().await,
    }
}

async fn read_line(stream: &AsyncFd<File>, buffer: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    let mut input = [0; 512];
    loop {
        if let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=index).collect();
            return Ok(Some(line));
        }
        if buffer.len() > 4096 {
            buffer.clear();
        }
        let mut readiness = stream.readable().await?;
        match readiness.try_io(|stream| stream.get_ref().read(&mut input)) {
            Ok(Ok(0)) => return Ok(None),
            Ok(Ok(count)) => buffer.extend_from_slice(&input[..count]),
            Ok(Err(error)) => return Err(error),
            Err(_) => continue,
        }
    }
}

pub fn submit_login_code(pending: &Path, target: Option<&str>, code: &str) -> Result<()> {
    let code = validate_code(code)?;
    let entries = match fs::read_dir(pending) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::Login("No sign-in is waiting for a code.".into()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut active = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some((provider, slug)) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".code"))
            .and_then(|name| name.split_once('-'))
        else {
            continue;
        };
        if provider != "claude" || !valid_slug_component(slug) {
            continue;
        }
        if let Some(target) = target
            && target != slug
            && target != format!("{provider}/{slug}")
        {
            continue;
        }
        match open_writer(&entry.path()) {
            Ok(writer) => active.push(writer),
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENXIO | libc::ENOENT | libc::ELOOP)
                ) => {}
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut writer = match active.len() {
        0 => return Err(Error::Login("No sign-in is waiting for a code.".into())),
        1 => active.pop().expect("one active reader"),
        _ => {
            return Err(Error::Login(
                "Several sign-ins are waiting for a code. Pass --slug.".into(),
            ));
        }
    };
    let message = format!("{code}\n");
    match writer.write(message.as_bytes()) {
        Ok(length) if length == message.len() => Ok(()),
        Ok(_) => Err(Error::Login(
            "Could not send the complete sign-in code.".into(),
        )),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Err(Error::Login(
            "The sign-in is no longer waiting for a code.".into(),
        )),
        Err(error) => Err(error.into()),
    }
}

fn valid_slug_component(slug: &str) -> bool {
    !slug.is_empty()
        && slug != "."
        && slug != ".."
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn validate_code(code: &str) -> Result<&str> {
    let code = code.trim();
    if code.is_empty() || code.len() > MAX_CODE_LENGTH || code.chars().any(char::is_control) {
        return Err(Error::Login(
            "A sign-in code must be a single nonempty line of up to 511 bytes.".into(),
        ));
    }
    Ok(code)
}

fn mkfifo(path: &Path) -> io::Result<()> {
    let bytes = CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::mkfifo(bytes.as_ptr(), 0o600) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn open_writer(path: &Path) -> io::Result<File> {
    // Check before opening so a malicious filename cannot be used to open a
    // device. O_NOFOLLOW and metadata on the open descriptor guard replacement.
    if !fs::symlink_metadata(path)?.file_type().is_fifo() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected a FIFO.",
        ));
    }
    let bytes = CString::new(path.as_os_str().as_bytes())?;
    let fd = unsafe {
        libc::open(
            bytes.as_ptr(),
            libc::O_WRONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let writer = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    if !writer.metadata()?.file_type().is_fifo() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected a FIFO.",
        ));
    }
    Ok(writer)
}
