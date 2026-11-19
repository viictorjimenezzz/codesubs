#!/bin/bash
set -euo pipefail

package_dir="$(cd "$(dirname "$0")" && pwd)"
project_dir="$(cd "$package_dir/../.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/subscription-manager-swift.XXXXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT

swift build --package-path "$package_dir"
swiftc -parse-as-library -o "$scratch/subscription-client-checks" \
    "$package_dir/Sources/SubscriptionManagerApp/Models/SubscriptionModels.swift" \
    "$package_dir/Sources/SubscriptionManagerApp/Services/CLIProcess.swift" \
    "$package_dir/Sources/SubscriptionManagerApp/Services/SubscriptionClient.swift" \
    "$package_dir/Tests/SubscriptionManagerAppTests/SubscriptionClientChecks.swift"

"$scratch/subscription-client-checks" "$project_dir/fixtures/protocol/account-list.json"
