//! OAuth 2.0 / OIDC surface: discovery, JWKS, `/authorize`, `/oauth2/token`, `/oauth2/userinfo`.

use std::{collections::HashMap, net::SocketAddr};

use axum::{
    Form, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use rand::RngCore;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    Client, RedirectMode, Shared,
    events::{self, now_ms},
    html::{self, esc},
};

pub const TXN_TTL_SECS: u64 = 15 * 60;
const CODE_TTL_SECS: u64 = 60;
const SESSION_TTL_SECS: u64 = 12 * 3600;
const ACCESS_TTL_SECS: u64 = 15 * 60;
const REFRESH_TTL_SECS: u64 = 30 * 24 * 3600;

/// A pending authorization request, waiting for the user to prove an email address.
#[derive(Serialize, Deserialize, Clone)]
pub struct Txn {
    pub flow: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: String,
    pub scope: String,
    pub login_hint: Option<String>,
    pub redirect_mode: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CodeRec {
    flow: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    sub: String,
    email: String,
    nonce: Option<String>,
    scope: String,
    auth_time: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub sub: String,
    pub email: String,
    pub auth_time: u64,
}

#[derive(Serialize, Deserialize)]
struct RefreshRec {
    flow: String,
    client_id: String,
    sub: String,
    email: String,
    scope: String,
    auth_time: u64,
}

pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    B64.encode(b)
}

pub fn now_s() -> u64 {
    (now_ms() / 1000) as u64
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

pub fn session_cookie_name(state: &Shared) -> &'static str {
    if state.cfg.secure_cookies { "__Host-p6_session" } else { "p6_session" }
}

pub fn set_cookie(state: &Shared, name: &str, value: &str, path: &str, max_age: u64) -> HeaderValue {
    let secure = if state.cfg.secure_cookies { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}")).unwrap()
}

pub fn client_ip(state: &Shared, headers: &HeaderMap, peer: SocketAddr) -> String {
    if state.cfg.trust_forwarded_for {
        if let Some(ip) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()) {
            return ip.trim().to_string();
        }
    }
    peer.ip().to_string()
}

// ---------------------------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------------------------

pub async fn discovery(State(state): State<Shared>) -> Json<Value> {
    let iss = &state.cfg.issuer;
    Json(json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/authorize"),
        "token_endpoint": format!("{iss}/oauth2/token"),
        "userinfo_endpoint": format!("{iss}/oauth2/userinfo"),
        "jwks_uri": format!("{iss}/.well-known/jwks.json"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "scopes_supported": ["openid", "email", "offline_access"],
        "authorization_response_iss_parameter_supported": true,
    }))
}

pub async fn jwks(State(state): State<Shared>) -> Json<Value> {
    Json(state.keys.jwks())
}

