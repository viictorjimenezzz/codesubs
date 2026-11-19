#!/bin/zsh
set -euo pipefail

project_dir="${0:A:h:h}"
app_name="Codesubs"
source_app="$project_dir/dist/$app_name.app"
install_dir="$HOME/Applications"
installed_app="$install_dir/$app_name.app"
installed_executable="$installed_app/Contents/MacOS/codesubs-app"
cli_dir="$HOME/.local/bin"
cli_path="$cli_dir/codesubs"

source "$project_dir/scripts/install-common.sh"

"$project_dir/scripts/build-app.sh"
require_idle_cli "$cli_path"
mkdir -p "$install_dir"

quit_app com.viictorjimenezzz.codesubs "$installed_executable"

mkdir -p "$cli_dir"
if [[ -L "$cli_path" ]]; then rm "$cli_path"; fi
install -m 755 "$source_app/Contents/Helpers/codesubs" "$cli_path"

rm -rf "$installed_app"
cp -R "$source_app" "$installed_app"

open "$installed_app"

echo "Installed $app_name in $installed_app"
echo "Standalone command-line tool installed at $cli_path"
