use axum::extract::State;
use axum::http::StatusCode;
use babylon_core::hub::Hub;
use std::sync::Arc;

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn readyz(State(hub): State<Arc<Hub>>) -> (StatusCode, &'static str) {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(hub.store.reader())
        .await
    {
        Ok(_) => (StatusCode::OK, "ready"),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "not ready"),
    }
}
