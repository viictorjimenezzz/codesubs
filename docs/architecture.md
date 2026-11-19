# Codesubs architecture

```text
SwiftUI app ──subprocess + JSON──┐
                              ├── codesubs CLI ──calls── Rust library
Agents/scripts ──commands──────┘                           ↑
                                                 Rust consumers
```

The repository has two Rust crates in one Cargo workspace and a separate Swift package:

```text
crates/core/
  src/manager.rs       public Codesubs API
  src/models/          accounts, quotas, sign-in and reset values
  src/services/        account operations, refresh, selection, resets
  src/providers/       Codex app-server and Claude CLI integrations
  src/storage/         version 3 manifest, account folders, locks, operation journals
  src/runtime/         subprocesses, deadlines, environment, executable discovery
  src/platform/        macOS Keychain
  tests/               account lifecycle and concurrent-writer checks
crates/cli/
  src/commands/        argument-to-library adapters
  src/output/          JSON, human output and error mapping
  src/shell.rs         shell quoting and process replacement
  tests/               executable and protocol checks
apps/macos/
  Sources/CodesubsApp/ SwiftUI views, view model, async CLI client
  Tests/               shared-fixture and fake-process checks
fixtures/protocol/     shared wire examples
```

`codesubs-core` exposes `Codesubs` and its public value types. The library owns all account behavior; it does not print, parse arguments, open UI, exit, or change the host process's environment. Provider and clock injection support offline tests.

`codesubs-cli` owns parsing, formatting, exit codes and child execution. The SwiftUI app invokes its bundled `Contents/Helpers/codesubs` asynchronously, decodes JSON and presents results. App scheduling, browser presentation, clipboard and confirmations stay in Swift.

The default vault is `~/.codesubs`. An explicit CLI `--home` takes precedence over `CODESUBS_HOME`, the current `~/.config/codesubs/config.json` user setting, and the default, in that order. Only manifest version 3 and JSON protocol version 1 are supported.

Repository updates use stable interprocess locks and atomic manifest replacement. Per-account locks serialize managed operations; stale refreshes cannot restore deleted or renamed accounts. Operation journals protect current add/reconnect/rename/remove transactions and make interrupted operations recoverable. These are ordinary operation safeguards, not format conversion machinery.

Claude credential identity is tied to the exact account path. Folder renames stage and verify the corresponding Keychain entry before committing and cleaning up. Provider CLIs own refresh tokens. Codex helpers are short-lived to avoid stale credentials. Reset retries reuse an idempotency key.

The supported platform is macOS. There is no daemon, FFI layer, or extra SDK wrapper. Other Rust programs import the library; other languages use the documented CLI.
