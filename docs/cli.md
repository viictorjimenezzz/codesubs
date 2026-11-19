# Standalone CLI

Build with `./scripts/build-cli.sh` and install with `./scripts/install-cli.sh`. Cargo and a Rust toolchain are required to build; the resulting binary does not require the Swift app. Ensure `~/.local/bin` is on PATH.

```sh
subscription-manager list --json
subscription-manager refresh --json
subscription-manager pick claude --min 20 --max-age 900
subscription-manager pick codex --refresh --min 20
subscription-manager env claude/work --sh
subscription-manager exec codex/work -- codex exec 'fix the tests'
subscription-manager add claude --email someone@example.com
subscription-manager add-code --stdin --slug work
subscription-manager reconnect claude/work --json-events
subscription-manager rename claude/work personal
subscription-manager remove claude/personal --yes
subscription-manager reset codex/work
subscription-manager reset codex/work --yes
subscription-manager doctor --json
subscription-manager info --json
```

Accounts resolve by `provider/slug`, unique slug, or email. Refresh works with the app closed. `list` is cached; default `pick` retains legacy stale fallback, while `--max-age` and `--refresh` require fresh data. Choosing an account does not reserve quota.

All supported writers share interprocess locks. Stop older app/CLI versions before writer cutover. Provider sessions started outside the manager do not participate in its locks. Preserve exact paths and never copy provider credentials. Moving the default login requires explicitly passing `add --from-default`; it changes which login the ordinary provider command uses.

For programmatic consumers see [protocol.md](protocol.md). Use `--home /absolute/scratch/path` to test independently of real credentials. Offline tests use scratch stores/fake providers; live provider checks are explicitly opt-in.
