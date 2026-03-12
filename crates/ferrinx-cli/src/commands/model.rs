use crate::client::HttpClient;
use crate::commands::RegisterModelRequest;
use crate::config::CliConfig;
use crate::error::Result;
use crate::output::{self, ModelDetail};
use clap::Subcommand;
use std::collections::HashMap;

#[derive(Subcommand)]
pub enum ModelCommands {
    Upload {
        model_path: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        version: String,
        #[arg(long)]
        model_config: Option<String>,
    },
    Register {
        server_path: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        version: String,
        #[arg(long)]
        model_config: Option<String>,
    },
    List {
        #[arg(short, long)]
        name: Option<String>,
    },
    Info {
        model_id: String,
    },
    Update {
        model_id: String,
        #[arg(short, long)]
        name: Option<String>,
        #[arg(short, long)]
        version: Option<String>,
        #[arg(long)]
        model_config: Option<String>,
    },
    Delete {
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        version: String,
    },
}

#[derive(Debug, serde::Deserialize)]
struct ModelResponse {
    id: String,
    name: String,
    version: String,
    #[allow(dead_code)]
    file_path: Option<String>,
    #[allow(dead_code)]
    file_size: Option<i64>,
    is_valid: bool,
    validation_error: Option<String>,
}

pub async fn handle_model(
    cmd: ModelCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<()> {
    match cmd {
        ModelCommands::Upload {
            model_path,
            name,
            version,
            model_config: config_path,
        } => {
            let mut form_data = HashMap::new();
            form_data.insert("name".to_string(), name.clone());
            form_data.insert("version".to_string(), version.clone());

            let response: ModelResponse = client
                .upload_with_config("/models/upload", &model_path, form_data, config_path.as_deref())
                .await?;

            output::print_success("Model uploaded");
            println!("Model ID: {}", response.id);
            println!("Name: {}", response.name);
            println!("Version: {}", response.version);
            if !response.is_valid {
                println!("Validation error: {:?}", response.validation_error);
            }
        }
        ModelCommands::Register {
            server_path,
            name,
            version,
            model_config: config_path,
        } => {
            let config_content = if let Some(path) = config_path {
                Some(std::fs::read_to_string(&path)?)
            } else {
                None
            };

            let request = RegisterModelRequest {
                file_path: server_path,
                name,
                version,
                config: config_content,
            };
            
            let response: ModelResponse = client.post("/models/register", &request).await?;

            output::print_success("Model registered");
            println!("Model ID: {}", response.id);
            println!("Name: {}", response.name);
            println!("Version: {}", response.version);
            if !response.is_valid {
                println!("Validation error: {:?}", response.validation_error);
            }
        }
        ModelCommands::List { name } => {
            let mut path = "/models".to_string();
            if let Some(n) = name {
                path = format!("{}?name={}", path, n);
            }

            let models: Vec<ModelDetail> = client.get(&path).await?;
            output::print_models(&models, config.output_format)?;
        }
        ModelCommands::Info { model_id } => {
            let model: ModelDetail = client.get(&format!("/models/{}", model_id)).await?;
            output::print_output(&model, config.output_format)?;
        }
        ModelCommands::Update {
            model_id,
            name,
            version,
            model_config: config_path,
        } => {
            #[derive(serde::Serialize)]
            struct UpdateModelRequest {
                #[serde(skip_serializing_if = "Option::is_none")]
                name: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                version: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                config: Option<String>,
            }
            
            let config_content = if let Some(path) = config_path {
                Some(std::fs::read_to_string(&path)?)
            } else {
                None
            };

            let request = UpdateModelRequest { 
                name, 
                version,
                config: config_content,
            };
            let model: ModelDetail = client.put(&format!("/models/{}", model_id), &request).await?;
            output::print_success(&format!("Model updated: {}", model.name));
        }
        ModelCommands::Delete { name, version } => {
            client.delete_void(&format!("/models/{}/{}", name, version)).await?;
            output::print_success(&format!("Model deleted: {}:{}", name, version));
        }
    }

    Ok(())
}
