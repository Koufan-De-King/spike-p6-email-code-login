# P6 findings so far

Running log of what the tests have shown. Each line links to its evidence. Hypothesis numbers
(H1…) refer to [architecture.md §6](architecture.md#6-hypotheses--candidate-blockers).

## Round 1 — Android emulators, 2026-10-20

**Setup:**

| Item | Value |
|---|---|
| Laptop | Koufan's Linux laptop, Docker stack at `http://127.0.0.1:8080`, reached from the emulators via `adb reverse` |
| Probe build | Flutter 3.47.7 debug build, `flutter_web_auth_2` 5.1.0 |
| Android 9 emulator | API 28, Google Play image, Chrome |
| Android 8 emulator | API 26, Google APIs x86_64 image, **Chrome 69** |
| Driver | `scripts/android-run.sh` |
| Event log | [`evidence/android-emulator/spike-events.json`](evidence/android-emulator/spike-events.json) |

These are emulators, not phones. They show what Android and Chrome do. They don't show what
low-RAM hardware, Transsion browsers or real mobile data do.

### Results

| Run | Android 9 | Android 8 |
|---|---|---|
| A · private-use scheme, 303 | ✅ after the manifest fix below ([05](evidence/android-emulator/05-a9-A-success-after-affinity-fix.png)) | ✅ |
| B · "Return to Another" page | ✅ after one tap ([06](evidence/android-emulator/06-a9-B-return-to-app-page.png), [07](evidence/android-emulator/07-a9-B-success-after-tap.png)) | ✅ ([12](evidence/android-emulator/12-a8-B-success.png)) |
| C · claimed HTTPS | ⏸ can't test yet: the real host fails TLS ([08](evidence/android-emulator/08-a9-C-real-host-tls-failure.png)) | not run |
| H1 · app process killed while the user reads the email | ❌ **login lost** ([09](evidence/android-emulator/09-a9-H1-login-lost-after-kill.png)) | ❌ **login lost** ([13](evidence/android-emulator/13-a8-H1-login-lost-after-sigkill.png)) |

Time in the browser for a successful run was 10–16 s on the emulator. Most of it is typing the email and the code.

### F1 — App killed while away = login silently lost (H1 confirmed) · **blocker**

- **Where:** Android 8 and 9.
- **What happens:**
  1. The user is on the code page.
  2. Android kills the app's process. Tested as a SIGKILL, the way the low-memory killer does it.
  3. The user types the code. The IdP issues the authorization code and redirects to the app.
  4. Android cold-starts the app. `flutter_web_auth_2` keeps its pending login only in memory, so
     the redirect has nothing waiting for it. The code is never redeemed.
- **What the user sees:** a fresh, signed-out app with no error.
- **Evidence:** the event log shows `code.issued` with no `token.issued` after it. The probe's
  lost-flow detector fired.
- **How the tests killed the app:**
  - `am kill` was not enough on Android 8. The process survived, probably held up by the plugin's
    `KeepAliveService`. A real low-memory kill doesn't ask, so the Android 8 test used
    `run-as … kill -9`.
  - Android 9 died with plain `am kill`.
- **Fix (as designed):**
  - The Rust core persists the pending flow (verifier, state, nonce) before opening the browser.
  - The app handles the `digital.camer.another:/oauth2redirect` intent itself on a cold start
    (e.g. `app_links`), and the core finishes the exchange.
  - `flutter_web_auth_2` alone can't do this. Keep it for opening the tab, or replace it.

### F2 — Wrong `taskAffinity` = success hidden behind the browser · **blocker, fixed in the probe**

- **What happens:** the login succeeded and the token was issued, but Chrome's Custom Tab stayed
  in front showing the code page ([04](evidence/android-emulator/04-a9-stuck-on-custom-tab-after-success.png)).
- **Cause:**
  - Flutter's template sets `android:taskAffinity=""` on `MainActivity`.
  - The plugin's README also recommends `taskAffinity=""` on its `CallbackActivity`.
  - Chrome launches the callback with `NEW_TASK`. With no shared affinity, the callback lands in a
    task of its own, so its `CLEAR_TOP` can't pop the Custom Tab off the app's task.
- **Fix:** remove `taskAffinity=""` from **both** activities. The probe now does this.
- **Trade-off for ADR 15:** Flutter sets that attribute to harden against task hijacking
  ("StrandHogg"). Dropping it needs a security note, or another way back to the app.

### F3 — Chrome's first run can eat the login request · **friction, sometimes a blocker**

- **When:** Chrome is installed but has never been opened. That's common where Chrome isn't the
  user's default browser.
- **What happens:** tapping Sign in first shows "Welcome to Chrome" and its terms
  ([01](evidence/android-emulator/01-a9-chrome-first-run-terms.png), [10](evidence/android-emulator/10-a8-chrome69-first-run-terms.png)).
- **Android 9 (Play image's Chrome):** after accepting, Chrome opened its home page and **the login
  request was lost** ([02](evidence/android-emulator/02-a9-request-lost-after-first-run.png)).
  Going back to the app showed "User canceled login"
  ([03](evidence/android-emulator/03-a9-app-reports-canceled.png)), so a retry works.
- **Android 8 (Chrome 69):** after accepting, Chrome **kept** the request and showed our page
  ([11](evidence/android-emulator/11-a8-request-kept-after-first-run.png)).
- **Mitigation:** treat "canceled" as "try again" in the UI, not as an error. Consider detecting a
  Custom Tabs provider that has never run.

### F4 — `auth.another.camer.digital` fails TLS today · **setup, for the architect**

- **Observed:** the name resolves to Cloudflare (`2606:4700:…`). Both Chrome and `openssl` get a
  handshake failure (`ERR_SSL_VERSION_OR_CIPHER_MISMATCH`, alert 40).
- **Likely cause:** Cloudflare's Universal SSL covers `*.camer.digital`, but not a second-level
  name like `auth.another.camer.digital`. That needs an advanced or custom edge certificate, or a
  DNS-only record with the wildcard certificate on the origin.
- **This is an outside guess.** The architect should confirm.
- **Blocks:** variant C and the recorded runs on the real host.

### F5 — Emulator-only notes (not findings about phones)

- The Android 8 image that ships the Play Store is 32-bit x86, and current Flutter doesn't build
  for 32-bit x86 (`INSTALL_FAILED_NO_MATCHING_ABIS`). Real Android 8 phones are ARM, which works.
  We used the x86_64 Google APIs image instead.
- That 32-bit image ships **Chrome 58** (2017). A phone that never updates Chrome can be that old.
  The pages don't need JS, which is why they still worked on Chrome 69.

### Confirmed working (no action)

- `inputmode=numeric` opens the number pad for the code on both Chrome versions.
- The `iss` and `state` checks, PKCE, the token exchange and ID-token decoding in the probe app
  all work against the stand-in.

## Round 2 — Linux desktop, 2026-10-20

**Setup:** Ubuntu 24.04, **GNOME on Wayland**, Chrome 155 as the default browser. The stack runs
on the same laptop, and the person at the keyboard typed the email and the code.

| Run | Result | Browser leg |
|---|---|---|
| Rust CLI (`another-login`), the core's desktop flow | ✅ ([1](evidence/linux/1-cli-chrome-success.png)) | 37 s |
| Probe app, plugin defaults | ❌ the plugin opened its own embedded web view, not Chrome ([2](evidence/linux/2-flutter-embedded-webview-default.png)) | — |
| Probe app, button D, `useWebview: false` | ✅ ✓ with the email ([3](evidence/linux/3-flutter-system-browser-success.png)), but the window **did not come to the front** | 23 s |

The loopback flow works on Linux exactly as designed. That's the same mechanism Windows and macOS
(if loopback) will use.

### F6 — `flutter_web_auth_2` defaults to an embedded web view on Linux/Windows · **must override**

- **What happens:** with default options, the plugin uses `desktop_webview_window` (WebKitGTK)
  instead of the system browser. RFC 8252 forbids that.
- **Why it failed:** in that mode no loopback listener is started. The web view followed our
  redirect to `127.0.0.1:<port>` and got "Connection refused".
- **Fix:** always pass `useWebview: false` on desktop.
- **Also raise the timeout.** The plugin's default is 5 minutes, after which it reports
  "canceled". That's short for an email code.
- **Packaging cost:** even with the web view off, the Linux build links `libwebkit2gtk-4.1`.
  Every Linux user needs that library installed. Building needs `libwebkit2gtk-4.1-dev`.
- **Alternative:** the Rust core owns the loopback listener, as the CLI does, and the plugin isn't
  used on desktop at all. That drops the WebKitGTK dependency.

### F7 — The app can't bring itself back to the front on Wayland · **UX**

- **What happens:** the login completes, but the browser keeps focus. The plugin's
  `WindowToFront` is an X11-era trick, and GNOME on Wayland ignores self-raising without an
  `xdg-activation` token.
- **The tab's text is misleading:** the plugin's page says "This page should automatically close".
  Browsers don't let a page close a tab it didn't open, so it stays.
- **Fix:**
  - Request activation properly (xdg-activation), or show a desktop notification ("Signed in —
    open Another").
  - Serve our own wording in the tab ("You're signed in — return to Another"), as the CLI does.
- **Check on Windows:** its foreground lock has a similar rule.

## Next

- Real phones: Transsion with its stock browser (H2); low-RAM hardware for natural H1.
- iOS 15/16 and macOS 12 need a Mac.
- Windows 10 needs the CLI built for Windows. The probe app with button D also works there (same plugin code as Linux); check F7 on Windows.
- Variant C waits on F4.
