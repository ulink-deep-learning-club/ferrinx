use crate::error::{CliError, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Toml,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Toml => write!(f, "toml"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default = "default_api_url")]
    pub api_url: String,
    pub api_key: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub output_format: OutputFormat,
    #[serde(default)]
    pub verify_ssl: bool,
}

fn default_api_url() -> String {
    "http://localhost:8080/api/v1".to_string()
}

fn default_timeout() -> u64 {
    30
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
            api_key: None,
            timeout: default_timeout(),
            output_format: OutputFormat::Table,
            verify_ssl: true,
        }
    }
}

impl CliConfig {
    pub fn config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or(CliError::HomeNotFound)?;
        Ok(home.join(".ferrinx").join("config.toml"))
    }

    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = if let Some(p) = path {
            PathBuf::from(p)
        } else {
            Self::config_path()?
        };

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: CliConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "api_url" => self.api_url = value.to_string(),
            "api_key" => self.api_key = Some(value.to_string()),
            "timeout" => {
                self.timeout = value.parse().map_err(|_| {
                    CliError::InvalidInput(format!("Invalid timeout value: {}", value))
                })?;
            }
            "output_format" => {
                self.output_format = match value.to_lowercase().as_str() {
                    "table" => OutputFormat::Table,
                    "json" => OutputFormat::Json,
                    "toml" => OutputFormat::Toml,
                    _ => {
                        return Err(CliError::InvalidInput(format!(
                            "Invalid output format: {}",
                            value
                        )))
                    }
                };
            }
            "verify_ssl" => {
                self.verify_ssl = value.parse().map_err(|_| {
                    CliError::InvalidInput(format!("Invalid boolean value: {}", value))
                })?;
            }
            _ => {
                return Err(CliError::InvalidInput(format!(
                    "Unknown configuration key: {}",
                    key
                )))
            }
        }
        Ok(())
    }
}
