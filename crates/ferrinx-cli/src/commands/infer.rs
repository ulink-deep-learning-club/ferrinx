use crate::client::HttpClient;
use crate::commands::{AsyncInferRequest, AsyncInferResponse, SyncInferRequest, SyncInferResponse};
use crate::config::CliConfig;
use crate::error::Result;
use crate::output;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum InferCommands {
    Sync {
        model_id: String,
        #[arg(short, long)]
        input: String,
        #[arg(short, long)]
        output: Option<String>,
    },
    Async {
        model_id: String,
        #[arg(short, long)]
        input: String,
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
            input,
            output,
        } => {
            let inputs = super::parse_input(&input)?;

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
            input,
            priority,
        } => {
            let inputs = super::parse_input(&input)?;

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
