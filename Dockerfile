FROM node:24-bookworm-slim AS web
WORKDIR /build/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/index.html ui/tsconfig.json ui/vite.config.ts ./
COPY ui/src ./src
RUN npm run build

FROM rust:1.98.1-bookworm AS backend
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=web /build/ui/dist ./ui/dist
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN groupadd --gid 10001 filesync \
    && useradd --uid 10001 --gid filesync --no-create-home filesync \
    && mkdir -p /data /etc/filesync \
    && chown filesync:filesync /data
COPY --from=backend /build/target/release/filesync /usr/local/bin/filesync
COPY packaging/docker/hub.toml /etc/filesync/hub.toml
USER 10001:10001
EXPOSE 8787
VOLUME ["/data"]
# The hub handles SIGINT for graceful shutdown.
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/filesync"]
CMD ["--config", "/etc/filesync/hub.toml", "hub"]
