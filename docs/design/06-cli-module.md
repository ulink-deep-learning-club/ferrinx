# ferrinx-cli 模块设计

## 1. 模块职责

`ferrinx-cli` 是命令行客户端，职责包括：
- 用户交互界面
- 通过 HTTP 与 API 服务通信
- 管理员命令（用户、模型、API Key 管理）
- 推理命令（同步/异步）
- 配置管理

**关键特性**：
- 轻量级二进制文件
- 不依赖 core/db
- 仅通过 HTTP 通信
- 支持交互式和非交互模式

## 2. 核心结构设计

### 2.1 CLI 入口

```rust
// src/main.rs

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ferrinx")]
#[command(about = "Ferrinx ONNX Inference CLI", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    /// Configuration file path
    #[arg(short, long, global = true)]
    config: Option<String>,
    
    /// API base URL
    #[arg(short, long, global = true)]
    url: Option<String>,
    
    /// API key
    #[arg(short, long, global = true)]
    api_key: Option<String>,
    
    /// Output format
    #[arg(short, long, global = true, value_enum)]
    output: Option<OutputFormat>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authentication commands
    Auth(AuthCommands),
    
    /// Administrator commands
    Admin(AdminCommands),
    
    /// API Key management
    ApiKey(ApiKeyCommands),
    
    /// Model management
    Model(ModelCommands),
    
    /// Inference commands
    Infer(InferCommands),
    
    /// Task management
    Task(TaskCommands),
    
    /// Configuration management
    Config(ConfigCommands),
    
    /// System status
    Status,
}

#[derive(Parser)]
enum AuthCommands {
    /// Login with username and password
    Login {
        /// Username
        #[arg(short, long)]
        username: String,
        
        /// Password (will prompt if not provided)
        #[arg(short, long)]
        password: Option<String>,
    },
    
    /// Logout and invalidate temporary API key
    Logout,
}

#[derive(Parser)]
enum AdminCommands {
    /// Create a new user
    CreateUser {
        /// Username
        #[arg(short, long)]
        username: String,
        
        /// Password (will prompt if not provided)
        #[arg(short, long)]
        password: Option<String>,
        
        /// User role
        #[arg(short, long, default_value = "user")]
        role: String,
    },
    
    /// List all users
    ListUsers,
    
    /// Delete a user
    DeleteUser {
        /// User ID
        user_id: String,
    },
    
    /// Bootstrap the system (create first admin)
    Bootstrap {
        /// Username
        #[arg(short, long)]
        username: String,
        
        /// Password (will prompt if not provided)
        #[arg(short, long)]
        password: Option<String>,
    },
}

#[derive(Parser)]
enum ApiKeyCommands {
    /// Create a new API key
    Create {
        /// Key name
        #[arg(short, long)]
        name: String,
        
        /// Permissions (JSON)
        #[arg(short, long)]
        permissions: Option<String>,
        
        /// Expiration in days
        #[arg(short, long)]
        expires: Option<u32>,
    },
    
    /// List API keys
    List,
    
    /// Get API key details
    Info {
        /// Key ID
        key_id: String,
    },
    
    /// Revoke an API key
    Revoke {
        /// Key ID
        key_id: String,
    },
    
    /// Update API key
    Update {
        /// Key ID
        key_id: String,
        
        /// New name
        #[arg(short, long)]
        name: Option<String>,
        
        /// New permissions (JSON)
        #[arg(short, long)]
        permissions: Option<String>,
    },
}

#[derive(Parser)]
enum ModelCommands {
    /// Upload a model
    Upload {
        /// Model file path
        model_path: String,
        
        /// Model name
        #[arg(short, long)]
        name: String,
        
        /// Model version
        #[arg(short, long)]
        version: String,
    },
    
    /// Register a model on server
    Register {
        /// Model file path on server
        server_path: String,
        
        /// Model name
        #[arg(short, long)]
        name: String,
        
        /// Model version
        #[arg(short, long)]
        version: String,
    },
    
    /// List models
    List {
        /// Filter by name
        #[arg(short, long)]
        name: Option<String>,
    },
    
    /// Get model details
    Info {
        /// Model ID
        model_id: String,
    },
    
    /// Delete a model
    Delete {
        /// Model ID
        model_id: String,
    },
}

#[derive(Parser)]
enum InferCommands {
    /// Run synchronous inference
    Sync {
        /// Model ID
        model_id: String,
        
        /// Input JSON file or JSON string
        #[arg(short, long)]
        input: String,
        
        /// Output file
        #[arg(short, long)]
        output: Option<String>,
    },
    
    /// Run asynchronous inference
    Async {
        /// Model ID
        model_id: String,
        
        /// Input JSON file or JSON string
        #[arg(short, long)]
        input: String,
        
        /// Priority (high, normal, low)
        #[arg(short, long, default_value = "normal")]
        priority: String,
    },
}

#[derive(Parser)]
enum TaskCommands {
    /// List tasks
    List {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
        
        /// Limit
        #[arg(short, long)]
        limit: Option<usize>,
    },
    
    /// Get task status
    Status {
        /// Task ID
        task_id: String,
    },
    
    /// Cancel a task
    Cancel {
        /// Task ID
        task_id: String,
    },
}

#[derive(Parser)]
enum ConfigCommands {
    /// Set configuration
    Set {
        /// Configuration key
        key: String,
        
        /// Configuration value
        value: String,
    },
    
    /// Show configuration
    Show,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Toml,
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let cli = Cli::parse();
    
    // 加载配置
    let mut config = load_config(cli.config.as_deref())?;
    
    // 命令行参数覆盖配置文件
    if let Some(url) = cli.url {
        config.api_url = url;
    }
    if let Some(api_key) = cli.api_key {
        config.api_key = Some(api_key);
    }
    if let Some(output) = cli.output {
        config.output_format = output;
    }
    
    // 创建 HTTP 客户端
    let client = HttpClient::new(&config)?;
    
    // 执行命令
    match cli.command {
        Commands::Auth(cmd) => handle_auth(cmd, &client, &config).await?,
        Commands::Admin(cmd) => handle_admin(cmd, &client, &config).await?,
        Commands::ApiKey(cmd) => handle_api_key(cmd, &client, &config).await?,
        Commands::Model(cmd) => handle_model(cmd, &client, &config).await?,
        Commands::Infer(cmd) => handle_infer(cmd, &client, &config).await?,
        Commands::Task(cmd) => handle_task(cmd, &client, &config).await?,
        Commands::Config(cmd) => handle_config(cmd, &config)?,
        Commands::Status => handle_status(&client).await?,
    }
    
    Ok(())
}
```

