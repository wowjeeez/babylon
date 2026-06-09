use babylon_server::config::Config;
use babylon_server::serve;
use std::time::Duration;

async fn wait_healthz(url: &str) -> anyhow::Result<String> {
    for _ in 0..100 {
        if let Ok(resp) = reqwest::get(url).await {
            if let Ok(body) = resp.text().await {
                return Ok(body);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("healthz never responded at {url}")
}

#[tokio::test]
async fn server_boots_and_healthz_ok() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("boot.db");
    let db_path = db_path.to_string_lossy().into_owned();

    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let bind = format!("127.0.0.1:{port}");

    let cfg = Config {
        db_path,
        bind: bind.clone(),
        dev_no_auth: true,
        allow_funnel: false,
    };

    let srv = tokio::spawn(async move {
        let _ = serve::run(cfg).await;
    });

    let body = wait_healthz(&format!("http://{bind}/healthz")).await?;
    assert_eq!(body, "ok");

    srv.abort();
    Ok(())
}
