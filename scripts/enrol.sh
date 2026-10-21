#!/usr/bin/env bash
# Enrol a phone: make it generate a key, check what it says about that key, and
# pin it.
#
#   nix develop ~/Code/recall#android --command ./scripts/enrol.sh [<ip[:port]>]
#
# Over a cable or local adb, as part of the check: no certificate can prove the
# phone that made a chain is the one in your hand.
#
# `attest` (console/src/attest.rs) checks what the phone claims; on any failure
# nothing is pinned.
set -euo pipefail
cd "$(dirname "$0")/.."

PACKAGE="org.xinutec.console"
DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"
STATUS_URL="https://android.googleapis.com/attestation/status"
ADB="${ANDROID_HOME:+$ANDROID_HOME/platform-tools/adb}"
ADB="${ADB:-adb}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Fresh, so a compromised phone cannot have its answer ready.
CHALLENGE="$(openssl rand -hex 32)"

# A Pixel 5 is often connected too; enrol only the Pixel 9.
for EP in "${1:-}" "10.100.0.12:5555"; do
  [ -z "$EP" ] && continue
  [[ "$EP" == *:* ]] || EP="$EP:5555"
  "$ADB" connect "$EP" 2>&1 | grep -qiE "connected|already" || continue
  MODEL="$("$ADB" -s "$EP" shell getprop ro.product.model 2>/dev/null | tr -d '\r')"
  [ "$MODEL" = "Pixel 9" ] && DEVICE="$EP" && break
  echo "  skip $EP — reports model '$MODEL', not 'Pixel 9'." >&2
done
if [ -z "${DEVICE:-}" ]; then
  echo "Pixel 9 not reachable. Pass its ip:port as an argument." >&2
  exit 1
fi

echo "=== asking $DEVICE for a key ==="
"$ADB" -s "$DEVICE" shell am start -S -n "$PACKAGE/.MainActivity" \
  --es enrol_challenge "$CHALLENGE" >/dev/null
# The activity returns before StrongBox has written the chain.
sleep 5

# exec-out: `adb shell`'s pty turns LF into CRLF and breaks the PEM.
"$ADB" -s "$DEVICE" exec-out run-as "$PACKAGE" cat "files/enrolment.pem" > "$WORK/chain.pem"
if [ ! -s "$WORK/chain.pem" ]; then
  echo "The phone wrote no chain. Is the app installed (console/android/deploy.sh)," >&2
  echo "and did it come to the foreground?" >&2
  exit 1
fi
echo "  $(grep -c 'BEGIN CERTIFICATE' "$WORK/chain.pem") certificates"

echo "=== Google's revocation list ==="
if ! curl -fsS "$STATUS_URL" -o "$WORK/status.json"; then
  echo "Could not fetch $STATUS_URL. Enrolling without it would leave the one" >&2
  echo "check that catches an extracted key silently unperformed." >&2
  exit 1
fi

echo "=== checking the claim ==="
# The pin is the last line, and only printed when every check passed.
PIN="$(nix develop -c cargo run -q -p console --bin attest -- \
  "$WORK/chain.pem" "$CHALLENGE" "$WORK/status.json" | tee /dev/stderr | tail -1)"

mkdir -p "$DIR"
chmod 700 "$DIR"
if grep -qxF "$PIN" "$DIR/clients" 2>/dev/null; then
  echo "already pinned: $PIN"
else
  echo "$PIN" >> "$DIR/clients"
  echo "pinned: $PIN"
fi
echo
echo "Restart the console for it to take effect (scripts/console.sh)."
echo "Revoking is deleting that line from $DIR/clients and restarting."
