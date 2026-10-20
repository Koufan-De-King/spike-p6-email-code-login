#!/usr/bin/env bash
# Drives one probe-app login on a connected emulator/phone against the local stack.
#   scripts/android-run.sh <label> <button: A|B|C> [email]
# BEFORE_CODE="<shell>" runs after the email is sent and before the code is typed (e.g. kill the app).
# TAP_RETURN=1 taps the "Return to Another" button variant B shows.
# Needs: adb on PATH, the stack up, the probe APK built. Screenshots land in ./android-run-<label>/.
set -euo pipefail
LABEL=$1; BTN=$2; EMAIL=${3:-$LABEL@example.cm}
APK=$(dirname "$0")/../probe_app/build/app/outputs/flutter-apk/app-debug.apk
OUT=android-run-$LABEL; mkdir -p "$OUT"; n=0
shot() { n=$((n+1)); adb exec-out screencap -p > "$OUT/$n-$1.png"; echo "  screenshot $OUT/$n-$1.png"; }
fill() { adb shell input tap 540 "$1"; adb shell input keyevent 123; for _ in $(seq 1 90); do adb shell input keyevent 67; done; adb shell input text "$2"; }
declare -A Y=([A]=819 [B]=945 [C]=1071)

adb install -r "$APK" >/dev/null
adb reverse tcp:8080 tcp:8080 >/dev/null; adb reverse tcp:8025 tcp:8025 >/dev/null
adb shell am force-stop digital.camer.another.p6_probe
adb shell monkey -p digital.camer.another.p6_probe -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1; sleep 4
fill 345 "http://127.0.0.1:8080"; fill 492 "$LABEL"; adb shell input keyevent 111; sleep 1
adb shell input tap 540 "${Y[$BTN]}"; sleep 5; shot login-page
adb shell input tap 540 680; sleep 1; adb shell input text "$EMAIL"; adb shell input keyevent 66; sleep 3
CODE=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=to:$EMAIL&limit=1" | grep -o '"Subject":"[0-9]\{6\}' | grep -o '[0-9]\{6\}')
echo "  code $CODE"
if [ -n "${BEFORE_CODE:-}" ]; then echo "  before-code hook: $BEFORE_CODE"; eval "$BEFORE_CODE"; sleep 2; shot before-code; fi
adb shell input text "$CODE"; adb shell input keyevent 66; sleep 6; shot after-code
if [ "${TAP_RETURN:-}" = 1 ]; then adb shell input tap 540 600; sleep 4; shot after-return; fi
adb shell dumpsys activity activities | grep -m1 mResumedActivity | sed 's/^ */  foreground: /'
