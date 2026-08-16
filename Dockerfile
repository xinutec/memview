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
# Stamp the build into the bundle (frontend/scripts/stamp-version.mjs), so the
# page can say which build it is. The context has no .git, so the commit comes
# from GIT_SHA — passed by CI, and 'dev' for a plain local build.
ARG GIT_SHA=dev
RUN GIT_SHA="$GIT_SHA" node scripts/stamp-version.mjs
# The viewer's bundle only — NOT `pnpm run build`, which would also build the
# console. Nothing that can drive Claude Code belongs in an image that runs on
# an internet-facing host. See docs/agent-console.md. ⚠ `ng build` directly means
# npm's prebuild hook does not run, which is why the stamp is its own step above.
RUN pnpm exec ng build --configuration production

# --- backend (deps cached in their own layer) ---
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# ⚠ **EVERY workspace member's MANIFEST, or nothing loads.** Cargo reads all of
# them before it compiles anything, so a missing one fails the build in 0.1s with
# an error naming the workspace rather than the file — the trap recorded in
# reference_cargo_workspace_docker_priming when coach grew a second crate. This
# list is not decoration: it has to gain a line every time `members` in
# Cargo.toml does. It did not when `reader` arrived on 2026-08-07, and the image
# job was red for 21 runs while `verify` stayed green, because the gate does not
# build the image and nothing else looks.
#
# ⚠ **It happened again on 2026-08-16, with `bash-oracle`.** Same failure, same
# invisibility: the gate passed, the image job went red on push. The list is now
# checked rather than remembered — `scripts/workspace-members.sh`, a gate row —
# because a comment saying "add a line here" has now failed twice.
COPY console/Cargo.toml console/
COPY reader/Cargo.toml reader/
COPY bash-oracle/Cargo.toml bash-oracle/
# The console's stub needs a main.rs as well as a lib.rs: its manifest names a
# `default-run`, and a manifest whose default target does not exist fails to
# parse — before any of this compiles, and with an error that names the manifest
# rather than the missing file. `reader` declares no targets at all, so an empty
# lib.rs is the whole of it and its five src/bin/* are simply not discovered.
RUN mkdir -p src console/src reader/src bash-oracle/src \
    && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
    && echo '' > console/src/lib.rs && echo 'fn main() {}' > console/src/main.rs \
    && echo '' > reader/src/lib.rs \
    && echo '' > bash-oracle/src/lib.rs \
    && cargo build --release --bin memview && rm -rf src
COPY src/ src/
# ⚠ **reader's REAL source, unlike the console's.** The viewer links it
# (`reader = { path = "reader" }`), so the stub above would compile an empty
# crate and every use of it would fail to resolve. The console is the opposite
# case and stays a stub on purpose — see below. Only `src/`: the tests, examples
# and their tree-sitter dev-dependency are measurement kept beside the grammars,
# and an image must not carry a C toolchain to hold them.
COPY reader/src/ reader/src/
# --bin memview, never a bare build: the workspace also holds the console, which
# runs Claude Code subprocesses on the Mac and must never be inside an image
# that runs on an internet-facing host. See docs/agent-console.md.
# The console keeps its stub source from the layer above rather than getting the
# real one: a manifest with no target at all fails to parse, and copying the code
# in would put a way to run Claude Code inside the image. `touch` so the viewer's
# real sources are newer than the primed artefacts and actually rebuild — and
# reader's with them, or the primed empty crate is what gets linked.
RUN touch src/main.rs src/lib.rs reader/src/lib.rs \
    && cargo build --release --bin memview

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
