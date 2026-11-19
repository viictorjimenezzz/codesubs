#!/bin/zsh
set -euo pipefail

project_dir="${0:A:h:h}"
app_name="Subscription Manager"
source_app="$project_dir/dist/$app_name.app"
install_dir="$HOME/Applications"
installed_app="$install_dir/$app_name.app"
installed_executable="$installed_app/Contents/MacOS/subscription-manager-app"
previous_executable="$installed_app/Contents/MacOS/subscription-manager"
cli_dir="$HOME/.local/bin"
cli_link="$cli_dir/subscription-manager"

source "$project_dir/scripts/install-common.sh"

"$project_dir/scripts/build-app.sh"
require_idle_cli "$cli_link"
mkdir -p "$install_dir"

# Retire earlier versions: Codex Quota 1.x and Quota Manager 2.x. Their
# existing account paths are preserved by the Rust core.
legacy_app="$install_dir/Codex Quota.app"
quit_app "com.viictorjimenezzz.codex-quota" "$legacy_app/Contents/MacOS/CodexQuota"
rm -rf "$legacy_app"
previous_app="$install_dir/Quota Manager.app"
quit_app "com.viictorjimenezzz.quota-manager" "$previous_app/Contents/MacOS/quota-manager"
rm -rf "$previous_app"
if [[ -L "$cli_dir/quota-manager" ]]; then rm "$cli_dir/quota-manager"; fi

quit_app "com.viictorjimenezzz.subscription-manager" "$previous_executable"
quit_app "com.viictorjimenezzz.subscription-manager" "$installed_executable"
rm -rf "$installed_app"
cp -R "$source_app" "$installed_app"

mkdir -p "$cli_dir"
if [[ -L "$cli_link" ]]; then rm "$cli_link"; fi
install -m 755 "$source_app/Contents/Helpers/subscription-manager" "$cli_link"

open "$installed_app"

echo "Installed $app_name in $installed_app"
echo "Standalone command-line tool installed at $cli_link"
echo "Existing accounts and logins were preserved."
