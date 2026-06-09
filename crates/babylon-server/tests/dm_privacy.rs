mod common;

use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use babylon_server::config::Config;
use babylon_server::serve;
use common::{call, client_for_handle, free_port, wait_healthz};
use serde_json::json;

#[tokio::test]
async fn dm_channel_not_joinable_by_stranger() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("dm_privacy.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("alice")?, AgentKind::Agent)
            .await?;
        hub.mint_token(&Handle::parse("bob")?, AgentKind::Agent)
            .await?;
        hub.mint_token(&Handle::parse("carol")?, AgentKind::Agent)
            .await?;
    }

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: true,
        allow_funnel: false,
    };
    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let alice = client_for_handle(&mcp_url, "alice").await?;
    let carol = client_for_handle(&mcp_url, "carol").await?;

    let dm_result = call(
        &alice,
        "dm",
        json!({ "to": "bob", "kind": "note", "summary": "hello bob" }),
    )
    .await?;
    let dm_channel = dm_result
        .get("channel")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("dm should return a channel name"))?
        .to_string();

    assert!(
        dm_channel.starts_with("dm:"),
        "dm channel should start with dm:, got: {dm_channel}"
    );

    let join_result = call(&carol, "join_channel", json!({ "name": dm_channel })).await;
    assert!(
        join_result.is_err(),
        "carol should not be able to join alice+bob DM channel, but got: {join_result:?}"
    );

    let _ = alice.cancel().await;
    let _ = carol.cancel().await;
    srv.abort();
    Ok(())
}

#[tokio::test]
async fn dm_messages_not_visible_in_carols_catch_up() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("dm_privacy_catchup.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("alice")?, AgentKind::Agent)
            .await?;
        hub.mint_token(&Handle::parse("bob")?, AgentKind::Agent)
            .await?;
        hub.mint_token(&Handle::parse("carol")?, AgentKind::Agent)
            .await?;
    }

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: true,
        allow_funnel: false,
    };
    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let alice = client_for_handle(&mcp_url, "alice").await?;
    let carol = client_for_handle(&mcp_url, "carol").await?;

    call(
        &alice,
        "dm",
        json!({ "to": "bob", "kind": "note", "summary": "secret message" }),
    )
    .await?;

    let catch_up = call(&carol, "catch_up", json!({})).await?;
    let messages = catch_up
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let dm_visible = messages.iter().any(|m| {
        m.get("sum")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("secret message"))
    });

    assert!(
        !dm_visible,
        "carol should not see alice+bob DM in catch_up, got: {catch_up}"
    );

    let _ = alice.cancel().await;
    let _ = carol.cancel().await;
    srv.abort();
    Ok(())
}
