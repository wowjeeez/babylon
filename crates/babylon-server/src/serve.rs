use crate::config::Config;
use crate::health::{healthz, readyz};
use crate::middleware::{AuthState, auth};
use crate::perimeter::dev_no_auth_allowed;
use anyhow::{Context, bail};
use axum::{Router, routing::get};
use babylon_core::hub::Hub;
use babylon_mcp::BabylonServer;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;

const BODY_LIMIT: usize = 512 * 1024;

pub async fn run(cfg: Config) -> anyhow::Result<()> {
    if cfg.dev_no_auth && !dev_no_auth_allowed(&cfg.bind) {
        bail!("DEV_NO_AUTH refused: bind {} is not loopback", cfg.bind);
    }

    let hub = Hub::new(&cfg.db_path).await.context("open hub / migrate")?;

    let hub_for_mcp = hub.clone();
    let mcp = StreamableHttpService::new(
        move || Ok::<_, std::io::Error>(BabylonServer::new(hub_for_mcp.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let mcp_router =
        Router::new()
            .nest_service("/mcp", mcp)
            .layer(axum::middleware::from_fn_with_state(
                AuthState {
                    hub: hub.clone(),
                    dev_no_auth: cfg.dev_no_auth,
                },
                auth,
            ));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(hub.clone())
        .merge(mcp_router)
        .layer(ConcurrencyLimitLayer::new(256))
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT));

    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .context("bind")?;
    tracing::info!(bind = %cfg.bind, "babylon listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
