use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("API error [{code}]: {message}")]
    ApiError { code: String, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("TOML serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("HTTP client error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Home directory not found")]
    HomeNotFound,

    #[error("Authentication required")]
    AuthRequired,

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Operation cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, CliError>;
