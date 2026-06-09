use axum::extract::State;
use axum::http::StatusCode;
use babylon_core::hub::Hub;
use std::sync::Arc;
use std::time::Duration;

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn readyz(State(hub): State<Arc<Hub>>) -> (StatusCode, &'static str) {
    let reader_ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(hub.store.reader())
        .await
        .is_ok();

    if !reader_ok {
        return (StatusCode::SERVICE_UNAVAILABLE, "not ready");
    }

    let writer_result = tokio::time::timeout(Duration::from_secs(2), async {
        hub.store
            .with_writer(|c| {
                Box::pin(async {
                    sqlx::query("SELECT 1").execute(c).await?;
                    Ok(())
                })
            })
            .await
    })
    .await;

    match writer_result {
        Ok(Ok(())) => (StatusCode::OK, "ready"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "not ready"),
    }
}
