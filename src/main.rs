use anyhow::Result;
use clap::{Parser, Subcommand};
use filesync::{
    agent,
    config::{self, AgentConfig, HubConfig, Mapping},
    hub,
    model::{Edge, NewNode},
    tui,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Two-tier file distribution (development preview)")]
struct Cli {
    #[arg(long, global = true, default_value = "config/agent.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the central service; use --config config/hub.toml.
    Hub,
    /// Run the background agent independently of the TUI.
    Agent {
        #[arg(long)]
        once: bool,
    },
    /// Read current agent status as JSON.
    Status,
    /// Read the first 100 pending/conflicting/failed items as JSON.
    Pending,
    /// Claim a file, or a prefix ending in /, on the running local agent.
    Claim {
        #[arg(long)]
        upstream: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: PathBuf,
    },
    /// Validate agent configuration without changing it.
    Check,
    /// Initial administration through the hub API.
    Admin {
        #[arg(long)]
        hub: String,
        #[arg(long, default_value = "FILESYNC_ADMIN_TOKEN")]
        token_env: String,
        #[arg(long)]
        ca_cert: Option<PathBuf>,
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    /// Returns a node credential once; handle the JSON as a secret.
    NewNode { name: String },
    AddEdge {
        upstream: String,
        downstream: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "filesync=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    filesync::init_crypto();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()?;
    runtime.block_on(run(Cli::parse()))
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Hub) => hub::serve(config::read::<HubConfig>(&cli.config)?).await,
        Some(Command::Agent { once }) => agent::run(&cli.config, once).await,
        Some(Command::Check) => {
            let config: AgentConfig = config::read(&cli.config)?;
            config.validate()?;
            println!("configuration valid");
            Ok(())
        }
        Some(Command::Admin {
            hub,
            token_env,
            ca_cert,
            action,
        }) => {
            config::validate_url(&hub)?;
            let client = agent::client(ca_cert.as_deref())?;
            let token = config::secret(&token_env)?;
            let response = match action {
                AdminAction::NewNode { name } => {
                    client
                        .post(format!("{}/v1/nodes", hub.trim_end_matches('/')))
                        .bearer_auth(token)
                        .json(&NewNode { name })
                        .send()
                        .await?
                }
                AdminAction::AddEdge {
                    upstream,
                    downstream,
                } => {
                    client
                        .post(format!("{}/v1/edges", hub.trim_end_matches('/')))
                        .bearer_auth(token)
                        .json(&Edge {
                            upstream,
                            downstream,
                        })
                        .send()
                        .await?
                }
            }
            .error_for_status()?;
            let text = response.text().await?;
            if !text.is_empty() {
                println!("{text}");
            }
            Ok(())
        }
        command => {
            let config: AgentConfig = config::read(&cli.config)?;
            config.validate()?;
            if command.is_none() {
                return tui::run(&config).await;
            }
            let url = agent::control_url(&config)?;
            let token = config::node_secret(&config)?;
            let client = agent::client(None)?;
            let response = match command.unwrap() {
                Command::Status => {
                    client
                        .get(format!("{url}/status"))
                        .bearer_auth(token)
                        .send()
                        .await?
                }
                Command::Pending => {
                    client
                        .get(format!("{url}/pending"))
                        .bearer_auth(token)
                        .send()
                        .await?
                }
                Command::Claim { upstream, from, to } => {
                    client
                        .post(format!("{url}/claim"))
                        .bearer_auth(token)
                        .json(&Mapping { upstream, from, to })
                        .send()
                        .await?
                }
                _ => unreachable!(),
            }
            .error_for_status()?;
            let text = response.text().await?;
            if !text.is_empty() {
                println!("{text}");
            }
            Ok(())
        }
    }
}
