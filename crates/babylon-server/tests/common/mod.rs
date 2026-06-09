#![allow(dead_code)]

use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

pub async fn wait_healthz(url: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        if let Ok(resp) = reqwest::get(url).await {
            if let Ok(body) = resp.text().await {
                if body == "ok" {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("healthz never responded at {url}")
}

pub async fn client_for_handle(
    url: &str,
    handle: &str,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    let mut headers = HashMap::new();
    headers.insert(
        HeaderName::from_static("x-babylon-handle"),
        HeaderValue::from_str(handle)?,
    );
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.to_string()).custom_headers(headers);
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    Ok(().serve(transport).await?)
}

pub async fn client_for_token(
    url: &str,
    token: &str,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    let config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .auth_header(token.to_string());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    Ok(().serve(transport).await?)
}

pub async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> anyhow::Result<Value> {
    let mut request = CallToolRequestParams::new(tool);
    if let Some(obj) = args.as_object() {
        request = request.with_arguments(obj.clone());
    }
    let result = client.call_tool(request).await?;
    Ok(result.structured_content.unwrap_or(Value::Null))
}

pub fn free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}
