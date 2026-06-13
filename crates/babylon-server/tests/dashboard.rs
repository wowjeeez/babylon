mod common;

use babylon_server::config::Config;
use babylon_server::serve;
use common::{call, client_for_token, free_port, wait_healthz};
use reqwest::RequestBuilder;
use serde_json::json;

async fn start_server(
    owner_login: Option<&str>,
) -> anyhow::Result<(String, u16, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let db_path = dir
        .path()
        .join("dashboard.db")
        .to_string_lossy()
        .into_owned();
    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: false,
        allow_funnel: true,
        owner_login: owner_login.map(ToString::to_string),
        allowed_hosts: vec![],
    };

    tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    wait_healthz(&format!("http://{bind}/healthz")).await?;
    Ok((format!("http://{bind}"), port, dir))
}

const OWNER: &str = "owner@example.com";
const CSRF: &str = "csrf-token-value";

fn as_owner(req: RequestBuilder) -> RequestBuilder {
    req.header("tailscale-user-login", OWNER)
}

fn with_csrf(req: RequestBuilder) -> RequestBuilder {
    as_owner(req)
        .header("content-type", "application/json")
        .header("cookie", format!("babylon_csrf={CSRF}"))
        .header("x-babylon-csrf", CSRF)
}

#[tokio::test]
async fn overview_no_owner_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(None).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/api/overview")))
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn overview_wrong_owner_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/overview"))
        .header("tailscale-user-login", "someone@else.com")
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn overview_correct_owner_returns_200_with_keys() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/api/overview")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );

    let body: serde_json::Value = resp.json().await?;
    assert!(body.get("pin").is_some());
    assert!(body["health"].get("ready").is_some());
    assert!(body.get("stats").is_some());
    assert!(body.get("agents").is_some());
    assert!(body.get("channels").is_some());
    assert!(body.get("open_questions").is_some());
    assert!(body.get("open_tasks").is_some());
    assert!(body.get("db_bytes").is_some());
    Ok(())
}

