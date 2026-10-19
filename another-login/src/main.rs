//! `another-login`: the desktop half of the P6 spike, and the shape of the Rust core's
//! `auth::begin() → session()` on Windows and macOS.
//!
//! 1. bind an ephemeral port on 127.0.0.1 (RFC 8252 §7.3), make PKCE + state + nonce;
//! 2. open the system browser at `/authorize`;
//! 3. wait for the browser to hit `http://127.0.0.1:<port>/callback`;
//! 4. redeem the code, verify the ID token against JWKS, print who signed in and how long it took.
//!
//! `--headless` replaces the human and the browser with an HTTP client that reads the code from
//! Mailpit — the compose e2e test. It still goes through the real loopback listener.
//!
//! Usage:
//!   another-login [--issuer URL] [--client-id ID] [--platform LABEL] [--no-open]
//!   another-login --headless --email EMAIL [--mailpit URL]

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn rand_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    B64.encode(b)
}

struct Args {
    issuer: String,
    client_id: String,
    platform: String,
    open: bool,
    headless: bool,
    email: Option<String>,
    mailpit: String,
}

fn args() -> Args {
    let mut a = Args {
        issuer: std::env::var("ISSUER").unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
        client_id: "another-app".into(),
        platform: format!("cli-{}", std::env::consts::OS),
        open: true,
        headless: false,
        email: None,
        mailpit: std::env::var("MAILPIT_URL").unwrap_or_else(|_| "http://127.0.0.1:8025".into()),
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--issuer" => a.issuer = it.next().expect("--issuer URL"),
            "--client-id" => a.client_id = it.next().expect("--client-id ID"),
            "--platform" => a.platform = it.next().expect("--platform LABEL"),
            "--no-open" => a.open = false,
            "--headless" => a.headless = true,
            "--email" => a.email = Some(it.next().expect("--email EMAIL")),
            "--mailpit" => a.mailpit = it.next().expect("--mailpit URL"),
            other => panic!("unknown argument {other}"),
        }
    }
    a.issuer = a.issuer.trim_end_matches('/').to_string();
    a
}

#[tokio::main]
async fn main() {
    if let Err(e) = run(args()).await {
        eprintln!("✗ {e}");
        std::process::exit(1);
    }
}

async fn run(a: Args) -> Res<()> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?;
    let disco: Value = http.get(format!("{}/.well-known/openid-configuration", a.issuer)).send().await?.error_for_status()?.json().await?;
    if disco["issuer"] != a.issuer.as_str() {
        return Err(format!("discovery issuer {} ≠ {}", disco["issuer"], a.issuer).into());
    }

    // 1. Loopback listener on an OS-assigned port. 127.0.0.1, never `localhost` (RFC 8252 §8.3).
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());

    let verifier = rand_b64(48); // 64 chars, inside 43..=128
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let state = rand_b64(16);
    let nonce = rand_b64(16);

    let mut auth = Url::parse(disco["authorization_endpoint"].as_str().ok_or("no authorization_endpoint")?)?;
    auth.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &a.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", "openid email offline_access")
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "login")
        .append_pair("spike_platform", &a.platform);
    if let Some(e) = &a.email {
        auth.query_pairs_mut().append_pair("login_hint", e);
    }

    let started = Instant::now();
    eprintln!("→ listening on {redirect_uri}");

    // 2. Hand the URL to a browser — the user's, or the headless driver.
    let driver = if a.headless {
        let email = a.email.clone().ok_or("--headless needs --email")?;
        Some(tokio::spawn(headless_browser(auth.to_string(), email, a.mailpit.clone())))
    } else {
        eprintln!("→ opening the system browser at\n  {auth}");
        if a.open && webbrowser::open(auth.as_str()).is_err() {
            eprintln!("  (could not open a browser; open the URL above yourself)");
        }
        None
    };

    // 3. Wait for the redirect. Ignore anything that isn't `/callback` (favicon probes etc.).
    //    In headless mode a failed driver ends the wait instead of leaving us on the listener.
    let wait = tokio::time::timeout(Duration::from_secs(15 * 60), accept_callback(&listener));
    let params = match driver {
        None => wait.await??,
        Some(mut d) => tokio::select! {
            p = wait => { let p = p??; d.await??; p }
            r = &mut d => { r??; return Err("headless driver finished without a callback".into()) }
        },
    };
    let browser_leg = started.elapsed();

    if let Some(err) = params.get("error") {
        return Err(format!("authorization error: {err} {}", params.get("error_description").cloned().unwrap_or_default()).into());
    }
    if params.get("state") != Some(&state) {
        return Err("state mismatch — refusing the code".into());
    }
    if params.get("iss").map(String::as_str) != Some(a.issuer.as_str()) {
        return Err("iss parameter missing or wrong (RFC 9207) — refusing the code".into());
    }
    let code = params.get("code").ok_or("no code in the callback")?;

    // 4. Redeem and verify.
    let tok: Value = http
        .post(disco["token_endpoint"].as_str().ok_or("no token_endpoint")?)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", a.client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await?
        .json()
        .await?;
    if let Some(e) = tok.get("error") {
        return Err(format!("token error: {e} {}", tok["error_description"]).into());
    }
    let jwks: Value = http.get(disco["jwks_uri"].as_str().ok_or("no jwks_uri")?).send().await?.json().await?;
    let id = verify_jwt(tok["id_token"].as_str().ok_or("no id_token")?, &jwks)?;
    if id["iss"] != a.issuer.as_str() || id["aud"] != a.client_id.as_str() || id["nonce"] != nonce.as_str() {
        return Err("id_token iss/aud/nonce check failed".into());
    }

    println!("✓ signed in as {} (sub {})", id["email"].as_str().unwrap_or("?"), id["sub"].as_str().unwrap_or("?"));
    println!("  browser leg {:.1}s, total {:.1}s, refresh token: {}", browser_leg.as_secs_f64(), started.elapsed().as_secs_f64(), tok.get("refresh_token").is_some());
    Ok(())
}