### 2.2 HTTP 客户端

```rust
// src/client.rs

use reqwest::Client;
use serde::de::DeserializeOwned;

pub struct HttpClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl HttpClient {
    pub fn new(config: &CliConfig) -> Result<Self, CliError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout))
            .build()?;
        
        Ok(Self {
            client,
            base_url: config.api_url.clone(),
            api_key: config.api_key.clone(),
            timeout: Duration::from_secs(config.timeout),
        })
    }
    
    /// GET 请求
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        
        let mut request = self.client.get(&url);
        
        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }
        
        let response = request.send().await?;
        
        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }
        
        let body: ApiResponse<T> = response.json().await?;
        
        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }
    
    /// POST 请求
    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        
        let mut request = self.client.post(&url).json(body);
        
        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }
        
        let response = request.send().await?;
        
        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }
        
        let body: ApiResponse<T> = response.json().await?;
        
        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }
    
    /// 上传文件
    pub async fn upload<T: DeserializeOwned>(
        &self,
        path: &str,
        file_path: &str,
        form_data: HashMap<&str, String>,
    ) -> Result<T, CliError> {
        let url = format!("{}{}", self.base_url, path);
        
        let file = tokio::fs::File::open(file_path).await?;
        let file_size = file.metadata().await?.len();
        
        let mut form = reqwest::multipart::Form::new();
        
        // 添加文件
        let part = reqwest::multipart::Part::stream_with_length(
            reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file)),
            file_size,
        )
        .file_name(file_path.split('/').last().unwrap_or("model.onnx").to_string());
        
        form = form.part("file", part);
        
        // 添加其他字段
        for (key, value) in form_data {
            form = form.text(key, value);
        }
        
        let mut request = self.client.post(&url).multipart(form);
        
        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }
        
        let response = request.send().await?;
        
        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }
        
        let body: ApiResponse<T> = response.json().await?;
        
        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }
    
    async fn handle_error_response(&self, response: reqwest::Response) -> CliError {
        let status = response.status();
        
        match response.json::<ApiResponse<()>>().await {
            Ok(body) => {
                if let Some(error) = body.error {
                    CliError::ApiError {
                        code: error.code,
                        message: error.message,
                    }
                } else {
                    CliError::HttpError {
                        status: status.as_u16(),
                        message: status.to_string(),
                    }
                }
            }
            Err(_) => CliError::HttpError {
                status: status.as_u16(),
                message: status.to_string(),
            },
        }
    }
}
```

