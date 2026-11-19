#!/bin/zsh
# Explicit live quota read: provider CLIs may refresh their own tokens.
# Never starts a login, logs out, or redeems resets.
set -euo pipefail
project_dir="${0:A:h:h}"
if [[ "${1:-}" != "--live" ]]; then
  echo 'Usage: ./scripts/integration-test.sh --live [provider/slug ...]' >&2
  echo 'This reads installed provider CLIs and refreshes stored account quota.' >&2
  exit 2
fi
shift
cli="$("$project_dir/scripts/build-cli.sh" dev)"
if (( $# == 0 )); then
  "$cli" refresh --json | python3 -c 'import json,sys; r=json.load(sys.stdin); print(json.dumps(r,indent=2)); sys.exit(bool(r["failures"]))'
else
  for reference in "$@"; do
    "$cli" refresh "$reference" --json | python3 -c 'import json,sys; r=json.load(sys.stdin); print(json.dumps(r,indent=2)); sys.exit(bool(r["failures"]))'
  done
fi
