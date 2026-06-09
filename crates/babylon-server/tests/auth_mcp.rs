mod common;

use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use babylon_server::config::Config;
use babylon_server::serve;
use common::{call, client_for_token, free_port, wait_healthz};
use serde_json::json;

#[tokio::test]
async fn prod_auth_valid_token_can_register_and_list() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("auth_mcp.db");
    let db_path = db_path.to_string_lossy().into_owned();

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

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let client = client_for_token(&mcp_url, &token).await?;
    let result = call(&client, "register", json!({})).await?;
    assert_eq!(
        result.get("handle").and_then(|v| v.as_str()),
        Some("code"),
        "register should return handle=code, got: {result}"
    );

    let channels = call(&client, "list_channels", json!({})).await?;
    assert!(
        channels.get("channels").is_some(),
        "list_channels should return channels key, got: {channels}"
    );

    let _ = client.cancel().await;
    srv.abort();
    Ok(())
}

#[tokio::test]
async fn prod_auth_bogus_token_rejected() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("auth_mcp_bogus.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    {
        let hub = Hub::new(&db_path).await?;
        hub.mint_token(&Handle::parse("code")?, AgentKind::Agent)
            .await?;
    }

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

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let result = client_for_token(&mcp_url, "bogus-token-xxxxxxxx").await;
    assert!(
        result.is_err(),
        "connecting with bogus token should fail at initialize, got Ok"
    );

    srv.abort();
    Ok(())
}

#[tokio::test]
async fn prod_auth_revoked_token_rejected() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("auth_mcp_revoked.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    let token = {
        let hub = Hub::new(&db_path).await?;
        let token = hub
            .mint_token(&Handle::parse("code")?, AgentKind::Agent)
            .await?;
        hub.revoke_token(&Handle::parse("code")?).await?;
        token
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

    let mcp_url = format!("http://{bind}/mcp");
    wait_healthz(&format!("http://{bind}/healthz")).await?;

    let result = client_for_token(&mcp_url, &token).await;
    assert!(
        result.is_err(),
        "connecting with revoked token should fail at initialize, got Ok"
    );

    srv.abort();
    Ok(())
}
