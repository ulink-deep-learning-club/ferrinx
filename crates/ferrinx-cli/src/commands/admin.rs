use crate::client::HttpClient;
use crate::commands::CreateUserRequest;
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
    UpdateUser {
        user_id: String,
        #[arg(short = 'U', long)]
        username: Option<String>,
        #[arg(short, long)]
        password: Option<String>,
        #[arg(short, long)]
        role: Option<String>,
        #[arg(short, long)]
        active: Option<bool>,
    },
    DeleteUser {
        user_id: String,
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
        AdminCommands::UpdateUser {
            user_id,
            username,
            password,
            role,
            active,
        } => {
            #[derive(serde::Serialize)]
            struct UpdateUserRequest {
                #[serde(skip_serializing_if = "Option::is_none")]
                username: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                password: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                role: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                is_active: Option<bool>,
            }

            let request = UpdateUserRequest {
                username,
                password,
                role,
                is_active: active,
            };
            let user: User = client.put(&format!("/admin/users/{}", user_id), &request).await?;
            output::print_success(&format!("User updated: {}", user.username));
        }
        AdminCommands::DeleteUser { user_id } => {
            client.delete_void(&format!("/admin/users/{}", user_id)).await?;
            output::print_success(&format!("User deleted: {}", user_id));
        }
    }

    Ok(())
}
