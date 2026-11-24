//! Drives `agy`'s full-screen sign-in through a pseudo-terminal.
//!
//! DETAILS >>> The CLI has no non-interactive login. It shows a method menu,
//! prints a Google URL, and accepts the authorization code in a text field.
//! The field only submits on the kitty-keyboard Enter sequence, and a plain
//! carriage return sent together with the pasted code is swallowed, so the code
//! and Enter go as separate writes. The login ends when the token file appears.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, sleep_until};

use super::{link_probe, probe_token};
use crate::runtime::{LoginCodes, configure_account, wait_for_cancel};
use crate::{Error, LoginEvent, LoginProgress, LoginRequest, Provider, Result};

static ESCAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1B\][^\x07\x1B]*(?:\x07|\x1B\\)|\x1B\[[0-9;?<>=]*[ -/]*[@-~]|\x1B[()][0-9A-Za-z]|\x1B[=>78DEM]")
        .expect("valid terminal escape expression")
});
static AUTH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://accounts\.google\.com/o/oauth2/auth\?[^\s]*?state=[A-Za-z0-9_-]{22}")
        .expect("valid Antigravity authentication URL expression")
});

const KITTY_ENTER: &[u8] = b"\x1b[13u";
const URL_TIMEOUT: Duration = Duration::from_secs(45);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_SCREEN: usize = 64 * 1024;

pub(super) async fn run(
    executable: &std::path::Path,
    mut request: LoginRequest,
    progress: LoginProgress,
) -> Result<()> {
    let home = request.home.clone();
    link_probe(&home)?;
    let token = probe_token(&home);
    let (master, slave) = open_pty()?;
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave))
        .env("TERM", "xterm-256color")
        .kill_on_drop(true);
    configure_account(&mut command, Provider::Antigravity, &home);
    // Own session and controlling terminal, so the full-screen UI starts and
    // the whole process tree can be stopped through its group.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop(command);
    let _guard = ProcessGroup(child.id());

    let mut reader = File::from(master.try_clone()?);
    let mut writer = File::from(master);
    let (sender, mut output) = mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        while let Ok(count @ 1..) = reader.read(&mut buffer) {
            if sender.send(buffer[..count].to_vec()).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut screen = String::new();
    let mut menu_answered = false;
    let mut codes: Option<LoginCodes> = None;
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    loop {
        let deadline = started
            + if codes.is_some() {
                LOGIN_TIMEOUT
            } else {
                URL_TIMEOUT
            };
        tokio::select! {
            _ = wait_for_cancel(&mut request.cancel) => return Err(Error::Cancelled),
            _ = sleep_until(deadline) => {
                return Err(Error::Timeout(
                    if codes.is_some() { "Antigravity browser sign-in" } else { "Antigravity sign-in URL" }.into(),
                ));
            }
            chunk = output.recv() => match chunk {
                Some(bytes) => {
                    screen.push_str(&ESCAPES.replace_all(&String::from_utf8_lossy(&bytes), ""));
                    if screen.len() > MAX_SCREEN {
                        screen.drain(..screen.len() - MAX_SCREEN);
                    }
                }
                None => return finished(&token),
            },
            code = next_code(codes.as_mut()) => {
                writer.write_all(code?.trim().as_bytes())?;
                sleep(Duration::from_millis(400)).await;
                writer.write_all(KITTY_ENTER)?;
            }
            _ = poll.tick(), if codes.is_some() => {
                if std::fs::metadata(&token).is_ok_and(|meta| meta.len() > 0) {
                    // Let the CLI finish writing before it is stopped.
                    sleep(Duration::from_secs(1)).await;
                    return finished(&token);
                }
            }
        }
        if !menu_answered && screen.contains("Select login method") {
            menu_answered = true;
            writer.write_all(b"\r")?;
        }
        if codes.is_none()
            && let Some(url) = AUTH_URL.find(&screen)
        {
            codes = Some(LoginCodes::open(
                &request.pending_directory,
                &request.slug,
                request.accept_terminal_input,
            )?);
            let mut url = url.as_str().to_owned();
            if let Some(email) = &request.expected_email {
                // The CLI cannot choose the Google account; preselect it.
                url.push_str("&login_hint=");
                url.push_str(&percent_encode(email));
            }
            progress(LoginEvent::SignInUrl { url });
            progress(LoginEvent::WaitingForCode {
                slug: request.slug.clone(),
            });
        }
    }
}

async fn next_code(codes: Option<&mut LoginCodes>) -> Result<String> {
    match codes {
        Some(codes) => codes.next().await,
        None => std::future::pending().await,
    }
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn finished(token: &std::path::Path) -> Result<()> {
    if std::fs::metadata(token).is_ok_and(|meta| meta.len() > 0) {
        Ok(())
    } else {
        Err(Error::Login(
            "Antigravity exited before the sign-in completed.".into(),
        ))
    }
}

fn open_pty() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 50,
        ws_col: 300,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let status = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe { Ok((OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))) }
}

/// Stops the CLI and anything it started when the login ends or is dropped.
struct ProcessGroup(Option<u32>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            unsafe {
                libc::kill(-(group as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_sign_in_url_from_a_wrapped_terminal_line() {
        let screen = "Your browser should open automatically. If not:\n https://accounts.google.com/o/oauth2/auth?access_type=offline&client_id=1.apps.googleusercontent.com&code_challenge=abc&state=sao_WnUn0RAANinmCYZn7QCopy and paste the URL";
        assert_eq!(
            AUTH_URL.find(screen).unwrap().as_str(),
            "https://accounts.google.com/o/oauth2/auth?access_type=offline&client_id=1.apps.googleusercontent.com&code_challenge=abc&state=sao_WnUn0RAANinmCYZn7Q"
        );
    }

    #[test]
    fn the_login_hint_is_percent_encoded() {
        assert_eq!(percent_encode("me+a@x.com"), "me%2Ba%40x.com");
    }

    #[test]
    fn terminal_escapes_are_removed() {
        let raw = "\x1b[>4;2m\x1b[1;32mSelect login method:\x1b[0m\x1b]0;title\x07";
        assert_eq!(ESCAPES.replace_all(raw, ""), "Select login method:");
    }
}
