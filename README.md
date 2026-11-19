# Subscription Manager

A Rust library and standalone CLI for managing **Codex (ChatGPT)** and **Claude Code** subscription accounts, with a native SwiftUI menu-bar app as an optional client. Keep several accounts logged in, read their quota, and select an account for an agent without logging in again.

The Rust core owns account behavior, providers, storage, and locking. The CLI exposes those operations to agents and the macOS app through a documented JSON protocol. See [library usage](docs/library.md), [CLI commands](docs/cli.md), and [the app protocol](docs/protocol.md).

By default, each account lives in its own folder under `~/.subscription-manager`; retained legacy vaults may use `~/.quota-manager`. `subscription-manager info --json` reports the exact root. Ordinary managed-account operations leave default `~/.codex` and `~/.claude` logins alone; `add --from-default` explicitly moves a default login.

## Features

- Any number of Codex and Claude Code accounts, each permanently logged in
- Banked Codex and Claude resets shown per account; Codex resets can be applied from the account's **⋯** menu
- Weekly quota, 5-hour session quota and per-model weekly limits, with reset times
- Green, orange, and red quota bars
- Automatic refresh every 15 minutes, or on demand
- A shared credentials folder plus a `subscription-manager` command for agents
- Native translucent SwiftUI menu-bar window
- Optional launch at login
- No analytics and no external service operated by this project

## Requirements

- macOS 13 or newer (optimized for macOS Tahoe), Apple Silicon
- To build: Rust 1.89+ with Cargo; Swift 5.9+ through Apple Command Line Tools for the app
- For Codex accounts: the Codex CLI (`brew install --cask codex`)
- For Claude Code accounts: Claude Code (`brew install --cask claude-code`) and a Claude Pro, Max, Team or Enterprise subscription

The provider CLIs are found through the process `PATH` and common install folders. For custom locations (including a Finder-launched app), set an absolute `SUBSCRIPTION_MANAGER_CODEX_EXECUTABLE` or `SUBSCRIPTION_MANAGER_CLAUDE_EXECUTABLE` path.

## Install

```sh
./scripts/install.sh
```

The script builds and ad-hoc signs the app and bundled Rust helper, installs the app as `~/Applications/Subscription Manager.app`, installs a separate command-line binary at `~/.local/bin/subscription-manager`, and launches the app.

For the CLI alone, without the app:

```sh
./scripts/install-cli.sh
```

Look for the white Codex icon in the menu bar.

Upgrading from **Quota Manager** or **Codex Quota**: the installer quits and removes the old app and its `quota-manager` command. Existing accounts stay logged in; the core preserves current account paths and handles older layouts as follows:

- An existing Quota Manager `~/.quota-manager` vault is opened in place when the new default vault does not exist, preserving exact Claude Keychain paths. `subscription-manager info --json` reports the selected root.
- Codex Quota's profiles in `~/Library/Application Support/CodexQuota` move into `~/.subscription-manager/codex/`, named after their email addresses.

Re-enable "Launch at login" from the menu afterwards, because the app has a new identity.

## Adding accounts

Accounts are added by an agent, not from the app. Ask your agent to add one; it follows the `addGuidePath` reported by `subscription-manager info --json`, which boils down to:

```sh
subscription-manager add claude --email someone@example.com   # prints a sign-in URL
subscription-manager add-code '<code>'                         # only if Claude shows a code
```

Open the URL in a browser logged into that email (or a private window) and approve. The command signs in inside a new folder named after the email (for example `claude/someone-example`), refuses to save a login for any other email, and registers the account; the running app shows it within seconds. With `--from-default` it instead moves the login that plain `codex` or `claude` uses into Subscription Manager.

Both CLIs refresh their tokens automatically. A new browser login is only needed if a provider revokes the session; then click **Sign in** on the account row. It signs in again inside the same folder, so the account keeps its name and agents keep working.

## Banked resets

Codex occasionally grants free rate-limit resets. Each Codex row shows how many are banked. To spend one, open the row's **⋯** menu and choose **Apply reset…**, then confirm. If no current limit is eligible, Codex reports it and no credit is used. Agents can do the same with `subscription-manager reset codex/<slug>` (shows the count) and `--yes` (applies one, only after the user agrees).

Claude also grants saved limit resets. Each Claude row shows the available count, with individual grant descriptions and expiration dates in its **⋯** menu. Grants can cover different limits: a full reset and a model-specific reset are not interchangeable. Expired, future, spent, and paused grants are excluded from the count. Choose **Open Claude usage…**, sign into the matching email, and use **Settings > Usage** to redeem one. The app only reads the balance and never spends a Claude reset. If the balance cannot be read, it shows **Resets unavailable**, rather than reporting zero.

## Switching accounts from agents

Every account is a ready-to-use home folder:

```text
~/.subscription-manager/
  accounts.json          accounts, remaining quota, folder and env var for each
  README.md              these rules, for agents
  codex/<slug>/          use as CODEX_HOME
  claude/<slug>/         use as CLAUDE_CONFIG_DIR
```

Point an agent at an account with one environment variable:

```sh
CODEX_HOME="$(subscription-manager path codex/work)" codex exec "…"
CLAUDE_CONFIG_DIR="$(subscription-manager path claude/personal)" claude -p "…"
```

