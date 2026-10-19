//! Server-rendered pages. Deliberately no JavaScript: the spike must tell browser behaviour apart
//! from SPA behaviour, and an old Android WebView-based browser should still get a working form.

use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<style>
:root{{--bg:#fafaf7;--fg:#1b1b18;--muted:#6b6b63;--line:#deddd5;--accent:#0b6b4f;--err:#a3271f}}
@media (prefers-color-scheme:dark){{:root{{--bg:#151513;--fg:#ecebe4;--muted:#9c9b92;--line:#33322d;--accent:#4fc39a;--err:#ef7a70}}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}}
main{{max-width:28rem;margin:0 auto;padding:2.5rem 1rem}}
h1{{font-size:1.5rem;margin:0 0 .5rem}}p{{margin:.5rem 0 1rem}}.muted{{color:var(--muted);font-size:.9rem}}
label{{display:block;font-weight:600;margin:1rem 0 .35rem}}
input{{width:100%;font:inherit;padding:.75rem;border:1px solid var(--line);border-radius:.5rem;background:transparent;color:inherit}}
input.code{{font-size:1.6rem;letter-spacing:.4em;text-align:center;font-variant-numeric:tabular-nums}}
button,.button{{display:block;width:100%;margin-top:1rem;padding:.8rem;font:inherit;font-weight:600;border:0;border-radius:.5rem;background:var(--accent);color:#fff;text-align:center;text-decoration:none}}
button.link{{background:none;color:var(--accent);padding:.4rem;font-weight:500}}
a{{color:var(--accent)}}.err{{color:var(--err)}}code{{font-size:.85em}}
table{{border-collapse:collapse;width:100%;font-size:.8rem;margin:.5rem 0 1.5rem}}td{{border-top:1px solid var(--line);padding:.3rem .4rem;vertical-align:top;word-break:break-all}}
td.ua{{color:var(--muted);max-width:18rem}}details{{margin:.5rem 0}}summary{{cursor:pointer}}
body:has(table) main{{max-width:72rem}}
</style></head><body><main>{body}</main></body></html>"#,
        title = esc(title),
    )
}

/// `form-action` is intentionally absent from the CSP: Chrome applies it to the redirect that
/// follows a form POST, and the code form's redirect goes to `127.0.0.1:<port>` or a private-use
/// scheme — `form-action 'self'` would block the very hop this spike exists to test.
pub fn respond(status: StatusCode, title: &str, body: &str) -> Response {
    let mut res = (status, axum::response::Html(page(title, body))).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    res
}

pub fn error(status: StatusCode, msg: &str) -> Response {
    respond(
        status,
        "Sign-in problem",
        &format!("<h1>Something went wrong</h1><p class=err>{}</p><p class=muted>Go back to the Another app and start signing in again.</p>", esc(msg)),
    )
}
