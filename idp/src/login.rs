//! The hosted login pages: enter an email, receive a six-digit code, type it back.

use std::{collections::HashMap, net::SocketAddr};

use axum::{
    Form,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use hmac::{Hmac, Mac};
use lettre::{AsyncTransport, Message};
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::{
    Shared, events,
    html::{self, esc},
    oauth::{self, Session, Txn, cookie, now_s},
};

#[derive(Serialize, Deserialize)]
struct Otp {
    email: String,
    digest: String,
    sent_at: u64,
}

fn mac(state: &Shared, parts: &[&str]) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(&state.pepper).unwrap();
    for p in parts {
        m.update(p.as_bytes());
        m.update(&[0x1f]);
    }
    m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn normalize_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_lowercase();
    let (local, domain) = e.split_once('@')?;
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && e.len() <= 254
        && !e.contains(char::is_whitespace)
        && !domain.contains('@');
    ok.then_some(e)
}

fn mask(email: &str) -> String {
    match email.split_once('@') {
        Some((l, d)) => format!("{}•••@{}", l.chars().next().unwrap_or('•'), d),
        None => "your email".into(),
    }
}

async fn load_txn(state: &Shared, id: &str) -> Option<Txn> {
    let mut r = state.redis.clone();
    let raw: Option<String> = r.get(format!("p6:txn:{id}")).await.ok()?;
    serde_json::from_str(&raw?).ok()
}

/// The transaction must exist and, unless disabled for an experiment, be bound to this browser.
async fn guard(state: &Shared, headers: &HeaderMap, id: &str) -> Result<Txn, Response> {
    let Some(txn) = load_txn(state, id).await else {
        return Err(html::error(StatusCode::GONE, "This sign-in link has expired."));
    };
    if state.cfg.require_txn_cookie && cookie(headers, "p6_txn").as_deref() != Some(id) {
        events::record(state, &txn.flow, "login.txn_cookie_missing", headers, json!({})).await;
        return Err(html::error(
            StatusCode::BAD_REQUEST,
            "This browser lost the sign-in cookie (finding: the browser dropped or isolated cookies between pages).",
        ));
    }
    Ok(txn)
}

fn email_form(id: &str, hint: &str, err: Option<&str>) -> String {
    format!(
        r#"<h1>Sign in to Another</h1><p>We'll email you a six-digit code.</p>
{}<form method=post action="/login/{id}/email">
<label for=email>Email address</label>
<input id=email name=email type=email inputmode=email autocomplete=email autocapitalize=none spellcheck=false required autofocus value="{}">
<button type=submit>Email me a code</button></form>"#,
        err.map(|e| format!("<p class=err>{}</p>", esc(e))).unwrap_or_default(),
        esc(hint),
    )
}

pub async fn email_page(State(state): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let txn = match guard(&state, &headers, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    events::record(&state, &txn.flow, "login.email_page", &headers, json!({})).await;
    html::respond(StatusCode::OK, "Sign in", &email_form(&id, txn.login_hint.as_deref().unwrap_or(""), None))
}

#[derive(Deserialize)]
pub struct EmailForm {
    email: String,
}

async fn bump(state: &Shared, key: String, window: u64) -> u64 {
    let mut r = state.redis.clone();
    let n: u64 = r.incr(&key, 1).await.unwrap_or(u64::MAX);
    if n == 1 {
        let _: Result<(), _> = r.expire(&key, window as i64).await;
    }
    n
}

pub async fn submit_email(
    State(state): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<EmailForm>,
) -> Response {
    let txn = match guard(&state, &headers, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let Some(email) = normalize_email(&f.email) else {
        return html::respond(StatusCode::BAD_REQUEST, "Sign in", &email_form(&id, &f.email, Some("That doesn't look like an email address.")));
    };

    let ip = oauth::client_ip(&state, &headers, peer);
    let per_email = bump(&state, format!("p6:rl:email:{}", mac(&state, &[&email])), 15 * 60).await;
    let per_ip = bump(&state, format!("p6:rl:ip:{ip}"), 3600).await;
    if per_email > state.cfg.send_limit_per_email || per_ip > state.cfg.send_limit_per_ip {
        events::record(&state, &txn.flow, "login.rate_limited", &headers, json!({"per_email": per_email, "per_ip": per_ip})).await;
        return html::respond(StatusCode::TOO_MANY_REQUESTS, "Sign in", &email_form(&id, &email, Some("Too many codes requested. Wait a few minutes and try again.")));
    }

    let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32));
    let otp = Otp { email: email.clone(), digest: mac(&state, &[&id, &code]), sent_at: now_s() };
    let mut r = state.redis.clone();
    let stored: redis::RedisResult<()> = r.set_ex(format!("p6:otp:{id}"), serde_json::to_string(&otp).unwrap(), state.cfg.otp_ttl_secs).await;
    if let Err(e) = stored {
        tracing::error!(error = %e, "redis");
        return html::error(StatusCode::SERVICE_UNAVAILABLE, "Sign-in is temporarily unavailable. Try again in a moment.");
    }
    let _: Result<(), _> = r.del(format!("p6:otp-attempts:{id}")).await;

    // The code leads the subject so it reads from the lock-screen notification: on a low-RAM
    // Android phone, every app switch to the mail client risks the OS killing our app.
    let msg = Message::builder()
        .from(state.cfg.mail_from.parse().unwrap())
        .to(match email.parse() {
            Ok(m) => m,
            Err(_) => return html::respond(StatusCode::BAD_REQUEST, "Sign in", &email_form(&id, &email, Some("That address can't receive email."))),
        })
        .subject(format!("{code} is your Another sign-in code"))
        .body(format!(
            "Your Another sign-in code is:\n\n    {code}\n\nIt expires in {} minutes. If you didn't try to sign in, ignore this email.\n",
            state.cfg.otp_ttl_secs / 60
        ))
        .unwrap();
    let t0 = events::now_ms();
    if let Err(e) = state.mailer.send(msg).await {
        events::record(&state, &txn.flow, "login.smtp_failed", &headers, json!({"error": e.to_string()})).await;
        return html::error(StatusCode::BAD_GATEWAY, "We couldn't send the email. Try again in a moment.");
    }
    events::record(&state, &txn.flow, "login.code_sent", &headers, json!({"smtp_ms": events::now_ms() - t0})).await;
    Redirect::to(&format!("/login/{id}/code")).into_response()
}

