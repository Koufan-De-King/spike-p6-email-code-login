# Mac setup — macOS 12 and iOS 15/16 runs

For the Intel Mac. Everything else this round needs is in the repo. Results go into
[findings.md](findings.md) as round 3 (macOS) and round 4 (iOS).

## 0. Check the macOS version first

 → About This Mac.

| Your Mac runs | Can build the Flutter app? | Can be the macOS 12 test machine? |
|---|---|---|
| macOS **13.5 or newer** | Yes | No: use a macOS 12 VM (Parallels / VMware Fusion on Intel) or another Mac, and copy the built `.app` over |
| macOS **12** | No: Flutter 3.47 needs Xcode ≥ 15, which won't install on 12. The Rust CLI still builds | Yes: build the `.app` on a newer Mac, copy it here |

The probe targets **iOS 15.0** and **macOS 12.0**, which are Flutter 3.47's own minimums. Our
oldest targets are right at the floor.

## 1. Get the repo and start the stack

The repo is private, so the Mac needs GitHub access (`gh auth login`, or an SSH key on your account).

```bash
git clone git@github.com:Koufan-De-King/spike-p6-email-code-login.git
cd spike-p6-email-code-login
docker compose up -d --build          # Docker Desktop for Mac, Intel build
docker compose run --rm e2e           # should end with "✓ signed in as e2e@example.cm"
```

Then open http://127.0.0.1:8080/spike (the timeline) and http://127.0.0.1:8025 (Mailpit, where
the codes land).

**Run the stack on the Mac itself.** The iOS simulator shares the Mac's network, so
`http://127.0.0.1:8080` works unchanged from macOS apps, the CLI and the simulator.

## 2. Tools

| Tool | How |
|---|---|
| Xcode | App Store (or developer.apple.com for an older one). Open it once to accept the licence and install components. Then Xcode → Settings → **Components**: add the **iOS 15** and **iOS 16** simulator runtimes |
| Flutter | `git clone --depth 1 -b stable https://github.com/flutter/flutter.git ~/development/flutter`, add `~/development/flutter/bin` to `PATH`, run `flutter doctor` and fix what it lists for **Xcode** (CocoaPods if it asks: `brew install cocoapods`). Android/Chrome lines can be ignored here |
| Rust | https://rustup.rs |

Pin the same Flutter as the Linux run if `flutter doctor` reports a different stable:
`git -C ~/development/flutter checkout 3.47.7`.

## 3. macOS 12 runs

### 3a. Rust CLI: the loopback flow, as on Windows/Linux (H8)

```bash
cargo run -p another-login -- --platform macos12-cli-safari
```

It opens the default browser on our login page. Type an email, get the code from Mailpit, and
type it in. The terminal should print `✓ signed in as …`.

Then repeat with Chrome as the default browser (`--platform macos12-cli-chrome`).

**Watch for (H8):**
- whether Safari 15 warns or refuses when an `https`/`http` page redirects to `http://127.0.0.1:<port>`;
- whether the CLI's "You're signed in" tab appears.

### 3b. Probe app: ASWebAuthenticationSession

```bash
cd probe_app
flutter run -d macos --dart-define=P6_ISSUER=http://127.0.0.1:8080
```

To run on a macOS 12 machine you didn't build on:
`flutter build macos --dart-define=P6_ISSUER=…`, copy `build/macos/Build/Products/Release/p6_probe.app`
over, and right-click → Open the first time (it's unsigned).

| Run | What to do | Watch for |
|---|---|---|
| A, ephemeral **on** | Click A | A sign-in sheet attached to the app window, our page, then ✓ |
| A, ephemeral **off** | Toggle off, click A | The system alert *"p6_probe wants to use 127.0.0.1 to sign in"* (H5). Film it |
| B | Click B | "Return to Another", then tap → ✓ |
| C | Click C | Expected to fail. The real host fails TLS today (F4) |

The plugin on macOS uses ASWebAuthenticationSession, not loopback, so button D is hidden there.

## 4. iOS 15 / 16 runs

### 4a. Simulator

```bash
cd probe_app
open -a Simulator                      # File → Open Simulator → pick an iOS 15.x device, then a 16.x one
flutter devices                        # note the simulator id
flutter run -d <simulator-id> --dart-define=P6_ISSUER=http://127.0.0.1:8080
```

| Run | What to do | Watch for |
|---|---|---|
| A, ephemeral on | Read the code from Mailpit on the Mac, type it in the sheet | ✓ in the app |
| A, **switch apps** (H6) | After the email step, swipe home, open Safari in the simulator, open `http://127.0.0.1:8025`, read the code, switch back to p6_probe | Is the sign-in sheet still there? If iOS cancelled it, **H6 is a blocker** |
| A, ephemeral off (H5) | Toggle off | The "wants to use … to Sign In" alert |
| C (H4) | Click C | Expected to fail on iOS < 17.4. Film the failure |

Record videos with `xcrun simctl io booted recordVideo p6-ios16-A.mov` (Ctrl-C to stop).

### 4b. Real iPhone (if you have one on 15.x or 16.x)

iPhones can't be downgraded. An iPhone 6s/7 tops out at iOS 15, an iPhone 8/X at iOS 16.

1. Connect it by cable. In Xcode → Settings → Accounts add your Apple ID.
2. Open `probe_app/ios/Runner.xcworkspace`. Under Runner → Signing & Capabilities, pick your
   personal team. If the bundle id `digital.camer.another.p6Probe` is taken, change it to
   something unique.
3. On iOS 16: Settings → Privacy & Security → **Developer Mode** on, then restart.
4. The phone can't reach `127.0.0.1` on the Mac. Restart the stack with the Mac's Wi-Fi IP:
   `ISSUER=http://<mac-ip>:8080 docker compose up -d`, then
   `flutter run -d <iphone> --dart-define=P6_ISSUER=http://<mac-ip>:8080`.
5. The first launch may say "Untrusted Developer": Settings → General → VPN & Device Management →
   trust your Apple ID. A free account's install expires after 7 days.

Run the same table as the simulator. On a real phone, H6 can also use the real Mail app if the
code reaches a real inbox.

## 5. After each session

```bash
curl -s http://127.0.0.1:8080/spike/events.json > docs/evidence/<platform>/spike-events.json
```

Put screenshots or videos in `docs/evidence/<platform>/`. Add a round to `findings.md`: what ran,
✅/❌, and anything that surprised you. Then commit and push.

## Already fixed in the repo (don't redo)

- `macos/Runner/*.entitlements` include `com.apple.security.network.client`. Without it, the
  sandboxed app can't make the token request.
- `useWebview: false` and the 15-minute timeout (Linux/Windows only; harmless on Apple platforms).
- `P6_ISSUER` build-time default for the issuer field.
