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
codesubs api add melious --key-stdin < key.txt
codesubs api list --json
codesubs api check [account] --json
codesubs api set-models melious/default --big glm-5.3 --small glm-5.3-flash
codesubs api remove melious/default --yes
codesubs exec melious/default -- claude
codesubs exec melious/default --agent codex -- codex
codesubs doctor --json
codesubs info --json
```

API accounts use `service/slug` (slug defaults to `default`) and resolve in `env`, `path` and `exec` after subscription accounts. They need an agent: `--agent claude|codex`, or the command's name. `env` prints the key, so use it only to `eval` into a shell you trust. Subscription `pick`, `refresh` and the quota table ignore them.

Accounts resolve by `provider/slug`, unique slug, or email. Refresh works with the app closed. `list` is cached; default `pick` allows cached fallback, while `--max-age` and `--refresh` require fresh data. Choosing an account does not reserve quota.

All supported writers share interprocess locks. Provider sessions started outside the manager do not participate in its locks. Preserve exact paths and never copy provider credentials. Moving the default login requires explicitly passing `add --from-default`; it changes which login the ordinary provider command uses.

For programmatic consumers see [protocol.md](protocol.md). Use `--home /absolute/scratch/path` to test independently of real credentials. Offline tests use scratch stores/fake providers; live provider checks are explicitly opt-in.
