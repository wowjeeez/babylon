use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use axum::middleware::Next;
use axum::response::Response;
use babylon_core::hub::Hub;
use babylon_core::types::Handle;
use babylon_mcp::AuthedHandle;
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    pub hub: Arc<Hub>,
    pub dev_no_auth: bool,
}

pub async fn auth(
    State(st): State<AuthState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let handle = if st.dev_no_auth {
        let h = req
            .headers()
            .get("x-babylon-handle")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        match h {
            Some(h) => {
                let exists = st.hub.agent_exists(&h).await.unwrap_or(false);
                if !exists {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Some(h)
            }
            None => None,
        }
    } else {
        let token = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        match token {
            Some(t) => st.hub.resolve_token(t).await.ok().map(Handle::into_string),
            None => None,
        }
    };
    let Some(handle) = handle else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    st.hub.presence.touch(&handle);
    req.extensions_mut().insert(AuthedHandle(handle));
    Ok(next.run(req).await)
}
