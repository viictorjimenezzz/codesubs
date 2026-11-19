#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cli_dir="${SUBSCRIPTION_MANAGER_BIN_DIR:-$HOME/.local/bin}"
cli_path="$cli_dir/codesubs"
previous_cli="$cli_dir/subscription-manager"
cli_binary="$("$project_dir/scripts/build-cli.sh")"
source "$project_dir/scripts/install-common.sh"
require_idle_cli "$previous_cli"
require_idle_cli "$cli_path"
for app_root in "$HOME/Applications" /Applications; do
  quit_app com.viictorjimenezzz.subscription-manager "$app_root/Subscription Manager.app/Contents/MacOS/subscription-manager"
  quit_app com.viictorjimenezzz.codex-quota "$app_root/Codex Quota.app/Contents/MacOS/CodexQuota"
  quit_app com.viictorjimenezzz.quota-manager "$app_root/Quota Manager.app/Contents/MacOS/quota-manager"
done
mkdir -p "$cli_dir"
if [[ -L "$cli_path" ]]; then rm "$cli_path"; fi
install -m 755 "$cli_binary" "$cli_path"
rm -f "$previous_cli"
printf 'Installed CLI at %s\n' "$cli_path"
