#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export FILESYNC_MANIFEST="$project_dir/ui/src-tauri/Cargo.toml"
exec sh "$project_dir/scripts/cargo-local.sh" "$@"