fn code_form(id: &str, email: &str, err: Option<&str>) -> String {
    format!(
        r#"<h1>Check your email</h1><p>We sent a code to <b>{}</b>. It's in the subject line, so you can read it from the notification.</p>
{}<form method=post action="/login/{id}/code">
<label for=code>Six-digit code</label>
<input id=code class=code name=code inputmode=numeric pattern="[0-9 ]*" maxlength=7 autocomplete=one-time-code required autofocus>
<button type=submit>Sign in</button></form>
<form method=post action="/login/{id}/email"><input type=hidden name=email value="{}"><button class=link type=submit>Send a new code</button></form>
<p class=muted><a href="/login/{id}">Use a different email</a></p>"#,
        esc(&mask(email)),
        err.map(|e| format!("<p class=err>{}</p>", esc(e))).unwrap_or_default(),
        esc(email),
    )
}

pub async fn code_page(State(state): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let txn = match guard(&state, &headers, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let mut r = state.redis.clone();
    let raw: Option<String> = r.get(format!("p6:otp:{id}")).await.unwrap_or(None);
    let Some(otp) = raw.and_then(|s| serde_json::from_str::<Otp>(&s).ok()) else {
        return Redirect::to(&format!("/login/{id}")).into_response();
    };
    events::record(&state, &txn.flow, "login.code_page", &headers, json!({})).await;
    html::respond(StatusCode::OK, "Enter code", &code_form(&id, &otp.email, None))
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

pub async fn submit_code(State(state): State<Shared>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<CodeForm>) -> Response {
    let txn = match guard(&state, &headers, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let mut r = state.redis.clone();
    let raw: Option<String> = r.get(format!("p6:otp:{id}")).await.unwrap_or(None);
    let Some(otp) = raw.and_then(|s| serde_json::from_str::<Otp>(&s).ok()) else {
        return html::respond(StatusCode::GONE, "Sign in", &email_form(&id, "", Some("That code expired. Request a new one.")));
    };

    let attempts = bump(&state, format!("p6:otp-attempts:{id}"), state.cfg.otp_ttl_secs).await;
    if attempts > state.cfg.otp_max_attempts as u64 {
        let _: Result<(), _> = r.del(format!("p6:otp:{id}")).await;
        events::record(&state, &txn.flow, "login.code_locked", &headers, json!({"attempts": attempts})).await;
        return html::respond(StatusCode::TOO_MANY_REQUESTS, "Sign in", &email_form(&id, &otp.email, Some("Too many wrong codes. Request a new one.")));
    }

    let typed: String = f.code.chars().filter(|c| !c.is_whitespace()).collect();
    let digest = mac(&state, &[&id, &typed]);
    if !bool::from(digest.as_bytes().ct_eq(otp.digest.as_bytes())) {
        events::record(&state, &txn.flow, "login.code_wrong", &headers, json!({"attempts": attempts})).await;
        let left = state.cfg.otp_max_attempts as u64 - attempts;
        return html::respond(StatusCode::BAD_REQUEST, "Enter code", &code_form(&id, &otp.email, Some(&format!("That code isn't right. {left} tries left."))));
    }

    complete(&state, &headers, &id, txn, otp).await.unwrap_or_else(|e| e)
}

async fn complete(state: &Shared, headers: &HeaderMap, id: &str, txn: Txn, otp: Otp) -> Result<Response, Response> {
    let mut r = state.redis.clone();
    let _: () = crate::try_redis!(r.del(&[format!("p6:otp:{id}"), format!("p6:otp-attempts:{id}"), format!("p6:txn:{id}")]).await);

    // We own our subjects (lightbridge-authz ADR-0025): the first verified login mints an opaque id.
    let user_key = format!("p6:user:{}", mac(state, &[&otp.email]));
    let _: bool = crate::try_redis!(r.set_nx(&user_key, oauth::random_token(15)).await);
    let sub: String = crate::try_redis!(r.get(&user_key).await);
    let _: () = crate::try_redis!(r.set(format!("p6:user-email:{sub}"), &otp.email).await);

    events::record(state, &txn.flow, "login.code_ok", headers, json!({"seconds_since_sent": now_s().saturating_sub(otp.sent_at)})).await;
    let sess = Session { sub, email: otp.email, auth_time: now_s() };
    let session_cookie = oauth::new_session(state, &sess).await?;
    let clear_txn = oauth::set_cookie(state, "p6_txn", "", "/login", 0);
    oauth::finish(state, headers, txn, &sess, vec![session_cookie, clear_txn]).await
}

/// Where a claimed-HTTPS redirect lands when the OS did NOT hand it to the app — itself a finding.
pub async fn claimed_https_fallback(State(state): State<Shared>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    events::record(&state, "claimed-https", "claimed_https.fell_through", &headers, json!({"has_code": q.contains_key("code")})).await;
    html::respond(
        StatusCode::OK,
        "Open the app",
        "<h1>Almost there</h1><p>Your phone opened this page in the browser instead of the Another app. That means the app link isn't verified on this device.</p>",
    )
}
