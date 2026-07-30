# Multi-stage build: Angular frontend + Rust backend in one image (the backend
# serves the bundle and the API). Mirrors the fleet's xinutec/<app>:latest
# convention — see kubes/memview/k8s.
#
# The image carries the *viewer*, never the corpus. The memory directory is a
# mount supplied at runtime, pushed up from the Mac, so a published image can
# never contain a memory. That separation is the whole reason this repo is safe
# to have public.

# --- frontend ---
FROM node:24-alpine AS frontend
WORKDIR /fe
COPY frontend/package.json frontend/package-lock.json ./
# git: the shared layout harness is a git dependency (github:xinutec/ui-harness),
# so npm ci clones it — node:alpine ships no git.
RUN apk add --no-cache git ca-certificates && npm ci
COPY frontend/ .
RUN npx ng build --configuration production

# --- backend (deps cached in their own layer) ---
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
    && cargo build --release && rm -rf src
COPY src/ src/
RUN touch src/main.rs src/lib.rs && cargo build --release

# --- runtime ---
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# 65532 is the conventional "nonroot" id, matched by k8s/01-app.yaml.
RUN groupadd --gid 65532 memview \
    && useradd --uid 65532 --gid memview --no-create-home --shell /usr/sbin/nologin memview
WORKDIR /app
COPY --from=backend /app/target/release/memview /usr/local/bin/memview
COPY --from=frontend /fe/dist/memview-web/browser ./public
ENV STATIC_DIR=/app/public \
    BIND_ADDR=0.0.0.0:8091 \
    MEMORY_DIR=/corpus \
    SHARE_STATE=/state/share-state.json
USER memview
EXPOSE 8091
CMD ["memview"]
