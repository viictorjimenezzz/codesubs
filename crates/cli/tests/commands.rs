use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn run(root: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_subscription-manager"))
        .env("SUBSCRIPTION_MANAGER_HOME", root.path())
        .args(args)
        .output()
        .expect("run CLI")
}
fn seed(root: &TempDir) {
    fs::write(root.path().join("accounts.json"),serde_json::to_vec(&json!({
        "version":2,"accounts":[{
            "id":"0A07E658-1BDE-4EE0-B802-AE0AEA9B0B7F","provider":"claude","slug":"work",
            "email":"me@example.com","plan":"max","lastUpdated":"2026-11-18T10:00:00Z",
            "quota":{"windows":[{"kind":"weekly","label":"Week","usedPercent":12,"resetsAt":"2099-01-01T00:00:00Z","windowDurationMinutes":10080}]}
        }]
    })).unwrap()).unwrap();
}
#[test]
fn empty_store_works_without_app_and_info_is_versioned() {
    let root = TempDir::new().unwrap();
    let output = run(&root, &["list", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([])
    );
    let output = run(&root, &["info", "--json"]);
    let info: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["protocolVersion"], 1);
    assert_eq!(info["root"], root.path().to_str().unwrap());
}
#[test]
fn list_preserves_array_contract_and_adds_full_app_snapshot() {
    let root = TempDir::new().unwrap();
    seed(&root);
    let output = run(&root, &["list", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let accounts: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(accounts.is_array());
    assert_eq!(accounts[0]["reference"], "claude/work");
    assert_eq!(accounts[0]["availablePercent"], 88);
    assert_eq!(accounts[0]["quota"]["windows"][0]["usedPercent"], 12);
    assert_eq!(
        accounts[0]["env"]["CLAUDE_CONFIG_DIR"],
        root.path().join("claude/work").to_str().unwrap()
    );
    assert!(accounts[0]["id"].is_string());
}
#[test]
fn exec_clears_conflicting_auth_and_preserves_exit_status() {
    let root = TempDir::new().unwrap();
    seed(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_subscription-manager"))
        .env("SUBSCRIPTION_MANAGER_HOME", root.path())
        .env("ANTHROPIC_API_KEY", "must-be-removed")
        .args([
            "exec",
            "claude/work",
            "--",
            "/bin/sh",
            "-c",
            "test -z \"${ANTHROPIC_API_KEY+x}\" && printf '%s' \"$CLAUDE_CONFIG_DIR\"; exit 42",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        root.path().join("claude/work").to_str().unwrap()
    );
}
#[test]
fn malformed_manifest_is_not_replaced() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("accounts.json"), "not json").unwrap();
    let output = run(&root, &["list", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "corrupt_manifest");
    assert_eq!(
        fs::read_to_string(root.path().join("accounts.json")).unwrap(),
        "not json"
    );
}
#[test]
fn unknown_commands_fail_and_never_launch_ui() {
    let root = TempDir::new().unwrap();
    let output = run(&root, &["nonsense", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "invalid_input");
}
#[test]
fn strict_pick_rejects_stale_cache_and_invalid_thresholds() {
    let root = TempDir::new().unwrap();
    seed(&root);
    let output = run(&root, &["pick", "claude", "--max-age", "0", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(&root, &["pick", "claude", "--min", "101", "--json"]);
    assert_eq!(output.status.code(), Some(2));
}
#[test]
fn remove_requires_explicit_confirmation() {
    let root = TempDir::new().unwrap();
    seed(&root);
    let output = run(&root, &["remove", "claude/work", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let output = run(&root, &["list", "--json"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn env_shell_output_quotes_paths_and_removes_overrides() {
    let root = TempDir::with_prefix("it's-a-test").unwrap();
    seed(&root);
    let output = run(&root, &["env", "claude/work", "--sh"]);
    assert!(output.status.success());
    let env = String::from_utf8(output.stdout).unwrap();
    assert!(env.contains("'\\''"));
    assert!(env.contains("unset CLAUDE_CODE_OAUTH_TOKEN ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN"));
}

#[test]
fn explicit_home_never_adopts_legacy_accounts() {
    let user = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let legacy = user.path().join("Library/Application Support/CodexQuota");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("accounts.json"),
        "this invalid legacy file must not be read",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_subscription-manager"))
        .env_remove("SUBSCRIPTION_MANAGER_HOME")
        .env("HOME", user.path())
        .args(["--home", scratch.path().to_str().unwrap(), "list", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([])
    );
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "this invalid legacy file must not be read"
    );
}

#[test]
fn shared_swift_fixture_round_trips_and_expired_reset_totals_update_without_refresh() {
    let root = TempDir::new().unwrap();
    let mut accounts: Value =
        serde_json::from_str(include_str!("../../../fixtures/protocol/account-list.json")).unwrap();
    accounts[0]["quota"]["resetCredits"]["grants"][0]["expiresAt"] = json!("2020-01-01T00:00:00Z");
    accounts[0]["home"] = json!(root.path().join("claude/work"));
    accounts[0]["env"]["CLAUDE_CONFIG_DIR"] = json!(root.path().join("claude/work"));
    fs::write(
        root.path().join("accounts.json"),
        serde_json::to_vec(&json!({"version":2,"accounts":accounts})).unwrap(),
    )
    .unwrap();
    let before = fs::read(root.path().join("accounts.json")).unwrap();
    let output = run(&root, &["list", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(emitted[0]["availablePercent"], 72);
    assert_eq!(emitted[0]["quota"]["resetCredits"]["available"], 0);
    assert!(emitted[0]["quota"]["resetCredits"]["nextExpiry"].is_null());
    assert_eq!(emitted[1]["quota"]["credits"]["balance"], 8.5);
    assert_eq!(
        fs::read(root.path().join("accounts.json")).unwrap(),
        before,
        "read-only output must not rewrite cached observations"
    );
}

#[cfg(unix)]
#[test]
fn standalone_codex_lifecycle_uses_one_core_without_the_app() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let vault = root.path().join("vault");
    let executable = root.path().join("fake-codex");
    fs::write(
        &executable,
        include_str!("../../core/tests/provider_fixtures/codex-app-server.sh"),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let call = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_subscription-manager"))
            .env("SUBSCRIPTION_MANAGER_HOME", &vault)
            .env("HOME", root.path().join("user-home"))
            .env("SUBSCRIPTION_MANAGER_CODEX_EXECUTABLE", &executable)
            .env(
                "SUBSCRIPTION_MANAGER_CLAUDE_EXECUTABLE",
                root.path().join("missing-claude"),
            )
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let added = call(&[
        "add",
        "codex",
        "--email",
        "codex@example.test",
        "--slug",
        "work",
        "--json",
    ]);
    assert_eq!(added["account"]["reference"], "codex/work");
    let id = added["account"]["id"].clone();
    let refreshed = call(&["refresh", "codex/work", "--json"]);
    assert!(refreshed["failures"].as_array().unwrap().is_empty());
    assert_eq!(refreshed["accounts"][0]["availablePercent"], 33);
    let picked = call(&["pick", "codex", "--refresh", "--min", "20", "--json"]);
    assert_eq!(picked["account"]["id"], id);
    let renamed = call(&["rename", "codex/work", "personal", "--json"]);
    assert_eq!(renamed["account"]["id"], id);
    assert!(!vault.join("codex/work").exists());
    assert!(vault.join("codex/personal").is_dir());
    let receipt = call(&[
        "reset",
        "codex/personal",
        "--yes",
        "--idempotency-key",
        "12345678-1234-1234-1234-123456789012",
        "--json",
    ]);
    assert_eq!(receipt["outcome"], "alreadyRedeemed");
    let removed = call(&["remove", "codex/personal", "--yes", "--json"]);
    assert_eq!(removed["removed"], "codex/personal");
    assert!(!vault.join("codex/personal").exists());
    assert_eq!(call(&["list", "--json"]), json!([]));
}
