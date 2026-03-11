use crate::client::HttpClient;
use crate::config::CliConfig;
use crate::error::Result;
use crate::output;
use clap::Subcommand;
use ferrinx_common::InferenceTask;

#[derive(Subcommand)]
pub enum TaskCommands {
    List {
        #[arg(short, long)]
        status: Option<String>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    Status {
        task_id: String,
    },
    Cancel {
        task_id: String,
    },
}

pub async fn handle_task(
    cmd: TaskCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<()> {
    match cmd {
        TaskCommands::List { status, limit } => {
            let mut path = "/tasks".to_string();
            let mut params = Vec::new();

            if let Some(s) = status {
                params.push(format!("status={}", s));
            }
            if let Some(l) = limit {
                params.push(format!("limit={}", l));
            }

            if !params.is_empty() {
                path = format!("{}?{}", path, params.join("&"));
            }

            let tasks: Vec<InferenceTask> = client.get(&path).await?;
            output::print_tasks(&tasks, config.output_format)?;
        }
        TaskCommands::Status { task_id } => {
            let task: InferenceTask = client.get(&format!("/tasks/{}", task_id)).await?;
            output::print_task_status(&task, config.output_format)?;
        }
        TaskCommands::Cancel { task_id } => {
            #[derive(serde::Deserialize)]
            struct CancelResponse {
                task_id: uuid::Uuid,
                status: String,
            }

            let response: CancelResponse = client
                .post(&format!("/tasks/{}/cancel", task_id), &serde_json::json!({}))
                .await?;

            output::print_success(&format!("Task cancelled: {}", response.task_id));
            println!("Status: {}", response.status);
        }
    }

    Ok(())
}
