#!/bin/sh
# Tauri runner and standalone Cargo runner for the Windows MSVC target.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export CARGO_HOME="${FILESYNC_CARGO_HOME:-$project_dir/.cache/cargo}"
export RUSTUP_TOOLCHAIN=1.98.1
export XWIN_CACHE_DIR="$project_dir/.cache/xwin"
export XWIN_ARCH=x86_64
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"
# Keep the fully static CRT selected above. Tauri's alternative mode excludes
# libucrt and conflicts with cargo-xwin's +crt-static linker arguments.
export STATIC_VCRUNTIME=false
export PATH="${FILESYNC_LLVM_BIN:-/usr/local/opt/llvm@20/bin}:${FILESYNC_LLD_BIN:-/usr/local/opt/lld@20/bin}:$project_dir/.cache/windows-tools/bin:$PATH"
build_command=${1:-build}
if [ "$#" -gt 0 ]; then shift; fi
scratch_dir=$(mktemp -d /tmp/filesync-windows.XXXXXX)
trap 'rmdir "$scratch_dir"' EXIT
cd "$scratch_dir"
cargo-xwin "$build_command" --manifest-path "${FILESYNC_MANIFEST:-$project_dir/Cargo.toml}" "$@"
