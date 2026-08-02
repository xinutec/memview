#!/usr/bin/env bash
# Give the console a key of its own, so it can be reached from off this machine.
#
#   ./scripts/console-identity.sh [<address>...]
#
# Self-signed, and deliberately so. Nothing that connects to this console consults
# a trust store — the phone pins the public key printed at the end — so a
# publicly-issued certificate would buy nothing and cost an ACME client on the Mac,
# a Cloudflare token in the path, a renewal that can expire at 3am, and the
# console's name in a certificate transparency log. See docs/agent-console.md.
#
# The material lives outside the repository. This repository is public.
set -euo pipefail
cd "$(dirname "$0")/.."

DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"

if [ -s "$DIR/server.key" ]; then
  echo "There is already a key at $DIR/server.key." >&2
  echo "Replacing it locks out every phone built against the old one; delete it" >&2
  echo "by hand if that is what you want." >&2
  exit 1
fi

# Every address the console might be reached on. Chromium ignores the common name
# entirely, so a certificate with no subjectAltName matches nothing — and this is
# also what makes it a version-3 certificate at all: macOS ships LibreSSL, whose
# `req -x509` emits a v1 certificate when no extension asks otherwise, and rustls
# refuses those with `UnsupportedCertVersion` and no hint about why.
ADDRESSES=("$@")
if [ ${#ADDRESSES[@]} -eq 0 ]; then
  ADDRESSES=(127.0.0.1)
  for IF in $(ifconfig -l); do
    ADDR="$(ipconfig getifaddr "$IF" 2>/dev/null || true)"
    [ -n "$ADDR" ] && ADDRESSES+=("$ADDR")
  done
  # The VPN address is on a utun interface, which ipconfig does not answer for.
  VPN="$(ifconfig 2>/dev/null | awk '/inet 10\.100\./ {print $2}' | head -1)"
  [ -n "$VPN" ] && ADDRESSES+=("$VPN")
fi

SAN=""
for ADDR in "${ADDRESSES[@]}"; do
  SAN="${SAN:+$SAN,}IP:$ADDR"
done

mkdir -p "$DIR"
chmod 700 "$DIR"

echo "generating a P-256 key for: ${ADDRESSES[*]}"
# Two steps rather than `req -newkey ec`: LibreSSL's -pkeyopt is not the same
# animal as OpenSSL's, and this pair of commands means the same thing to both.
openssl ecparam -name prime256v1 -genkey -noout -out "$DIR/server.key"
chmod 600 "$DIR/server.key"
openssl req -new -x509 -key "$DIR/server.key" -out "$DIR/server.crt" \
  -days 3650 -subj "/CN=agent console" \
  -addext "subjectAltName=$SAN" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=serverAuth"

# Ten years, and no renewal reminder, because nothing checks the dates: the phone
# compares public keys and the console's own gate ignores expiry on purpose. The
# date is there because X.509 demands one.

PIN="$(nix develop -c cargo run -q -p console --bin pin -- "$DIR/server.crt")"

cat <<INSTRUCTIONS

  key   $DIR/server.key
  cert  $DIR/server.crt

The phone pins this key. Put it in console/android/console.env — which is
gitignored, and holds the two things a build needs to know:

  CONSOLE_URL=https://<one of: ${ADDRESSES[*]}>:8097/
  CONSOLE_SERVER_PIN=$PIN

Then enrol the phone (scripts/enrol.sh) and start the console (scripts/console.sh),
which picks this up on its own and binds off loopback once a client is pinned.
INSTRUCTIONS
