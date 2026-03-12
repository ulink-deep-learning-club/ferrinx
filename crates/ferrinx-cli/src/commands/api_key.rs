use crate::client::HttpClient;
use crate::commands::{CreateApiKeyRequest, UpdateApiKeyRequest};
use crate::config::CliConfig;
use crate::error::Result;
use crate::output;
use clap::Subcommand;
use ferrinx_common::ApiKeyInfo;

#[derive(Subcommand)]
pub enum ApiKeyCommands {
    Create {
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        permissions: Option<String>,
        #[arg(short, long)]
        expires: Option<u32>,
    },
    List,
    Info {
        key_id: String,
    },
    Revoke {
        key_id: String,
    },
    Update {
        key_id: String,
        #[arg(short, long)]
        name: Option<String>,
        #[arg(short, long)]
        permissions: Option<String>,
    },
}

pub async fn handle_api_key(
    cmd: ApiKeyCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<()> {
    match cmd {
        ApiKeyCommands::Create {
            name,
            permissions,
            expires,
        } => {
            let perms = permissions
                .map(|p| super::parse_permissions(&p))
                .transpose()?;

            let request = CreateApiKeyRequest {
                name,
                permissions: perms,
                expires_in_days: expires,
            };

            #[derive(serde::Deserialize)]
            struct CreateKeyResponse {
                key_id: uuid::Uuid,
                key: String,
                name: String,
            }

            let response: CreateKeyResponse = client.post("/api-keys", &request).await?;

            output::print_success("API key created");
            println!("Key ID: {}", response.key_id);
            println!("Key: {}", response.key);
            println!("Name: {}", response.name);
            output::print_info("Save the key securely - it will not be shown again");
        }
        ApiKeyCommands::List => {
            let keys: Vec<ApiKeyInfo> = client.get("/api-keys").await?;
            output::print_api_keys(&keys, config.output_format)?;
        }
        ApiKeyCommands::Info { key_id } => {
            let key: ApiKeyInfo = client.get(&format!("/api-keys/{}", key_id)).await?;
            output::print_output(&key, config.output_format)?;
        }
        ApiKeyCommands::Revoke { key_id } => {
            let _: serde_json::Value = client.delete(&format!("/api-keys/{}", key_id)).await?;
            output::print_success(&format!("API key revoked: {}", key_id));
        }
        ApiKeyCommands::Update {
            key_id,
            name,
            permissions,
        } => {
            let perms = permissions
                .map(|p| super::parse_permissions(&p))
                .transpose()?;

            let request = UpdateApiKeyRequest {
                name,
                permissions: perms,
            };

            let key: ApiKeyInfo = client
                .put(&format!("/api-keys/{}", key_id), &request)
                .await?;

            output::print_success(&format!("API key updated: {}", key.name));
        }
    }

    Ok(())
}
