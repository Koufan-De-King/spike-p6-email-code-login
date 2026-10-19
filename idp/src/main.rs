//! P6 spike: an email one-time-code login for native apps, through the system browser.
//!
//! Throwaway code. It mirrors `authz-idp`'s public surface (discovery, JWKS, `/authorize`,
//! `/oauth2/token`, the RFC 8252 §7.3 loopback rule) so what the spike learns about each OS's
//! browser transfers to the real service, but it owns its users in Redis instead of Postgres and
//! renders its pages server-side instead of through the `authz-ui` SPA.

mod events;
mod html;
mod jwt;
mod login;
mod oauth;

use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Router,
    routing::{get, post},
};
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use rand::RngCore;
use redis::aio::ConnectionManager;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Client {
    pub client_id: String,
    /// Public clients (no secret) are the only kind a native app can be.
    #[serde(default = "yes")]
    pub public: bool,
    pub redirect_uris: Vec<String>,
    pub scopes: Vec<String>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct ClientsFile {
    clients: Vec<Client>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectMode {
    /// `303 See Other` straight from the code-form POST to the app's redirect URI.
    SeeOther,
    /// A "Return to Another" page with a link; the user's tap is the gesture some browsers want
    /// before they hand a private-use scheme to an app.
    Interstitial,
}

impl RedirectMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "303" | "see_other" => Some(Self::SeeOther),
            "interstitial" => Some(Self::Interstitial),
            _ => None,
        }
    }
}

pub struct Config {
    pub issuer: String,
    pub secure_cookies: bool,
    pub clients: HashMap<String, Client>,
    pub mail_from: String,
    pub otp_ttl_secs: u64,
    pub otp_max_attempts: u32,
    pub send_limit_per_email: u64,
    pub send_limit_per_ip: u64,
    pub require_txn_cookie: bool,
    pub redirect_mode_custom_scheme: RedirectMode,
    pub access_token_audience: String,
    pub well_known_dir: Option<PathBuf>,
    pub trust_forwarded_for: bool,
}

pub struct AppState {
    pub cfg: Config,
    pub redis: ConnectionManager,
    pub keys: jwt::Keys,
    pub mailer: AsyncSmtpTransport<Tokio1Executor>,
    /// HMAC key for code digests and email-derived Redis keys. Never logged.
    pub pepper: [u8; 32],
}

pub type Shared = Arc<AppState>;

/// A Redis failure ends the request with a 503 instead of a panic. `ConnectionManager` reconnects
/// in the background, so the next request after a Redis restart succeeds.
#[macro_export]
macro_rules! try_redis {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "redis");
                return Err($crate::html::error(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Sign-in is temporarily unavailable. Try again in a moment.",
                ));
            }
        }
    };
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name).map(|v| matches!(v.as_str(), "1" | "true" | "yes")).unwrap_or(default)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(env("RUST_LOG", "info")).init();

    let issuer = env("ISSUER", "http://127.0.0.1:8080").trim_end_matches('/').to_string();
    let clients_path = env("CLIENTS_FILE", "config/clients.json");
    let clients: ClientsFile = serde_json::from_str(
        &std::fs::read_to_string(&clients_path).unwrap_or_else(|e| panic!("{clients_path}: {e}")),
    )
    .expect("clients file is not valid JSON");

    let pepper = match std::env::var("OTP_PEPPER") {
        Ok(hex) if hex.len() == 64 => {
            let mut out = [0u8; 32];
            for (i, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("OTP_PEPPER is hex");
            }
            out
        }
        _ => {
            tracing::warn!("OTP_PEPPER unset: using a random one, so codes die with the process");
            let mut out = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut out);
            out
        }
    };

    let cfg = Config {
        secure_cookies: issuer.starts_with("https://"),
        issuer,
        clients: clients.clients.into_iter().map(|c| (c.client_id.clone(), c)).collect(),
        mail_from: env("MAIL_FROM", "Another <no-reply@another.camer.digital>"),
        otp_ttl_secs: env_num("OTP_TTL_SECS", 600),
        otp_max_attempts: env_num("OTP_MAX_ATTEMPTS", 5),
        send_limit_per_email: env_num("SEND_LIMIT_PER_EMAIL", 5),
        send_limit_per_ip: env_num("SEND_LIMIT_PER_IP", 30),
        require_txn_cookie: env_bool("REQUIRE_TXN_COOKIE", true),
        redirect_mode_custom_scheme: RedirectMode::parse(&env("CUSTOM_SCHEME_REDIRECT", "303"))
            .expect("CUSTOM_SCHEME_REDIRECT is 303 or interstitial"),
        access_token_audience: env("ACCESS_TOKEN_AUDIENCE", "another-api"),
        well_known_dir: std::env::var("WELL_KNOWN_DIR").ok().map(PathBuf::from),
        trust_forwarded_for: env_bool("TRUST_FORWARDED_FOR", false),
    };

    let redis = redis::Client::open(env("REDIS_URL", "redis://127.0.0.1:6379")).expect("REDIS_URL");
    let redis = ConnectionManager::new(redis).await.expect("redis unreachable");

    let mailer = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(env("SMTP_HOST", "127.0.0.1"))
        .port(env_num("SMTP_PORT", 1025))
        .build();

    let state: Shared = Arc::new(AppState { cfg, redis, keys: jwt::Keys::generate(), mailer, pepper });
    tracing::info!(issuer = %state.cfg.issuer, clients = state.cfg.clients.len(), "p6 idp starting");

    let app = Router::new()
        .route("/.well-known/openid-configuration", get(oauth::discovery))
        .route("/.well-known/jwks.json", get(oauth::jwks))
        .route("/.well-known/{file}", get(oauth::app_association))
        .route("/authorize", get(oauth::authorize))
        .route("/oauth2/token", post(oauth::token))
        .route("/oauth2/userinfo", get(oauth::userinfo))
        .route("/login/{txn}", get(login::email_page))
        .route("/login/{txn}/email", post(login::submit_email))
        .route("/login/{txn}/code", get(login::code_page).post(login::submit_code))
        .route("/app/callback", get(login::claimed_https_fallback))
        .route("/spike", get(events::dashboard))
        .route("/spike/events.json", get(events::json))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);

    let bind: SocketAddr = env("BIND", "0.0.0.0:8080").parse().expect("BIND");
    let listener = tokio::net::TcpListener::bind(bind).await.expect("bind");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}
