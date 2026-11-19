# Proposal: Rust library and CLI, with a SwiftUI app

Status: implemented, 2026-11-18. The Rust core, standalone CLI, and SwiftUI subprocess client now live in this repository. The design map below records the agreed boundaries; see [library.md](library.md), [cli.md](cli.md), and [protocol.md](protocol.md) for current usage.

## Architecture

All subscription behavior belongs in a reusable Rust library. A thin Rust CLI calls that library. The SwiftUI app invokes the CLI asynchronously using a documented JSON protocol. Agents use the same CLI; Rust consumers can import the library directly.

```text
SwiftUI app ──subprocess + JSON──┐
                              ├── Rust CLI ──calls── Rust core library
Agents/scripts ──commands──────┘                         ↑
                                               Other Rust consumers
```

Start with two crates in one Cargo workspace. Providers, storage, and platform adapters are internal library modules initially. Workspaces support shared builds, dependencies, and a root lockfile: [Cargo workspace reference](https://doc.rust-lang.org/cargo/reference/workspaces.html).

Keep macOS support initially: the existing Claude authentication integration depends on macOS Keychain. Isolate it behind a platform adapter; additional platforms require implementation and validation.

## Proposed layout

```text
codesubs/
├── Cargo.toml                         # Workspace members: crates/core, crates/cli
├── Cargo.lock
├── crates/
│   ├── core/                          # Package: codesubs-core
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                 # Supported public exports
│   │   │   ├── manager.rs             # Public Codesubs API
│   │   │   ├── config.rs              # Explicit vault root/runtime options
│   │   │   ├── error.rs               # Typed domain/operation errors
│   │   │   ├── models/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── account.rs
│   │   │   │   ├── quota.rs
│   │   │   │   ├── login.rs
│   │   │   │   └── reset.rs
│   │   │   ├── services/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── accounts.rs         # Add, reconnect, rename, remove
│   │   │   │   ├── refresh.rs
│   │   │   │   ├── selection.rs        # Resolve, rank, check freshness
│   │   │   │   └── resets.rs           # Preview, redeem, retry idempotently
│   │   │   ├── providers/
│   │   │   │   ├── mod.rs              # Provider interface and capabilities
│   │   │   │   ├── codex/
│   │   │   │   │   ├── mod.rs
│   │   │   │   │   ├── app_server.rs
│   │   │   │   │   └── parser.rs
│   │   │   │   └── claude/
│   │   │   │       ├── mod.rs
│   │   │   │       ├── client.rs
│   │   │   │       ├── usage.rs
│   │   │   │       └── resets.rs
│   │   │   ├── storage/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── repository.rs       # Locked read-modify-write transactions
│   │   │   │   ├── manifest.rs         # Versioned on-disk codec
│   │   │   │   ├── homes.rs            # Exact paths, verified moves, rollback
│   │   │   │   ├── locks.rs            # Cross-process/account coordination
│   │   │   │   ├── migrations.rs
│   │   │   │   └── shared_config.rs
│   │   │   ├── runtime/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── process.rs          # Execution, deadlines, cancellation
│   │   │   │   ├── executables.rs      # Locate codex/claude
│   │   │   │   └── environment.rs      # Home selection and conflicting vars
│   │   │   └── platform/
│   │   │       ├── mod.rs
│   │   │       └── macos/
│   │   │           ├── mod.rs
│   │   │           └── keychain.rs
│   │   ├── tests/                     # Repository/lifecycle/provider tests
│   │   └── examples/basic.rs           # Library consumer without CLI/UI
│   └── cli/                           # Package: codesubs-cli
│       ├── Cargo.toml                 # Binary: codesubs
│       ├── src/
│       │   ├── main.rs                # Parse → call core → render → exit
│       │   ├── args.rs
│       │   ├── commands/              # list/add/add-code/refresh/pick/env/exec/
│       │   │                         # rename/remove/reset/doctor
│       │   ├── output/
│       │   │   ├── mod.rs
│       │   │   ├── json.rs            # Explicit wire DTOs/versioning
│       │   │   ├── table.rs
│       │   │   └── errors.rs          # Error and exit-code mapping
│       │   └── shell.rs               # Shell quoting and process replacement
│       └── tests/                     # CLI subprocess/JSON/exit-code tests
├── apps/
│   └── macos/
│       ├── Package.swift
│       ├── Sources/CodesubsApp/
│       │   ├── CodesubsApp.swift
│       │   ├── Models/                # Decodable CLI response types
│       │   ├── Services/
│       │   │   └── SubscriptionClient.swift  # Async Process wrapper
│       │   ├── ViewModels/
│       │   │   └── AccountViewModel.swift
│       │   ├── Views/
│       │   └── Platform/              # Browser, pasteboard, launch at login
│       ├── Resources/                 # App metadata, icons, artwork
│       └── Tests/                     # Protocol decoding and fake responses
├── fixtures/                          # Sanitized provider/manifest/wire samples
├── docs/
│   ├── architecture-proposal.md
│   ├── cli.md
│   ├── protocol.md
│   └── library.md
└── scripts/
    ├── build-cli.sh
    ├── build-app.sh                   # Bundles matching Rust CLI
    ├── install-cli.sh                 # Independent of app installation
    ├── install.sh
    ├── test.sh                        # Offline Rust + Swift checks
    └── integration-test.sh            # Opt-in live provider checks
```

This is a destination map; create files as implementations are ported. `lib.rs`, `main.rs`, and crate-local `tests/` follow [Cargo's standard layout](https://doc.rust-lang.org/cargo/guide/project-layout.html).

## Ownership boundaries

| Component | Responsibility |
| --- | --- |
| `codesubs-core` | Account lifecycle, providers, authentication workflows, quotas, selection, credential relocation, storage, locking |
| `codesubs-cli` | Argument parsing, human/JSON output, shell rendering, exit codes, child-command execution |
| SwiftUI app | Presentation, refresh scheduling, notifications, browser/pasteboard, confirmations; invokes CLI |

`manager.rs` is the supported Rust API. Keep implementation modules private by default, re-export required values/options, and provide injection seams for providers, repository, credential operations, and time. The library never parses command-line arguments, prints to stdout, exits, opens UI, or changes the caller's global environment. Return typed results, progress events, and execution contexts instead.

The CLI calls public library operations. Its command handlers contain no provider requests, selection algorithms, manifest editing, or Keychain logic. `exec` applies the library's environment additions/removals to the child process.

Provider capabilities distinguish reading Claude reset balances from redeeming Codex resets. Preserve unknown-versus-zero balances, provider-owned token refresh, fresh short-lived helpers, and reset idempotency across retries.

## Swift app and CLI contract

`SubscriptionClient.swift` is the app's only subscription backend. For example, it invokes `codesubs refresh claude/work --json`, decodes the result, and updates the view model. Swift does not read `accounts.json`, manage credentials, or reproduce account-selection rules.

- Bundle the matching CLI at `Contents/Helpers/codesubs` and invoke that absolute path. Permit an explicit development override. The standalone CLI remains independently installable.
- Use argument arrays, asynchronous execution, concurrent stdout/stderr draining, deadlines, and cancellation. Do not invoke shell command strings.
- Preserve existing `list --json` array output initially. Add an explicit protocol version for app/new-command responses before changing shapes. Shared fixtures verify Rust encoders and Swift decoders.
- JSON stdout contains documented machine output only; diagnostics go to stderr. Include stable error codes, observation times, freshness, and partial refresh failures. `exec` passes through the child's output/status.
- The existing “Copy environment” action calls `env`; shell quoting remains in the CLI.
- Keep onboarding in a long-running originating CLI process. When requested, emit JSON Lines login progress and completion events. `add-code` routes a code to that live process through local IPC. Filter provider output out of the machine event stream; a killed login process is not a resumable session.
- Reload state using `list` on menu activation and after mutations. Keep recurring refresh in the app. Add polling if external-agent changes must appear while the menu remains open.
- Confirmation belongs to clients; reset redemption, account removal, and default-login adoption remain explicit operations.

No daemon or Rust/Swift FFI is required initially. Future service/MCP adapters can consume the Rust API without implementing account behavior again.

## Storage and concurrent agents

Retain `~/.subscription-manager`, account IDs/references, provider home paths, and `SUBSCRIPTION_MANAGER_HOME`. Source migration must not relocate credentials. Claude Keychain identity depends on the exact configuration-directory string.

The current app and CLI perform whole-list saves and onboarding retries. Atomic replacement does not protect the read-modify-write transaction. The Rust repository must coordinate writers before it controls the real vault:

- Lock a stable file outside replaceable manifests, reload under lock, and apply narrow updates. Do not hold the manifest lock during browser/network waits.
- Coordinate manager-owned operations per account across processes. Reject refresh commits made stale by removal/rename. Reserve pending slugs and recheck identity uniqueness on commit.
- Stage, verify, commit, and recover folder/Keychain moves. Keep the old login until verification and manifest commit succeed; exclude secrets from recovery metadata.
- Reject corrupt/unsupported manifests rather than treating them as empty. Introduce revision/recovery metadata through a versioned migration.
- Stop the old app before enabling Rust writes. Existing old binaries do not participate in the new locking protocol.

Locks cannot coordinate raw provider commands outside the manager. Selection remains advisory and does not reserve quota. Distinguish availability projected from elapsed reset times from newly observed provider data.

## Migration map and order

| Existing Swift code | Destination |
| --- | --- |
| `QuotaModels.swift` | Rust models; retain logos and UI runtime state in Swift |
| `QuotaBackend.swift` | Rust provider interface and runtime executable lookup |
| `CodexAppServerClient.swift`, Codex payload parser | `providers/codex/` |
| `ClaudeCLIClient.swift`, `ClaudeUsageParser.swift`, `ClaudeResetClient.swift` | `providers/claude/`; split runtime process and macOS Keychain adapters |
| `Vault.swift` | `storage/`; core-owned agent guide templates updated for standalone CLI |
| `AccountOnboarding.swift` | Core lifecycle/login services; CLI arguments and rendering |
| `AccountStore.swift` | Core services plus thin Swift `AccountViewModel` |
| `CommandLineTool.swift` | Rust commands; selection/resolution in core |
| App entry, views, resources | `apps/macos/`; remove combined CLI/app dispatch |
| Existing tests | Rust behavior fixtures, CLI subprocess tests, Swift protocol fixtures |

1. Add the workspace, core models/parsers, and port behavior fixtures. Keep the existing working app while these pieces remain read-only.
2. Port storage, providers, lifecycle operations, and coordination. Use scratch stores/fake providers to verify concurrent updates, rename rollback, cancellation, and stale-result rejection.
3. Complete the Rust CLI with existing commands and standalone refresh/rename/remove/doctor. Verify JSON/exit contracts and independent installation against scratch stores.
4. Move SwiftUI into `apps/macos`, replace app business logic with the CLI client/view model, bundle Rust, and perform the writer cutover. Preserve credential paths. Relocate the Swift package manifest and remove obsolete Swift logic once the new app builds.

Completion means account operations work with the app closed, the app uses those operations, and another Rust crate can import the library without the CLI. Default tests stay offline; live provider checks remain opt-in.

## Baseline validation

The original Swift baseline passed its 22 self-tests before migration. The new implementation is verified by `./scripts/test.sh` (Rust provider/lifecycle/CLI tests and compiler-only Swift protocol/process checks) and `./scripts/build-app.sh`. Tests use scratch stores and fake providers. Live provider endpoints and real Keychain mutations have not been exercised.
