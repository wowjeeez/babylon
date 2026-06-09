use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use babylon_core::error::Error as CoreError;
use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct ProvisionState {
    pub hub: Arc<Hub>,
    pub owner_login: Option<String>,
}

#[derive(Deserialize)]
pub struct ProvisionRequest {
    pub handle: String,
}

#[derive(Serialize)]
pub struct ProvisionResponse {
    pub handle: String,
    pub token: String,
}

pub async fn provision(
    State(state): State<ProvisionState>,
    headers: HeaderMap,
    Json(body): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, StatusCode> {
    let owner = if let Some(o) = &state.owner_login {
        o.clone()
    } else {
        tracing::warn!("provision denied: BABYLON_OWNER_LOGIN not configured");
        return Err(StatusCode::FORBIDDEN);
    };

    let caller = headers
        .get("tailscale-user-login")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if caller != owner {
        tracing::warn!(caller, "provision denied: caller is not owner");
        return Err(StatusCode::FORBIDDEN);
    }

    let handle = Handle::parse(&body.handle).map_err(|_| StatusCode::BAD_REQUEST)?;

    match state.hub.mint_token(&handle, AgentKind::Agent).await {
        Ok(token) => {
            tracing::info!(handle = handle.as_str(), caller, "provisioned token");
            Ok(Json(ProvisionResponse {
                handle: handle.as_str().to_string(),
                token,
            }))
        }
        Err(CoreError::HandleExists(_)) => Err(StatusCode::CONFLICT),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
