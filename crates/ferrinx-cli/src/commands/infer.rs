use crate::client::HttpClient;
use crate::commands::{AsyncInferRequest, AsyncInferResponse, SyncInferRequest, SyncInferResponse};
use crate::config::CliConfig;
use crate::error::{CliError, Result};
use crate::output;
use clap::Subcommand;
use serde::Deserialize;

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct ImageInferResponse {
    pub result: serde_json::Value,
    pub latency_ms: u64,
}

#[derive(Subcommand)]
pub enum InferCommands {
    Sync {
        #[arg(required_unless_present = "name")]
        model_id: Option<String>,
        #[arg(short, long, requires = "version")]
        name: Option<String>,
        #[arg(short, long, requires = "name")]
        version: Option<String>,
        #[arg(short = 'I', long, conflicts_with = "image")]
        input: Option<String>,
        #[arg(short, long, conflicts_with = "input")]
        image: Option<String>,
        #[arg(short = 'O', long)]
        output: Option<String>,
    },
    Async {
        #[arg(required_unless_present = "name")]
        model_id: Option<String>,
        #[arg(short, long, requires = "version")]
        name: Option<String>,
        #[arg(short, long, requires = "name")]
        version: Option<String>,
        #[arg(short = 'I', long, conflicts_with = "image")]
        input: Option<String>,
        #[arg(short, long, conflicts_with = "input")]
        image: Option<String>,
        #[arg(short, long, default_value = "normal")]
        priority: String,
    },
}

pub async fn handle_infer(
    cmd: InferCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<()> {
    match cmd {
        InferCommands::Sync {
            model_id,
            name,
            version,
            input,
            image,
            output,
        } => {
            if let Some(image_path) = image {
                let mut form_data = std::collections::HashMap::new();

                if let Some(id) = model_id {
                    form_data.insert("model_id".to_string(), id);
                } else if let (Some(n), Some(v)) = (name, version) {
                    form_data.insert("name".to_string(), n);
                    form_data.insert("version".to_string(), v);
                } else {
                    return Err(CliError::InvalidInput(
                        "Either model_id or name+version is required".to_string(),
                    ));
                }

                let response: ImageInferResponse = client
                    .upload_image("/inference/image", &image_path, form_data)
                    .await?;

                if let Some(output_file) = output {
                    let json = serde_json::to_string_pretty(&response.result)?;
                    tokio::fs::write(&output_file, json).await?;
                    output::print_success(&format!("Result saved to {}", output_file));
                } else {
                    output::print_output(&response, config.output_format)?;
                }
            } else if let Some(input_str) = input {
                let model_id = resolve_model_id(client, model_id, name, version).await?;
                let inputs = super::parse_input(&input_str)?;

                let request = SyncInferRequest { model_id, inputs };
                let response: SyncInferResponse = client.post("/inference/sync", &request).await?;

                if let Some(output_file) = output {
                    let json = serde_json::to_string_pretty(&response.outputs)?;
                    tokio::fs::write(&output_file, json).await?;
                    output::print_success(&format!("Result saved to {}", output_file));
                } else {
                    output::print_output(&response, config.output_format)?;
                }
            } else {
                return Err(CliError::InvalidInput(
                    "Either --input or --image is required".to_string(),
                ));
            }
        }
        InferCommands::Async {
            model_id,
            name,
            version,
            input,
            image,
            priority,
        } => {
            if let Some(image_path) = image {
                let mut form_data = std::collections::HashMap::new();

                if let Some(id) = model_id {
                    form_data.insert("model_id".to_string(), id);
                } else if let (Some(n), Some(v)) = (name, version) {
                    form_data.insert("name".to_string(), n);
                    form_data.insert("version".to_string(), v);
                } else {
                    return Err(CliError::InvalidInput(
                        "Either model_id or name+version is required".to_string(),
                    ));
                }

                let response: ImageInferResponse = client
                    .upload_image("/inference/image", &image_path, form_data)
                    .await?;

                output::print_success("Image inference completed");
                println!(
                    "Result: {}",
                    serde_json::to_string_pretty(&response.result)?
                );
                println!("Latency: {} ms", response.latency_ms);
            } else if let Some(input_str) = input {
                let model_id = resolve_model_id(client, model_id, name, version).await?;
                let inputs = super::parse_input(&input_str)?;

                let request = AsyncInferRequest {
                    model_id,
                    inputs,
                    options: crate::commands::InferOptions {
                        priority,
                        timeout: 300,
                    },
                };

                let response: AsyncInferResponse = client.post("/inference", &request).await?;

                output::print_output(&response, config.output_format)?;
            } else {
                return Err(CliError::InvalidInput(
                    "Either --input or --image is required".to_string(),
                ));
            }
        }
    }

    Ok(())
}

async fn resolve_model_id(
    client: &HttpClient,
    model_id: Option<String>,
    name: Option<String>,
    version: Option<String>,
) -> Result<String> {
    if let Some(id) = model_id {
        return Ok(id);
    }

    let name = name.ok_or_else(|| CliError::InvalidInput("Model name is required".to_string()))?;
    let version =
        version.ok_or_else(|| CliError::InvalidInput("Model version is required".to_string()))?;

    let model: crate::output::ModelDetail =
        client.get(&format!("/models/{}/{}", name, version)).await?;
    Ok(model.id)
}