### 2.3 命令处理器

```rust
// src/commands/mod.rs

pub async fn handle_infer(
    cmd: InferCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<(), CliError> {
    match cmd {
        InferCommands::Sync { model_id, input, output } => {
            // 解析输入
            let inputs = parse_input(&input)?;
            
            // 调用 API
            let request = SyncInferRequest { model_id, inputs };
            let response: SyncInferResponse = client.post("/api/v1/inference/sync", &request).await?;
            
            // 输出结果
            if let Some(output_file) = output {
                let json = serde_json::to_string_pretty(&response.outputs)?;
                tokio::fs::write(&output_file, json).await?;
                println!("Result saved to {}", output_file);
            } else {
                print_output(&response, config.output_format)?;
            }
        }
        
        InferCommands::Async { model_id, input, priority } => {
            let inputs = parse_input(&input)?;
            
            let request = AsyncInferRequest {
                model_id,
                inputs,
                options: InferOptions {
                    priority,
                    timeout: 300,
                },
            };
            
            let response: AsyncInferResponse = client.post("/api/v1/inference", &request).await?;
            
            println!("Task submitted: {}", response.task_id);
            println!("Status: {}", response.status);
        }
    }
    
    Ok(())
}

pub async fn handle_model(
    cmd: ModelCommands,
    client: &HttpClient,
    config: &CliConfig,
) -> Result<(), CliError> {
    match cmd {
        ModelCommands::Upload { model_path, name, version } => {
            let mut form_data = HashMap::new();
            form_data.insert("name", name);
            form_data.insert("version", version);
            
            let response: ModelUploadResponse = client
                .upload("/api/v1/models/upload", &model_path, form_data)
                .await?;
            
            println!("Model uploaded: {}", response.model_id);
            println!("Name: {}", response.name);
            println!("Version: {}", response.version);
        }
        
        ModelCommands::List { name } => {
            let mut path = "/api/v1/models".to_string();
            if let Some(name) = name {
                path = format!("{}?name={}", path, name);
            }
            
            let models: Vec<ModelInfo> = client.get(&path).await?;
            
            print_table(&models, &["ID", "Name", "Version", "Valid"])?;
        }
        
        // ... 其他命令
    }
    
    Ok(())
}
```

### 2.4 输出格式化

```rust
// src/output.rs

use prettytable::{Table, Row, Cell};

pub fn print_table<T: TableRow>(items: &[T], headers: &[&str]) -> Result<(), CliError> {
    let mut table = Table::new();
    
    // 添加表头
    table.add_row(Row::new(
        headers.iter().map(|h| Cell::new(h).style_spec("Fc")).collect()
    ));
    
    // 添加行
    for item in items {
        table.add_row(item.to_row());
    }
    
    table.printstd();
    
    Ok(())
}

pub trait TableRow {
    fn to_row(&self) -> Row;
}

impl TableRow for ModelInfo {
    fn to_row(&self) -> Row {
        Row::new(vec![
            Cell::new(&self.id.to_string()),
            Cell::new(&self.name),
            Cell::new(&self.version),
            Cell::new(if self.is_valid { "✓" } else { "✗" }),
        ])
    }
}

pub fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{}", json);
    Ok(())
}
```

## 3. 配置管理

```rust
// src/config.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub api_url: String,
    pub api_key: Option<String>,
    pub timeout: u64,
    pub output_format: OutputFormat,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080/api/v1".to_string(),
            api_key: None,
            timeout: 30,
            output_format: OutputFormat::Table,
        }
    }
}

impl CliConfig {
    pub fn load() -> Result<Self, CliError> {
        let config_path = Self::config_path()?;
        
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: CliConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }
    
    pub fn save(&self) -> Result<(), CliError> {
        let config_path = Self::config_path()?;
        
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        
        Ok(())
    }
    
    fn config_path() -> Result<PathBuf, CliError> {
        let home = dirs::home_dir().ok_or(CliError::HomeNotFound)?;
        Ok(home.join(".ferrinx").join("config.toml"))
    }
}
```

## 4. 设计要点

### 4.1 轻量级设计

- 不依赖 ferrinx-core 和 ferrinx-db
- 二进制文件小
- 仅通过 HTTP 通信

### 4.2 用户友好

- 自动提示输入密码
- 支持多种输出格式
- 清晰的错误消息

### 4.3 配置管理

- 配置文件存储在 `~/.ferrinx/config.toml`
- 支持环境变量覆盖
- 命令行参数优先级最高

### 4.4 错误处理

- 统一的错误类型
- 清晰的错误消息
- 自动重试（可选）
