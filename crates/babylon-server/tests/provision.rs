mod common;

use babylon_core::hub::Hub;
use babylon_server::config::Config;
use babylon_server::serve;
use common::{free_port, wait_healthz};

async fn start_server(owner_login: Option<&str>) -> anyhow::Result<(String, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let db_path = dir
        .path()
        .join("provision.db")
        .to_string_lossy()
        .into_owned();
    let port = free_port()?;
    let bind = format!("127.0.0.1:{port}");

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: true,
        allow_funnel: false,
        owner_login: owner_login.map(ToString::to_string),
    };

    tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    wait_healthz(&format!("http://{bind}/healthz")).await?;
    Ok((format!("http://{bind}"), dir))
}

#[tokio::test]
async fn provision_owner_new_handle_returns_200_and_valid_token() -> anyhow::Result<()> {
    let (base, dir) = start_server(Some("owner@example.com")).await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .header("tailscale-user-login", "owner@example.com")
        .body(r#"{"handle":"myagent"}"#)
        .send()
        .await?;

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await?;
    let handle = body["handle"].as_str().unwrap_or("");
    let token = body["token"].as_str().unwrap_or("");

    assert_eq!(handle, "myagent");
    assert!(token.starts_with("bbln_"), "token must start with bbln_");

    let db_path = dir
        .path()
        .join("provision.db")
        .to_string_lossy()
        .into_owned();
    let hub = Hub::new(&db_path).await?;
    let resolved = hub.resolve_token(token).await?;
    assert_eq!(resolved.as_str(), "myagent");

    Ok(())
}

#[tokio::test]
async fn provision_wrong_header_returns_403() -> anyhow::Result<()> {
    let (base, _dir) = start_server(Some("owner@example.com")).await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .header("tailscale-user-login", "someone@else.com")
        .body(r#"{"handle":"badactor"}"#)
        .send()
        .await?;

    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn provision_missing_header_returns_403() -> anyhow::Result<()> {
    let (base, _dir) = start_server(Some("owner@example.com")).await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .body(r#"{"handle":"noheader"}"#)
        .send()
        .await?;

    assert_eq!(resp.status(), 403);
    Ok(())
}

#[tokio::test]
async fn provision_existing_handle_returns_409() -> anyhow::Result<()> {
    let (base, _dir) = start_server(Some("owner@example.com")).await?;

    let client = reqwest::Client::new();
    let first = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .header("tailscale-user-login", "owner@example.com")
        .body(r#"{"handle":"duplicate"}"#)
        .send()
        .await?;
    assert_eq!(first.status(), 200);

    let second = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .header("tailscale-user-login", "owner@example.com")
        .body(r#"{"handle":"duplicate"}"#)
        .send()
        .await?;
    assert_eq!(second.status(), 409);
    Ok(())
}

#[tokio::test]
async fn provision_no_owner_login_returns_403() -> anyhow::Result<()> {
    let (base, _dir) = start_server(None).await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/provision"))
        .header("content-type", "application/json")
        .header("tailscale-user-login", "owner@example.com")
        .body(r#"{"handle":"anyone"}"#)
        .send()
        .await?;

    assert_eq!(resp.status(), 403);
    Ok(())
}
