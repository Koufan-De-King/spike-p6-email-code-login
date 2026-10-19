//! The spike's evidence trail: every step of every flow, with the browser's User-Agent and
//! timings, kept in a capped Redis list and shown at `/spike`. Pair this with the screen
//! recording per OS and the report writes itself.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse},
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Shared, html::esc};

const KEY: &str = "p6:events";
const CAP: isize = 2000;

#[derive(Serialize, Deserialize, Clone)]
pub struct Event {
    pub ts_ms: u128,
    pub flow: String,
    pub step: String,
    pub ua: String,
    pub detail: Value,
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
}

pub async fn record(state: &Shared, flow: &str, step: &str, headers: &HeaderMap, detail: Value) {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();
    tracing::info!(flow, step, %detail, "event");
    let ev = Event { ts_ms: now_ms(), flow: flow.into(), step: step.into(), ua, detail };
    let mut r = state.redis.clone();
    let _: Result<(), _> = r.lpush(KEY, serde_json::to_string(&ev).unwrap()).await;
    let _: Result<(), _> = r.ltrim(KEY, 0, CAP - 1).await;
}

async fn load(state: &Shared) -> Vec<Event> {
    let mut r = state.redis.clone();
    let raw: Vec<String> = r.lrange(KEY, 0, CAP - 1).await.unwrap_or_default();
    raw.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
}

pub async fn json(State(state): State<Shared>) -> impl IntoResponse {
    Json(load(&state).await)
}

pub async fn dashboard(State(state): State<Shared>) -> Html<String> {
    // Newest flow first; steps inside a flow oldest first, with the delta from the flow's start.
    let mut flows: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    for ev in load(&state).await {
        flows.entry(ev.flow.clone()).or_default().push(ev);
    }
    let mut flows: Vec<_> = flows.into_values().collect();
    for f in &mut flows {
        f.sort_by_key(|e| e.ts_ms);
    }
    flows.sort_by_key(|f| std::cmp::Reverse(f.first().map(|e| e.ts_ms).unwrap_or(0)));

    let mut body = String::from("<h1>P6 flows</h1><p class=muted>Newest first. Times are from the flow's <code>/authorize</code>.</p>");
    for f in flows.iter().take(50) {
        let start = f[0].ts_ms;
        let platform = f[0].detail.get("spike_platform").and_then(Value::as_str).unwrap_or("?");
        let last = f.last().unwrap();
        let ok = last.step == "token.issued";
        body.push_str(&format!(
            "<details{}><summary><b>{}</b> · {} · {} steps · {} · {:.1}s</summary><table>",
            if ok { "" } else { " open" },
            esc(platform),
            esc(&f[0].flow),
            f.len(),
            if ok { "✅ token issued".to_string() } else { format!("stopped at <code>{}</code>", esc(&last.step)) },
            (last.ts_ms - start) as f64 / 1000.0,
        ));
        for e in f {
            body.push_str(&format!(
                "<tr><td>+{:.1}s</td><td><code>{}</code></td><td><code>{}</code></td><td class=ua>{}</td></tr>",
                (e.ts_ms - start) as f64 / 1000.0,
                esc(&e.step),
                esc(&e.detail.to_string()),
                esc(&e.ua),
            ));
        }
        body.push_str("</table></details>");
    }
    Html(crate::html::page("P6 flows", &body))
}
