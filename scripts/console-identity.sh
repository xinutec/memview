#!/usr/bin/env bash
# Give the console a key of its own, so it can be reached from off this machine.
#
#   ./scripts/console-identity.sh [<address>...]
#
# Self-signed on purpose: the phone pins the public key printed at the end and
# consults no trust store, so a public certificate would add an ACME client, a
# renewal and a transparency-log entry for nothing.
#
# The key lives outside this public repository.
set -euo pipefail
cd "$(dirname "$0")/.."

DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"

if [ -s "$DIR/server.key" ]; then
  echo "There is already a key at $DIR/server.key." >&2
  echo "Replacing it locks out every phone built against the old one; delete it" >&2
  echo "by hand if that is what you want." >&2
  exit 1
fi

# Every address the console may be reached on. Chromium ignores the common name,
# and the extension also makes LibreSSL emit a v3 certificate; rustls refuses v1.
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
  # The address the phone actually uses: the tunnel's listener on isis, on no
  # interface here. Without it every handshake failed the name check before the
  # pin overrode it. Read from the tunnel script so the two agree.
  TUNNEL="$(sed -n 's/^LISTEN="\${CONSOLE_TUNNEL_LISTEN:-\([0-9.]*\):.*/\1/p' \
    "$(dirname "$0")/console-tunnel.sh" 2>/dev/null | head -1)"
  [ -n "$TUNNEL" ] && ADDRESSES+=("$TUNNEL")
fi

SAN=""
for ADDR in "${ADDRESSES[@]}"; do
  SAN="${SAN:+$SAN,}IP:$ADDR"
done

mkdir -p "$DIR"
chmod 700 "$DIR"

echo "generating a P-256 key for: ${ADDRESSES[*]}"
# Two steps, not `req -newkey ec`: LibreSSL's -pkeyopt differs from OpenSSL's.
openssl ecparam -name prime256v1 -genkey -noout -out "$DIR/server.key"
chmod 600 "$DIR/server.key"
openssl req -new -x509 -key "$DIR/server.key" -out "$DIR/server.crt" \
  -days 3650 -subj "/CN=agent console" \
  -addext "subjectAltName=$SAN" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=serverAuth"

# Ten years, never renewed: the phone compares keys and the gate ignores expiry.

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
