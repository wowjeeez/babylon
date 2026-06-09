use anyhow::bail;
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
            check_funnel(&cfg)?;
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

fn check_funnel(cfg: &Config) -> anyhow::Result<()> {
    if cfg.dev_no_auth {
        tracing::warn!("dev_no_auth=true: skipping funnel check (loopback dev mode)");
        return Ok(());
    }
    if cfg.allow_funnel {
        tracing::warn!("BABYLON_ALLOW_FUNNEL=1: skipping funnel check");
        return Ok(());
    }
    check_funnel_prod()
}

fn check_funnel_prod() -> anyhow::Result<()> {
    match tailscale_funnel_active() {
        Some(true) => reject_funnel_active(),
        Some(false) => {
            tracing::warn!("tailscale funnel check passed (Funnel is off)");
            Ok(())
        }
        None => reject_funnel_unknown(),
    }
}

fn reject_funnel_active() -> anyhow::Result<()> {
    tracing::error!(
        "tailscale Funnel is enabled; refusing to start in prod. Set BABYLON_ALLOW_FUNNEL=1 to override."
    );
    bail!("tailscale Funnel is active; set BABYLON_ALLOW_FUNNEL=1 to override or disable Funnel")
}

fn reject_funnel_unknown() -> anyhow::Result<()> {
    tracing::error!(
        "cannot verify perimeter: tailscale funnel status unavailable (CLI missing or parse failure). \
         Set BABYLON_ALLOW_FUNNEL=1 to skip."
    );
    bail!(
        "cannot verify tailscale funnel status (CLI missing or parse failure); \
         set BABYLON_ALLOW_FUNNEL=1 to skip"
    )
}
