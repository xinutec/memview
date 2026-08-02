#!/usr/bin/env bash
# Build the console APK and install it to the Pixel 9 over Wi-Fi. Run from
# console/android/ inside the borrowed Android dev shell:
#
#   nix develop ~/Code/recall#android --command ./deploy.sh [<ip[:port]>]
#
# This is a single-purpose handheld app on ONE phone (the Pixel 9). DHCP drifts the
# IP, so we key on the device *model*, never the IP: connect, verify it really is a
# Pixel 9, then install by serial. A bare `adb install` could hit the wrong device
# (a Pixel 5 is often also adb-connected) — so we never use it.
#
# Where the console is and which key it will accept come from console.env, which is
# NOT in the repository: this one is public, and the address of a machine on a
# private VPN does not belong in it. See README.md for what to put there.
set -euo pipefail
cd "$(dirname "$0")"

if [ -f console.env ]; then
  # shellcheck disable=SC1091
  set -a; . ./console.env; set +a
fi
if [ -z "${CONSOLE_URL:-}" ] || [ -z "${CONSOLE_SERVER_PIN:-}" ]; then
  echo "console.env is missing or incomplete — the app would build with no address." >&2
  echo "Run ../../scripts/console-identity.sh, then see README.md." >&2
  exit 1
fi

ADB="$ANDROID_HOME/platform-tools/adb"

# Braced: bash takes the ellipsis as part of the name otherwise, and `set -u` then
# reports an unbound variable whose name has an ellipsis in it.
echo "building APK for ${CONSOLE_URL}…"
./gradlew :app:assembleDebug -q
APK="$PWD/app/build/outputs/apk/debug/app-debug.apk"

# Endpoints to try, in order. :5555 (persistent `adb tcpip`) survives sleep, so try
# it first — VPN IP (stable, 10.100.0.12) then the LAN. The LAN lease drifts, so
# pass the real ip:port as an argument when both fail.
CANDIDATES=("${1:-}" "10.100.0.12:5555")

for EP in "${CANDIDATES[@]}"; do
  [ -z "$EP" ] && continue
  [[ "$EP" == *:* ]] || EP="$EP:5555"
  "$ADB" connect "$EP" 2>&1 | grep -qiE "connected|already" || continue
  MODEL="$("$ADB" -s "$EP" shell getprop ro.product.model 2>/dev/null | tr -d '\r')"
  if [ "$MODEL" != "Pixel 9" ]; then
    echo "  skip $EP — reports model '$MODEL', not 'Pixel 9'." >&2
    continue
  fi
  echo "=== installing to Pixel 9 ($EP) ==="
  "$ADB" -s "$EP" install -r "$APK"
  "$ADB" -s "$EP" shell am start -S -n org.xinutec.console/.MainActivity >/dev/null
  echo "  installed + launched on Pixel 9 ($EP)."
  echo "  not enrolled yet? run ../../scripts/enrol.sh $EP"
  exit 0
done

echo "Pixel 9 not reachable on :5555 (VPN or LAN). Re-enable wireless debugging or" >&2
echo "re-run 'adb tcpip 5555', then pass the ip:port as an argument." >&2
exit 1
