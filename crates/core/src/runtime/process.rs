use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::{Error, Provider, Result};

pub fn locate_executable(provider: Provider) -> Option<PathBuf> {
    let (name, override_key) = match provider {
        Provider::Codex => ("codex", "CODESUBS_CODEX_EXECUTABLE"),
        Provider::Claude => ("claude", "CODESUBS_CLAUDE_EXECUTABLE"),
    };
    if let Some(override_path) = env::var_os(override_key) {
        let path = PathBuf::from(override_path);
        return (path.is_absolute() && is_executable(&path)).then_some(path);
    }
    let mut candidates = Vec::new();
    if let Ok(current) = env::current_exe()
        && let Some(directory) = current.parent()
    {
        candidates.push(directory.join(name));
        if let Some(contents) = directory.parent() {
            candidates.push(contents.join("Resources").join(name));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join(name)));
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    if let (Provider::Claude, Some(home)) = (provider, &home) {
        candidates.push(home.join(".claude/local/claude"));
    }
    candidates.push(Path::new("/opt/homebrew/bin").join(name));
    candidates.push(Path::new("/usr/local/bin").join(name));
    if let Some(home) = home {
        candidates.push(home.join(".local/bin").join(name));
    }
    // Make relative PATH entries absolute before setting the child's cwd.
    let current = env::current_dir().ok();
    candidates
        .into_iter()
        .filter_map(|path| {
            if path.is_absolute() {
                Some(path)
            } else {
                current.as_ref().map(|directory| directory.join(path))
            }
        })
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

pub fn configure_account(command: &mut Command, provider: Provider, home: &Path) {
    command.current_dir(home);
    match provider {
        Provider::Codex => {
            command.env("CODEX_HOME", home);
        }
        Provider::Claude => {
            command.env("CLAUDE_CONFIG_DIR", home);
            for key in [
                "CLAUDE_CODE_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
            ] {
                command.env_remove(key);
            }
        }
    }
}

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

pub(crate) struct CommandProcess {
    pub child: Child,
    #[cfg(unix)]
    process_group: Option<u32>,
}

impl CommandProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command.kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn()?;
        Ok(Self {
            #[cfg(unix)]
            process_group: child.id(),
            child,
        })
    }

    pub async fn stop(&mut self) {
        self.signal_stop();
        let _ = self.child.wait().await;
    }

    fn signal_stop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.process_group.take() {
            // The child starts its own process group, so descendants with open
            // copies of its pipes are shut down as well.
            unsafe {
                libc::kill(-(group as libc::pid_t), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for CommandProcess {
    fn drop(&mut self) {
        self.signal_stop();
    }
}

pub(crate) struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
}

pub(crate) async fn run<I, S>(
    executable: &Path,
    args: I,
    account: Option<(Provider, &Path)>,
    input: Option<&[u8]>,
    deadline: Duration,
    name: &str,
) -> Result<ProcessOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        // Auth diagnostics can contain credentials. Discard stderr without
        // buffering it or allowing the child to block on an undrained pipe.
        .stderr(Stdio::null());
    if let Some((provider, home)) = account {
        configure_account(&mut command, provider, home);
    }
    let mut process = CommandProcess::spawn(&mut command)?;
    let stdin = process.child.stdin.take();
    let stdout = process.child.stdout.take().expect("piped stdout");
    let complete = async {
        let write = async {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                stdin.write_all(input).await?;
                stdin.shutdown().await?;
            }
            Ok::<_, io::Error>(())
        };
        let (_, stdout, status) =
            tokio::try_join!(write, read_output(stdout), process.child.wait())?;
        Ok::<_, io::Error>(ProcessOutput { status, stdout })
    };
    let result = tokio::time::timeout(deadline, complete).await;
    process.stop().await;
    result
        .map_err(|_| Error::Timeout(name.to_owned()))?
        .map_err(Error::from)
}

async fn read_output(reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
    let mut output = Vec::new();
    reader
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut output)
        .await?;
    if output.len() as u64 > OUTPUT_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Provider output exceeded 4 MiB.",
        ));
    }
    Ok(output)
}

pub(crate) async fn wait_for_cancel(cancel: &mut Option<watch::Receiver<bool>>) {
    if let Some(cancel) = cancel {
        loop {
            if *cancel.borrow_and_update() {
                return;
            }
            if cancel.changed().await.is_err() {
                break;
            }
        }
    }
    std::future::pending::<()>().await;
}
