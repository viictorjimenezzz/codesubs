use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::Instant;

#[cfg(unix)]
use crate::runtime::LoginCodes;
use crate::runtime::{CommandProcess, configure_account, create_private_dir, wait_for_cancel};
use crate::{Error, LoginEvent, LoginProgress, LoginRequest, Provider, Result};

static ESCAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1B\][^\x07\x1B]*(?:\x07|\x1B\\)|\x1B\[[0-9;?]*[A-Za-z]")
        .expect("valid terminal escape expression")
});
static AUTH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://[^\s<>]+oauth/authorize\?[^\s<>]+")
        .expect("valid Claude authentication URL expression")
});

pub fn authorization_url(bytes: &[u8], stream_finished: bool) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let cleaned = ESCAPES.replace_all(&text, "");
    let matched = AUTH_URL.find(&cleaned)?;
    if (!stream_finished && matched.end() == cleaned.len())
        || !super::super::valid_auth_url(matched.as_str())
    {
        return None;
    }
    Some(matched.as_str().to_owned())
}

const URL_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_CAPTURE: usize = 32 * 1024;

pub(super) async fn run(
    executable: &Path,
    mut request: LoginRequest,
    progress: LoginProgress,
) -> Result<()> {
    if request
        .cancel
        .as_ref()
        .is_some_and(|cancel| *cancel.borrow())
    {
        return Err(Error::Cancelled);
    }
    create_private_dir(&request.home)?;
    let mut command = Command::new(executable);
    command
        .args(["auth", "login", "--claudeai"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(email) = &request.expected_email {
        command.arg("--email").arg(email);
    }
    configure_account(&mut command, Provider::Claude, &request.home);
    let mut process = CommandProcess::spawn(&mut command)?;
    let mut stdin = process.child.stdin.take().expect("piped stdin");
    let mut stdout = process.child.stdout.take().expect("piped stdout");
    let mut stderr = process.child.stderr.take().expect("piped stderr");
    let deadline = Instant::now();
    let mut stdout_buffer = [0; 4096];
    let mut stderr_buffer = [0; 4096];
    let mut out = LoginOutput::default();
    let mut err = LoginOutput::default();
    let mut has_url = false;
    let mut status = None;
    #[cfg(unix)]
    let mut codes: Option<LoginCodes> = None;

    let result = async {
        loop {
            if let Some(status) = status
                && (has_url || (out.eof && err.eof))
            {
                return outcome(status, has_url);
            }

            let until = deadline + if has_url { LOGIN_TIMEOUT } else { URL_TIMEOUT };
            tokio::select! {
                _ = wait_for_cancel(&mut request.cancel) => return Err(Error::Cancelled),
                _ = tokio::time::sleep_until(until) => {
                    return Err(Error::Timeout(if has_url { "Claude browser sign-in" } else { "Claude sign-in URL" }.into()));
                }
                result = process.child.wait(), if status.is_none() => status = Some(result?),
                result = stdout.read(&mut stdout_buffer), if !out.eof => {
                    out.consume(&stdout_buffer, result?, has_url);
                }
                result = stderr.read(&mut stderr_buffer), if !err.eof => {
                    err.consume(&stderr_buffer, result?, has_url);
                }
                code = next_code(
                    #[cfg(unix)]
                    codes.as_mut()
                ) => {
                    let code = code?;
                    let write_code = async {
                        stdin.write_all(code.as_bytes()).await?;
                        stdin.write_all(b"\n").await?;
                        stdin.flush().await
                    };
                    tokio::select! {
                        biased;
                        _ = wait_for_cancel(&mut request.cancel) => return Err(Error::Cancelled),
                        result = tokio::time::timeout(Duration::from_secs(5), write_code) => {
                            match result {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
                                Ok(Err(error)) => return Err(error.into()),
                                Err(_) => return Err(Error::Timeout("Claude sign-in code delivery".into())),
                            }
                        }
                    }
                }
            }
            if !has_url && let Some(url) = out.authorization_url().or_else(|| err.authorization_url()) {
                #[cfg(unix)]
                if status.is_none() {
                    codes = Some(LoginCodes::open(
                        &request.pending_directory,
                        &request.slug,
                        request.accept_terminal_input,
                    )?);
                }
                progress(LoginEvent::SignInUrl { url });
                #[cfg(unix)]
                if codes.is_some() {
                    progress(LoginEvent::WaitingForCode { slug: request.slug.clone() });
                }
                has_url = true;
                out.bytes.clear();
                err.bytes.clear();
            }
        }
    }.await;
    process.stop().await;
    result
}

#[cfg(unix)]
async fn next_code(codes: Option<&mut LoginCodes>) -> Result<String> {
    match codes {
        Some(codes) => codes.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn next_code() -> Result<String> {
    std::future::pending().await
}

fn outcome(status: std::process::ExitStatus, has_url: bool) -> Result<()> {
    if status.success() && has_url {
        Ok(())
    } else if status.success() {
        Err(Error::Login(
            "Claude Code did not return a sign-in URL.".into(),
        ))
    } else {
        Err(Error::Login(format!(
            "Claude sign-in did not complete ({status})."
        )))
    }
}

#[derive(Default)]
struct LoginOutput {
    eof: bool,
    bytes: Vec<u8>,
}

impl LoginOutput {
    fn consume(&mut self, input: &[u8], count: usize, has_url: bool) {
        self.eof = count == 0;
        if !has_url {
            self.bytes.extend_from_slice(&input[..count]);
            if self.bytes.len() > MAX_CAPTURE {
                self.bytes.drain(..self.bytes.len() - MAX_CAPTURE);
            }
        }
    }

    fn authorization_url(&self) -> Option<String> {
        authorization_url(&self.bytes, self.eof)
    }
}
