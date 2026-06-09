use babylon_core::hub::Hub;
use babylon_core::types::{AgentKind, Handle};
use babylon_server::config::{self, Config};
use babylon_server::serve;
use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Serve,
    MintToken {
        handle: String,
        #[arg(long)]
        operator: bool,
    },
    RotateToken {
        handle: String,
    },
    RevokeToken {
        handle: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let cfg = config::Config::load()?;
    match cli.cmd {
        Cmd::Serve => {
            check_no_funnel(&cfg);
            serve::run(cfg).await
        }
        Cmd::MintToken { handle, operator } => {
            let hub = Hub::new(&cfg.db_path).await?;
            let kind = if operator {
                AgentKind::Operator
            } else {
                AgentKind::Agent
            };
            let token = hub.mint_token(&Handle::parse(&handle)?, kind).await?;
            eprintln!(
                "# token for {handle} (shown once; store in a 0600 EnvironmentFile as BABYLON_TOKEN):"
            );
            eprintln!("{token}");
            Ok(())
        }
        Cmd::RotateToken { handle } => {
            let hub = Hub::new(&cfg.db_path).await?;
            let token = hub.rotate_token(&Handle::parse(&handle)?).await?;
            eprintln!("{token}");
            Ok(())
        }
        Cmd::RevokeToken { handle } => {
            let hub = Hub::new(&cfg.db_path).await?;
            hub.revoke_token(&Handle::parse(&handle)?).await?;
            eprintln!("revoked {handle}");
            Ok(())
        }
    }
}

fn tailscale_funnel_active() -> Option<bool> {
    let output = std::process::Command::new("tailscale")
        .args(["serve", "status", "--json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()?;
    Some(
        value
            .get("AllowFunnel")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|m| m.values().any(|v| v.as_bool() == Some(true))),
    )
}

fn check_no_funnel(cfg: &Config) {
    if cfg.allow_funnel {
        return;
    }
    match tailscale_funnel_active() {
        Some(true) => tracing::warn!(
            "tailscale Funnel appears enabled; babylon expects a private perimeter. Set BABYLON_ALLOW_FUNNEL=1 to override."
        ),
        Some(false) => {}
        None => tracing::warn!("tailscale funnel status unavailable; skipping funnel check"),
    }
}
