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
# pnpm-workspace.yaml belongs in this layer, not with the sources: it carries the
# install-script allowlist, and without it esbuild never unpacks its binary and
# the build below fails on a dependency that looks installed.
COPY frontend/package.json frontend/pnpm-lock.yaml frontend/pnpm-workspace.yaml ./
# git: the shared layout harness is a git dependency (github:xinutec/ui-harness),
# so the install clones it — node:alpine ships no git.
#
# pnpm is taken unpinned. The host gets its copy from the flake, and pinning a
# second version here would be two numbers held level by hand; the lockfile is
# the thing that has to match, and --frozen-lockfile fails rather than drift.
RUN apk add --no-cache git ca-certificates \
    && npm install -g pnpm \
    && pnpm install --frozen-lockfile
COPY frontend/ .
# The viewer's bundle only — NOT `pnpm run build`, which would also build the
# console. Nothing that can drive Claude Code belongs in an image that runs on
# an internet-facing host. See docs/agent-console.md.
RUN pnpm exec ng build --configuration production

# --- backend (deps cached in their own layer) ---
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# The console's MANIFEST, and never its code. Cargo cannot load a workspace
# unless every member's Cargo.toml exists, so leaving this out fails the build in
# under a second — the trap recorded in reference_cargo_workspace_docker_priming
# when coach grew a second crate. A stub source is enough to prime the cache, and
# `--bin memview` means the console is never compiled, let alone shipped.
COPY console/Cargo.toml console/
RUN mkdir -p src console/src \
    && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
    && echo '' > console/src/lib.rs \
    && cargo build --release --bin memview && rm -rf src
COPY src/ src/
# --bin memview, never a bare build: the workspace also holds the console, which
# runs Claude Code subprocesses on the Mac and must never be inside an image
# that runs on an internet-facing host. See docs/agent-console.md.
# The console keeps its stub source from the layer above rather than getting the
# real one: a manifest with no target at all fails to parse, and copying the code
# in would put a way to run Claude Code inside the image. `touch` so the viewer's
# real sources are newer than the primed artefacts and actually rebuild.
RUN touch src/main.rs src/lib.rs && cargo build --release --bin memview

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
