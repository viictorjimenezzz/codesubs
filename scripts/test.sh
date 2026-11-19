#!/bin/zsh
set -euo pipefail
project_dir="${0:A:h:h}"
cargo test --manifest-path "$project_dir/Cargo.toml" --locked --workspace
"$project_dir/apps/macos/test.sh"
