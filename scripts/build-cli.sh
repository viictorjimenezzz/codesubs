#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cargo build --manifest-path "$project_dir/Cargo.toml" --release --locked -p subscription-cli --message-format=json-render-diagnostics | python3 -c '
import json, sys
executable = None
for line in sys.stdin:
    event = json.loads(line)
    if event.get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "subscription-manager":
        executable = event.get("executable") or executable
if not executable:
    raise SystemExit("Cargo did not report a subscription-manager executable")
print(executable)
'
