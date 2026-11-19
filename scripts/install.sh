#!/bin/zsh
set -euo pipefail

project_dir="${0:A:h:h}"
app_name="Codesubs"
source_app="$project_dir/dist/$app_name.app"
install_dir="$HOME/Applications"
installed_app="$install_dir/$app_name.app"
installed_executable="$installed_app/Contents/MacOS/codesubs-app"
retired_app="$install_dir/Subscription Manager.app"
cli_dir="$HOME/.local/bin"
cli_path="$cli_dir/codesubs"
previous_cli="$cli_dir/subscription-manager"

source "$project_dir/scripts/install-common.sh"

"$project_dir/scripts/build-app.sh"
require_idle_cli "$previous_cli"
require_idle_cli "$cli_path"
mkdir -p "$install_dir"

# Old Swift app versions must stop before Codesubs can use the same vault.
for app_root in "$install_dir" /Applications; do
  quit_app com.viictorjimenezzz.codex-quota "$app_root/Codex Quota.app/Contents/MacOS/CodexQuota"
  quit_app com.viictorjimenezzz.quota-manager "$app_root/Quota Manager.app/Contents/MacOS/quota-manager"
  quit_app com.viictorjimenezzz.subscription-manager "$app_root/Subscription Manager.app/Contents/MacOS/subscription-manager"
  quit_app com.viictorjimenezzz.subscription-manager "$app_root/Subscription Manager.app/Contents/MacOS/subscription-manager-app"
done
quit_app com.viictorjimenezzz.codesubs "$installed_executable"

mkdir -p "$cli_dir"
if [[ -L "$cli_path" ]]; then rm "$cli_path"; fi
install -m 755 "$source_app/Contents/Helpers/codesubs" "$cli_path"
rm -f "$previous_cli"

rm -rf "$installed_app"
cp -R "$source_app" "$installed_app"
rm -rf "$retired_app" "$install_dir/Quota Manager.app" "$install_dir/Codex Quota.app"
if [[ -L "$cli_dir/quota-manager" ]]; then rm "$cli_dir/quota-manager"; fi

open "$installed_app"

echo "Installed $app_name in $installed_app"
echo "Standalone command-line tool installed at $cli_path"
echo "Existing accounts and logins were preserved."