Or use the `subscription-manager` command:

```sh
subscription-manager list                       # accounts and remaining quota
subscription-manager list --json                # cached snapshot, for scripts
subscription-manager refresh --json             # live quota, works with the app closed
subscription-manager rename claude/work personal
subscription-manager remove claude/personal --yes
subscription-manager pick claude                # the Claude account with the most quota left
subscription-manager env claude/personal        # shell lines that select an account (fish or sh)
subscription-manager exec codex/work -- codex exec "fix the tests"
subscription-manager exec "$(subscription-manager pick codex --min 20)" -- codex exec "…"
eval "$(subscription-manager env claude/personal)"     # sh/zsh
subscription-manager env claude/personal | source      # fish
```

Accounts can be named as `<provider>/<slug>`, a unique slug, or an email address. Quota figures come from the latest successful refresh by any client. The app refreshes every 15 minutes while running; the standalone `refresh` command works without it. `pick` retains legacy stale fallback and treats elapsed reset times as projected full availability; use `--max-age 900` or `--refresh` to require fresh observations. Selection does not reserve quota.

### Rules that keep logins working

- **Use the exact path.** Claude Code stores each folder's login in the macOS Keychain under a key derived from the literal `CLAUDE_CONFIG_DIR` string. Use the absolute path from `accounts.json` or `subscription-manager path`, with no trailing slash and no symlinked alias; any other spelling looks logged out. `~` and `$HOME` are fine in a shell because they expand to the same string.
- **Never copy credentials.** Both providers rotate refresh tokens on use, so a copied `auth.json` or Keychain entry goes stale and can sign the original out. Always point at the folder in place.
- **Clear overriding variables.** `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` take priority over the folder's login. `subscription-manager env` and `exec` clear them for you.
- **Remove accounts through the app or `subscription-manager remove account --yes`**, not with `logout` inside a folder.
- Claude Code logins live in the Keychain, so `claude/<slug>` folders only work on this Mac. For machines without the Keychain (CI, containers), use `claude setup-token` instead.

### Shared settings

Your normal settings follow you into every account. The first time each account folder is prepared, Subscription Manager symlinks these items when they exist:

- from `~/.claude`: `settings.json`, `CLAUDE.md`, `agents`, `commands`, `hooks`, `output-styles`, `plugins`, `skills`, `keybindings.json`
- from `~/.codex`: `AGENTS.md`, `prompts`, `rules`, `skills`, `hooks.json`

Delete a link to give that account its own copy; it is not recreated. Credentials, history and per-account state (`.claude.json`, `auth.json`, sessions) are never shared. Codex `config.toml` is per account (Subscription Manager pins `cli_auth_credentials_store = "file"` there), so copy any MCP servers or model settings you want into it.

## Privacy

`~/.subscription-manager` and every account folder are readable only by your macOS user (`0700`). The folders contain credentials and must never be shared or committed. Removing an account logs it out, deletes its Keychain entry (Claude Code), and deletes its folder.

The app runs the locally installed `codex` and `claude` CLIs, which talk to OpenAI's and Anthropic's normal authentication and usage services. Reading Claude usage runs the local `/usage` command, which makes no model request and costs nothing. To read Claude reset grants, the app reads that account folder's current access token from the macOS Keychain into memory and makes a read-only request to Anthropic's usage endpoint. It never copies or stores the token or refreshes it itself. Nothing is sent to a developer-owned backend.

## Development

```sh
./scripts/test.sh                       # offline Rust + Swift tests
cargo test --workspace                  # library and CLI tests
./apps/macos/test.sh                    # app protocol/process tests (CLT compatible)
./scripts/build-cli.sh                  # target/release/subscription-manager
./scripts/build-app.sh                  # dist/Subscription Manager.app
./scripts/integration-test.sh --live    # opt-in live provider refresh
```

Set `SUBSCRIPTION_MANAGER_HOME` or pass CLI `--home` to use a scratch vault; scratch stores never adopt legacy real accounts. Use `SUBSCRIPTION_MANAGER_CLI` to point a development Swift app at an absolute Rust executable path. Stop old app/CLI versions before upgrading: old binaries do not participate in the Rust repository locking protocol.

```text
crates/core/   reusable Rust library, providers, persistence
crates/cli/    standalone commands and machine output
apps/macos/   SwiftUI presentation and asynchronous CLI client
fixtures/     shared protocol/provider samples
docs/         library, CLI, and protocol documentation
```

## Implementation notes

- Codex accounts use the structured local `codex app-server` methods for login, identity, and rate limits. The app-server is marked experimental, so a future Codex release may need a compatibility update.
- Claude Code accounts use `claude auth login`, `claude auth status --json`, and `claude -p /usage`. The `/usage` text is parsed strictly; if its format changes, the account shows an error instead of a wrong number, and `./scripts/integration-test.sh --live` checks live refresh behavior.
- Claude reset grants use the installed CLI's `GET /api/oauth/usage?cedar_ember=1&skip_spend=1` request. This is an undocumented provider endpoint; unavailable or unrecognized responses leave the reset balance unknown without blocking ordinary quota refresh. The CLI remains responsible for token refresh. `./scripts/integration-test.sh --live claude/<slug>` checks stored account quota and reset balances without opening a login or claiming a reset.
