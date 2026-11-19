#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cli_dir="${CODESUBS_BIN_DIR:-$HOME/.local/bin}"
cli_path="$cli_dir/codesubs"
cli_binary="$("$project_dir/scripts/build-cli.sh")"
source "$project_dir/scripts/install-common.sh"
require_idle_cli "$cli_path"
mkdir -p "$cli_dir"
if [[ -L "$cli_path" ]]; then rm "$cli_path"; fi
install -m 755 "$cli_binary" "$cli_path"
printf 'Installed CLI at %s\n' "$cli_path"
