#!/bin/sh
# Download dependencies on the host when containers cannot access registries.
# The Rust and Debian base images must already be loaded into Docker.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
image=${1:-filesync:0.1.0}
mkdir -p .local
docker image inspect rust:1.98.1-bookworm debian:bookworm-slim > /dev/null
npm --prefix ui ci
npm --prefix ui run build
sh scripts/cargo-local.sh vendor --locked "$project_dir/.local/docker-vendor" > .local/docker-vendor-config.toml
python3 - <<'PY'
from pathlib import Path
import shutil

root = Path.cwd()
context = root / '.local/docker-offline'
context.mkdir(parents=True, exist_ok=True)
for name in ['Cargo.toml', 'Cargo.lock', 'build.rs']:
    shutil.copy2(root / name, context / name)
for name in ['src', 'ui/dist', 'packaging/docker']:
    dest = context / name
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(root / name, dest)
vendor = context / 'vendor'
if vendor.exists():
    shutil.rmtree(vendor)
shutil.copytree(root / '.local/docker-vendor', vendor)
(context / '.cargo').mkdir(exist_ok=True)
(context / '.cargo/config.toml').write_text(
    '[source.crates-io]\nreplace-with = "vendored-sources"\n'
    '[source.vendored-sources]\ndirectory = "/build/vendor"\n'
)
dockerfile = (root / 'Dockerfile').read_text().split('FROM rust:', 1)[1]
dockerfile = 'FROM rust:' + dockerfile
dockerfile = dockerfile.replace('COPY --from=web /build/ui/dist ./ui/dist',
    'COPY ui/dist ./ui/dist\nCOPY vendor ./vendor\nCOPY .cargo ./.cargo')
dockerfile = dockerfile.replace('cargo build --release --locked',
    'cargo build --release --locked --offline')
(context / 'Dockerfile').write_text(dockerfile)
PY
docker build --network none --platform linux/amd64 -t "$image" .local/docker-offline
