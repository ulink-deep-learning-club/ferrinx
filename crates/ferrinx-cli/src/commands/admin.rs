use crate::client::HttpClient;
use crate::commands::{BootstrapRequest, CreateUserRequest, LoginResponse};
use crate::config::CliConfig;
use crate::error::{CliError, Result};
use crate::output;
use clap::Subcommand;
use ferrinx_common::User;

#[derive(Subcommand)]
pub enum AdminCommands {
    CreateUser {
        #[arg(short = 'U', long)]
        username: String,
        #[arg(short, long)]
        password: Option<String>,
        #[arg(short, long, default_value = "user")]
        role: String,
    },
    ListUsers,
    DeleteUser {
        user_id: String,
    },
    Bootstrap {
        #[arg(short = 'U', long)]
        username: String,
        #[arg(short, long)]
        password: Option<String>,
    },
}

pub async fn handle_admin(
    cmd: AdminCommands,
    client: &HttpClient,
    _config: &CliConfig,
) -> Result<()> {
    match cmd {
        AdminCommands::CreateUser {
            username,
            password,
            role,
        } => {
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

            let request = CreateUserRequest {
                username,
                password,
                role,
            };
            let user: User = client.post("/admin/users", &request).await?;

            output::print_success(&format!("User created: {}", user.username));
            println!("User ID: {}", user.id);
        }
        AdminCommands::ListUsers => {
            let users: Vec<serde_json::Value> = client.get("/admin/users").await?;
            output::print_users(&users, _config.output_format)?;
        }
        AdminCommands::DeleteUser { user_id } => {
            let _: serde_json::Value = client.delete(&format!("/admin/users/{}", user_id)).await?;
            output::print_success(&format!("User deleted: {}", user_id));
        }
        AdminCommands::Bootstrap { username, password } => {
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

            let request = BootstrapRequest { username, password };
            let response: LoginResponse = client.post("/admin/bootstrap", &request).await?;

            output::print_success("System bootstrapped successfully");
            println!("Admin user created");
            println!("API key: {}", response.api_key);
            if let Some(expires) = response.expires_at {
                println!("Expires at: {}", expires);
            }
        }
    }

    Ok(())
}
