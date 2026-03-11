use crate::client::HttpClient;
use crate::commands::{LoginRequest, LoginResponse};
use crate::config::CliConfig;
use crate::error::{CliError, Result};
use crate::output;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum AuthCommands {
    Login {
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: Option<String>,
    },
    Logout,
}

pub async fn handle_auth(
    cmd: AuthCommands,
    client: &HttpClient,
    config: &mut CliConfig,
) -> Result<()> {
    match cmd {
        AuthCommands::Login { username, password } => {
            let password = match password {
                Some(p) => p,
                None => {
                    if atty::is(atty::Stream::Stdin) {
                        rpassword::prompt_password("Password: ")
                            .map_err(|_| CliError::InvalidInput("Failed to read password".to_string()))?
                    } else {
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        input.trim().to_string()
                    }
                }
            };

            let request = LoginRequest { username, password };
            let response: LoginResponse = client.post("/auth/login", &request).await?;

            config.api_key = Some(response.api_key.clone());
            config.save()?;

            output::print_success("Login successful");
            if let Some(expires) = response.expires_at {
                println!("API key expires at: {}", expires);
            }
            println!("API key saved to configuration");
        }
        AuthCommands::Logout => {
            let _response: serde_json::Value = client.post("/auth/logout", &serde_json::json!({})).await?;

            config.api_key = None;
            config.save()?;

            output::print_success("Logout successful");
        }
    }

    Ok(())
}
