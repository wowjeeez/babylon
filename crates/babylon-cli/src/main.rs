use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use std::collections::HashMap;

#[derive(Parser)]
#[command(name = "babylon", about = "Babylon coordination hub client")]
struct Cli {
    #[arg(long, env = "BABYLON_URL", default_value = "http://127.0.0.1:8787/mcp")]
    url: String,
    #[arg(long, env = "BABYLON_TOKEN")]
    token: Option<String>,
    #[arg(long, env = "BABYLON_HANDLE")]
    handle: Option<String>,
    #[arg(long)]
    dev: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    CatchUp {
        #[arg(long)]
        only_mentions: bool,
    },
    Post {
        channel: String,
        kind: String,
        summary: String,
        #[arg(long)]
        body: Option<String>,
    },
    OpenTasks {
        #[arg(long)]
        owner: Option<String>,
    },
    Resolve {
        id: i64,
    },
    Ack {
        channel: String,
        up_to_id: i64,
    },
    Wait {
        #[arg(long, default_value_t = 25)]
        timeout_secs: u64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (tool, args) = match cli.cmd {
        Cmd::CatchUp { only_mentions } => (
            "catch_up",
            serde_json::json!({ "only_mentions": only_mentions }),
        ),
        Cmd::Post {
            channel,
            kind,
            summary,
            body,
        } => (
            "post",
            serde_json::json!({ "channel": channel, "kind": kind, "summary": summary, "body": body }),
        ),
        Cmd::OpenTasks { owner } => ("open_tasks", serde_json::json!({ "owner": owner })),
        Cmd::Resolve { id } => ("resolve", serde_json::json!({ "id": id })),
        Cmd::Ack { channel, up_to_id } => (
            "ack",
            serde_json::json!({ "channel": channel, "up_to_id": up_to_id }),
        ),
        Cmd::Wait { timeout_secs } => (
            "wait_for",
            serde_json::json!({ "timeout_secs": timeout_secs }),
        ),
    };

    let mut config = StreamableHttpClientTransportConfig::with_uri(cli.url);
    match (cli.token, cli.dev, cli.handle) {
        (Some(token), _, _) => {
            config = config.auth_header(token);
        }
        (None, true, Some(handle)) => {
            let name = HeaderName::from_static("x-babylon-handle");
            let value = HeaderValue::from_str(&handle).context("invalid handle header value")?;
            let mut headers = HashMap::new();
            headers.insert(name, value);
            config = config.custom_headers(headers);
        }
        (None, true, None) => {
            bail!("--dev requires BABYLON_HANDLE (or --handle)");
        }
        (None, false, _) => {
            bail!("set BABYLON_TOKEN, or --dev with BABYLON_HANDLE");
        }
    }

    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    let client = ().serve(transport).await.context("connect / initialize MCP client")?;

    let mut request = CallToolRequestParams::new(tool);
    if let Some(obj) = args.as_object() {
        request = request.with_arguments(obj.clone());
    }
    let result = client.call_tool(request).await.context("call_tool")?;

    let out = result.structured_content.unwrap_or(serde_json::Value::Null);
    println!("{}", serde_json::to_string_pretty(&out)?);

    client.cancel().await.ok();
    Ok(())
}
