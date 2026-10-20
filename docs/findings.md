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

## Round 3 — macOS 14 rehearsal (not macOS 12), 2026-10-20

**Setup:**

| Item | Value |
|---|---|
| Mac | Intel, **macOS 14.7.1** (23H222). Not the macOS 12 target: that run still needs a macOS 12 VM or Mac |
| Browsers | Chrome (the default), Safari 17.6 |
| Stack | Redis and Mailpit in Docker (colima). The IdP ran **natively** (`cargo run -p idp`), because pulling the Rust build image was too slow on this connection. Same code, same config |
| Probe build | Flutter **3.44.1** debug build (Linux used 3.47.7), `flutter_web_auth_2` 5.1.0 |
| Event log | [`evidence/macos14/spike-events.json`](evidence/macos14/spike-events.json) |

The automatic login and `negative.sh` both pass on the Mac.

### Results

| Run | Result | Browser leg | Evidence |
|---|---|---|---|
| CLI, loopback, Chrome (default) | ✅ | 39 s | [1](evidence/macos14/1-cli-chrome-success.webp) |
| CLI, loopback, Safari 17.6 (`--no-open` + `open -a Safari`) | ✅ no warning on the `http://127.0.0.1` redirect | 44 s | [2](evidence/macos14/2-cli-safari17-success.webp) |
| Probe A, ephemeral on, Chrome default | ✅ ×2, ran **in Chrome** | 36 s | — |
| Probe A, ephemeral off, Chrome default | ❌ ×3 "Couldn't communicate with a helper application", then ✅ on a later retry | 29 s | [3](evidence/macos14/3-probe-A-ephemeral-off-helper-error.webp), [8](evidence/macos14/8-probe-A-ephemeral-off-chrome-retry-success.png) |
| Probe B, ephemeral off, Chrome default | ✅ after the H5 alert and one tap | 124 s (incl. the alert) | [4](evidence/macos14/4-probe-B-wants-to-use-alert-chrome.webp), [5](evidence/macos14/5-probe-B-chrome-auth-window.webp), [6](evidence/macos14/6-probe-B-success.webp) |
| Probe A, ephemeral on, **Safari default** | ✅ Safari "Private Browsing" window | 34 s | [9](evidence/macos14/9-probe-A-ephemeral-on-safari-private-window.webp), [11](evidence/macos14/11-probe-A-safari-success-both.png) |
| Probe A, ephemeral off, **Safari default** | ✅ after the H5 alert (Safari icon) | 55 s (incl. the alert) | [10](evidence/macos14/10-probe-A-ephemeral-off-safari-alert.webp), [11](evidence/macos14/11-probe-A-safari-success-both.png) |
| Probe C, claimed https | ⏸ "User canceled" within 1 s; nothing reached the server | — | [7](evidence/macos14/7-probe-C-canceled.webp) |

H8 does not show up on Safari 17 or Chrome. It still has to be checked on Safari 15 (macOS 12).

### F8 — On macOS 14, ASWebAuthenticationSession runs in the *default* browser · **test-coverage note**

