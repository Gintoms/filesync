#!/bin/sh
# Keep dependencies and registry settings isolated from an obsolete global mirror.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_command=${1:-build}
if [ "$#" -gt 0 ]; then shift; fi
export CARGO_HOME="${FILESYNC_CARGO_HOME:-$project_dir/.cache/cargo}"
scratch_dir=$(mktemp -d /tmp/filesync-cargo.XXXXXX)
trap 'rmdir "$scratch_dir"' EXIT
cd "$scratch_dir"
cargo +1.98.1 "$build_command" --manifest-path "${FILESYNC_MANIFEST:-$project_dir/Cargo.toml}" "$@"
