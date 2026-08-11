#!/usr/bin/env bash
# The agent console as a launchd service. Entry point for org.xinutec.agent-console.
#
# `console.sh` is still the way to run one by hand. This is the supervised copy,
# and it differs in three ways that all follow from launchd's rules.
#
# **1. Nothing here may touch nix.** `nix-shell` hangs under launchd — a gotcha
# already paid for by the fleetwatch agents, which fold their interpreter in as a
# pinned env for exactly this reason. So this runs an already-built binary and
# never builds one. `console-upgrade.sh` is where building happens, from a
# terminal, where a dev shell works.
#
# **2. It runs an INSTALLED binary, not the cargo target.** Pointing a service at
# `~/Library/Caches/cargo/target/debug/console` makes `cargo clean` an outage.
# The upgrade script installs to $CONSOLE_BIN by atomic rename, which is also what
# makes an in-place upgrade safe: the running process keeps its open inode while
# the path gains new contents, and re-execs into them on SIGUSR2.
#
# **3. It `exec`s, so the job's pid IS the console.** That is the whole point.
# `Roster::handover` replaces the image with `execve` — same pid, so the `claude`
# children stay children and their pipes survive — and because the pid never
# changes, launchd does not notice an upgrade happened and its supervision is
# never racing the handover. Were this to stay in the shell as a parent, launchd
# would be watching bash, SIGUSR2 would land on the wrong process, and a crash of
# the console would look healthy.
#
# ⚠ **The tunnel is taken down by launchd, not by a trap.** `console.sh` traps EXIT
# to kill it, which cannot work here: `exec` destroys the shell that would run the
# trap. Instead the tunnel is left in this job's process group, and launchd's
# default (`AbandonProcessGroup` unset, i.e. false) kills what remains of the group
# when the job dies. That preserves the rule console.sh states — a standing tunnel
# to a console that is not running is a listening port on isis with nothing behind
# it — and it survives the console being killed in ways a trap would not.
#
# ⚠ **Restart with SIGUSR2, NEVER `launchctl kickstart -k`.** kickstart sends
# SIGTERM, which is the console's *stop* path and deliberately takes every session
# with it. Use `console-upgrade.sh`.
#
# **4. It is run from its INSTALLED copy, not from the checkout.**
# `console-upgrade.sh` puts this file and `console-tunnel.sh` in
# `~/.local/libexec` beside the binary, and the launchd job names that copy. The
# reason is a macOS rule measured 2026-08-09, when ~/Code became symlinks to an
# external volume: under launchd, **Apple's own binaries are refused the
# volume** — `/bin/sh` exec'ing a script there, and `/bin/bash` merely reading
# one, both die with `Operation not permitted` and exit 126 — while nix-store
# builds and anything in `~/.local/libexec` read and run the same paths fine.
# So the old `/bin/bash <checkout>/scripts/console-service.sh` would not have
# survived a restart, and would have failed with an empty log, because the place
# launchd writes the error is on the volume too.
#
# Nothing here should reintroduce a dependency on where the checkout is. The one
# path that still points into it — `$STATIC_DIR`, the built frontend — is opened
# by the console binary, which is not an Apple binary and is therefore allowed.
set -euo pipefail

# Where this script's siblings are, which is the install directory under launchd
# and `scripts/` when run by hand. Never `..` from here: the installed copy has
# no repo above it.
HERE="$(cd "$(dirname "$0")" && pwd)"

CONSOLE_BIN="${CONSOLE_BIN:-$HOME/.local/libexec/agent-console}"
DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"
REPO="${CONSOLE_REPO:-$HOME/Code/memview}"

if [ ! -x "$CONSOLE_BIN" ]; then
  echo "no console binary at $CONSOLE_BIN — run scripts/console-upgrade.sh once" >&2
  exit 78 # EX_CONFIG: a configuration problem, not a crash to be retried hard.
fi

if [ -s "$DIR/server.key" ] && [ -s "$DIR/clients" ]; then
  PINS="$(grep -v '^[[:space:]]*\(#.*\)\?$' "$DIR/clients" | paste -sd, -)"
  echo "gate: $(printf '%s\n' "$PINS" | tr ',' '\n' | wc -l | tr -d ' ') pinned client key(s)"
  export CONSOLE_TLS_CERT="$DIR/server.crt"
  export CONSOLE_TLS_KEY="$DIR/server.key"
  export CONSOLE_CLIENT_KEYS="$PINS"
  export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8097}"
  # Backgrounded and NOT waited on: see the process-group note above. It has its
  # own redial loop, so a dropped tunnel does not need this script's help.
  "$HERE/console-tunnel.sh" &
else
  echo "gate: not configured — loopback only (scripts/console-identity.sh sets it up)"
fi

export CONSOLE_DIRS="${CONSOLE_DIRS:-$HOME/Code}"
# Served from console-live rather than the build output: `ng build` deletes its
# whole output path, so a page loading during a build would ask for a font and get
# HTML. See console.sh. Only `pnpm run publish:console` writes here — a plain
# build stops at the output path, so an ablation cannot reach this directory.
export STATIC_DIR="${STATIC_DIR:-$REPO/frontend/dist/console-live}"

exec "$CONSOLE_BIN"
