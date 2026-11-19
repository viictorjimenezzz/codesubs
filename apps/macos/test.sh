#!/bin/bash
set -euo pipefail

package_dir="$(cd "$(dirname "$0")" && pwd)"
project_dir="$(cd "$package_dir/../.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/codesubs-swift.XXXXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT

swift build --package-path "$package_dir"
swiftc -parse-as-library -o "$scratch/subscription-client-checks" \
    "$package_dir/Sources/CodesubsApp/Models/SubscriptionModels.swift" \
    "$package_dir/Sources/CodesubsApp/Services/CLIProcess.swift" \
    "$package_dir/Sources/CodesubsApp/Services/SubscriptionClient.swift" \
    "$package_dir/Tests/CodesubsAppTests/SubscriptionClientChecks.swift"

"$scratch/subscription-client-checks" "$project_dir/fixtures/protocol/account-list.json"