/// `apple-app-site-association` / `assetlinks.json`, for the claimed-HTTPS redirect variant.
/// Served from `WELL_KNOWN_DIR` when the files exist there.
pub async fn app_association(State(state): State<Shared>, Path(file): Path<String>) -> Response {
    if !matches!(file.as_str(), "apple-app-site-association" | "assetlinks.json") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(dir) = &state.cfg.well_known_dir else { return StatusCode::NOT_FOUND.into_response() };
    match tokio::fs::read(dir.join(&file)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---------------------------------------------------------------------------------------------
// Redirect URI rule — exact match, or authz-idp's RFC 8252 §7.3 loopback carve-out
// (lightbridge-authz docs/rfc-8252-loopback-redirects.md, conditions 1–5).
// ---------------------------------------------------------------------------------------------

pub fn redirect_allowed(client: &Client, requested: &str) -> bool {
    if client.redirect_uris.iter().any(|r| r == requested) {
        return true;
    }
    // 1. public client only
    if !client.public {
        return false;
    }
    let Ok(req) = Url::parse(requested) else { return false };
    // 2 + 3. http on a loopback IP literal; `localhost` is refused
    let loopback = matches!(req.host(), Some(url::Host::Ipv4(ip)) if ip == std::net::Ipv4Addr::LOCALHOST)
        || matches!(req.host(), Some(url::Host::Ipv6(ip)) if ip == std::net::Ipv6Addr::LOCALHOST);
    if req.scheme() != "http" || !loopback {
        return false;
    }
    // 4. no fragment
    if req.fragment().is_some() {
        return false;
    }
    // 5. a registered loopback URI equal in everything but the port
    client.redirect_uris.iter().filter_map(|r| Url::parse(r).ok()).any(|reg| {
        reg.scheme() == req.scheme() && reg.host() == req.host() && reg.path() == req.path() && reg.query() == req.query()
    })
}

fn redirect_with(uri: &str, params: &[(&str, &str)]) -> String {
    let mut url = Url::parse(uri).expect("validated redirect_uri");
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    url.to_string()
}

fn authorize_error(state: &Shared, txn_like: (&str, Option<&str>), error: &str, desc: &str) -> Response {
    let (uri, st) = txn_like;
    let mut p = vec![("error", error), ("error_description", desc), ("iss", state.cfg.issuer.as_str())];
    if let Some(s) = st {
        p.push(("state", s));
    }
    Redirect::to(&redirect_with(uri, &p)).into_response()
}

// ---------------------------------------------------------------------------------------------
// /authorize
// ---------------------------------------------------------------------------------------------

pub async fn authorize(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let flow = random_token(9);
    let spike: serde_json::Map<String, Value> =
        q.iter().filter(|(k, _)| k.starts_with("spike_")).map(|(k, v)| (k.clone(), json!(v))).collect();

    // Until client_id and redirect_uri are both trusted, errors render here and never redirect.
    let Some(client) = q.get("client_id").and_then(|id| state.cfg.clients.get(id)) else {
        events::record(&state, &flow, "authorize.unknown_client", &headers, json!(spike)).await;
        return html::error(StatusCode::BAD_REQUEST, "Unknown client.");
    };
    let Some(redirect_uri) = q.get("redirect_uri").filter(|r| redirect_allowed(client, r)) else {
        events::record(&state, &flow, "authorize.bad_redirect_uri", &headers, json!({"redirect_uri": q.get("redirect_uri"), "spike": spike})).await;
        return html::error(StatusCode::BAD_REQUEST, "This redirect URI is not registered for the client.");
    };
    let st = q.get("state").map(String::as_str);
    let back = (redirect_uri.as_str(), st);

    if q.get("response_type").map(String::as_str) != Some("code") {
        return authorize_error(&state, back, "unsupported_response_type", "only response_type=code");
    }
    let challenge = q.get("code_challenge").filter(|c| (43..=128).contains(&c.len()));
    if q.get("code_challenge_method").map(String::as_str) != Some("S256") || challenge.is_none() {
        return authorize_error(&state, back, "invalid_request", "PKCE with S256 is required");
    }
    let scope = q.get("scope").cloned().unwrap_or_default();
    let scopes: Vec<&str> = scope.split_whitespace().collect();
    if !scopes.contains(&"openid") || scopes.iter().any(|s| !client.scopes.iter().any(|c| c == s)) {
        return authorize_error(&state, back, "invalid_scope", "openid is required; unknown scopes are refused");
    }

    let mut detail = json!({
        "client_id": client.client_id,
        "redirect_uri": redirect_uri,
        "redirect_kind": redirect_kind(redirect_uri),
        "prompt": q.get("prompt"),
    });
    detail.as_object_mut().unwrap().extend(spike.clone());

    let txn = Txn {
        flow: flow.clone(),
        client_id: client.client_id.clone(),
        redirect_uri: redirect_uri.clone(),
        state: q.get("state").cloned(),
        nonce: q.get("nonce").cloned(),
        code_challenge: challenge.unwrap().clone(),
        scope: scopes.join(" "),
        login_hint: q.get("login_hint").cloned(),
        redirect_mode: q.get("spike_redirect_mode").cloned(),
    };

    // An existing browser session skips the email step (SSO) unless the app asks for prompt=login.
    // In an ephemeral ASWebAuthenticationSession there is never one, by design.
    let prompt_login = q.get("prompt").is_some_and(|p| p.split_whitespace().any(|v| v == "login"));
    if !prompt_login {
        if let Some(sess) = load_session(&state, &headers).await {
            events::record(&state, &flow, "authorize.sso", &headers, detail).await;
            return finish(&state, &headers, txn, &sess, vec![]).await.unwrap_or_else(|e| e);
        }
    }

    let mut r = state.redis.clone();
    let id = random_token(18);
    let stored: Result<(), Response> = async {
        let _: () = crate::try_redis!(r.set_ex(format!("p6:txn:{id}"), serde_json::to_string(&txn).unwrap(), TXN_TTL_SECS).await);
        Ok(())
    }
    .await;
    if let Err(res) = stored {
        return res;
    }
    events::record(&state, &flow, "authorize.login_required", &headers, detail).await;

    // The cookie binds the transaction to this browser: the code form only works where it started.
    let mut res = Redirect::to(&format!("/login/{id}")).into_response();
    res.headers_mut().append(header::SET_COOKIE, set_cookie(&state, "p6_txn", &id, "/login", TXN_TTL_SECS));
    res
}

pub fn redirect_kind(uri: &str) -> &'static str {
    match Url::parse(uri).map(|u| u.scheme().to_string()).as_deref() {
        Ok("http") => "loopback",
        Ok("https") => "claimed_https",
        Ok(_) => "private_use_scheme",
        Err(_) => "invalid",
    }
}

async fn load_session(state: &Shared, headers: &HeaderMap) -> Option<Session> {
    let id = cookie(headers, session_cookie_name(state))?;
    let mut r = state.redis.clone();
    let raw: Option<String> = r.get(format!("p6:session:{id}")).await.ok()?;
    serde_json::from_str(&raw?).ok()
}

pub async fn new_session(state: &Shared, sess: &Session) -> Result<HeaderValue, Response> {
    let id = random_token(24);
    let mut r = state.redis.clone();
    let _: () = crate::try_redis!(r.set_ex(format!("p6:session:{id}"), serde_json::to_string(sess).unwrap(), SESSION_TTL_SECS).await);
    Ok(set_cookie(state, session_cookie_name(state), &id, "/", SESSION_TTL_SECS))
}

/// Mints the authorization code and sends the browser back to the app.
pub async fn finish(state: &Shared, headers: &HeaderMap, txn: Txn, sess: &Session, cookies: Vec<HeaderValue>) -> Result<Response, Response> {
    let code = random_token(32);
    let rec = CodeRec {
        flow: txn.flow.clone(),
        client_id: txn.client_id,
        redirect_uri: txn.redirect_uri.clone(),
        code_challenge: txn.code_challenge,
        sub: sess.sub.clone(),
        email: sess.email.clone(),
        nonce: txn.nonce,
        scope: txn.scope,
        auth_time: sess.auth_time,
    };
    let mut r = state.redis.clone();
    let _: () = crate::try_redis!(r.set_ex(format!("p6:code:{code}"), serde_json::to_string(&rec).unwrap(), CODE_TTL_SECS).await);

    let mut p = vec![("code", code.as_str()), ("iss", state.cfg.issuer.as_str())];
    if let Some(s) = &txn.state {
        p.push(("state", s));
    }
    let target = redirect_with(&txn.redirect_uri, &p);

    let kind = redirect_kind(&txn.redirect_uri);
    let mode = match kind {
        "private_use_scheme" => txn
            .redirect_mode
            .as_deref()
            .and_then(RedirectMode::parse)
            .unwrap_or(state.cfg.redirect_mode_custom_scheme),
        _ => RedirectMode::SeeOther,
    };
    events::record(state, &txn.flow, "code.issued", headers, json!({"redirect_kind": kind, "mode": format!("{mode:?}")})).await;

    let mut res = match mode {
        RedirectMode::SeeOther => Redirect::to(&target).into_response(),
        RedirectMode::Interstitial => html::respond(
            StatusCode::OK,
            "Signed in",
            &format!(
                "<h1>You're signed in</h1><p>Return to the Another app to continue.</p><a class=button href=\"{}\">Return to Another</a>",
                esc(&target)
            ),
        ),
    };
    for c in cookies {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    Ok(res)
}

// ---------------------------------------------------------------------------------------------
// /oauth2/token
// ---------------------------------------------------------------------------------------------

fn token_error(error: &str, desc: &str) -> Response {
    (StatusCode::BAD_REQUEST, [(header::CACHE_CONTROL, "no-store")], Json(json!({"error": error, "error_description": desc}))).into_response()
}

fn pkce_ok(verifier: &str, challenge: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        && B64.encode(Sha256::digest(verifier.as_bytes())) == challenge
}

pub async fn token(State(state): State<Shared>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    let mut r = state.redis.clone();
    let get = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
    match get("grant_type") {
        "authorization_code" => {
            // GETDEL: a code is redeemable exactly once, even under a race.
            let raw: Option<String> = r.get_del(format!("p6:code:{}", get("code"))).await.unwrap_or(None);
            let Some(rec) = raw.and_then(|s| serde_json::from_str::<CodeRec>(&s).ok()) else {
                return token_error("invalid_grant", "unknown, expired or already used code");
            };
            if rec.client_id != get("client_id") || rec.redirect_uri != get("redirect_uri") {
                events::record(&state, &rec.flow, "token.mismatch", &headers, json!({})).await;
                return token_error("invalid_grant", "client_id or redirect_uri does not match the code");
            }
            if !pkce_ok(get("code_verifier"), &rec.code_challenge) {
                events::record(&state, &rec.flow, "token.pkce_failed", &headers, json!({})).await;
                return token_error("invalid_grant", "PKCE verification failed");
            }
            let refresh = RefreshRec {
                flow: rec.flow.clone(),
                client_id: rec.client_id.clone(),
                sub: rec.sub.clone(),
                email: rec.email.clone(),
                scope: rec.scope.clone(),
                auth_time: rec.auth_time,
            };
            let body = mint(&state, &refresh, rec.nonce.as_deref(), true).await;
            events::record(&state, &rec.flow, "token.issued", &headers, json!({"grant": "authorization_code"})).await;
            body
        }
        "refresh_token" => {
            let raw: Option<String> = r.get_del(format!("p6:rt:{}", get("refresh_token"))).await.unwrap_or(None);
            let Some(rec) = raw.and_then(|s| serde_json::from_str::<RefreshRec>(&s).ok()) else {
                return token_error("invalid_grant", "unknown or already used refresh token");
            };
            if rec.client_id != get("client_id") {
                return token_error("invalid_grant", "client_id does not match");
            }
            events::record(&state, &rec.flow, "token.refreshed", &headers, json!({})).await;
            mint(&state, &rec, None, false).await
        }
        _ => token_error("unsupported_grant_type", "authorization_code or refresh_token"),
    }
}

async fn mint(state: &Shared, rec: &RefreshRec, nonce: Option<&str>, with_id_token: bool) -> Response {
    let now = now_s();
    let iss = &state.cfg.issuer;
    let access = state.keys.sign(&json!({
        "iss": iss, "sub": rec.sub, "aud": state.cfg.access_token_audience, "client_id": rec.client_id,
        "scope": rec.scope, "iat": now, "exp": now + ACCESS_TTL_SECS, "jti": random_token(12),
    }));
    let mut body = json!({
        "access_token": access, "token_type": "Bearer", "expires_in": ACCESS_TTL_SECS, "scope": rec.scope,
    });
    if with_id_token {
        body["id_token"] = json!(state.keys.sign(&json!({
            "iss": iss, "sub": rec.sub, "aud": rec.client_id, "azp": rec.client_id,
            "iat": now, "exp": now + ACCESS_TTL_SECS, "auth_time": rec.auth_time, "nonce": nonce,
            "email": rec.email, "email_verified": true, "amr": ["otp", "email"],
        })));
    }
    if rec.scope.split_whitespace().any(|s| s == "offline_access") {
        let rt = random_token(32);
        let mut r = state.redis.clone();
        let stored: redis::RedisResult<()> = r.set_ex(format!("p6:rt:{rt}"), serde_json::to_string(rec).unwrap(), REFRESH_TTL_SECS).await;
        if let Err(e) = stored {
            tracing::error!(error = %e, "redis");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "temporarily_unavailable"}))).into_response();
        }
        body["refresh_token"] = json!(rt);
    }
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

pub async fn userinfo(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let claims = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| state.keys.verify(t))
        .filter(|c| c["iss"] == state.cfg.issuer && c["exp"].as_u64().is_some_and(|e| e > now_s()));
    let Some(c) = claims else { return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")]).into_response() };
    let mut r = state.redis.clone();
    let email: Option<String> = r.get(format!("p6:user-email:{}", c["sub"].as_str().unwrap_or(""))).await.unwrap_or(None);
    Json(json!({"sub": c["sub"], "email": email, "email_verified": email.is_some()})).into_response()
}

