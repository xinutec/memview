#!/usr/bin/env bash
# Watch for the console claiming to hold the screen while Android holds nothing.
#
#   ./scripts/awake-watch.sh [hours]        # default 12
#   tail -f ~/Library/Logs/awake-watch.log
#
# A watcher, not a test: the fault was seen twice by hand and cannot be induced.
# Android's cached-app freezer stops the process about 40 s after screen-off, and
# a stopped page runs no JavaScript, so it cannot notice a lost lock. (Freezing
# alone does not drop KEEP_SCREEN_ON: the flag lives in WindowManager.)
#
# `dumpsys` alone cannot tell a fault from the button turned off, so the
# one-minute check is cheap and the page is read over CDP only when it looks
# wrong. A fault is `aria-pressed=true` with no lock.
set -euo pipefail

HOURS="${1:-12}"
LOG="$HOME/Library/Logs/awake-watch.log"
CDP="$HOME/Code/xinutec-infra/mac-mini/browser/cdp.py"
PKG=org.xinutec.console
PORT=9345

DEVICE="${ADB_DEVICE:-$(adb devices | awk '/\tdevice$/ {print $1; exit}')}"
if [ -z "$DEVICE" ]; then
  echo "no adb device — is the VPN up?" >&2
  exit 2
fi

say() { printf '%s %s\n' "$(date '+%F %T')" "$1" | tee -a "$LOG"; }

# The console is in front and the display is on; otherwise a lock means nothing.
# `mScreenState`, not `mWakefulness`: that says `Awake` with the screen off, when
# KEEP_SCREEN_ON is 0 on every window, so every dark minute scored as a fault.
watching() {
  local top on
  top=$(adb -s "$DEVICE" shell dumpsys activity activities 2>/dev/null | grep -c "topResumedActivity.*$PKG" || true)
  on=$(adb -s "$DEVICE" shell dumpsys display 2>/dev/null | grep -c 'mScreenState=ON' || true)
  [ "$top" -gt 0 ] && [ "$on" -gt 0 ]
}

# This app's windows only: other WebView wrappers hold the flag too, and counting
# theirs would hide the console's fault. A `fl=` line belongs to the last
# `package=` seen.
held() {
  adb -s "$DEVICE" shell dumpsys window windows 2>/dev/null | awk -v pkg="$PKG" '
    /package=/ { owner = $0; sub(/.*package=/, "", owner); sub(/ .*/, "", owner) }
    /fl=.*KEEP_SCREEN_ON/ { if (owner == pkg) n++ }
    END { print n + 0 }'
}

# A second instrument: `dumpsys power` attributes each wake lock to a package
#
#   SCREEN_BRIGHT_WAKE_LOCK 'WindowManager/displayId:0' … ws=WorkSource{10562 org.xinutec.console}
#
# It agrees with `held` when healthy, which says nothing about a fault, so a
# disagreement is logged as its own event rather than resolved.
attributed() {
  adb -s "$DEVICE" shell dumpsys power 2>/dev/null |
    grep -c "WAKE_LOCK.*ws=WorkSource{[0-9]* $PKG" || true
}

# The button's state, read over CDP; costs a port forward, so only on a missing lock.
pressed() {
  local pid
  pid=$(adb -s "$DEVICE" shell pidof "$PKG" 2>/dev/null | tr -d '\r')
  if [ -z "$pid" ]; then
    echo "no-process"
    return
  fi
  adb -s "$DEVICE" forward "tcp:$PORT" "localabstract:webview_devtools_remote_$pid" >/dev/null 2>&1 || true
  local answer
  answer=$("$CDP" --no-origin --port "$PORT" eval \
    'document.querySelector(".bar .awake")?.getAttribute("aria-pressed") ?? "no-button"' 2>/dev/null | tail -1 | tr -d '"' || true)
  adb -s "$DEVICE" forward --remove "tcp:$PORT" >/dev/null 2>&1 || true
  echo "${answer:-unreadable}"
}

say "watching $DEVICE for ${HOURS}h — a fault is a lit button over an unheld screen (screen-on gate: mScreenState)"
FAULTS=0
SPLITS=0
SAMPLES=0
DEADLINE=$(( $(date +%s) + HOURS * 3600 ))

# A periodic count, so a healthy run is told apart from a wedged or dead one.
PROGRESS_EVERY=30

while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  if watching; then
    SAMPLES=$((SAMPLES + 1))
    if [ $((SAMPLES % PROGRESS_EVERY)) -eq 0 ]; then
      say "still watching — $SAMPLES sample(s) with the console in front, $FAULTS fault(s), $SPLITS disagreement(s)"
    fi
    if [ "$(held)" -eq 0 ]; then
      POWER=$(attributed)
      case "$(pressed)" in
        true)
          if [ "$POWER" -gt 0 ]; then
            SPLITS=$((SPLITS + 1))
            say "instruments DISAGREE #$SPLITS — no window flag, but dumpsys power still attributes a wake lock to $PKG; not counted as a fault"
          else
            FAULTS=$((FAULTS + 1))
            say "FAULT #$FAULTS — button lit, and neither the window flag nor dumpsys power has a lock for $PKG"
          fi
          ;;
        false)
          : # the button is off; nothing is claimed and nothing is wrong
          ;;
        *)
          say "note — no lock, and the page could not be read"
          ;;
      esac
    fi
  fi
  sleep 60
done

say "done — $FAULTS fault(s), $SPLITS instrument disagreement(s), in $SAMPLES sample(s) with the console in front"
[ "$FAULTS" -eq 0 ]