async fn accept_callback(listener: &TcpListener) -> Res<HashMap<String, String>> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
        if url.path() != "/callback" {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let ok = params.contains_key("code");
        // What the user sees in the tab we leave behind. A desktop app should also raise its own
        // window here (Flutter: window_manager.focus()), because the browser keeps the foreground.
        let body = format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Another</title>\
             <body style='font:16px system-ui;text-align:center;padding:3rem'><h1>{}</h1><p>You can close this tab and return to Another.</p>",
            if ok { "You're signed in" } else { "Sign-in didn't finish" }
        );
        let _ = sock
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
            .await;
        return Ok(params);
    }
}

fn verify_jwt(token: &str, jwks: &Value) -> Res<Value> {
    let (input, sig) = token.rsplit_once('.').ok_or("malformed jwt")?;
    let (header, payload) = input.split_once('.').ok_or("malformed jwt")?;
    let header: Value = serde_json::from_slice(&B64.decode(header)?)?;
    if header["alg"] != "ES256" {
        return Err("unexpected alg".into());
    }
    let jwk = jwks["keys"].as_array().and_then(|ks| ks.iter().find(|k| k["kid"] == header["kid"])).ok_or("kid not in JWKS")?;
    let mut sec1 = vec![4u8];
    sec1.extend(B64.decode(jwk["x"].as_str().ok_or("x")?)?);
    sec1.extend(B64.decode(jwk["y"].as_str().ok_or("y")?)?);
    let key = VerifyingKey::from_sec1_bytes(&sec1)?;
    key.verify(input.as_bytes(), &Signature::from_slice(&B64.decode(sig)?)?)?;
    let claims: Value = serde_json::from_slice(&B64.decode(payload)?)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    if claims["exp"].as_u64().unwrap_or(0) <= now {
        return Err("id_token expired".into());
    }
    Ok(claims)
}

/// Plays the user: follows `/authorize`, submits the email, reads the code from Mailpit, submits it,
/// then follows the final redirect into our own loopback listener — like a browser would.
async fn headless_browser(authorize_url: String, email: String, mailpit: String) -> Res<()> {
    let b = reqwest::Client::builder().cookie_store(true).redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(20)).build()?;
    let base = Url::parse(&authorize_url)?;

    let res = b.get(&authorize_url).send().await?;
    let login = location(&base, &res).ok_or_else(|| format!("/authorize did not redirect to login ({})", res.status()))?;
    b.get(login.clone()).send().await?.error_for_status()?;

    let seen = mailpit_ids(&mailpit, &email).await?;
    let res = b.post(login.join(&format!("{}/email", login.path()))?).form(&[("email", email.as_str())]).send().await?;
    let code_page = location(&base, &res).ok_or_else(|| format!("email submit did not redirect ({})", res.status()))?;

    let code = read_code_from_mailpit(&mailpit, &email, &seen).await?;
    eprintln!("  (headless) code from Mailpit: {code}");

    let res = b.post(code_page).form(&[("code", code.as_str())]).send().await?;
    let back = location(&base, &res).ok_or_else(|| format!("code submit did not redirect ({})", res.status()))?;
    // The final hop to http://127.0.0.1:<port>/callback — our own listener answers it.
    reqwest::get(back).await?.error_for_status()?;
    Ok(())
}

fn location(base: &Url, res: &reqwest::Response) -> Option<Url> {
    res.status().is_redirection().then(|| res.headers().get("location")?.to_str().ok().and_then(|l| base.join(l).ok()))?
}

async fn mailpit_search(mailpit: &str, email: &str) -> Res<Vec<Value>> {
    let list: Value = reqwest::Client::new()
        .get(format!("{mailpit}/api/v1/search"))
        .query(&[("query", format!("to:{email}")), ("limit", "20".into())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(list["messages"].as_array().cloned().unwrap_or_default())
}

async fn mailpit_ids(mailpit: &str, email: &str) -> Res<HashSet<String>> {
    Ok(mailpit_search(mailpit, email).await?.iter().filter_map(|m| m["ID"].as_str().map(String::from)).collect())
}

/// The newest message for `email` that was not already in the inbox before we asked for a code.
async fn read_code_from_mailpit(mailpit: &str, email: &str, seen: &HashSet<String>) -> Res<String> {
    let re = regex::Regex::new(r"^(\d{6}) is your Another sign-in code$")?;
    for _ in 0..40 {
        for m in mailpit_search(mailpit, email).await? {
            if m["ID"].as_str().is_some_and(|id| seen.contains(id)) {
                continue;
            }
            if let Some(c) = m["Subject"].as_str().and_then(|s| re.captures(s)) {
                return Ok(c[1].to_string());
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!("no code for {email} in Mailpit").into())
}
