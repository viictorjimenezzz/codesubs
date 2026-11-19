# Standalone CLI

Build with `./scripts/build-cli.sh` and install with `./scripts/install-cli.sh`. Cargo and a Rust toolchain are required to build; the resulting binary does not require the Swift app. Ensure `~/.local/bin` is on PATH.

```sh
codesubs list --json
codesubs refresh --json
codesubs pick claude --min 20 --max-age 900
codesubs pick codex --refresh --min 20
codesubs env claude/work --sh
codesubs exec codex/work -- codex exec 'fix the tests'
codesubs add claude --email someone@example.com
codesubs add-code --stdin --slug work
codesubs reconnect claude/work --json-events
codesubs rename claude/work personal
codesubs remove claude/personal --yes
codesubs reset codex/work
codesubs reset codex/work --yes
codesubs doctor --json
codesubs info --json
```

Accounts resolve by `provider/slug`, unique slug, or email. Refresh works with the app closed. `list` is cached; default `pick` allows cached fallback, while `--max-age` and `--refresh` require fresh data. Choosing an account does not reserve quota.

All supported writers share interprocess locks. Provider sessions started outside the manager do not participate in its locks. Preserve exact paths and never copy provider credentials. Moving the default login requires explicitly passing `add --from-default`; it changes which login the ordinary provider command uses.

For programmatic consumers see [protocol.md](protocol.md). Use `--home /absolute/scratch/path` to test independently of real credentials. Offline tests use scratch stores/fake providers; live provider checks are explicitly opt-in.
