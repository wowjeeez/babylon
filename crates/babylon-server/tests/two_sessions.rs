use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use babylon_server::config::Config;
use babylon_server::serve;
use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

async fn wait_healthz(url: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        if let Ok(resp) = reqwest::get(url).await {
            if let Ok(body) = resp.text().await {
                if body == "ok" {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("healthz never responded at {url}")
}

async fn client_for(url: &str, handle: &str) -> anyhow::Result<RunningService<RoleClient, ()>> {
    let mut headers = HashMap::new();
    headers.insert(
        HeaderName::from_static("x-babylon-handle"),
        HeaderValue::from_str(handle)?,
    );
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.to_string()).custom_headers(headers);
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    Ok(().serve(transport).await?)
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> anyhow::Result<Value> {
    let mut request = CallToolRequestParams::new(tool);
    if let Some(obj) = args.as_object() {
        request = request.with_arguments(obj.clone());
    }
    let result = client.call_tool(request).await?;
    Ok(result.structured_content.unwrap_or(Value::Null))
}

#[tokio::test]
async fn cross_agent_wait_wakes_over_shared_hub() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("two_sessions.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let bind = format!("127.0.0.1:{port}");

    {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("code")?, AgentKind::Agent)
            .await?;
        hub.mint_token(&Handle::parse("bob")?, AgentKind::Agent)
            .await?;
    }

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: true,
        allow_funnel: false,
        owner_login: None,
        allowed_hosts: vec![],
    };
    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let code = client_for(&mcp_url, "code").await?;
    let bob = client_for(&mcp_url, "bob").await?;

    call(
        &code,
        "create_channel",
        json!({ "name": "deploy", "topic": "t" }),
    )
    .await?;
    call(&code, "join_channel", json!({ "name": "deploy" })).await?;
    call(&bob, "join_channel", json!({ "name": "deploy" })).await?;

    let waiter =
        tokio::spawn(async move { call(&bob, "wait_for", json!({ "timeout_secs": 5 })).await });

    tokio::time::sleep(Duration::from_millis(300)).await;
    call(
        &code,
        "post",
        json!({ "channel": "deploy", "kind": "note", "summary": "ping" }),
    )
    .await?;

    let woke = waiter.await??;

    assert_eq!(
        woke.get("woke").and_then(Value::as_bool),
        Some(true),
        "bob's wait_for should have woken; got: {woke}"
    );
    let messages = woke
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one message; got: {woke}"
    );
    assert_eq!(
        messages[0].get("sum").and_then(Value::as_str),
        Some("ping"),
        "message summary should be 'ping'; got: {woke}"
    );

    let _ = code.cancel().await;
    srv.abort();
    Ok(())
}
