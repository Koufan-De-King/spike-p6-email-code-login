# P6 test protocol — one video per OS

Every run shows up at `<issuer>/spike` with the browser's User-Agent and a timeline. Name each run
through the probe's *Platform label* (or `--platform` on the CLI) like `android9-tecno-spark4`.
After each session, export `<issuer>/spike/events.json` next to the video.

## Setup

| Where | Issuer | Mail |
|---|---|---|
| Laptop only | `http://127.0.0.1:8080` (`docker compose up -d`) | Mailpit, http://127.0.0.1:8025 |
| Phones on the same Wi-Fi | `ISSUER=http://<LAN-IP>:8080 docker compose up -d` | Mailpit |
| **Recorded runs** | `https://auth.another.camer.digital` (deployed image, real TLS) | Mailpit in the cluster, or the P13 relay into a real inbox on the phone |

To film the realistic mail step (switching to the mail app), the code has to reach a real inbox
on the device. Use P13's relay once it is up. Until then, open Mailpit's UI in a second browser
tab on the phone.

## Devices

| OS | Minimum | Ideally also |
|---|---|---|
| iOS 15 | any device on 15.x | — |
| iOS 16 | any device on 16.x | — |
| Android 8 | real device, ≤ 2 GB RAM if possible | emulator API 26 |
| Android 9 | Tecno/Infinix/itel with its stock browser as default | Samsung A-series, emulator API 28 |
| Windows 10 | 22H2 VM or laptop, default browser Edge | Firefox as default |
| macOS 12 | Monterey VM or laptop, Safari default | Chrome default |

## Runs per device

Film each run from tapping *Sign in* to the ✓ line.

**Mobile (probe app)**

1. **A · scheme, 303**, ephemeral on. Read the code from the notification without leaving the browser.
2. **A again**, but switch to the mail app to read the code, then come back. *(H1, H6)*
3. **B · interstitial**. *(H3)*
4. **C · claimed https**. *(H4 on iOS 15/16 — expected to fail; film the failure)*
5. iOS only: **A with ephemeral off**. *(H5 — film the system alert)*
6. Android only: Developer options → **Don't keep activities** on, then run 2 again. *(H1)* If the
   app shows "previous flow never completed", H1 is real.
7. Android only: set the stock / non-Chrome browser as default (or disable Chrome), then run 1. *(H2)*

**Desktop (Rust client — the core's loopback flow)**

```bash
cargo run -p another-login -- --issuer https://auth.another.camer.digital --platform win10-edge
```

1. Default browser. Watch for a firewall prompt and whether the app gets focus back. *(H7)*
2. macOS: Safari, then Chrome. *(H8)*
3. macOS: the probe app's variant A (ASWebAuthenticationSession), to compare.

## Per-run record (paste into the report)

| Run | Device / OS / browser | Variant | Result | Browser leg (s) | Code sent → typed (s) | Blocker? | Video |
|---|---|---|---|---|---|---|---|
| | | | ✅ / ❌ step | | from `/spike` | H# + one line | link |
