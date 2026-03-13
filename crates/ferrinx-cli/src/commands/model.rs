use crate::client::HttpClient;
use crate::commands::RegisterModelRequest;
use crate::config::CliConfig;
use crate::error::{CliError, Result};
use crate::output::{self, ModelDetail};
use clap::Subcommand;
use std::collections::HashMap;
use std::path::Path;

pub fn embed_labels_in_config(config_content: &str, config_path: &str) -> Result<String> {
    let mut config: ferrinx_core::model::config::ModelConfig = toml::from_str(config_content)
        .map_err(|e| crate::error::CliError::Config(format!("Invalid config TOML: {}", e)))?;

    let base_path = Path::new(config_path).parent().unwrap_or(Path::new("."));
    config.embed_labels(base_path);

    toml::to_string(&config)
        .map_err(|e| crate::error::CliError::Config(format!("Failed to serialize config: {}", e)))
}

/// Extract model file path from config if present, resolving relative to config directory
pub fn get_model_file_from_config(config_content: &str, config_path: &str) -> Result<Option<String>> {
    let config: ferrinx_core::model::config::ModelConfig = toml::from_str(config_content)
        .map_err(|e| CliError::Config(format!("Invalid config TOML: {}", e)))?;

    if let Some(ref model) = config.model {
        let base_path = Path::new(config_path).parent().unwrap_or(Path::new("."));
        let model_path = base_path.join(&model.file);
        Ok(Some(model_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

/// Resolve model file path from config, making it relative to config directory if needed
pub fn resolve_model_file_path(config_content: &str, config_path: &str) -> Result<Option<String>> {
    let config: ferrinx_core::model::config::ModelConfig = toml::from_str(config_content)
        .map_err(|e| CliError::Config(format!("Invalid config TOML: {}", e)))?;

    if let Some(ref model) = config.model {
        let base_path = Path::new(config_path).parent().unwrap_or(Path::new("."));
        let model_path = base_path.join(&model.file);
        Ok(Some(model_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

/// Extract model name from config if present
pub fn get_model_name_from_config(config_content: &str) -> Result<Option<String>> {
    let config: ferrinx_core::model::config::ModelConfig = toml::from_str(config_content)
        .map_err(|e| CliError::Config(format!("Invalid config TOML: {}", e)))?;

    Ok(config.meta.as_ref().and_then(|m| {
        if m.name.is_empty() {
            None
        } else {
            Some(m.name.clone())
        }
    }))
}

/// Extract model version from config if present
pub fn get_model_version_from_config(config_content: &str) -> Result<Option<String>> {
    let config: ferrinx_core::model::config::ModelConfig = toml::from_str(config_content)
        .map_err(|e| CliError::Config(format!("Invalid config TOML: {}", e)))?;

    Ok(config.meta.as_ref().and_then(|m| {
        if m.version.is_empty() {
            None
        } else {
            Some(m.version.clone())
        }
    }))
}

#[derive(Subcommand)]
pub enum ModelCommands {
    Upload {
        /// Local path to model file (optional if model.file is specified in config)
        model_path: Option<String>,
        /// Model name (optional if meta.name is specified in config)
        #[arg(short, long)]
        name: Option<String>,
        /// Model version (optional if meta.version is specified in config)
        #[arg(short, long)]
        version: Option<String>,
        /// Model configuration file
        #[arg(long)]
        model_config: Option<String>,
    },
    Register {
        /// Server path to model file (optional if model.file is specified in config)
        server_path: Option<String>,
        /// Model name (optional if meta.name is specified in config)
        #[arg(short, long)]
        name: Option<String>,
        /// Model version (optional if meta.version is specified in config)
        #[arg(short, long)]
        version: Option<String>,
        /// Model configuration file
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
            // Read config content if provided
            let config_content = if let Some(ref cfg_path) = config_path {
                Some(std::fs::read_to_string(cfg_path)?)
            } else {
                None
            };

            // Resolve model path: use provided path or get from config
            let resolved_model_path = if let Some(path) = model_path {
                path
            } else if let Some(ref content) = config_content {
                resolve_model_file_path(content, config_path.as_ref().unwrap())?
                    .ok_or_else(|| CliError::Config("model.file not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either model_path or --model-config must be provided".to_string()));
            };

            // Resolve name: use provided name or get from config
            let resolved_name = if let Some(n) = name {
                n
            } else if let Some(ref content) = config_content {
                get_model_name_from_config(content)?
                    .ok_or_else(|| CliError::Config("meta.name not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either --name or --model-config must be provided".to_string()));
            };

            // Resolve version: use provided version or get from config
            let resolved_version = if let Some(v) = version {
                v
            } else if let Some(ref content) = config_content {
                get_model_version_from_config(content)?
                    .ok_or_else(|| CliError::Config("meta.version not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either --version or --model-config must be provided".to_string()));
            };

            let mut form_data = HashMap::new();
            form_data.insert("name".to_string(), resolved_name.clone());
            form_data.insert("version".to_string(), resolved_version.clone());

            let response: ModelResponse = client
                .upload_with_config("/models/upload", &resolved_model_path, form_data, config_path.as_deref())
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
            // Read config content if provided
            let config_content = if let Some(ref cfg_path) = config_path {
                Some(std::fs::read_to_string(cfg_path)?)
            } else {
                None
            };

            // Resolve server path: use provided path or get from config
            let resolved_server_path = if let Some(path) = server_path {
                path
            } else if let Some(ref content) = config_content {
                get_model_file_from_config(content, config_path.as_ref().unwrap())?
                    .ok_or_else(|| CliError::Config("model.file not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either server_path or --model-config must be provided".to_string()));
            };

            // Resolve name: use provided name or get from config
            let resolved_name = if let Some(n) = name {
                n
            } else if let Some(ref content) = config_content {
                get_model_name_from_config(content)?
                    .ok_or_else(|| CliError::Config("meta.name not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either --name or --model-config must be provided".to_string()));
            };

            // Resolve version: use provided version or get from config
            let resolved_version = if let Some(v) = version {
                v
            } else if let Some(ref content) = config_content {
                get_model_version_from_config(content)?
                    .ok_or_else(|| CliError::Config("meta.version not specified in config".to_string()))?
            } else {
                return Err(CliError::Config("Either --version or --model-config must be provided".to_string()));
            };

            // Process config content with embedded labels if provided
            let processed_config = if let Some(ref content) = config_content {
                Some(embed_labels_in_config(content, config_path.as_ref().unwrap())?)
            } else {
                None
            };

            let request = RegisterModelRequest {
                file_path: resolved_server_path,
                name: resolved_name,
                version: resolved_version,
                config: processed_config,
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
