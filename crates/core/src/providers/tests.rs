use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use super::{ProviderBackend, claude, codex};
use crate::platform::keychain;
use crate::runtime::submit_login_code;
use crate::{Error, LoginEvent, LoginProgress, LoginRequest, QuotaWindowKind, ResetOutcome};

const CODEX_FIXTURE: &str = include_str!("../../tests/provider_fixtures/codex-app-server.sh");
const CLAUDE_FIXTURE: &str = include_str!("../../tests/provider_fixtures/claude-cli.sh");

fn executable(directory: &Path, name: &str, contents: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, contents).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn request(directory: &Path, name: &str) -> LoginRequest {
    LoginRequest {
        home: directory.join(name),
        expected_email: Some("claude@example.test".into()),
        pending_directory: directory.join("pending"),
        slug: name.into(),
        accept_terminal_input: false,
        cancel: None,
    }
}

fn progress() -> (LoginProgress, mpsc::UnboundedReceiver<LoginEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(move |event| tx.send(event).unwrap()), rx)
}

async fn next_progress(events: &mut mpsc::UnboundedReceiver<LoginEvent>) -> LoginEvent {
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn codex_protocol_uses_fresh_sessions_and_handles_early_notification() {
    let temp = tempfile::tempdir().unwrap();
    let backend = codex::CodexBackend::new(Some(executable(temp.path(), "codex", CODEX_FIXTURE)));
    let request = request(temp.path(), "codex-home");
    let home = request.home.clone();
    let account = backend.status(&home, true).await.unwrap();
    assert_eq!(account.email.as_deref(), Some("codex@example.test"));
    assert_eq!(account.plan.as_deref(), Some("plus"));
    assert_eq!(account.quota.available_percent(Utc::now()), Some(33));
    assert_eq!(account.quota.session().unwrap().label, "5h");
    assert_eq!(account.quota.reset_credits.unwrap().available, 1);
    assert_eq!(account.quota.credits.unwrap().balance, Some(2.5));
    let protocol = fs::read_to_string(home.join("rpc.log")).unwrap();
    assert_eq!(protocol, "handled\ninitialized\nforced\nnormal\n");

    let (progress, mut events) = progress();
    backend.login(request, progress).await.unwrap();
    assert_eq!(
        next_progress(&mut events).await,
        LoginEvent::SignInUrl {
            url: "https://example.test/codex/login".into(),
        }
    );
    let key = Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
    assert_eq!(
        backend.redeem_reset(&home, key).await.unwrap(),
        ResetOutcome::AlreadyRedeemed
    );
    let sessions = fs::read_to_string(home.join("sessions.log")).unwrap();
    assert_eq!(sessions.lines().count(), 3);
    for pid in sessions.lines() {
        let pid = pid.parse::<libc::pid_t>().unwrap();
        // Signal 0 checks existence. The app-server child is gone after each
        // independent operation, even before the enclosing runtime shuts down.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
    assert_eq!(
        fs::metadata(home.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn codex_cancellation_sends_rpc_and_reaps_the_helper() {
    let temp = tempfile::tempdir().unwrap();
    let backend = codex::CodexBackend::new(Some(executable(temp.path(), "codex", CODEX_FIXTURE)));
    let mut request = request(temp.path(), "codex-home");
    fs::create_dir(&request.home).unwrap();
    fs::write(request.home.join("pending-login"), "").unwrap();
    let (cancel, receiver) = watch::channel(false);
    request.cancel = Some(receiver);
    let home = request.home.clone();
    let progress: LoginProgress = Arc::new(move |_| {
        cancel.send_replace(true);
    });
    assert!(matches!(
        backend.login(request, progress).await,
        Err(Error::Cancelled)
    ));
    assert!(
        fs::read_to_string(home.join("rpc.log"))
            .unwrap()
            .contains("cancelled")
    );
}

#[tokio::test]
async fn claude_reads_usage_and_routes_code_to_live_cli() {
    let temp = tempfile::tempdir().unwrap();
    let backend =
        claude::ClaudeBackend::new(Some(executable(temp.path(), "claude", CLAUDE_FIXTURE)));
    let request = request(temp.path(), "claude-home");
    let home = request.home.clone();
    let account = backend.status(&home, false).await.unwrap();
    assert_eq!(account.email.as_deref(), Some("claude@example.test"));
    assert_eq!(account.quota.session().unwrap().used_percent, 2);
    assert_eq!(account.quota.windows[2].kind, QuotaWindowKind::WeeklyModel);
    assert_eq!(account.quota.available_percent(Utc::now()), Some(91));
    assert!(account.quota.reset_credits.is_none());
    backend.verify_credentials(&home).await.unwrap();

    let (progress, mut events) = progress();
    let pending = request.pending_directory.clone();
    let login = tokio::spawn(async move { backend.login(request, progress).await });
    assert_eq!(
        next_progress(&mut events).await,
        LoginEvent::SignInUrl {
            url: "https://example.test/cai/oauth/authorize?code=true&state=abcd".into(),
        }
    );
    assert_eq!(
        next_progress(&mut events).await,
        LoginEvent::WaitingForCode {
            slug: "claude-home".into()
        }
    );
    assert!(submit_login_code(&pending, Some("home"), "fake-code").is_err());
    submit_login_code(&pending, Some("claude/claude-home"), " fake-code ").unwrap();
    tokio::time::timeout(Duration::from_secs(3), login)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(home.join("login-complete").exists());
    assert!(submit_login_code(&pending, None, "fake-code").is_err());
}

#[tokio::test]
async fn claude_cancellation_closes_code_channel_before_return() {
    let temp = tempfile::tempdir().unwrap();
    let backend =
        claude::ClaudeBackend::new(Some(executable(temp.path(), "claude", CLAUDE_FIXTURE)));
    let mut request = request(temp.path(), "claude-home");
    let pending = request.pending_directory.clone();
    let (cancel, receiver) = watch::channel(false);
    request.cancel = Some(receiver);
    let (progress, mut events) = progress();
    let login = tokio::spawn(async move { backend.login(request, progress).await });
    assert!(matches!(
        next_progress(&mut events).await,
        LoginEvent::SignInUrl { .. }
    ));
    assert!(matches!(
        next_progress(&mut events).await,
        LoginEvent::WaitingForCode { .. }
    ));
    assert!(submit_login_code(&pending, None, "invalid\nextra").is_err());
    cancel.send(true).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), login)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(submit_login_code(&pending, None, "fake-code").is_err());
}

#[test]
fn usage_dates_validate_local_time_and_calendar_rollover() {
    let now = Utc.with_ymd_and_hms(2026, 12, 30, 19, 0, 0).unwrap();
    let quota = claude::usage::quota(
        "Unrelated output with 88% used\nCurrent session: 2.5% used · resets 7:15pm (Europe/Zurich)\nCurrent week (all models): 9% used • resets Jan 2 at 9am (Europe/Zurich)\nCurrent week (Fable): 100% used",
        now,
    ).unwrap();
    assert_eq!(quota.windows.len(), 3);
    assert_eq!(quota.windows[0].used_percent, 3);
    assert_eq!(
        quota.windows[0].resets_at,
        Utc.with_ymd_and_hms(2026, 12, 31, 18, 15, 0).single()
    );
    assert_eq!(
        quota.windows[1].resets_at,
        Utc.with_ymd_and_hms(2027, 1, 2, 8, 0, 0).single()
    );
    assert_eq!(quota.windows[2].label, "Fable");
    assert!(quota.windows[2].resets_at.is_none());
    assert!(claude::usage::quota("Unrelated output with 88% used", now).is_none());
    let date = claude::usage::reset_date("Dec 31, 2026 at 12am (UTC)", now);
    assert_eq!(date, Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).single());
    assert!(claude::usage::reset_date("Feb 31 at 9am (UTC)", now).is_none());
    assert!(claude::usage::reset_date("9:65pm", now).is_none());
}

#[test]
fn reset_grants_distinguish_zero_unknown_paused_and_expired() {
    let now = Utc.with_ymd_and_hms(2026, 11, 11, 11, 0, 0).unwrap();
    let credits = claude::reset::parse(&json!({"cedar_ember": {"eligible": true, "grants": [
        {"label": "Expired", "resets_left": 1, "ends_at": "2026-11-11T11:00:00Z", "paused": false},
        {"label": "Active", "resets_left": 2, "ends_at": "2026-11-12T11:00:00.123Z", "paused": false},
        {"label": "No expiry", "resets_left": 1, "ends_at": null, "paused": false, "usable_now": false},
        {"label": "Future", "resets_left": 3, "starts_at": "2026-11-12T19:00:00+08:00", "paused": false},
        {"label": "Paused", "resets_left": 9, "paused": true},
        {"label": "Spent", "resets_left": 0, "paused": false}
    ]}}), now).unwrap();
    assert_eq!(credits.available, 3);
    assert_eq!(credits.grants.as_ref().unwrap().len(), 6);
    assert_eq!(credits.next_expiry.unwrap().timestamp_subsec_millis(), 123);
    let empty = claude::reset::parse(
        &json!({"cedar_ember": {"eligible": true, "grants": []}}),
        now,
    );
    assert_eq!(empty.unwrap().available, 0);
    assert!(claude::reset::parse(&json!({}), now).is_none());
    assert!(
        claude::reset::parse(
            &json!({"cedar_ember":{"eligible": false, "grants":[]}}),
            now
        )
        .is_none()
    );
    for value in [json!("1"), json!(1.2), json!(-1), json!(1_000_001)] {
        let malformed = json!({"cedar_ember":{"eligible":true,"grants":[{"label":"Bad count","resets_left":value,"paused":false}]}});
        assert!(claude::reset::parse(&malformed, now).is_none());
    }
    let malformed = json!({"cedar_ember":{"eligible":true,"grants":[{"label":"Bad date","resets_left":1,"ends_at":"unknown","paused":false}]}});
    assert!(claude::reset::parse(&malformed, now).is_none());
}

#[test]
fn codex_prefers_named_limit_and_keeps_unknown_reset_outcome() {
    let quota = codex::payload::quota(&json!({
        "rateLimits": {"secondary": {"usedPercent": 1, "windowDurationMins": 10080}},
        "rateLimitsByLimitId": {
            "base_model_inference": {"primary": {"usedPercent": 13, "windowDurationMins": 10080}},
            "codex": {"primary": {"usedPercent": 100, "windowDurationMins": 10080}}
        },
        "rateLimitResetCredits": {"credits": [
            {"status": "available", "expiresAt": 1796165951},
            {"status": "available", "expiresAt": 1796070676},
            {"status": "redeemed", "expiresAt": 1700000000}
        ]}
    }))
    .unwrap();
    assert_eq!(quota.weekly().unwrap().used_percent, 100);
    let resets = quota.reset_credits.unwrap();
    assert_eq!(resets.available, 2);
    assert_eq!(resets.next_expiry.unwrap().timestamp(), 1796070676);
    assert_eq!(
        codex::payload::reset_outcome(&json!({"outcome": "future"})),
        Some(ResetOutcome::Other("future".into()))
    );
    assert_eq!(
        codex::payload::reset_outcome(&json!({"outcome": "nothingToReset"})),
        Some(ResetOutcome::NothingToReset)
    );
}

#[test]
fn claude_keychain_identity_and_ansi_url_match_provider_behavior() {
    assert_eq!(
        keychain::service_name("/Users/test/.codesubs/claude/work"),
        "Claude Code-credentials-e5ffe7f2"
    );
    assert_eq!(
        keychain::service_name("/tmp/é"),
        keychain::service_name("/tmp/e\u{0301}")
    );
    let url = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz";
    let output = format!("Visit: \x1b]8;;{url}\x07{url}\x1b]8;;\x07\nPaste code here > ");
    assert_eq!(
        claude::login::authorization_url(output.as_bytes(), false).as_deref(),
        Some(url)
    );
}

#[tokio::test]
async fn process_deadline_reaps_child_waiting_on_inherited_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("child-pid");
    let executable = executable(
        temp.path(),
        "provider-timeout",
        "#!/bin/sh\nsleep 30 &\nprintf '%s\\n' \"$$\" > \"$1\"\nwait\n",
    );
    let result = crate::runtime::run(
        &executable,
        [&marker],
        None,
        None,
        // Allow the shell to write its PID even when other test processes
        // are starting in parallel. The fixture otherwise waits 30 seconds.
        Duration::from_secs(3),
        "fixture timeout",
    )
    .await;
    assert!(matches!(result, Err(Error::Timeout(_))));
    let pid = fs::read_to_string(marker)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
