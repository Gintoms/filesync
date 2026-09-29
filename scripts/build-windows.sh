#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
npm --prefix ui ci --no-fund --no-audit
npm --prefix ui run build
sh scripts/cargo-windows.sh build --locked --release --target x86_64-pc-windows-msvc
node scripts/prepare-desktop.mjs x86_64-pc-windows-msvc
export FILESYNC_MANIFEST="$project_dir/ui/src-tauri/Cargo.toml"
export PATH="${FILESYNC_LLVM_BIN:-/usr/local/opt/llvm@20/bin}:${FILESYNC_LLD_BIN:-/usr/local/opt/lld@20/bin}:$PATH"
cd ui
exec npx tauri build --runner "$project_dir/scripts/cargo-windows.sh" --target x86_64-pc-windows-msvc --bundles nsis "$@"
