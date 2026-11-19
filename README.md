# Codesubs

A Rust library and CLI for managing **Codex** and **Claude Code** subscription accounts, with an optional native macOS menu-bar app.

Keep several accounts signed in, read their quotas, and run an agent using a selected account. The Rust core owns account operations, provider integrations, and storage. The app calls the same CLI that agents use.

## Install

Requires macOS 13+, Rust 1.89+ with Cargo, and Swift 5.9+ through Apple Command Line Tools to build the app. Install the provider CLIs you use: Codex and/or Claude Code.

```sh
./scripts/install-cli.sh   # standalone codesubs command
./scripts/install.sh       # Codesubs.app and the CLI
```

The CLI installs to `~/.local/bin/codesubs`; add `~/.local/bin` to your terminal's PATH. The app installs to `~/Applications/Codesubs.app`.

## Commands

```sh
codesubs add claude --email you@example.com
codesubs list
codesubs refresh --json
codesubs pick claude --refresh --min 20
codesubs exec codex/work -- codex exec 'fix the tests'
codesubs exec claude/personal -- claude -p 'review this code'
codesubs rename claude/work personal
codesubs remove claude/personal --yes
codesubs --help
```

Account references are `provider/slug`, a unique slug, or an email. `exec` selects the account's environment and clears conflicting authentication variables. `env account --sh` or `--fish` prints shell setup instead.

`list` reads cached quota. `refresh` fetches current data without requiring the app. `pick` prefers recent observations; `--max-age SECONDS` or `--refresh` requires fresh data. Elapsed reset times produce an availability estimate, and selection does not reserve quota.

## Account storage

```text
~/.codesubs/
  accounts.json
  README.md
  ADD.md
  codex/<slug>/
  claude/<slug>/
```

Set `CODESUBS_HOME` or pass `--home PATH` to choose a different vault. A persistent user setting is `~/.config/codesubs/config.json` containing `{"vaultRoot":"/absolute/path"}`. `codesubs info --json` reports its exact location. The current manifest format is version 3; other versions are rejected.

Credentials stay in their account folders and macOS Keychain. Use the exact path from `codesubs path account`; Claude Keychain entries depend on that string. Never copy credentials or log out from inside an account folder. Remove accounts with Codesubs instead. Provider CLIs own token refresh.

Normal managed-account operations leave default `~/.codex` and `~/.claude` logins alone. `add --from-default` explicitly moves a default login and requires stopping any provider sessions using it first.

Account folders link shared settings, skills, and instructions from your default provider folders. Removing a link opts that account out; credentials, history, and sessions are not shared. Codex `config.toml` is per account.

## Sign-in and resets

`add` prints a sign-in URL and verifies the expected email before saving an account. If Claude shows an authorization code, send it to the running login with `codesubs add-code --stdin --slug NAME`. `reconnect account` signs in again inside the existing account folder.

`reset account` previews banked resets. `reset account --yes` explicitly redeems a Codex reset; reuse `--idempotency-key UUID` to retry an uncertain request. Claude reset grants are read-only in Codesubs; redeem them in Claude Settings > Usage. Unknown balances stay unknown rather than appearing as zero.

## Configuration

- `CODESUBS_HOME`: account vault.
- `CODESUBS_CLI`: absolute CLI path for the development app or verification script.
- `CODESUBS_CODEX_EXECUTABLE` / `CODESUBS_CLAUDE_EXECUTABLE`: absolute provider CLI paths.
- `CODESUBS_BIN_DIR`: standalone CLI installation directory.

Provider discovery otherwise uses the process PATH and standard installation folders. The app bundles its own matching CLI.

## Development

```sh
./scripts/test.sh                       # offline Rust and Swift checks
cargo test --workspace
./apps/macos/test.sh
./scripts/build-cli.sh
./scripts/build-app.sh                  # dist/Codesubs.app
./scripts/integration-test.sh --live    # explicit live provider refresh
```

Tests use scratch vaults and fake providers. Live integration reads can refresh provider tokens; they do not start sign-in or redeem resets.

```text
crates/core/   reusable Rust library, providers, storage, process handling
crates/cli/    commands, JSON output, shell integration
apps/macos/   SwiftUI app and asynchronous CLI client
fixtures/     shared protocol examples
```

See [library usage](docs/library.md), [CLI commands](docs/cli.md), [the JSON protocol](docs/protocol.md), and [architecture](docs/architecture.md).

The project operates no backend or analytics service. Codex uses its local app-server protocol; Claude usage is parsed from the installed CLI. The optional Claude reset probe reads the current token into memory for a provider request and never stores or refreshes it. Changes to provider protocols may require adapter updates.
