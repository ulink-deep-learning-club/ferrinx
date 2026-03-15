use crate::config::OutputFormat;
use crate::error::Result;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Attribute, Cell, CellAlignment, Color,
    ContentArrangement, Table,
};
use ferrinx_common::{ApiKeyDetail, TaskDetail};
use serde::Serialize;

pub fn print_output<T: Serialize>(value: &T, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => print_json(value),
        OutputFormat::Toml => print_toml(value),
        OutputFormat::Table => print_json(value),
    }
}

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{}", json);
    Ok(())
}

pub fn print_toml<T: Serialize>(value: &T) -> Result<()> {
    let toml = toml::to_string_pretty(value)?;
    println!("{}", toml);
    Ok(())
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ModelDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub is_valid: bool,
    pub input_shapes: Option<serde_json::Value>,
    pub output_shapes: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

pub fn print_models(models: &[ModelDetail], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Table => {
            let mut table = create_table(&["ID", "Name", "Version", "Valid", "Created"]);

            for model in models {
                let short_id = if model.id.len() > 8 {
                    &model.id[..8]
                } else {
                    &model.id
                };
                table.add_row(vec![
                    Cell::new(short_id),
                    Cell::new(&model.name),
                    Cell::new(&model.version),
                    Cell::new(if model.is_valid { "✓" } else { "✗" })
                        .fg(if model.is_valid {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_attribute(Attribute::Bold),
                    Cell::new(&model.created_at),
                ]);
            }

            println!("{table}");
        }
        _ => print_output(&models, format)?,
    }
    Ok(())
}

pub fn print_api_keys(keys: &[ApiKeyDetail], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Table => {
            let mut table = create_table(&["ID", "Name", "Active", "Temporary", "Expires"]);

            for key in keys {
                let short_id = if key.id.len() > 8 {
                    &key.id[..8]
                } else {
                    &key.id
                };
                table.add_row(vec![
                    Cell::new(short_id),
                    Cell::new(&key.name),
                    Cell::new(if key.is_active { "✓" } else { "✗" })
                        .fg(if key.is_active {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_attribute(Attribute::Bold),
                    Cell::new(if key.is_temporary { "✓" } else { "✗" })
                        .fg(if key.is_temporary {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_attribute(Attribute::Bold),
                    Cell::new(key.expires_at.as_deref().unwrap_or("Never")),
                ]);
            }

            println!("{table}");
        }
        _ => print_output(&keys, format)?,
    }
    Ok(())
}

pub fn print_tasks(tasks: &[TaskDetail], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Table => {
            let mut table = create_table(&["ID", "Model", "Status", "Priority", "Created"]);

            for task in tasks {
                let task_id_short = if task.task_id.len() > 8 {
                    &task.task_id[..8]
                } else {
                    &task.task_id
                };
                let model_id_short = if task.model_id.len() > 8 {
                    &task.model_id[..8]
                } else {
                    &task.model_id
                };
                table.add_row(vec![
                    Cell::new(task_id_short),
                    Cell::new(model_id_short),
                    Cell::new(&task.status),
                    Cell::new("-"),
                    Cell::new(&task.created_at),
                ]);
            }

            println!("{table}");
        }
        _ => print_output(&tasks, format)?,
    }
    Ok(())
}

pub fn print_task_status(task: &TaskDetail, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Table => {
            let mut table = create_table(&["Field", "Value"]);

            table.add_row(vec![Cell::new("ID"), Cell::new(&task.task_id)]);
            table.add_row(vec![Cell::new("Model ID"), Cell::new(&task.model_id)]);
            table.add_row(vec![Cell::new("Status"), Cell::new(&task.status)]);
            table.add_row(vec![Cell::new("Created"), Cell::new(&task.created_at)]);

            if let Some(completed) = &task.completed_at {
                table.add_row(vec![Cell::new("Completed"), Cell::new(completed)]);
            }

            if let Some(latency) = task.latency_ms {
                table.add_row(vec![
                    Cell::new("Latency"),
                    Cell::new(format!("{} ms", latency)),
                ]);
            }

            if let Some(error) = &task.error_message {
                table.add_row(vec![Cell::new("Error"), Cell::new(error)]);
            }

            if let Some(outputs) = &task.outputs {
                table.add_row(vec![
                    Cell::new("Outputs"),
                    Cell::new(format!("{:?}", outputs)),
                ]);
            }

            println!("{table}");
        }
        _ => print_output(task, format)?,
    }
    Ok(())
}

pub fn print_users(users: &[serde_json::Value], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Table => {
            let mut table = create_table(&["ID", "Username", "Role", "Active", "Created"]);

            for user in users {
                let id = user["id"].as_str().unwrap_or("");
                let short_id = if id.len() > 8 { &id[..8] } else { id };

                table.add_row(vec![
                    Cell::new(short_id),
                    Cell::new(user["username"].as_str().unwrap_or("")),
                    Cell::new(user["role"].as_str().unwrap_or("")),
                    Cell::new(if user["is_active"].as_bool().unwrap_or(false) {
                        "✓"
                    } else {
                        "✗"
                    })
                    .fg(if user["is_active"].as_bool().unwrap_or(false) {
                        Color::Green
                    } else {
                        Color::Red
                    })
                    .add_attribute(Attribute::Bold),
                    Cell::new(user["created_at"].as_str().unwrap_or("")),
                ]);
            }

            println!("{table}");
        }
        _ => print_output(&users, format)?,
    }
    Ok(())
}

fn create_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let header_row: Vec<Cell> = headers
        .iter()
        .map(|h| {
            Cell::new(h)
                .add_attribute(Attribute::Bold)
                .set_alignment(CellAlignment::Center)
        })
        .collect();
    table.set_header(header_row);

    table
}

pub fn print_success(message: &str) {
    println!("✓ {}", message);
}

pub fn print_error(message: &str) {
    eprintln!("✗ {}", message);
}

pub fn print_info(message: &str) {
    println!("ℹ {}", message);
}
