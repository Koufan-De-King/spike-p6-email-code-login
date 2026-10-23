# spike-p6 — email-code login through the system browser

Throwaway prototype for **P6** (pre-start spikes, Oct 16–23): can `authz-idp` sign users in with an
emailed one-time code, through the system browser, on iOS 15/16, Android 8/9, Windows 10 and
macOS 12? Feeds ADR 15.

- **New here? Start with [docs/context.md](docs/context.md).**
- Design and hypotheses: [docs/architecture.md](docs/architecture.md)
- What to film per OS: [docs/test-protocol.md](docs/test-protocol.md)
- Run the tests yourself, by hand: [docs/manual-testing.md](docs/manual-testing.md) (Linux, Android emulators); Mac: [docs/mac-setup.md](docs/mac-setup.md)

## What's here

| Path | What |
|---|---|
| `idp/` | Rust/axum IdP with `authz-idp`'s public surface: discovery, JWKS, `/authorize` (S256 required, RFC 8252 loopback rule), email → code pages (no JS), `/oauth2/token` (code + refresh), `/oauth2/userinfo`, and `/spike`, a per-flow evidence timeline |
| `another-login/` | Rust native client: the desktop loopback flow the core's `auth` module will run (ephemeral `127.0.0.1` port, PKCE, state, nonce, `iss` check, ID-token verification). `--headless` drives the whole flow, reading the code from Mailpit |
| `probe_app/` | Flutter probe for iOS/Android/macOS (ASWebAuthenticationSession, Custom Tabs) with three redirect variants. **Untested.** See its README |
| `scripts/negative.sh` | 14 checks on the refusal paths (`localhost`, wrong path, plain PKCE, missing cookie, wrong verifier, reuse, lockout) |

## Run

```bash
docker compose up -d --build        # idp on :8080, Mailpit on :8025
docker compose run --rm e2e         # headless end-to-end, through the real loopback hop
./scripts/negative.sh               # refusal paths
cargo run -p another-login          # same flow in your real browser; the code is in Mailpit
```

Then open http://127.0.0.1:8080/spike for the timeline and http://127.0.0.1:8025 for the mail.

**Windows without Docker:** unzip the Windows builds of Redis and Mailpit under
`%LOCALAPPDATA%\p6-tools\redis\` and `%LOCALAPPDATA%\p6-tools\mailpit\`, install Rust and the
Visual Studio C++ Build Tools, then:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows-stack.ps1                         # build + start (-Stop to stop)
powershell -ExecutionPolicy Bypass -File scripts\windows-login.ps1 -Platform win11-edge   # one recorded login
```

Knobs (env on `docker compose up`): `ISSUER`, `CUSTOM_SCHEME_REDIRECT=303|interstitial`,
`REQUIRE_TXN_COOKIE=true|false`. Apps can also pick the redirect style per run with
`spike_redirect_mode=interstitial` on `/authorize`; any `spike_*` parameter is logged on `/spike`.

The per-IP limit is 30 code sends an hour. If repeated test runs hit it, `docker compose restart redis` resets it.
