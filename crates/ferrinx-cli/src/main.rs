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
    Bootstrap,
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

async fn handle_bootstrap(client: &HttpClient, config: &mut CliConfig) -> Result<()> {
    use ferrinx_cli::error::CliError;

    #[derive(serde::Deserialize)]
    struct BootstrapResponse {
        api_key: String,
        #[allow(dead_code)]
        user_id: String,
        username: String,
        password: String,
    }

    let response: BootstrapResponse = match client.post_raw("/bootstrap", serde_json::json!({})).await {
        Ok(res) => res,
        Err(e) => {
            if e.to_string().contains("System already initialized") {
                return Err(CliError::InvalidInput(
                    "System is already initialized. Run 'ferrinx auth login' to authenticate.".to_string()
                ));
            }
            return Err(e);
        }
    };

    println!("✓ System initialized successfully!\n");
    println!("  Admin user: {}", response.username);
    println!("  Password:   {}", response.password);
    println!("  API key:    {}\n", response.api_key);
    println!("The API key has been saved to ~/.ferrinx/config.toml");
    println!("You can now use other CLI commands.\n");
    println!("Try: ferrinx model list");

    config.api_key = Some(response.api_key);
    config.save()?;

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        // Print the full error chain for debugging
        let mut source = std::error::Error::source(&e);
        while let Some(err) = source {
            eprintln!("  Caused by: {}", err);
            source = std::error::Error::source(err);
        }
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
        Commands::Bootstrap => {
            handle_bootstrap(&client, &mut config).await?;
        }
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