- **What happens:** with Chrome as the default, the sign-in sheet was a Chrome window (Chrome's
  user agent in the event log, Chrome's icon on the H5 alert). With Safari as the default, it was
  a Safari window ("Private Browsing" when ephemeral).
- **Why it matters:** macOS 14 lets the default browser serve ASWebAuthenticationSession. macOS 12
  can't, so there it is always Safari 15. **This Mac can't stand in for macOS 12**, even for the
  probe app's browser engine.
- **For the product:** on macOS 14+, users with Chrome or another default browser get that
  browser's login window, with that browser's cookies, autofill and extensions. Any ADR 15
  wording about "Safari" on macOS should say "the system's web authentication session".

### F9 — First non-ephemeral session via Chrome can fail at launch · **friction, flaky**

- **What happens:** the first three ephemeral-off attempts failed instantly with
  `EUNKNOWN, Couldn't communicate with a helper application.` Nothing reached the server. Minutes
  later, the same button with the same setting worked (after the H5 alert).
- **Not reproduced** with Safari as the default. The system log kept nothing from those seconds.
- **Likely cause:** the hand-off from AuthenticationServices to Chrome's provider failed on the
  first try. This is an unconfirmed guess.
- **Mitigation:** treat this error like "canceled": offer a retry, don't show it as a hard error.
  Prefer `prefersEphemeralWebBrowserSession = true`, which worked every time and has no H5 alert.

### H5 confirmed on macOS 14

- With ephemeral off, macOS shows *"p6_probe" Wants to Use "127.0.0.1" to Sign In* before the
  page loads, with the default browser's icon on it.
- With ephemeral on, there is no alert, but also no shared browser cookies, so there's no
  "already signed in" shortcut. For email-code login that costs nothing: there is no IdP session
  worth reusing.

## Round 4 — iOS 18.3 simulator rehearsal (not iOS 15/16), 2026-10-20

**Setup:**

| Item | Value |
|---|---|
| Device | iPhone SE (3rd gen) simulator, **iOS 18.3.1**, on the Intel Mac from round 3. This was the only runtime installed; the 15.5 and 16.4 runtimes are still downloading |
| Stack | Same as round 3. The simulator shares the Mac's network, so `http://127.0.0.1:8080` works unchanged |
| Probe build | Flutter 3.44.1 debug, `flutter_web_auth_2` 5.1.0 |
| Video | [`evidence/ios18-sim/p6-ios18-sim.mp4`](evidence/ios18-sim/p6-ios18-sim.mp4), all runs, 8 min |
| Event log | [`evidence/ios18-sim/spike-events.json`](evidence/ios18-sim/spike-events.json) |

### Results

| Run | Result | Browser leg | Evidence |
|---|---|---|---|
| A, ephemeral on | ✅ | 34 s | video |
| A, ephemeral on, **switch apps (H6)** | ✅ The user left the sheet twice: once to the Home screen, once to Safari to read the code in Mailpit. Each time, the sheet came back on the same page, and the login completed | 95 s | video, ~2:20–3:30 |
| A, ephemeral off (H5) | ✅ after the "wants to use … to Sign In" alert | 50 s (incl. the alert) | [1](evidence/ios18-sim/1-A-ephemeral-off-alert.png), [2](evidence/ios18-sim/2-A-ephemeral-off-success.webp) |
| B, "Return to Another" page | ✅ after one tap | 43 s | [3](evidence/ios18-sim/3-B-return-to-app-page.png), [4](evidence/ios18-sim/4-B-success.webp) |
| C, claimed https | ❌ refused before loading (expected, see F10) | — | [5](evidence/ios18-sim/5-C-needs-associated-domains.png) |

**H6 holds on iOS 18:** iOS doesn't cancel a pending ASWebAuthenticationSession while the user is
in another app. On a real iPhone the switch would be to the Mail app. This must be re-run on 15
and 16 before it counts.

### F10 — Claimed-https callbacks need Associated Domains (`webcredentials`) · **cost for variant C**

- **What iOS says (much clearer than macOS, see round 3):** *"Application with identifier
  digital.camer.another.p6Probe is not associated with domain auth.another.camer.digital. Using
  HTTPS callbacks requires Associated Domains using the `webcredentials` service type for
  auth.another.camer.digital."*
- **What variant C needs, on top of iOS ≥ 17.4 (H4):**
  1. the app's entitlement `com.apple.developer.associated-domains` = `webcredentials:auth.another.camer.digital`;
  2. that host serving `/.well-known/apple-app-site-association` over valid TLS. The stand-in
     already serves one with a `webcredentials` entry (`config/well-known`), but the real host
     fails TLS today (F4);
  3. a **paid** Apple Developer team, because free personal teams can't add the Associated
     Domains capability. The team ID also has to replace `TEAMID1234` in the AASA file.
- **For ADR 15:** iOS 15/16 can't use C at all (H4), so the app needs the private-use scheme
  anyway. C only adds value on 17.4+, and costs the three items above.

## Round 5 — Windows desktop, 2026-10-20

**Setup:**

| Item | Value |
|---|---|
| Machine | Ghislain's laptop, **Windows 11 Pro 24H2** (build 26100). Stands in for Windows 10: no Windows 10 machine was available |
| Stack | Native, no Docker (no admin rights, no WSL): Redis 8.10.2 and Mailpit 1.31.6 Windows builds, `idp` built with Rust 1.99.1 (MSVC). Started with [`scripts/windows-stack.ps1`](../scripts/windows-stack.ps1) |
| Client | `another-login` (release build), launched with [`scripts/windows-login.ps1`](../scripts/windows-login.ps1) |
| Security | Windows Firewall **on** for all profiles, with "notify when a new app is blocked"; Defender real-time protection on |
| Event log | [`evidence/windows/spike-events.json`](evidence/windows/spike-events.json) (runs 1–2), [`run0-spike-events.json`](evidence/windows/run0-spike-events.json) |

A person typed the email and the code. Screen recordings are kept by Ghislain (not in the repo).

| Run | Default browser | Result | Browser leg | Code sent → typed |
|---|---|---|---|---|
| Automatic (`--headless`) | — | ✅ | 0.1 s | — |
| `scripts/negative.sh` | — | ✅ 14/14 PASS | — | — |
| 0 · unobserved trial | Chrome 155 | ✅ ([log](evidence/windows/run0-cli.log)) | 306 s | 18 s |
| 1 · Edge | **Edge 155** (`Edg/155` in the User-Agent) | ✅ ([log](evidence/windows/run1-cli-edge.log)) | 121 s | 34 s (4 codes sent, see F13) |
| 2 · Chrome | **Chrome 155** | ✅ ([log](evidence/windows/run2-cli-chrome.log)) | 38 s | 25 s |

The loopback flow works on Windows with both browsers, unchanged from Linux. In each run the CLI
opened the console, then the browser came up on top of it at our email page.

### F11 — No firewall prompt on Windows · **H7 (firewall) killed**

- Listening on `127.0.0.1:<ephemeral>` didn't trigger Windows Firewall, with the firewall on and
  its notifications enabled.
- **Evidence:** no prompt seen in runs 1–2, and Windows holds no firewall rule for
  `another-login.exe`. Answering a prompt, either way, would have left one.
- **Keep it that way:** the core must bind `127.0.0.1` only. Binding `0.0.0.0` (or `[::]`) is what
  triggers the prompt.

### F12 — The app doesn't come back to the front on Windows · **UX, same as F7**

- After the code, the browser stays in front showing "You're signed in" and the CLI's console stays
  behind it. Seen with Edge and Chrome.
- Windows' foreground lock stops a background process from raising itself, as on GNOME/Wayland.
- **Fix (as F7):** for the Flutter app, try `window_manager.focus()` / `SetForegroundWindow`
  (Windows usually turns it into a taskbar flash), plus a toast notification ("Signed in — return
  to Another"). Make the tab's text say "return to Another" rather than suggesting it closes itself.

### F13 — Users resend the code · **UX, for the login pages**

- In run 1 four codes were sent within 50 s (12:25:48, 12:26:29, 12:26:33, 12:26:38). The one that
  worked was the last. The tester confirmed pressing "Send a new code" several times; Mailpit
  delivers instantly, so it wasn't a slow email. Nothing stops a user from doing the same.
- Each send replaces the previous code, so a user who types the code from an earlier email gets
  "That code isn't right". Four sends also use 4 of the 5-per-15-min allowance.
- **Consider for `authz-ui`:** keep the last few codes valid until expiry, or disable "Send a new
  code" for ~30 s with a countdown.

### F14 — 5 minutes in the browser is real · **supports F6**

- Run 0 spent 306 s in the browser, run 1 121 s. The CLI waits as long as needed, but
  `flutter_web_auth_2`'s default timeout is 5 minutes. Run 0 would have been reported as
  "canceled" by the plugin.
- **Why it matters:** if the app stops waiting before the server does, the user can still finish
  in the browser, but the redirect hits a closed port. The login is lost silently, like F1.
- **Fix:** the app should wait as long as the server keeps the login open: 15 min (the
  authorization request's lifetime). Show a "Cancel" button in the app so a user who gave up
  isn't stuck waiting. Or let the core own the loopback listener (no plugin on desktop).

### Windows notes (not findings about the flow)

- **Default browser lives in a new place on Windows 11 24H2.** `UrlAssociations\https\UserChoice`
  kept saying Edge while `UserChoiceLatest\ProgId` said Chrome, and Chrome was what opened. Anything
  that reports "which browser" must read `UserChoiceLatest` first. The User-Agent on `/spike` is the
  reliable record.
- **Windows PowerShell 5.1 garbles the CLI's `→` and `✓`** unless the console is switched to UTF-8,
  and wraps stderr lines in `NativeCommandError`. Cosmetic; the launcher handles both.
- **Edge runs in the background even with no window open** (Startup boost). With Edge as the default,
  the browser opened fast because it was already running.

## Next

- Real phones: Transsion with its stock browser (H2); low-RAM hardware for natural H1.
- **Parked (2026-10-20):** macOS 12 and iOS 15/16.
  - macOS 12: no machine available (F8: macOS 14 is not a stand-in). When one turns up, copy the built `.app` and the CLI over.
  - iOS 15/16 simulators: the runtimes (~7 GB each) were too slow to download on the office connection. Get them over a faster network (Xcode → Settings → Components), then repeat round 4's table on each, H6 and C (H4) especially. Real iPhones still to find.
- Windows: one run on a real Windows 10 22H2 machine or VM to confirm round 5 (Windows 11 was the
  stand-in). The probe app with button D (`useWebview: false`) on Windows needs Flutter installed;
  the C++ Build Tools are already on Ghislain's laptop.
- Variant C waits on F4.
