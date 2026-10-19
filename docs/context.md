# Context — read this first

For Ghislain (and anyone joining P6). Written 2026-10-19. The full session this came out of is in
[transcript.md](transcript.md).

## The question

Spike P6: can a user sign in to the Another app with a **one-time code sent by email**, entered
in the **phone's or computer's own browser** (not a web view inside the app), on **iOS 15/16,
Android 8/9, Windows 10 and macOS 12**?

**Done when:** a video per OS, plus a list of whatever blocks it. The answer feeds ADR 15. The
spike is boxed at 3 days. The readout is Friday Oct 23.

## What this repo is, and what it is not

Everything here is **disposable test equipment**, written from scratch for the spike.

| Folder | What it is | What it is **not** |
|---|---|---|
| `idp/` | A **stand-in** login server. Outwardly it behaves like `authz-idp`: same URLs, same rules for which redirect addresses it accepts, PKCE required. It does send the email (into Mailpit, a fake inbox) and does check the code: expiry, 5 tries, rate limits, single use | Not a copy of `authz-idp`. No code is shared with `lightbridge-authz`, which was not touched. No Postgres, no Keycloak, no React login pages; users live in Redis |
| `another-login/` | A small command-line program that logs in the way the **desktop** app will: opens the browser, waits on `127.0.0.1` for the redirect, swaps the code for tokens, checks the ID token. `--headless` runs the whole thing automatically | Not the app's Rust core. One file, no Flutter bridge, no token storage. Use it as a reference for the core |
| `probe_app/` | A throwaway Flutter app with three buttons, for **phones** (and macOS). It opens the system browser to log in and shows whether the phone hands control back to the app | Not the Flutter–Rust interface. It does its own login maths in Dart. **Never compiled**: there was no Flutter SDK on the machine it was written on |
| `docs/architecture.md` | The design, plus **H1–H11**, the list of things we suspect might break, each with a test | — |
| `docs/test-protocol.md` | What to film on each OS | Written before the logistics below were worked out. Still to be updated |

None of the real pieces have been started yet: email codes in the real `authz-idp`, or login in
the real Rust core. The spike exists to find out what they must handle before anyone builds them.

## The method

The unknowns are on the **devices**, not the server:

- Does the browser hand control back to the app?
- Does Android kill the app while the user goes to read their email?
- Does iOS 15 support the kind of callback we want?

To find out, we need a server that behaves like the real one **today**, and the real one doesn't
do email codes yet. So:

1. Run the stand-in somewhere every device can reach.
2. Log in from each device.
3. Film it. Every login also shows up, step by step with timings and the browser's identity, at
   `<server>/spike`.
4. Write down what worked, what broke, and the fix. Then throw the stand-in away.

**Limitation:** the real login pages are a React app; the stand-in's are plain HTML, on purpose,
so a failure can be pinned on the browser rather than on JavaScript. A device that passes here
should get one confirmation run against the real pages.

## What has been verified (on Koufan's Linux laptop)

- The whole login runs end to end, with the code read from Mailpit, both inside Docker
  (`docker compose run --rm e2e`) and from the host.
- `scripts/negative.sh`: 14 checks pass. They cover `localhost` refused, wrong callback path
  refused, plain PKCE refused, missing browser cookie refused, wrong verifier refused, code reuse
  refused, and lockout after 5 wrong codes.
- If Redis restarts mid-session, the server answers 503 once and then recovers. It no longer
  crashes.

Nothing has been run on a phone, on Windows or on a Mac yet.

## How the testing will work (agreed in principle, not set up yet)

- **The CLI never goes on a phone.**
  - Phones run the test app.
  - Windows 10 and macOS 12 run the CLI.
  - One laptop runs the server.
- **Reaching the server:** a Cloudflare quick tunnel gives the laptop a public `https://` address.
  - HTTPS avoids iOS and Android blocking plain-`http` calls from the app.
  - It also lets phones test over mobile data.
  - Later, the stand-in can be deployed on `auth.another.camer.digital`.
- **Reading the code:** emails only reach Mailpit. Read the code on the laptop, or open Mailpit in
  the phone's browser. Switching to the browser backgrounds the app just like switching to Gmail
  would, so it is good enough for the "app gets killed" test.
- **iPhone:**
  - It needs a Mac with Xcode + Flutter, installed over a cable. A free Apple ID works; the app
    expires after 7 days, and iOS 16 needs Developer Mode on.
  - iPhones can't be downgraded, so we need phones stuck on old versions: an iPhone 6s or 7
    tops out at iOS 15, an iPhone 8 or X at iOS 16.
- **Android:**
  - Build an `.apk`, send it to the phone and install it.
  - The emulator (API 26/28) is fine for the "Don't keep activities" test.
  - The non-Chrome default browser test needs a real cheap phone (Tecno, Infinix, itel).
- **Windows / macOS:** build `another-login` with `cargo` on the machine, or download it from CI
  once a workflow exists. A Mac stuck on macOS 12 probably can't build the Flutter app; build it
  on a newer Mac and copy it over.

## Not done yet / open

- [ ] Get `probe_app` to compile (Android first; needs no Apple account).
- [ ] Add the tunnel to `compose.yaml`.
- [ ] CI workflow that builds the CLI (Windows/macOS/Linux) and the Android `.apk`.
- [ ] Rewrite `docs/test-protocol.md` with the logistics above.
- [ ] Find the devices: iPhone on 15, iPhone on 16, Android 8, Android 9 (ideally Transsion),
      Windows 10, macOS 12.
- [ ] Move the repo to `vymalo`. Koufan's account can't create repos there; an org admin must
      create it or accept a transfer.

**Decisions ADR 15 will need, whatever the videos show:**

- With email codes, `authz-idp` logs people in **itself**, without Keycloak. How does that fit
  with its current Keycloak brokering (ADR-0019)?
- Today `authz-idp` **refuses** a login from someone with no account (ADR-0024). Email-code login
  creates new people, so first-login account creation has to happen on this path.

## Quick start

```bash
docker compose up -d --build        # server on :8080, Mailpit on :8025
docker compose run --rm e2e         # automatic end-to-end check
./scripts/negative.sh               # refusal checks
cargo run -p another-login          # log in with your real browser; the code is in Mailpit
```

Then look at http://127.0.0.1:8080/spike and http://127.0.0.1:8025.
