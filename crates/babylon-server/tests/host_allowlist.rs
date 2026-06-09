mod common;

use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use babylon_server::config::Config;
use babylon_server::serve;
use common::{call, client_for_token_with_client, free_port, wait_healthz};
use serde_json::json;
use std::net::SocketAddr;

const TS_HOST: &str = "babylon.taild4189d.ts.net";

#[tokio::test]
async fn mcp_reachable_via_configured_tailnet_host() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir
        .path()
        .join("host_allow.db")
        .to_string_lossy()
        .into_owned();
    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    let token = {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("code")?, AgentKind::Agent)
            .await?
    };

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: false,
        allow_funnel: true,
        owner_login: None,
        allowed_hosts: vec![TS_HOST.to_string()],
    };
    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let client = reqwest::Client::builder().resolve(TS_HOST, addr).build()?;
    let mcp_url = format!("http://{TS_HOST}:{port}/mcp");

    let svc = client_for_token_with_client(&mcp_url, &token, client).await?;
    let result = call(&svc, "register", json!({})).await?;
    assert_eq!(
        result.get("handle").and_then(|v| v.as_str()),
        Some("code"),
        "register via tailnet host should return handle=code, got: {result}"
    );

    let _ = svc.cancel().await;
    srv.abort();
    Ok(())
}

#[tokio::test]
async fn mcp_rejects_unconfigured_host() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir
        .path()
        .join("host_reject.db")
        .to_string_lossy()
        .into_owned();
    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    let token = {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("code")?, AgentKind::Agent)
            .await?
    };

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: false,
        allow_funnel: true,
        owner_login: None,
        allowed_hosts: vec![],
    };
    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let client = reqwest::Client::builder().resolve(TS_HOST, addr).build()?;
    let resp = client
        .post(format!("http://{TS_HOST}:{port}/mcp"))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        status,
        reqwest::StatusCode::FORBIDDEN,
        "unconfigured tailnet host must be rejected by rmcp host check; body: {body}"
    );

    srv.abort();
    Ok(())
}
