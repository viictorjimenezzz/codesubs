# Rust library

`crates/core` is the reusable `codesubs-core` package. It does not depend on the CLI or Swift app. Another local Rust package can add:

```toml
[dependencies]
codesubs-core = { path = "/absolute/path/to/codesubs/crates/core" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use codesubs_core::{ManagerConfig, Codesubs};

#[tokio::main]
async fn main() -> codesubs_core::Result<()> {
    let manager = Codesubs::new(ManagerConfig::from_environment()?)?;
    for account in manager.list().await? {
        println!("{}", account.reference());
    }
    let report = manager.refresh(Some("claude/work")).await?;
    for failure in report.failures {
        eprintln!("{}: {}", failure.reference, failure.message);
    }
    Ok(())
}
```

Use `ManagerConfig::new(vault_root, user_home)` to isolate a test or embedding instead of relying on process environment. Operations return typed `Error` values with stable `code()` values; they do not print, exit, or present UI. The caller owns scheduling and confirmation.

Public manager operations cover `list`, `resolve`, `pick`, `env`, `refresh`, `refresh_provider`, `add`, `reconnect`, `rename`, `remove`, `preview_reset`, `redeem_reset`, `submit_code`, and `doctor`. `env` returns exact home and environment additions/removals for a child process without changing the host's environment. Account selection is advisory; no quota is reserved.

Login operations accept a `LoginProgress` callback and optional cancellation receiver. Provider helpers are short-lived except while an interactive login is active. Consumers must retain the operation until cancellation cleanup completes. Use `--idempotency-key` at the CLI or the corresponding UUID argument in the library when retrying an uncertain reset.

`Codesubs::with_backends` supports injected provider implementations for offline testing. Default tests use temporary stores and fake processes; real credential-provider access is not required. Domain structures, manifest serialization, and the CLI wire schema are separate concerns: consumers should use public Rust values rather than depending on the manifest's internal metadata.

The current supported platform is macOS. Provider credential handling is behind platform adapters; the core owning a Rust implementation does not automatically make existing macOS Keychain logins usable on other machines.
