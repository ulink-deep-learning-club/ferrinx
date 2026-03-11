use crate::client::HttpClient;
use crate::commands::{RegisterModelRequest, UploadModelResponse};
use crate::config::CliConfig;
use crate::error::Result;
use crate::output;
use clap::Subcommand;
use ferrinx_common::ModelInfo;
use std::collections::HashMap;

#[derive(Subcommand)]
pub enum ModelCommands {
    Upload {
        model_path: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        version: String,
    },
    Register {
        server_path: String,
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        version: String,
    },
    List {
        #[arg(short, long)]
        name: Option<String>,
    },
    Info {
        model_id: String,
    },
    Delete {
        model_id: String,
    },
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
        } => {
            let mut form_data = HashMap::new();
            form_data.insert("name".to_string(), name.clone());
            form_data.insert("version".to_string(), version.clone());

            let response: UploadModelResponse = client
                .upload("/models/upload", &model_path, form_data)
                .await?;

            output::print_success("Model uploaded");
            println!("Model ID: {}", response.model_id);
            println!("Name: {}", response.name);
            println!("Version: {}", response.version);
        }
        ModelCommands::Register {
            server_path,
            name,
            version,
        } => {
            let request = RegisterModelRequest {
                file_path: server_path,
                name,
                version,
            };

            let response: UploadModelResponse = client.post("/models/register", &request).await?;

            output::print_success("Model registered");
            println!("Model ID: {}", response.model_id);
            println!("Name: {}", response.name);
            println!("Version: {}", response.version);
        }
        ModelCommands::List { name } => {
            let mut path = "/models".to_string();
            if let Some(n) = name {
                path = format!("{}?name={}", path, n);
            }

            let models: Vec<ModelInfo> = client.get(&path).await?;
            output::print_models(&models, config.output_format)?;
        }
        ModelCommands::Info { model_id } => {
            let model: ModelInfo = client.get(&format!("/models/{}", model_id)).await?;
            output::print_output(&model, config.output_format)?;
        }
        ModelCommands::Delete { model_id } => {
            let _: serde_json::Value = client.delete(&format!("/models/{}", model_id)).await?;
            output::print_success(&format!("Model deleted: {}", model_id));
        }
    }

    Ok(())
}
