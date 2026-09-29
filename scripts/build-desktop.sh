#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" = Darwin ]; then
  exec sh "$project_dir/scripts/build-macos.sh" "$@"
fi
cd "$project_dir"
npm --prefix ui ci --no-fund --no-audit
npm --prefix ui run build
sh scripts/cargo-local.sh build --release --locked
node scripts/prepare-desktop.mjs
cd ui
exec npx tauri build --runner "$project_dir/scripts/cargo-desktop.sh" "$@"
