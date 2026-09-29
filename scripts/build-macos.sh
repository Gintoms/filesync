#!/bin/sh
# Build both executables as Universal 2 binaries inside one macOS application.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" != Darwin ]; then
  echo 'The macOS universal build requires macOS and Xcode Command Line Tools.' >&2
  exit 1
fi
for arg in "$@"; do
  case "$arg" in
    --target|--target=*|-t)
      echo 'This script always builds universal-apple-darwin; omit --target.' >&2
      exit 1
      ;;
  esac
done
cd "$project_dir"
rustup target add --toolchain 1.98.1 aarch64-apple-darwin x86_64-apple-darwin
npm --prefix ui ci --no-fund --no-audit
npm --prefix ui run build
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  sh scripts/cargo-local.sh build --release --locked --target "$target"
  node scripts/prepare-desktop.mjs "$target"
done
build_root=${CARGO_TARGET_DIR:-$project_dir/target}
mkdir -p "$build_root/universal-apple-darwin/release"
xcrun lipo -create \
  "$build_root/aarch64-apple-darwin/release/filesync" \
  "$build_root/x86_64-apple-darwin/release/filesync" \
  -output "$build_root/universal-apple-darwin/release/filesync"
xcrun lipo "$build_root/universal-apple-darwin/release/filesync" -verify_arch arm64 x86_64
node scripts/prepare-desktop.mjs universal-apple-darwin
if [ "$#" -eq 0 ]; then set -- --bundles app; fi
cd ui
exec npx tauri build --runner "$project_dir/scripts/cargo-desktop.sh" --target universal-apple-darwin "$@"
