#!/usr/bin/env bash
# Watch for the console claiming to hold the screen while Android holds nothing.
#
#   ./scripts/awake-watch.sh [hours]        # default 12
#   tail -f ~/Library/Logs/awake-watch.log
#
# WHY A WATCHER AND NOT A TEST. memview#892 was found twice by hand, hours apart,
# and never reproduced deliberately.
#
# ⚠ **What was measured about freezing is NARROWER than "the freeze theory is
# dead", which is how it first got written down.** On 2026-08-15 `am freeze
# --sticky` for 45s with the app in FRONT left KEEP_SCREEN_ON standing — so
# freezing does not strip the window flag, because the flag lives in
# WindowManager and outlives the process being stopped. It says nothing about the
# real sequence: Android's cached-app freezer SIGSTOPs a process ~40s after
# SCREEN-OFF (memory `reference_android_cached_app_freezer`), and a stopped
# process runs no JavaScript — so the page cannot notice a lock it lost, and
# cannot re-take one, for as long as it is frozen. That is precisely why the fix
# is a heartbeat that measures its own LATENESS rather than a timer that trusts
# it fired. Do not restate the narrow measurement as the broad claim.
#
# There is still no way to induce the fault on demand, and a green run of
# anything proves only that the fault did not happen while it ran.
#
# What is left is the symptom, which is unambiguous and cheap to sample: the app
# says it is holding the screen and no window on the device is. This records that
# over hours, so #892 closes on evidence instead of on nobody having noticed.
#
# ⚠ **`dumpsys` alone cannot tell a fault from you turning the button off.** A
# missing lock is only wrong if the button is still lit, and that lives in the
# page. So the cheap check runs every minute and the expensive one — CDP into the
# WebView, read `aria-pressed` — runs ONLY when the cheap one looks wrong. A
# confirmed fault is `pressed=true` with no lock.
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

# Whether the console is the resumed activity AND the display is on. A lock means
# nothing while the screen is off, and nothing while another app is in front.
#
# ⚠ **`mWakefulness` is NOT the display, and reading it here made the gate lie.**
# It is the power state machine: it says `Awake` for a device that is up with its
# screen off, which was seen directly on 2026-08-15 — the phone had gone dark and
# locked while this reported `Awake`. That is fatal rather than merely noisy,
# because `KEEP_SCREEN_ON` is 0 on EVERY window whenever the screen is off, so a
# dark phone with the button still lit is scored as a fault every single minute.
# `dumpsys display`'s `mScreenState` answers the question actually being asked.
watching() {
  local top on
  top=$(adb -s "$DEVICE" shell dumpsys activity activities 2>/dev/null | grep -c "topResumedActivity.*$PKG" || true)
  on=$(adb -s "$DEVICE" shell dumpsys display 2>/dev/null | grep -c 'mScreenState=ON' || true)
  [ "$top" -gt 0 ] && [ "$on" -gt 0 ]
}

# ⚠ **Only THIS app's window counts.** A plain `grep -c KEEP_SCREEN_ON` counts the
# whole device, and the fleet is a dozen WebView wrappers with the same button:
# `org.xinutec.heatcam` was holding one at the same moment as the console, seen
# 2026-08-15. Any other app holding one would read as the console holding one, so
# the watcher would go quiet on exactly the fault it exists to catch.
#
# `fl=` and `package=` live in the same window block, so the package last seen is
# the one a flag line belongs to.
held() {
  adb -s "$DEVICE" shell dumpsys window windows 2>/dev/null | awk -v pkg="$PKG" '
    /package=/ { owner = $0; sub(/.*package=/, "", owner); sub(/ .*/, "", owner) }
    /fl=.*KEEP_SCREEN_ON/ { if (owner == pkg) n++ }
    END { print n + 0 }'
}

# The SECOND instrument, and the reason there are two. `dumpsys power` lists the
# live wake locks with the package attributed directly:
#
#   SCREEN_BRIGHT_WAKE_LOCK 'WindowManager/displayId:0' … ws=WorkSource{10562 org.xinutec.console}
#
# It was the ground truth used on 2026-08-07 to prove `navigator.wakeLock` works
# in this WebView at all (memory `reference_android_webview_cdp`), and it needs no
# block-pairing, so it cannot be defeated by another app the way a device-wide
# `grep -c` is. Measured agreeing with `held` in the healthy state on 2026-08-15 —
# ⚠ which per this repo's own hard-won rule is NOT evidence that they agree in the
# fault state. So both are read, and a disagreement is reported as its own event
# rather than silently resolved: two instruments that part company are a finding
# about the instruments, and believing either one alone is how the last three
# measurement bugs here survived.
attributed() {
  adb -s "$DEVICE" shell dumpsys power 2>/dev/null |
    grep -c "WAKE_LOCK.*ws=WorkSource{[0-9]* $PKG" || true
}

# The button's own answer. Only asked when the lock is missing, because it costs a
# port forward and a websocket every time.
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

# ⚠ **Silence has to be distinguishable from death.** A watcher that only speaks
# on a fault reads exactly the same whether it is healthy, wedged, or was killed
# an hour ago — and a run that sampled nothing looks like a run that found
# nothing. Both mistakes were made in one night getting here. So it says how much
# it has actually seen, periodically, and the count is the evidence rather than
# the quiet.
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
