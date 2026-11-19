#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
app_dir="$project_dir/apps/macos"
app_name="Codesubs"
executable_name="codesubs-app"
bundle_dir="$project_dir/dist/$app_name.app"
cli_helper="$bundle_dir/Contents/Helpers/codesubs"
icon_source="$app_dir/Resources/Assets/app-icon.png"
icon_work_dir="$(mktemp -d /tmp/codesubs-icon.XXXXXX)"
iconset_dir="$icon_work_dir/AppIcon.iconset"
trap 'rm -rf "$icon_work_dir"' EXIT

cli_binary="$("$project_dir/scripts/build-cli.sh")"
swift build --package-path "$app_dir" -c release
swift_bin_dir="$(swift build --package-path "$app_dir" -c release --show-bin-path)"
rm -rf "$bundle_dir"
mkdir -p "$bundle_dir/Contents/MacOS" "$bundle_dir/Contents/Helpers" "$bundle_dir/Contents/Resources"
cp "$swift_bin_dir/$executable_name" "$bundle_dir/Contents/MacOS/$executable_name"
cp "$cli_binary" "$cli_helper"
cp "$app_dir/Resources/Info.plist" "$bundle_dir/Contents/Info.plist"
cli_version="$("$cli_binary" version)"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $cli_version" "$bundle_dir/Contents/Info.plist"
cp -R "$app_dir/Resources/Assets" "$bundle_dir/Contents/Resources/Assets"

mkdir -p "$iconset_dir"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$icon_source" --out "$iconset_dir/icon_${size}x${size}.png" >/dev/null
  double_size=$((size * 2))
  sips -z "$double_size" "$double_size" "$icon_source" --out "$iconset_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset_dir" -o "$bundle_dir/Contents/Resources/AppIcon.icns"
chmod +x "$bundle_dir/Contents/MacOS/$executable_name" "$cli_helper"
codesign --force --sign - "$cli_helper"
codesign --force --deep --sign - "$bundle_dir"
printf '%s\n' "$bundle_dir"
