use clap::{Parser, Subcommand};
use ferrinx_cli::client::HttpClient;
use ferrinx_cli::commands::{
    handle_admin, handle_api_key, handle_auth, handle_config, handle_infer, handle_model,
    handle_task, AdminCommands, ApiKeyCommands, AuthCommands, ConfigCommands, InferCommands,
    ModelCommands, TaskCommands,
};
use ferrinx_cli::config::CliConfig;
use ferrinx_cli::error::Result;

#[derive(Parser)]
#[command(name = "ferrinx")]
#[command(about = "Ferrinx ONNX Inference CLI", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, global = true)]
    config: Option<String>,

    #[arg(short, long, global = true)]
    url: Option<String>,

    #[arg(short, long, global = true)]
    api_key: Option<String>,

    #[arg(short, long, global = true, value_enum)]
    output: Option<ferrinx_cli::config::OutputFormat>,
}

#[derive(Subcommand)]
enum Commands {
    Auth {
        #[command(subcommand)]
        cmd: AuthCommands,
    },
    Admin {
        #[command(subcommand)]
        cmd: AdminCommands,
    },
    ApiKey {
        #[command(subcommand)]
        cmd: ApiKeyCommands,
    },
    Model {
        #[command(subcommand)]
        cmd: ModelCommands,
    },
    Infer {
        #[command(subcommand)]
        cmd: InferCommands,
    },
    Task {
        #[command(subcommand)]
        cmd: TaskCommands,
    },
    Config {
        #[command(subcommand)]
        cmd: ConfigCommands,
    },
    Status,
}

async fn handle_status(client: &HttpClient) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct HealthResponse {
        status: String,
        version: String,
        uptime_secs: u64,
    }

    let health: HealthResponse = client.get("/health").await?;

    println!("Status: {}", health.status);
    println!("Version: {}", health.version);
    println!("Uptime: {}s", health.uptime_secs);

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let mut config = CliConfig::load(cli.config.as_deref())?;

    if let Some(url) = cli.url {
        config.api_url = url;
    }
    if let Some(api_key) = cli.api_key {
        config.api_key = Some(api_key);
    }
    if let Some(output) = cli.output {
        config.output_format = output;
    }

    let client = HttpClient::new(&config)?;

    match cli.command {
        Commands::Auth { cmd } => {
            handle_auth(cmd, &client, &mut config).await?;
        }
        Commands::Admin { cmd } => {
            handle_admin(cmd, &client, &config).await?;
        }
        Commands::ApiKey { cmd } => {
            handle_api_key(cmd, &client, &config).await?;
        }
        Commands::Model { cmd } => {
            handle_model(cmd, &client, &config).await?;
        }
        Commands::Infer { cmd } => {
            handle_infer(cmd, &client, &config).await?;
        }
        Commands::Task { cmd } => {
            handle_task(cmd, &client, &config).await?;
        }
        Commands::Config { cmd } => {
            handle_config(cmd, &mut config)?;
        }
        Commands::Status => {
            handle_status(&client).await?;
        }
    }

    Ok(())
}
