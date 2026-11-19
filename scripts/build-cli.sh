#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
# Live integration checks use the dev profile; packaged installs use release.
profile="${1:-release}"
cargo build --manifest-path "$project_dir/Cargo.toml" --profile "$profile" --locked -p codesubs-cli --bin codesubs --message-format=json-render-diagnostics | python3 -c '
import json, sys
executable = None
for line in sys.stdin:
    event = json.loads(line)
    if event.get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "codesubs":
        executable = event.get("executable") or executable
if not executable:
    raise SystemExit("Cargo did not report a codesubs executable")
print(executable)
'
