use crate::client::HttpClient;
use crate::commands::{AsyncInferRequest, AsyncInferResponse, SyncInferRequest, SyncInferResponse};
use crate::config::CliConfig;
use crate::error::{CliError, Result};
use crate::output;
use clap::Subcommand;

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
        #[arg(short, long, conflicts_with = "image")]
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
            let model_id = resolve_model_id(client, model_id, name, version).await?;
            let inputs = if let Some(image_path) = image {
                preprocess_image(client, &model_id, &image_path).await?
            } else if let Some(input_str) = input {
                super::parse_input(&input_str)?
            } else {
                return Err(CliError::InvalidInput("Either --input or --image is required".to_string()));
            };

            let request = SyncInferRequest { model_id, inputs };
            let response: SyncInferResponse = client.post("/inference/sync", &request).await?;

            if let Some(output_file) = output {
                let json = serde_json::to_string_pretty(&response.outputs)?;
                tokio::fs::write(&output_file, json).await?;
                output::print_success(&format!("Result saved to {}", output_file));
            } else {
                output::print_output(&response, config.output_format)?;
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
            let model_id = resolve_model_id(client, model_id, name, version).await?;
            let inputs = if let Some(image_path) = image {
                preprocess_image(client, &model_id, &image_path).await?
            } else if let Some(input_str) = input {
                super::parse_input(&input_str)?
            } else {
                return Err(CliError::InvalidInput("Either --input or --image is required".to_string()));
            };

            let request = AsyncInferRequest {
                model_id,
                inputs,
                priority,
            };

            let response: AsyncInferResponse = client.post("/inference", &request).await?;

            output::print_success("Task submitted");
            println!("Task ID: {}", response.task_id);
            println!("Status: {}", response.status);
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
    let version = version.ok_or_else(|| CliError::InvalidInput("Model version is required".to_string()))?;

    let model: crate::output::ModelDetail = client.get(&format!("/models/{}/{}", name, version)).await?;
    Ok(model.id)
}

async fn preprocess_image(
    client: &HttpClient,
    model_id: &str,
    image_path: &str,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    let model: crate::output::ModelDetail = client.get(&format!("/models/{}", model_id)).await?;

    let input_info = model.input_shapes
        .as_ref()
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| CliError::InvalidInput("Model has no input shape information".to_string()))?;

    let shape: Vec<i64> = input_info.get("shape")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .ok_or_else(|| CliError::InvalidInput("Invalid input shape format".to_string()))?;

    if shape.len() < 2 {
        return Err(CliError::InvalidInput("Invalid input shape: expected at least 2 dimensions".to_string()));
    }

    let height = shape[shape.len() - 2] as u32;
    let width = shape[shape.len() - 1] as u32;
    let channels = if shape.len() >= 4 { shape[1] as u32 } else { 1 };

    let img = image::open(image_path)
        .map_err(|e| CliError::InvalidInput(format!("Failed to load image: {}", e)))?;

    let resized = img.resize_exact(width, height, image::imageops::FilterType::Lanczos3);

    let mut data: Vec<f32> = Vec::with_capacity((width * height * channels) as usize);

    if channels == 1 {
        let gray = resized.to_luma8();
        for pixel in gray.pixels() {
            let normalized = pixel[0] as f32 / 255.0;
            data.push(normalized);
        }
    } else {
        let rgb = resized.to_rgb8();
        for pixel in rgb.pixels() {
            for c in 0..channels as usize {
                let normalized = pixel[c.min(2)] as f32 / 255.0;
                data.push(normalized);
            }
        }
    }

    let input_name = input_info.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("input");

    Ok(std::collections::HashMap::from([(
        input_name.to_string(),
        serde_json::Value::Array(data.into_iter().map(|v| serde_json::json!(v)).collect()),
    )]))
}
