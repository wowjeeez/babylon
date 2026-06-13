use crate::config::Config;
use crate::dashboard::{
    DashboardState, archive_channel, conversations, create_channel, dashboard_css, dashboard_guard,
    dashboard_js, dashboard_page, history, overview, post_message, tokens_mint, tokens_revoke,
    tokens_rotate,
};
use crate::health::{healthz, readyz};
use crate::middleware::{AuthState, auth};
use crate::perimeter::dev_no_auth_allowed;
use crate::provision::{ProvisionState, provision};
use anyhow::{Context, bail};
use axum::{
    Router,
    routing::{get, post},
};
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

    let mut http_cfg = StreamableHttpServerConfig::default();
    if cfg.allowed_hosts.iter().any(|h| h == "*") {
        http_cfg = http_cfg.disable_allowed_hosts();
    } else if !cfg.allowed_hosts.is_empty() {
        let mut hosts = http_cfg.allowed_hosts.clone();
        hosts.extend(cfg.allowed_hosts.iter().cloned());
        http_cfg = http_cfg.with_allowed_hosts(hosts);
    }

    let hub_for_mcp = hub.clone();
    let mcp = StreamableHttpService::new(
        move || Ok::<_, std::io::Error>(BabylonServer::new(hub_for_mcp.clone())),
        Arc::new(LocalSessionManager::default()),
        http_cfg,
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

    let provision_state = ProvisionState {
        hub: hub.clone(),
        owner_login: cfg.owner_login.clone(),
    };

    let health_router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(hub.clone());

    let provision_router = Router::new()
        .route("/provision", post(provision))
        .with_state(provision_state);

    let dashboard_state = DashboardState {
        hub: hub.clone(),
        owner_login: cfg.owner_login.clone(),
        allowed_hosts: cfg.allowed_hosts.clone(),
        db_path: cfg.db_path.clone(),
    };

    let dashboard_router = Router::new()
        .route("/dashboard", get(dashboard_page))
        .route("/dashboard/app.js", get(dashboard_js))
        .route("/dashboard/app.css", get(dashboard_css))
        .route("/api/overview", get(overview))
        .route("/api/conversations", get(conversations))
        .route("/api/history", get(history))
        .route("/api/tokens/mint", post(tokens_mint))
        .route("/api/tokens/rotate", post(tokens_rotate))
        .route("/api/tokens/revoke", post(tokens_revoke))
        .route("/api/channels", post(create_channel))
        .route("/api/channels/{name}/archive", post(archive_channel))
        .route("/api/messages", post(post_message))
        .layer(axum::middleware::from_fn_with_state(
            dashboard_state.clone(),
            dashboard_guard,
        ))
        .with_state(dashboard_state);

    let app = health_router
        .merge(provision_router)
        .merge(dashboard_router)
        .merge(mcp_router)
        .layer(ConcurrencyLimitLayer::new(256))
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT));

    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .context("bind")?;
    tracing::info!(bind = %cfg.bind, "babylon listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to install SIGTERM handler; falling back to ctrl-c only");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
