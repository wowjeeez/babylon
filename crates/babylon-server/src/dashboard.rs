use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use babylon_core::dto::{AdminChannelInfo, AgentInfo, GlobalStats, MsgSummary};
use babylon_core::error::Error as CoreError;
use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use base64ct::{Base64UrlUnpadded, Encoding as _};
use rand::RngCore;

const GIT_SHA: &str = env!("GIT_SHA");

const DASHBOARD_HTML: &str = include_str!("../assets/dashboard.html");
const DASHBOARD_JS: &str = include_str!("../assets/app.js");
const DASHBOARD_CSS: &str = include_str!("../assets/app.css");

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'";

fn generate_csrf_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Base64UrlUnpadded::encode_string(&bytes)
}

pub async fn dashboard_page() -> Response {
    let token = generate_csrf_token();
    let html = DASHBOARD_HTML.replace("{{CSRF}}", &token);

    let mut resp = Response::new(Body::from(html));
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if let Ok(cookie) = HeaderValue::from_str(&format!(
        "babylon_csrf={token}; SameSite=Strict; Secure; Path=/; Max-Age=86400"
    )) {
        headers.insert(header::SET_COOKIE, cookie);
    }
    resp
}

fn static_asset(body: &'static str, content_type: &'static str) -> Response {
    let mut resp = Response::new(Body::from(body));
    let headers = resp.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

pub async fn dashboard_js() -> Response {
    static_asset(DASHBOARD_JS, "text/javascript; charset=utf-8")
}

pub async fn dashboard_css() -> Response {
    static_asset(DASHBOARD_CSS, "text/css; charset=utf-8")
}

#[derive(Clone)]
pub struct DashboardState {
    pub hub: Arc<Hub>,
    pub owner_login: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub db_path: String,
}

fn host_only(value: &str) -> &str {
    let value = value.trim();
    let without_scheme = value.split_once("://").map_or(value, |(_, rest)| rest);
    let host_port = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    if let Some(stripped) = host_port.strip_prefix('[') {
        return stripped.split(']').next().unwrap_or(stripped);
    }
    host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
}

fn is_loopback(host: &str) -> bool {
    host == "127.0.0.1" || host == "localhost" || host == "::1"
}

fn host_allowed(host: &str, allowed: &[String]) -> bool {
    is_loopback(host) || allowed.iter().any(|h| host_only(h) == host)
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn dashboard_guard(
    State(state): State<DashboardState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(owner) = state.owner_login.as_deref() else {
        tracing::warn!("dashboard denied: BABYLON_OWNER_LOGIN not configured");
        return Err(StatusCode::FORBIDDEN);
    };

    let headers = req.headers();

    let caller = headers
        .get("tailscale-user-login")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if caller != owner {
        tracing::warn!(caller, "dashboard denied: caller is not owner");
        return Err(StatusCode::FORBIDDEN);
    }

    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map_or("", host_only);
    if !host_allowed(host, &state.allowed_hosts) {
        tracing::warn!(host, "dashboard denied: host not allowed");
        return Err(StatusCode::FORBIDDEN);
    }

    if req.method() == axum::http::Method::POST {
        check_csrf(headers, &state.allowed_hosts)?;
    }

    Ok(next.run(req).await)
}

fn check_csrf(headers: &HeaderMap, allowed_hosts: &[String]) -> Result<(), StatusCode> {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let origin_host = host_only(origin);
        if !host_allowed(origin_host, allowed_hosts) {
            tracing::warn!(origin, "dashboard denied: origin not allowed");
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let cookie = cookie_value(headers, "babylon_csrf").unwrap_or("");
    let header_tok = headers
        .get("x-babylon-csrf")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if cookie.is_empty() || header_tok.is_empty() || !ct_eq(cookie, header_tok) {
        tracing::warn!("dashboard denied: csrf check failed");
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

fn err_status(e: &CoreError) -> StatusCode {
    match e {
        CoreError::UnknownHandle(_) | CoreError::UnknownChannel(_) => StatusCode::NOT_FOUND,
        CoreError::HandleExists(_) | CoreError::ChannelExists(_) => StatusCode::CONFLICT,
        CoreError::NotAuthorized(_)
        | CoreError::NotAuthorizedToResolve(_)
        | CoreError::Unauthorized
        | CoreError::TokenRevoked => StatusCode::FORBIDDEN,
        CoreError::TaskNeedsAssignee
        | CoreError::BadName(_)
        | CoreError::TooLarge(_)
        | CoreError::BadReplyTarget(_)
        | CoreError::BadResolveTarget(_)
        | CoreError::NotAMember(_)
        | CoreError::NotSubscribed(_) => StatusCode::BAD_REQUEST,
        CoreError::Db(inner) => {
            tracing::error!(error = %inner, "dashboard database error");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn operator_handle() -> Result<Handle, StatusCode> {
    Handle::parse("operator").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Serialize)]
pub struct OverviewHealth {
    pub ok: bool,
    pub ready: bool,
}

#[derive(Serialize)]
pub struct OverviewResponse {
    pub pin: String,
    pub health: OverviewHealth,
    pub db_bytes: u64,
    pub stats: GlobalStats,
    pub agents: Vec<AgentInfo>,
    pub channels: Vec<AdminChannelInfo>,
    pub open_questions: Vec<MsgSummary>,
    pub open_tasks: Vec<MsgSummary>,
}

async fn readiness(hub: &Hub) -> bool {
    use std::time::Duration;
    let reader_ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(hub.store.reader())
        .await
        .is_ok();
    if !reader_ok {
        return false;
    }
    let writer_result = tokio::time::timeout(Duration::from_secs(2), async {
        hub.store
            .with_writer(|c| {
                Box::pin(async {
                    sqlx::query("SELECT 1").execute(c).await?;
                    Ok(())
                })
            })
            .await
    })
    .await;
    matches!(writer_result, Ok(Ok(())))
}

pub async fn overview(State(state): State<DashboardState>) -> Result<Response, StatusCode> {
    state
        .hub
        .ensure_operator()
        .await
        .map_err(|e| err_status(&e))?;
    let operator = operator_handle()?;

    let ready = readiness(&state.hub).await;
    let db_bytes = std::fs::metadata(&state.db_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let counts = state.hub.global_stats().await.map_err(|e| err_status(&e))?;
    let agents = state.hub.list_agents().await.map_err(|e| err_status(&e))?;
    let channels = state
        .hub
        .admin_channels()
        .await
        .map_err(|e| err_status(&e))?;
    let open_questions = state
        .hub
        .open_questions(&operator, false, None)
        .await
        .map_err(|e| err_status(&e))?;
    let open_tasks = state
        .hub
        .open_tasks(&operator, false, None, None)
        .await
        .map_err(|e| err_status(&e))?;

    let resp = OverviewResponse {
        pin: GIT_SHA.to_string(),
        health: OverviewHealth { ok: true, ready },
        db_bytes,
        stats: counts,
        agents,
        channels,
        open_questions,
        open_tasks,
    };
    Ok(no_store(Json(resp).into_response()))
}

pub async fn conversations(State(state): State<DashboardState>) -> Result<Response, StatusCode> {
    let conversations = state
        .hub
        .conversations()
        .await
        .map_err(|e| err_status(&e))?;
    Ok(no_store(
        Json(json!({ "conversations": conversations })).into_response(),
    ))
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub channel: Option<String>,
    #[serde(default)]
    pub before: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

const DEFAULT_HISTORY_LIMIT: i64 = 50;
const MAX_HISTORY_LIMIT: i64 = 200;

pub async fn history(
    State(state): State<DashboardState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Response, StatusCode> {
    let channel = q
        .channel
        .filter(|c| !c.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let limit = q
        .limit
        .filter(|l| *l > 0)
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .min(MAX_HISTORY_LIMIT);
    let messages = state
        .hub
        .channel_history(&channel, q.before, limit)
        .await
        .map_err(|e| err_status(&e))?;
    Ok(no_store(
        Json(json!({ "messages": messages })).into_response(),
    ))
}

#[derive(Deserialize)]
pub struct MintRequest {
    pub handle: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub handle: String,
    pub token: String,
}

pub async fn tokens_mint(
    State(state): State<DashboardState>,
    Json(body): Json<MintRequest>,
) -> Result<Response, StatusCode> {
    let handle = Handle::parse(&body.handle).map_err(|_| StatusCode::BAD_REQUEST)?;
    let kind = match body.kind.as_deref() {
        Some("operator") => AgentKind::Operator,
        _ => AgentKind::Agent,
    };
    let token = state
        .hub
        .mint_token(&handle, kind)
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(
        caller = "operator",
        action = "mint",
        target = handle.as_str(),
        "token minted"
    );
    Ok(no_store(
        Json(TokenResponse {
            handle: handle.into_string(),
            token,
        })
        .into_response(),
    ))
}

#[derive(Deserialize)]
pub struct HandleRequest {
    pub handle: String,
}

pub async fn tokens_rotate(
    State(state): State<DashboardState>,
    Json(body): Json<HandleRequest>,
) -> Result<Response, StatusCode> {
    let handle = Handle::parse(&body.handle).map_err(|_| StatusCode::BAD_REQUEST)?;
    let token = state
        .hub
        .rotate_token(&handle)
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(
        caller = "operator",
        action = "rotate",
        target = handle.as_str(),
        "token rotated"
    );
    Ok(no_store(
        Json(TokenResponse {
            handle: handle.into_string(),
            token,
        })
        .into_response(),
    ))
}

pub async fn tokens_revoke(
    State(state): State<DashboardState>,
    Json(body): Json<HandleRequest>,
) -> Result<Response, StatusCode> {
    let handle = Handle::parse(&body.handle).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .hub
        .revoke_token(&handle)
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(
        caller = "operator",
        action = "revoke",
        target = handle.as_str(),
        "token revoked"
    );
    Ok(no_store(Json(json!({ "ok": true })).into_response()))
}

#[derive(Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub topic: String,
}

pub async fn create_channel(
    State(state): State<DashboardState>,
    Json(body): Json<CreateChannelRequest>,
) -> Result<Response, StatusCode> {
    if body.name.starts_with("dm:") {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .hub
        .ensure_operator()
        .await
        .map_err(|e| err_status(&e))?;
    let operator = operator_handle()?;
    state
        .hub
        .create_channel(&operator, &body.name, &body.topic)
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(caller = "operator", action = "create_channel", target = %body.name, "channel created");
    Ok(no_store(Json(json!({ "ok": true })).into_response()))
}

pub async fn archive_channel(
    State(state): State<DashboardState>,
    Path(name): Path<String>,
) -> Result<Response, StatusCode> {
    if name.starts_with("dm:") {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .hub
        .ensure_operator()
        .await
        .map_err(|e| err_status(&e))?;
    let operator = operator_handle()?;
    state
        .hub
        .archive_channel(&operator, &name)
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(caller = "operator", action = "archive_channel", target = %name, "channel archived");
    Ok(no_store(Json(json!({ "ok": true })).into_response()))
}

#[derive(Deserialize)]
pub struct PostMessageRequest {
    pub channel: String,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

const VALID_KINDS: [&str; 6] = ["question", "answer", "decision", "status", "note", "task"];

pub async fn post_message(
    State(state): State<DashboardState>,
    Json(body): Json<PostMessageRequest>,
) -> Result<Response, StatusCode> {
    if !VALID_KINDS.contains(&body.kind.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if body.kind == "task" && body.mentions.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .hub
        .ensure_operator()
        .await
        .map_err(|e| err_status(&e))?;
    let operator = operator_handle()?;
    let id = state
        .hub
        .post(
            &operator,
            &body.channel,
            &body.kind,
            &body.summary,
            body.body.as_deref(),
            &body.mentions,
            None,
        )
        .await
        .map_err(|e| err_status(&e))?;
    tracing::info!(caller = "operator", action = "post", target = %body.channel, "message posted");
    Ok(no_store(Json(json!({ "id": id })).into_response()))
}

#[cfg(test)]
mod tests {
    use super::{ct_eq, host_allowed, host_only, is_loopback};

    #[test]
    fn host_only_strips_scheme_and_port() {
        assert_eq!(host_only("https://evil.com:443/x"), "evil.com");
        assert_eq!(host_only("127.0.0.1:8787"), "127.0.0.1");
        assert_eq!(host_only("http://[::1]:8787"), "::1");
        assert_eq!(
            host_only("babylon.example.ts.net"),
            "babylon.example.ts.net"
        );
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("evil.com"));
    }

    #[test]
    fn host_allowlist() {
        let allowed = vec!["babylon.example.ts.net".to_string()];
        assert!(host_allowed("127.0.0.1", &allowed));
        assert!(host_allowed("babylon.example.ts.net", &allowed));
        assert!(!host_allowed("evil.com", &allowed));
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "x"));
    }
}