#[tokio::test]
async fn post_missing_csrf_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.post(format!("{base}/api/tokens/mint")))
        .header("content-type", "application/json")
        .body(r#"{"handle":"agentx"}"#)
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn post_mismatched_csrf_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.post(format!("{base}/api/tokens/mint")))
        .header("content-type", "application/json")
        .header("cookie", "babylon_csrf=aaa")
        .header("x-babylon-csrf", "bbb")
        .body(r#"{"handle":"agentx"}"#)
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn post_evil_origin_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = with_csrf(client.post(format!("{base}/api/tokens/mint")))
        .header("origin", "https://evil.com")
        .body(r#"{"handle":"agentx"}"#)
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn post_loopback_origin_returns_200() -> anyhow::Result<()> {
    let (base, port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = with_csrf(client.post(format!("{base}/api/tokens/mint")))
        .header("origin", format!("http://127.0.0.1:{port}"))
        .body(r#"{"handle":"agentx"}"#)
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    Ok(())
}

#[tokio::test]
async fn mint_rotate_revoke_lifecycle() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let resp = with_csrf(client.post(format!("{base}/api/tokens/mint")))
        .body(r#"{"handle":"alpha"}"#)
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["handle"], "alpha");
    assert!(
        body["token"].as_str().unwrap_or("").starts_with("bbln_"),
        "token must start with bbln_"
    );

    let dup = with_csrf(client.post(format!("{base}/api/tokens/mint")))
        .body(r#"{"handle":"alpha"}"#)
        .send()
        .await?;
    assert_eq!(dup.status(), 409);

    let rot = with_csrf(client.post(format!("{base}/api/tokens/rotate")))
        .body(r#"{"handle":"alpha"}"#)
        .send()
        .await?;
    assert_eq!(rot.status(), 200);
    let rot_body: serde_json::Value = rot.json().await?;
    assert!(
        rot_body["token"]
            .as_str()
            .unwrap_or("")
            .starts_with("bbln_")
    );

    let rot_missing = with_csrf(client.post(format!("{base}/api/tokens/rotate")))
        .body(r#"{"handle":"ghost"}"#)
        .send()
        .await?;
    assert_eq!(rot_missing.status(), 404);

    let rev_missing = with_csrf(client.post(format!("{base}/api/tokens/revoke")))
        .body(r#"{"handle":"ghost"}"#)
        .send()
        .await?;
    assert_eq!(rev_missing.status(), 404);

    let rev = with_csrf(client.post(format!("{base}/api/tokens/revoke")))
        .body(r#"{"handle":"alpha"}"#)
        .send()
        .await?;
    assert_eq!(rev.status(), 200);
    let rev_body: serde_json::Value = rev.json().await?;
    assert_eq!(rev_body["ok"], true);

    Ok(())
}

#[tokio::test]
async fn channel_lifecycle() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let create = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(create.status(), 200);

    let dm = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"dm:x","topic":"nope"}"#)
        .send()
        .await?;
    assert_eq!(dm.status(), 400);

    let dup = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(dup.status(), 409);

    let archive = with_csrf(client.post(format!("{base}/api/channels/ops/archive")))
        .send()
        .await?;
    assert_eq!(archive.status(), 200);

    let archive_dm = with_csrf(client.post(format!("{base}/api/channels/dm:x/archive")))
        .send()
        .await?;
    assert_eq!(archive_dm.status(), 400);

    Ok(())
}

#[tokio::test]
async fn dashboard_page_owner_returns_200_with_csrf_and_csp() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/dashboard")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        csp.contains("script-src 'self'"),
        "CSP must restrict scripts"
    );
    assert!(csp.contains("default-src 'none'"));

    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(set_cookie.contains("babylon_csrf="), "must set csrf cookie");
    assert!(set_cookie.contains("Path=/"));
    assert!(set_cookie.contains("SameSite=Strict"));

    let body = resp.text().await?;
    assert!(
        body.contains(r#"<meta name="csrf" content=""#),
        "page must embed csrf meta"
    );
    assert!(
        !body.contains("{{CSRF}}"),
        "csrf placeholder must be replaced"
    );
    Ok(())
}

#[tokio::test]
async fn dashboard_page_non_owner_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/dashboard"))
        .header("tailscale-user-login", "intruder@example.com")
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn dashboard_js_owner_returns_javascript() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/dashboard/app.js")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("javascript"), "content-type must be javascript");
    Ok(())
}

#[tokio::test]
async fn dashboard_css_owner_returns_css() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/dashboard/app.css")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("css"), "content-type must be css");
    Ok(())
}

#[test]
fn app_js_never_assigns_innerhtml() {
    let js = include_str!("../assets/app.js");
    assert!(
        !js.contains("innerHTML"),
        "app.js must not use innerHTML (XSS sink); render via textContent/createElement only"
    );
    assert!(
        !js.contains("insertAdjacentHTML"),
        "app.js must not use insertAdjacentHTML"
    );
    assert!(!js.contains("outerHTML"), "app.js must not use outerHTML");
}

#[test]
fn app_js_wires_conversations_view() {
    let js = include_str!("../assets/app.js");
    assert!(
        js.contains("/api/conversations"),
        "app.js must fetch the conversations list"
    );
    assert!(
        js.contains("/api/history"),
        "app.js must fetch thread history"
    );
    assert!(
        js.contains("encodeURIComponent"),
        "app.js must encode the channel name for the history query"
    );
}

#[test]
fn dashboard_html_has_conversations_section() {
    let html = include_str!("../assets/dashboard.html");
    assert!(
        html.contains("conv-list"),
        "dashboard must include the conversation sidebar list"
    );
    assert!(
        html.contains("conv-thread"),
        "dashboard must include the conversation thread pane"
    );
    assert!(
        html.contains("conv-composer"),
        "dashboard must include the conversation composer"
    );
}

#[tokio::test]
async fn conversations_no_owner_returns_403() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(None).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/api/conversations")))
        .send()
        .await?;
    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn conversations_returns_seeded_dm() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let create = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(create.status(), 200);

    let token_a = mint(&base, "alfa").await?;
    let _token_b = mint(&base, "bravo").await?;
    seed_dm(&base, &token_a, "bravo").await?;

    let resp = as_owner(client.get(format!("{base}/api/conversations")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let body: serde_json::Value = resp.json().await?;
    let empty = Vec::new();
    let convs = body["conversations"].as_array().unwrap_or(&empty);
    assert!(convs.iter().any(|c| c["name"] == "ops"));
    assert!(
        convs.iter().any(|c| c["name"] == "dm:alfa+bravo"),
        "must include the seeded dm channel"
    );
    Ok(())
}

async fn seed_dm(base: &str, from_token: &str, to: &str) -> anyhow::Result<()> {
    let mcp_url = format!("{base}/mcp");
    let from = client_for_token(&mcp_url, from_token).await?;
    call(
        &from,
        "dm",
        json!({ "to": to, "kind": "note", "summary": "seed" }),
    )
    .await?;
    let _ = from.cancel().await;
    Ok(())
}

async fn mint(base: &str, handle: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let resp = with_csrf(client.post(format!("{base}/api/tokens/mint")))
        .body(format!(r#"{{"handle":"{handle}"}}"#))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;
    Ok(body["token"].as_str().unwrap_or("").to_string())
}

#[tokio::test]
async fn history_owner_gated_and_paginates() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let no_owner = reqwest::Client::new()
        .get(format!("{base}/api/history?channel=ops"))
        .send()
        .await?;
    assert_eq!(no_owner.status(), 403);

    let create = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(create.status(), 200);

    let mut ids = Vec::new();
    for i in 0..3 {
        let resp = with_csrf(client.post(format!("{base}/api/messages")))
            .body(format!(
                r#"{{"channel":"ops","kind":"note","summary":"m{i}","body":"b{i}"}}"#
            ))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let b: serde_json::Value = resp.json().await?;
        ids.push(b["id"].as_i64().unwrap_or(0));
    }

    let resp = as_owner(client.get(format!("{base}/api/history?channel=ops")))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let body: serde_json::Value = resp.json().await?;
    let empty = Vec::new();
    let msgs = body["messages"].as_array().unwrap_or(&empty);
    let got: Vec<i64> = msgs.iter().map(|m| m["id"].as_i64().unwrap_or(0)).collect();
    assert_eq!(got, ids, "oldest to newest");
    assert_eq!(msgs[0]["body"], "b0", "bodies present");

    let before = as_owner(client.get(format!("{base}/api/history?channel=ops&before={}", ids[1])))
        .send()
        .await?;
    assert_eq!(before.status(), 200);
    let before_body: serde_json::Value = before.json().await?;
    let before_ids: Vec<i64> = before_body["messages"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|m| m["id"].as_i64().unwrap_or(0))
        .collect();
    assert_eq!(before_ids, vec![ids[0]], "before paginates");

    Ok(())
}

#[tokio::test]
async fn history_missing_channel_returns_400() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/api/history")))
        .send()
        .await?;
    assert_eq!(resp.status(), 400);
    Ok(())
}

#[tokio::test]
async fn history_unknown_channel_returns_404() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();
    let resp = as_owner(client.get(format!("{base}/api/history?channel=nope")))
        .send()
        .await?;
    assert_eq!(resp.status(), 404);
    Ok(())
}

#[tokio::test]
async fn operator_can_post_note_into_dm() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let token_a = mint(&base, "alfa").await?;
    let _token_b = mint(&base, "bravo").await?;
    seed_dm(&base, &token_a, "bravo").await?;

    let note = with_csrf(client.post(format!("{base}/api/messages")))
        .body(r#"{"channel":"dm:alfa+bravo","kind":"note","summary":"op peek"}"#)
        .send()
        .await?;
    assert_eq!(note.status(), 200, "operator may god-send into a dm");

    let hist = as_owner(client.get(format!("{base}/api/history?channel=dm:alfa%2Bbravo")))
        .send()
        .await?;
    assert_eq!(hist.status(), 200);
    let body: serde_json::Value = hist.json().await?;
    let empty = Vec::new();
    let msgs = body["messages"].as_array().unwrap_or(&empty);
    assert!(
        msgs.iter()
            .any(|m| m["from"] == "operator" && m["summary"] == "op peek"),
        "operator message must appear in dm history"
    );

    Ok(())
}

#[tokio::test]
async fn history_returns_xss_body_verbatim() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let create = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(create.status(), 200);

    let payload = "<img src=x onerror=alert(1)>";
    let post = with_csrf(client.post(format!("{base}/api/messages")))
        .body(serde_json::to_string(&serde_json::json!({
            "channel": "ops",
            "kind": "note",
            "summary": "xss",
            "body": payload,
        }))?)
        .send()
        .await?;
    assert_eq!(post.status(), 200);

    let hist = as_owner(client.get(format!("{base}/api/history?channel=ops")))
        .send()
        .await?;
    assert_eq!(hist.status(), 200);
    let body: serde_json::Value = hist.json().await?;
    let empty = Vec::new();
    let found = body["messages"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .any(|m| m["body"] == payload);
    assert!(found, "xss body must round-trip verbatim");
    Ok(())
}

#[tokio::test]
async fn message_posting() -> anyhow::Result<()> {
    let (base, _port, _dir) = start_server(Some(OWNER)).await?;
    let client = reqwest::Client::new();

    let create = with_csrf(client.post(format!("{base}/api/channels")))
        .body(r#"{"name":"ops","topic":"operations"}"#)
        .send()
        .await?;
    assert_eq!(create.status(), 200);

    let note = with_csrf(client.post(format!("{base}/api/messages")))
        .body(r#"{"channel":"ops","kind":"note","summary":"hello"}"#)
        .send()
        .await?;
    assert_eq!(note.status(), 200);
    let note_body: serde_json::Value = note.json().await?;
    assert!(note_body.get("id").is_some());

    let task_no_mentions = with_csrf(client.post(format!("{base}/api/messages")))
        .body(r#"{"channel":"ops","kind":"task","summary":"do it"}"#)
        .send()
        .await?;
    assert_eq!(task_no_mentions.status(), 400);

    let dm_unknown = with_csrf(client.post(format!("{base}/api/messages")))
        .body(r#"{"channel":"dm:x","kind":"note","summary":"secret"}"#)
        .send()
        .await?;
    assert_eq!(dm_unknown.status(), 404);

    let bad_kind = with_csrf(client.post(format!("{base}/api/messages")))
        .body(r#"{"channel":"ops","kind":"bogus","summary":"x"}"#)
        .send()
        .await?;
    assert_eq!(bad_kind.status(), 400);

    Ok(())
}
