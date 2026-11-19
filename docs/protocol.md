# CLI protocol v1

The macOS app launches its bundled `Contents/Helpers/codesubs` directly. Scripts may use the independently installed binary. Arguments are passed as an array; no shell is required. `--home PATH` or `SUBSCRIPTION_MANAGER_HOME` selects a scratch/alternate vault.

Commands accept `--json --protocol-version 1`. Unknown protocol versions fail. Successful JSON commands emit one JSON value on stdout. Errors emit `{ "protocolVersion": 1, "error": { "code": "...", "message": "..." } }` on stderr with a nonzero status. Diagnostics never mix into JSON stdout. `exec` is an exception: its streams and status belong to the child.

- `list [codex|claude] --json`: preserves the existing top-level array. Each entry contains `id`, `provider`, `slug`, optional `email`, `plan`, `quota`, `lastUpdated`, plus `reference`, exact `home`, and `env`. Available percentage fields are derived when quota exists. Times are ISO-8601. Snapshots are cached observations; elapsed reset projections are not newly observed quota.
- `info --json`: `{protocolVersion, version, root, guidePath, addGuidePath}`.
- `refresh [account] --json`: `{protocolVersion, accounts: [...], failures: [{reference, code, message}]}`. Partial account failures are returned with exit 0; callers must inspect `failures`. Command-level failures use stderr and a nonzero exit.
- `pick provider [--min N] [--max-age SECONDS] [--refresh] --json`: `{protocolVersion, account, availablePercent, freshness, projected}`. Without `--json`, prints only `provider/slug`. Explicit max age requires a fresh observation; the legacy default permits stale fallback.
- `env account --json`: `{protocolVersion, home, set: {...}, unset: [...]}`. Without JSON, `--sh` or `--fish` selects executable shell lines.
- `path account --json`: `{protocolVersion, home}`.
- `rename account slug --json`: `{protocolVersion, account}`.
- `remove account --yes --json`: `{protocolVersion, removed: "provider/slug"}`.
- `reset account --json`: reset preview. `--yes` explicitly redeems a supported reset and returns `{protocolVersion, outcome, message, account, idempotencyKey}`. `--idempotency-key UUID` allows retrying the same redemption after an uncertain result.
- `doctor --json`: versioned vault/provider diagnostics; never initiates login or redeems resets.

## Login events

`add provider --email EMAIL --json-events` and `reconnect account --json-events` keep running until login completes. They emit newline-delimited JSON:

```json
{"protocolVersion":1,"type":"login","event":"login","loginID":"...","authURL":"https://...","browserOpened":true,"acceptsCode":true,"reference":"claude/work"}
{"protocolVersion":1,"type":"waitingForCode","event":"waitingForCode","slug":"work","loginID":"...","reference":"claude/work"}
{"protocolVersion":1,"type":"completed","event":"complete","account":{}}
```

The `account` example above is abbreviated; successful results contain the full account DTO. `reference` may be null during new-account onboarding. A terminal event with `type: "error"` and `code`/`message` accompanies errors on stderr. Login URLs are sensitive session data; do not persist event logs.

Submit a fallback code to the live login with `add-code --stdin --slug work --json`, passing the code on stdin. The positional code form remains compatible with existing scripts. Do not treat `loginID` as a durable/resumable session handle. SIGINT/SIGTERM cancel an owned login and allow rollback before exit.

## Exit statuses

- `0`: command success (inspect refresh failures for partial success).
- `1`: operation/provider/storage failure.
- `2`: invalid arguments, account reference, or confirmation missing.
- `3`: no eligible account.
- `127`: executable could not be launched/found.
- `130`: cancelled login.

`exec` replaces the CLI on Unix and preserves the child's exit/signal behavior. Output schemas are distinct from the on-disk manifest and provider wire formats; additions should remain backwards compatible within v1.
