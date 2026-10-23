# Manual testing — bring everything up and test by hand

Plain steps for running the P6 tests yourself, on the Linux laptop. Other platforms have their
own guides:

| Platform | Guide |
|---|---|
| Linux (CLI + probe app), Android emulators | this page |
| macOS, iOS | [mac-setup.md](mac-setup.md) |
| Windows (no Docker) | `scripts/windows-stack.ps1` + `scripts/windows-login.ps1`; see round 5 in [findings.md](findings.md) and the README |

New terminals already have Flutter, Java and the Android tools on the `PATH` (`~/.flutter-dev-env.sh`
is sourced from `~/.bashrc`).

## 1. Start the stack (always first)

```bash
cd ~/dev/spike-p6-email-code-login && docker compose up -d --build
```

Check it's healthy. This should print `ok`:

```bash
curl -s http://127.0.0.1:8080/healthz
```

Keep two pages open:

- **Mailpit** (where the codes arrive): http://127.0.0.1:8025
- **Timeline** of every login step: http://127.0.0.1:8080/spike

## 2. Quick automatic checks (optional)

```bash
cd ~/dev/spike-p6-email-code-login && docker compose run --rm e2e
```

It should end with `✓ signed in as e2e@example.cm`.

```bash
cd ~/dev/spike-p6-email-code-login && ./scripts/negative.sh
```

All 14 lines should say `PASS`. The IdP allows 30 code emails per hour from one address. If
repeated runs hit that, reset it with `docker compose restart redis`.

## 3. Linux: the Rust CLI with your browser

This is how the desktop app's core logs in.

```bash
cd ~/dev/spike-p6-email-code-login && cargo run -p another-login -- --platform linux-cli-chrome
```

1. Your default browser opens on "Sign in to Another".
2. Enter any email.
3. Copy the code from Mailpit and enter it.

The terminal prints `✓ signed in as …`, and the tab says "You're signed in".

## 4. Linux: the Flutter probe app

```bash
cd ~/dev/spike-p6-email-code-login/probe_app && flutter run -d linux --dart-define=P6_ISSUER=http://127.0.0.1:8080
```

1. In the window, set *Platform label* (e.g. `linux-chrome`).
2. Click **D · loopback 127.0.0.1 (desktop)**.
3. Do the email and code steps in the browser.

The app shows ✓ with the email. Bring its window back yourself: on Wayland it can't raise itself
(F7).

## 5. Android emulator

### Boot it

Use `p6-android9`, or `p6-android8-x64` for Android 8. **Not** `p6-android8`: it's 32-bit and
the app won't install on it (F5). A window opens; wait for the home screen.

```bash
emulator -avd p6-android9 -no-snapshot-save &
```

### Build the app with the local issuer baked in

Redo this only after code changes.

```bash
cd ~/dev/spike-p6-email-code-login/probe_app && flutter build apk --debug --dart-define=P6_ISSUER=http://127.0.0.1:8080
```

### Install it and point the emulator's `127.0.0.1` at the laptop

The port forwarding resets every time the emulator restarts. Redo both commands after each boot.

```bash
adb install -r build/app/outputs/flutter-apk/app-debug.apk
```

```bash
adb reverse tcp:8080 tcp:8080 && adb reverse tcp:8025 tcp:8025
```

### A login by hand

1. Open **P6 probe** in the emulator and set the label (e.g. `android9-manual`).
2. Tap **A**.
3. In the Chrome tab, enter an email. Read the code from Mailpit on the laptop and type it in.

The app comes back with ✓.

- **B:** the tab stops on "Return to Another". Tap the button.
- **Chrome's first run:** on a fresh emulator, Chrome first shows its own welcome and terms
  screen. Accept it yourself. It may lose the login request (F3); tap A again if so.

### Reproduce H1, the blocker

1. Start a login with A and submit the email.
2. While you're on the code page, kill the app the way a low-RAM phone would:

   ```bash
   adb shell run-as digital.camer.another.p6_probe kill -9 $(adb shell pidof digital.camer.another.p6_probe)
   ```

3. Type the code.

The app reopens signed out. On its next start it shows "⚠ previous flow never completed". On
`/spike`, that flow stops at `code.issued` with no `token.issued` (F1).

### Or let the script drive it

The script takes a label and a button (A, B or C). Screenshots land in the current folder.

```bash
cd ~/dev/spike-p6-email-code-login && ./scripts/android-run.sh android9-auto A
```

Two optional hooks:

- `TAP_RETURN=1` taps B's button.
- `BEFORE_CODE='<command>'` runs between the email and the code, e.g. the kill above.

## 6. Reading the results

- **`/spike`:** each login is one block. A good one ends in `token.issued`. A failed one shows the
  step where it stopped.
- **Raw log, to keep as evidence:**

  ```bash
  curl -s http://127.0.0.1:8080/spike/events.json > ~/dev/spike-p6-email-code-login/docs/evidence/spike-events-$(date +%F).json
  ```

## 7. Put everything down

```bash
adb emu kill; pkill -x p6_probe; cd ~/dev/spike-p6-email-code-login && docker compose down
```

`adb emu kill` stops one emulator. Run it again if two are up.
