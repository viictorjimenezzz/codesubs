#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

struct Scratch {
    _root: tempfile::TempDir,
    user: PathBuf,
    vault: PathBuf,
    control: PathBuf,
}

impl Scratch {
    fn new(vault_prefix: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        let vault = root.path().join(vault_prefix);
        let control = root.path().join("control");
        for directory in [&user, &vault, &control] {
            fs::create_dir(directory).unwrap();
        }
        Self {
            _root: root,
            user,
            vault,
            control,
        }
    }

    fn cli(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codesubs"));
        command
            .env("HOME", &self.user)
            .env("CODESUBS_HOME", &self.vault);
        command
    }

    fn seed_claude(&self) {
        let manifest = json!({
            "version": 3,
            "accounts": [{
                "id": "0a07e658-1bde-4ee0-b802-ae0aea9b0b7f",
                "provider": "claude",
                "slug": "work"
            }]
        });
        fs::write(self.vault.join("accounts.json"), manifest.to_string()).unwrap();
    }

    fn login(&self, slug: &str) -> tokio::process::Child {
        let fixture = self.control.join("codex-fixture");
        write_executable(&fixture, CODEX_FIXTURE);
        let mut command = tokio::process::Command::from(self.cli());
        command
            .env("CODESUBS_CODEX_EXECUTABLE", &fixture)
            .env("CODESUBS_TEST_CONTROL", &self.control)
            .args(["add", "codex", "--slug", slug, "--json-events"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    fn assert_rolled_back(&self, slug: &str) {
        assert!(!self.vault.join("codex").join(slug).exists());
        assert!(self.control.join("cancelled").exists());
        let output = self.cli().args(["list", "--json"]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!([])
        );
    }
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn exec_forwards_non_utf8_argument() {
    let scratch = Scratch::new("vault");
    scratch.seed_claude();
    let executable = scratch.control.join("echo-fixture");
    write_executable(&executable, "#!/bin/sh\nprintf '%s' \"$1\"\nexit 37\n");
    let value = b"argument-\xff\xfe with spaces";
    let output = scratch
        .cli()
        .args(["exec", "claude/work", "--"])
        .arg(executable)
        .arg(OsString::from_vec(value.to_vec()))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, value);
}

#[test]
fn fish_environment_round_trips_backslashes_and_single_quotes() {
    let candidates = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("fish"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let Some(fish) = candidates
        .into_iter()
        .chain(
            [
                "/opt/homebrew/bin/fish",
                "/usr/local/bin/fish",
                "/usr/bin/fish",
            ]
            .map(PathBuf::from),
        )
        .find(|path| {
            path.is_file() && fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        })
    else {
        return;
    };
    let scratch = Scratch::new("two\\\\backslashes-and-a-quote' here");
    scratch.seed_claude();
    let output = scratch
        .cli()
        .args(["env", "claude/work", "--fish"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snippet = String::from_utf8(output.stdout).unwrap();
    let program = format!(
        "{snippet}\nfor name in CLAUDE_CODE_OAUTH_TOKEN ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN\n    if set -q $name; exit 13; end\nend\nprintf '%s' \"$CLAUDE_CONFIG_DIR\""
    );
    let output = Command::new(fish)
        .args(["--no-config", "-c", &program])
        .env("HOME", &scratch.user)
        .env("XDG_CONFIG_HOME", &scratch.user)
        .env("CLAUDE_CODE_OAUTH_TOKEN", "fixture")
        .env("ANTHROPIC_API_KEY", "fixture")
        .env("ANTHROPIC_AUTH_TOKEN", "fixture")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        scratch
            .vault
            .join("claude/work")
            .to_str()
            .unwrap()
            .as_bytes()
    );
}

#[test]
fn json_events_bad_commands_emit_a_terminal_error_event() {
    let scratch = Scratch::new("vault");
    for args in [["list", "--json-events"], ["add", "--json-events"]] {
        let output = scratch.cli().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let event: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(event["protocolVersion"], 1);
        assert_eq!(event["type"], "error");
        assert_eq!(event["code"], "invalid_input");
        let diagnostic: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(diagnostic["error"]["code"], "invalid_input");
    }
}

async fn finish_login(child: tokio::process::Child) -> Output {
    tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("fake sign-in should cancel promptly")
        .unwrap()
}

fn assert_cancelled(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(diagnostic["error"]["code"], "cancelled");
}

#[tokio::test]
async fn sigterm_cancels_fake_login_and_rolls_back_new_home() {
    let scratch = Scratch::new("vault");
    let mut child = scratch.login("signal");
    fs::write(scratch.control.join("emit-login"), "").unwrap();
    let pid = child.id().unwrap().to_string();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let event = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("login URL");
    let event: Value = serde_json::from_str(&event).unwrap();
    assert_eq!(event["type"], "login");
    assert_eq!(event["authURL"], "https://example.test/login");
    let signal = Command::new("/bin/kill")
        .args(["-TERM", &pid])
        .status()
        .unwrap();
    assert!(signal.success());
    let event = tokio::time::timeout(Duration::from_secs(5), stdout.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("cancellation event");
    let event: Value = serde_json::from_str(&event).unwrap();
    assert_eq!(event["type"], "error");
    assert_eq!(event["code"], "cancelled");
    assert_cancelled(&finish_login(child).await);
    scratch.assert_rolled_back("signal");
}

#[tokio::test]
async fn closed_progress_pipe_cancels_fake_login_and_rolls_back_new_home() {
    let scratch = Scratch::new("vault");
    let mut child = scratch.login("broken");
    drop(child.stdout.take());
    fs::write(scratch.control.join("emit-login"), "").unwrap();
    let output = finish_login(child).await;
    assert_cancelled(&output);
    scratch.assert_rolled_back("broken");
}

const CODEX_FIXTURE: &str = r#"#!/bin/sh
set -eu
[ "$#" -eq 2 ] && [ "$1" = app-server ] && [ "$2" = --stdio ]
while IFS= read -r request; do
    id=$(printf '%s\n' "$request" | /usr/bin/sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$id" ;;
        *'"method":"initialized"'*) ;;
        *'"method":"account/login/start"'*)
            attempt=0
            while [ ! -e "$CODESUBS_TEST_CONTROL/emit-login" ]; do
                [ "$attempt" -lt 100 ] || exit 13
                /bin/sleep 0.025
                attempt=$((attempt + 1))
            done
            printf '%s\n' '{"fake":"credential"}' > "$CODEX_HOME/auth.json"
            printf '{"id":%s,"result":{"loginId":"fixture-login","authUrl":"https://example.test/login"}}\n' "$id"
            ;;
        *'"method":"account/login/cancel"'*'"loginId":"fixture-login"'*)
            printf 'received\n' > "$CODESUBS_TEST_CONTROL/cancelled"
            printf '{"id":%s,"result":{"status":"canceled"}}\n' "$id"
            ;;
        *) exit 12 ;;
    esac
done
"#;
