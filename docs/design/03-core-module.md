# ferrinx-core 模块设计

## 1. 模块职责

`ferrinx-core` 是核心业务逻辑层，职责包括：
- ONNX 模型加载与管理
- 推理引擎执行（基于 `ort`）
- 模型缓存管理（LRU）
- 模型存储抽象
- 推理并发控制

**关键特性**：
- CPU 密集推理使用 `spawn_blocking`
- 并发限制使用 `Semaphore`
- LRU 缓存减少模型加载延迟
- 存储后端可插拔（Local/S3）

## 2. 核心结构设计

### 2.1 推理引擎

```rust
// src/inference/engine.rs

use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use ort::Session;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 推理引擎
pub struct InferenceEngine {
    /// 模型缓存（LRU）
    cache: Arc<RwLock<ModelCache>>,
    /// 并发限制信号量
    semaphore: Arc<Semaphore>,
    /// 推理超时
    timeout: Duration,
    /// ONNX 执行提供者
    execution_provider: ExecutionProvider,
    /// GPU 设备 ID
    gpu_device_id: u32,
}

/// 模型缓存
struct ModelCache {
    sessions: LruCache<String, Arc<Session>>,
    max_size: usize,
}

impl ModelCache {
    fn new(max_size: usize) -> Self {
        Self {
            sessions: LruCache::new(NonZeroUsize::new(max_size).unwrap()),
            max_size,
        }
    }
    
    fn get(&mut self, model_id: &str) -> Option<Arc<Session>> {
        self.sessions.get(model_id).cloned()
    }
    
    fn put(&mut self, model_id: String, session: Arc<Session>) {
        self.sessions.put(model_id, session);
    }
    
    fn len(&self) -> usize {
        self.sessions.len()
    }
}

impl InferenceEngine {
    pub fn new(config: &OnnxConfig) -> Result<Self, CoreError> {
        Ok(Self {
            cache: Arc::new(RwLock::new(ModelCache::new(config.cache_size))),
            semaphore: Arc::new(Semaphore::new(config.cache_size)),
            timeout: Duration::from_secs(30),
            execution_provider: config.execution_provider.clone(),
            gpu_device_id: config.gpu_device_id,
        })
    }
    
    /// 执行推理
    pub async fn infer(
        &self,
        model_id: &str,
        model_path: &str,
        inputs: InferenceInput,
    ) -> Result<InferenceOutput, CoreError> {
        let start = Instant::now();
        
        // 1. 获取并发许可
        let _permit = self.semaphore.acquire().await
            .map_err(|_| CoreError::ConcurrencyLimitReached)?;
        
        // 2. 获取或加载模型 Session
        let session = self.get_or_load_session(model_id, model_path).await?;
        
        // 3. 准备输入张量
        let input_tensors = self.prepare_inputs(&session, inputs)?;
        
        // 4. spawn_blocking 执行推理
        let session_clone = session.clone();
        let input_tensors_clone = input_tensors.clone();
        let timeout = self.timeout;
        
        let result = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                session_clone.run(input_tensors_clone)
            }).await
                .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))?
        }).await
            .map_err(|_| CoreError::InferenceTimeout)?;
        
        // 5. 解析输出
        let outputs = self.parse_outputs(result?)?;
        let latency_ms = start.elapsed().as_millis() as u64;
        
        Ok(InferenceOutput {
            outputs,
            latency_ms,
        })
    }
    
    /// 获取或加载模型 Session
    async fn get_or_load_session(
        &self,
        model_id: &str,
        model_path: &str,
    ) -> Result<Arc<Session>, CoreError> {
        // 先尝试从缓存读取
        {
            let cache = self.cache.read().await;
            if let Some(session) = cache.get(model_id) {
                return Ok(session);
            }
        }
        
        // 缓存未命中，加载模型
        let session = self.load_session(model_path).await?;
        
        // 写入缓存
        {
            let mut cache = self.cache.write().await;
            cache.put(model_id.to_string(), session.clone());
        }
        
        Ok(session)
    }
    
    /// 加载 ONNX Session
    async fn load_session(&self, model_path: &str) -> Result<Arc<Session>, CoreError> {
        let execution_provider = self.execution_provider.clone();
        let gpu_device_id = self.gpu_device_id;
        
        // spawn_blocking 加载模型（文件 I/O + ONNX 初始化）
        let session = tokio::task::spawn_blocking(move || {
            let mut builder = Session::builder()
                .map_err(|e| CoreError::SessionCreationFailed(e.to_string()))?;
            
            match execution_provider {
                ExecutionProvider::CPU => {
                    // CPU 默认提供者
                }
                ExecutionProvider::CUDA => {
                    builder = builder
                        .with_cuda()
                        .map_err(|e| CoreError::ExecutionProviderError(e.to_string()))?
                        .with_device_id(gpu_device_id);
                }
                ExecutionProvider::TensorRT => {
                    builder = builder
                        .with_tensorrt()
                        .map_err(|e| CoreError::ExecutionProviderError(e.to_string()))?
                        .with_device_id(gpu_device_id);
                }
            }
            
            builder
                .with_model_from_file(model_path)
                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))
        })
        .await
        .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))??;
        
        Ok(Arc::new(session))
    }
    
    /// 准备输入张量
    fn prepare_inputs(
        &self,
        session: &Session,
        inputs: InferenceInput,
    ) -> Result<HashMap<String, ort::Value>, CoreError> {
        let mut input_tensors = HashMap::new();
        
        for (input_name, input_data) in inputs.inputs {
            let input_info = session
                .inputs
                .get(&input_name)
                .ok_or_else(|| CoreError::InputNotFound(input_name.clone()))?;
            
            let tensor = self.value_to_tensor(input_data, &input_info.input_type)?;
            input_tensors.insert(input_name, tensor);
        }
        
        Ok(input_tensors)
    }
    
    /// 将 JSON 值转换为 ONNX 张量
    fn value_to_tensor(
        &self,
        value: serde_json::Value,
        input_type: &ort::InputType,
    ) -> Result<ort::Value, CoreError> {
        // 根据输入类型创建张量
        match input_type {
            ort::InputType::Tensor { ty, dimensions } => {
                match ty {
                    ort::TensorElementType::Float32 => {
                        let data: Vec<f32> = serde_json::from_value(value)?;
                        let shape: Vec<usize> = dimensions
                            .iter()
                            .map(|d| *d as usize)
                            .collect();
                        Ok(ort::Value::from_array(
                            ndarray::ArrayD::from_shape_vec(shape, data)?
                        )?)
                    }
                    ort::TensorElementType::Int64 => {
                        let data: Vec<i64> = serde_json::from_value(value)?;
                        let shape: Vec<usize> = dimensions
                            .iter()
                            .map(|d| *d as usize)
                            .collect();
                        Ok(ort::Value::from_array(
                            ndarray::ArrayD::from_shape_vec(shape, data)?
                        )?)
                    }
                    // ... 其他类型
                    _ => Err(CoreError::UnsupportedTensorType),
                }
            }
            _ => Err(CoreError::UnsupportedInputType),
        }
    }
    
    /// 解析输出
    fn parse_outputs(
        &self,
        outputs: HashMap<String, ort::Value>,
    ) -> Result<HashMap<String, serde_json::Value>, CoreError> {
        let mut result = HashMap::new();
        
        for (output_name, output_value) in outputs {
            let json_value = self.tensor_to_json(&output_value)?;
            result.insert(output_name, json_value);
        }
        
        Ok(result)
    }
    
    /// 将 ONNX 张量转换为 JSON
    fn tensor_to_json(&self, value: &ort::Value) -> Result<serde_json::Value, CoreError> {
        // 根据张量类型提取数据并转换为 JSON
        // 实现细节省略
        unimplemented!()
    }
    
    /// 预加载模型
    pub async fn preload_models(
        &self,
        models: &[(String, String)], // (model_id, model_path)
    ) -> Result<(), CoreError> {
        for (model_id, model_path) in models {
            match self.get_or_load_session(model_id, model_path).await {
                Ok(_) => info!("Preloaded model: {}", model_id),
                Err(e) => warn!("Failed to preload model {}: {}", model_id, e),
            }
        }
        Ok(())
    }
    
    /// 清除缓存
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.sessions.clear();
    }
    
    /// 获取缓存状态
    pub async fn cache_status(&self) -> CacheStatus {
        let cache = self.cache.read().await;
        CacheStatus {
            loaded_models: cache.len(),
            max_size: cache.max_size,
        }
    }
    
    /// 获取并发状态
    pub fn concurrency_status(&self) -> ConcurrencyStatus {
        ConcurrencyStatus {
            available_permits: self.semaphore.available_permits(),
            total_permits: self.semaphore.total_permits(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStatus {
    pub loaded_models: usize,
    pub max_size: usize,
}

#[derive(Debug, Clone)]
pub struct ConcurrencyStatus {
    pub available_permits: usize,
    pub total_permits: usize,
}
```

### 2.2 模型加载器

```rust
// src/model/loader.rs

use ort::Session;
use std::path::Path;

/// 模型加载器
pub struct ModelLoader {
    storage: Arc<dyn ModelStorage>,
}

impl ModelLoader {
    pub fn new(storage: Arc<dyn ModelStorage>) -> Self {
        Self { storage }
    }
    
    /// 从存储加载模型文件到内存
    pub async fn load_model_data(&self, path: &str) -> Result<Vec<u8>, CoreError> {
        self.storage.load(path).await
    }
    
    /// 验证模型文件
    pub async fn validate_model(&self, data: &[u8]) -> Result<ModelMetadata, CoreError> {
        // 1. 检查 ONNX magic number
        self.check_onnx_magic(data)?;
        
        // 2. 解析模型元信息
        let metadata = self.extract_metadata(data)?;
        
        Ok(metadata)
    }
    
    /// 检查 ONNX 文件头
    fn check_onnx_magic(&self, data: &[u8]) -> Result<(), CoreError> {
        // ONNX protobuf 不像其他格式有明确的 magic number
        // 但可以通过尝试解析来验证
        if data.len() < 4 {
            return Err(CoreError::InvalidModelFormat("File too small".to_string()));
        }
        
        // 简单的 protobuf 结构检查
        // ONNX 文件通常以 0x08 或 0x0a 开头
        if data[0] != 0x08 && data[0] != 0x0a {
            return Err(CoreError::InvalidModelFormat(
                "Invalid ONNX file header".to_string()
            ));
        }
        
        Ok(())
    }
    
    /// 提取模型元信息
    fn extract_metadata(&self, data: &[u8]) -> Result<ModelMetadata, CoreError> {
        // 临时文件创建 Session 以提取元信息
        let temp_file = tempfile::NamedTempFile::new()?;
        tokio::fs::write(&temp_file, data).await?;
        
        let session = Session::builder()
            .with_model_from_file(temp_file.path())
            .map_err(|e| CoreError::ModelParseFailed(e.to_string()))?;
        
        let mut inputs = Vec::new();
        for (name, input) in session.inputs {
            inputs.push(TensorInfo {
                name,
                shape: match input.input_type {
                    ort::InputType::Tensor { dimensions, .. } => {
                        dimensions.iter().map(|d| *d as i64).collect()
                    }
                    _ => vec![],
                },
                element_type: match input.input_type {
                    ort::InputType::Tensor { ty, .. } => format!("{:?}", ty),
                    _ => "unknown".to_string(),
                },
            });
        }
        
        let mut outputs = Vec::new();
        for (name, output) in session.outputs {
            outputs.push(TensorInfo {
                name,
                shape: match output.output_type {
                    ort::OutputType::Tensor { dimensions, .. } => {
                        dimensions.iter().map(|d| *d as i64).collect()
                    }
                    _ => vec![],
                },
                element_type: match output.output_type {
                    ort::OutputType::Tensor { ty, .. } => format!("{:?}", ty),
                    _ => "unknown".to_string(),
                },
            });
        }
        
        Ok(ModelMetadata {
            inputs,
            outputs,
            opset_version: None, // 需要从模型元数据中提取
            producer_name: None,
        })
    }
    
    /// 验证模型可执行性（可选，较重）
    pub async fn validate_executable(
        &self,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), CoreError> {
        let temp_file = tempfile::NamedTempFile::new()?;
        tokio::fs::write(&temp_file, data).await?;
        
        // 尝试创建 Session
        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                Session::builder()
                    .with_model_from_file(temp_file.path())
            })
        ).await
            .map_err(|_| CoreError::ValidationTimeout)??;
        
        result.map_err(|e| CoreError::SessionCreationFailed(e.to_string()))?;
        
        Ok(())
    }
}

/// 模型元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub inputs: Vec<TensorInfo>,
    pub outputs: Vec<TensorInfo>,
    pub opset_version: Option<i64>,
    pub producer_name: Option<String>,
}

/// 张量信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorInfo {
    pub name: String,
    pub shape: Vec<i64>,
    pub element_type: String,
}
```

### 2.3 存储抽象层

```rust
// src/storage/mod.rs

use async_trait::async_trait;

/// 模型存储接口
#[async_trait]
pub trait ModelStorage: Send + Sync {
    /// 保存模型文件，返回存储路径
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError>;
    
    /// 加载模型文件
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError>;
    
    /// 删除模型文件
    async fn delete(&self, path: &str) -> Result<(), StorageError>;
    
    /// 检查文件是否存在
    async fn exists(&self, path: &str) -> Result<bool, StorageError>;
    
    /// 获取文件大小
    async fn size(&self, path: &str) -> Result<u64, StorageError>;
}

/// 本地存储实现
pub struct LocalStorage {
    base_path: PathBuf,
}

impl LocalStorage {
    pub fn new(base_path: &str) -> Result<Self, StorageError> {
        let path = PathBuf::from(base_path);
        
        // 确保目录存在
        std::fs::create_dir_all(&path)
            .map_err(|e| StorageError::IoError(e))?;
        
        Ok(Self { base_path: path })
    }
}

#[async_trait]
impl ModelStorage for LocalStorage {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError> {
        let filename = format!("{}.onnx", model_id);
        let path = self.base_path.join(&filename);
        
        tokio::fs::write(&path, data).await
            .map_err(|e| StorageError::IoError(e))?;
        
        Ok(path.to_string_lossy().to_string())
    }
    
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        tokio::fs::read(path).await
            .map_err(|e| StorageError::IoError(e))
    }
    
    async fn delete(&self, path: &str) -> Result<(), StorageError> {
        tokio::fs::remove_file(path).await
            .map_err(|e| StorageError::IoError(e))
    }
    
    async fn exists(&self, path: &str) -> Result<bool, StorageError> {
        Ok(tokio::fs::metadata(path).await.is_ok())
    }
    
    async fn size(&self, path: &str) -> Result<u64, StorageError> {
        let metadata = tokio::fs::metadata(path).await
            .map_err(|e| StorageError::IoError(e))?;
        Ok(metadata.len())
    }
}

/// S3 存储实现（可选）
#[cfg(feature = "s3-storage")]
pub struct S3Storage {
    bucket: String,
    client: aws_sdk_s3::Client,
}

#[cfg(feature = "s3-storage")]
impl S3Storage {
    pub async fn new(config: &S3Config) -> Result<Self, StorageError> {
        let config = aws_config::load_from_env().await;
        let client = aws_sdk_s3::Client::new(&config);
        
        Ok(Self {
            bucket: config.bucket.clone(),
            client,
        })
    }
}

#[cfg(feature = "s3-storage")]
#[async_trait]
impl ModelStorage for S3Storage {
    async fn save(&self, model_id: &str, data: &[u8]) -> Result<String, StorageError> {
        let key = format!("models/{}.onnx", model_id);
        
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(data.to_vec().into())
            .send()
            .await
            .map_err(|e| StorageError::S3Error(e.to_string()))?;
        
        Ok(format!("s3://{}/{}", self.bucket, key))
    }
    
    async fn load(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let key = path
            .strip_prefix(&format!("s3://{}/", self.bucket))
            .ok_or_else(|| StorageError::InvalidPath(path.to_string()))?;
        
        let output = self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::S3Error(e.to_string()))?;
        
        let data = output
            .body
            .collect()
            .await
            .map_err(|e| StorageError::S3Error(e.to_string()))?
            .to_vec();
        
        Ok(data)
    }
    
    // ... 其他方法实现
}
```

### 2.4 并发限制器

```rust
// src/inference/limiter.rs

use tokio::sync::Semaphore;
use std::sync::Arc;

/// 推理并发限制器
pub struct InferenceLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrency: usize,
}

impl InferenceLimiter {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            max_concurrency,
        }
    }
    
    /// 获取执行许可
    pub async fn acquire(&self) -> Result<InferencePermit, CoreError> {
        let permit = self.semaphore.acquire().await
            .map_err(|_| CoreError::ConcurrencyLimitReached)?;
        
        Ok(InferencePermit { permit })
    }
    
    /// 尝试立即获取许可
    pub fn try_acquire(&self) -> Option<InferencePermit> {
        self.semaphore.try_acquire().ok().map(|permit| InferencePermit { permit })
    }
    
    /// 获取可用许可数
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }
    
    /// 获取最大并发数
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }
}

/// 推理执行许可
pub struct InferencePermit {
    permit: tokio::sync::SemaphorePermit<'static>,
}
```

## 3. 错误处理

```rust
// src/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    
    #[error("Model load failed: {0}")]
    ModelLoadFailed(String),
    
    #[error("Invalid model format: {0}")]
    InvalidModelFormat(String),
    
    #[error("Model parse failed: {0}")]
    ModelParseFailed(String),
    
    #[error("Session creation failed: {0}")]
    SessionCreationFailed(String),
    
    #[error("Inference failed: {0}")]
    InferenceFailed(String),
    
    #[error("Inference timeout")]
    InferenceTimeout,
    
    #[error("Concurrency limit reached")]
    ConcurrencyLimitReached,
    
    #[error("Input not found: {0}")]
    InputNotFound(String),
    
    #[error("Unsupported tensor type")]
    UnsupportedTensorType,
    
    #[error("Unsupported input type")]
    UnsupportedInputType,
    
    #[error("Execution provider error: {0}")]
    ExecutionProviderError(String),
    
    #[error("Validation timeout")]
    ValidationTimeout,
    
    #[error("Blocking task failed: {0}")]
    BlockingTaskFailed(String),
    
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    
    #[error("ONNX error: {0}")]
    OrtError(#[from] ort::Error),
    
    #[error("Ndarray error: {0}")]
    NdarrayError(String),
}

impl From<ndarray::ShapeError> for CoreError {
    fn from(err: ndarray::ShapeError) -> Self {
        CoreError::NdarrayError(err.to_string())
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("S3 error: {0}")]
    S3Error(String),
    
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    
    #[error("File not found: {0}")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

## 4. 模块组织

```rust
// src/lib.rs

pub mod error;
pub mod inference;
pub mod model;
pub mod storage;

pub use error::*;
pub use inference::*;
pub use model::*;
pub use storage::*;

// 重导出常用类型
pub use inference::engine::{InferenceEngine, CacheStatus, ConcurrencyStatus};
pub use model::loader::{ModelLoader, ModelMetadata};
pub use storage::{ModelStorage, LocalStorage};
```

## 5. 使用示例

### 5.1 同步推理

```rust
use ferrinx_core::{InferenceEngine, InferenceInput};
use ferrinx_common::Config;

async fn run_sync_inference() -> Result<(), Box<dyn std::error::Error>> {
    // 创建推理引擎
    let config = Config::from_file("config.toml")?;
    let engine = InferenceEngine::new(&config.onnx)?;
    
    // 准备输入
    let inputs = InferenceInput {
        inputs: vec![
            ("input.1".to_string(), json!([[1.0, 2.0, 3.0]])),
        ].into_iter().collect(),
    };
    
    // 执行推理
    let result = engine.infer(
        "model-123",
        "/models/model-123.onnx",
        inputs,
    ).await?;
    
    println!("Output: {:?}", result.outputs);
    println!("Latency: {}ms", result.latency_ms);
    
    Ok(())
}
```

### 5.2 模型上传与验证

```rust
use ferrinx_core::{ModelLoader, LocalStorage};

async fn upload_model(model_data: Vec<u8>) -> Result<ModelMetadata, CoreError> {
    // 创建存储和加载器
    let storage = Arc::new(LocalStorage::new("./models")?);
    let loader = ModelLoader::new(storage.clone());
    
    // 验证模型
    let metadata = loader.validate_model(&model_data).await?;
    
    // 保存模型
    let model_id = uuid::Uuid::new_v4().to_string();
    let path = storage.save(&model_id, &model_data).await?;
    
    println!("Model saved to: {}", path);
    println!("Metadata: {:?}", metadata);
    
    Ok(metadata)
}
```

### 5.3 预加载模型

```rust
async fn preload_models(engine: &InferenceEngine) -> Result<(), CoreError> {
    let models = vec![
        ("model-1".to_string(), "/models/model-1.onnx".to_string()),
        ("model-2".to_string(), "/models/model-2.onnx".to_string()),
    ];
    
    engine.preload_models(&models).await?;
    
    // 检查缓存状态
    let status = engine.cache_status().await;
    println!("Loaded {} models", status.loaded_models);
    
    Ok(())
}
```

## 6. 测试策略

### 6.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    fn setup_test_storage() -> (TempDir, LocalStorage) {
        let temp_dir = TempDir::new().unwrap();
        let storage = LocalStorage::new(temp_dir.path().to_str().unwrap()).unwrap();
        (temp_dir, storage)
    }
    
    #[tokio::test]
    async fn test_local_storage_save_load() {
        let (_temp_dir, storage) = setup_test_storage();
        
        let data = vec![1, 2, 3, 4, 5];
        let path = storage.save("test-model", &data).await.unwrap();
        
        assert!(storage.exists(&path).await.unwrap());
        
        let loaded = storage.load(&path).await.unwrap();
        assert_eq!(loaded, data);
    }
    
    #[tokio::test]
    async fn test_inference_engine_cache() {
        let config = OnnxConfig {
            cache_size: 2,
            preload: vec![],
            execution_provider: ExecutionProvider::CPU,
            gpu_device_id: 0,
        };
        
        let engine = InferenceEngine::new(&config).unwrap();
        
        // 检查初始缓存为空
        let status = engine.cache_status().await;
        assert_eq!(status.loaded_models, 0);
        
        // 加载模型后缓存增加
        // (需要真实模型文件进行测试)
    }
    
    #[tokio::test]
    async fn test_concurrency_limiter() {
        let limiter = InferenceLimiter::new(2);
        
        let permit1 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available(), 1);
        
        let permit2 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available(), 0);
        
        // 第三个应该阻塞
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            limiter_clone.acquire().await.unwrap();
        });
        
        // 等待一小段时间，确认第三个获取被阻塞
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!handle.is_finished());
        
        // 释放一个许可
        drop(permit1);
        
        // 第三个获取应该成功
        tokio::time::timeout(Duration::from_millis(100), handle).await.unwrap().unwrap();
    }
}
```

### 6.2 集成测试

```rust
#[tokio::test]
#[ignore] // 需要真实 ONNX 模型
async fn test_real_model_inference() {
    let config = OnnxConfig::default();
    let engine = InferenceEngine::new(&config).unwrap();
    
    let inputs = InferenceInput {
        inputs: vec![
            ("input".to_string(), json!([[[1.0f32, 2.0, 3.0]]]),
        ].into_iter().collect(),
    };
    
    let result = engine.infer(
        "test-model",
        "tests/fixtures/test_model.onnx",
        inputs,
    ).await.unwrap();
    
    assert!(!result.outputs.is_empty());
    assert!(result.latency_ms > 0);
}
```

## 7. 性能优化

### 7.1 模型缓存预热

```rust
impl InferenceEngine {
    /// 启动时预加载热门模型
    pub async fn warmup(&self, model_paths: &[(String, String)]) -> Result<(), CoreError> {
        let start = Instant::now();
        
        for (model_id, model_path) in model_paths {
            match self.get_or_load_session(model_id, model_path).await {
                Ok(_) => info!("Warmed up model: {}", model_id),
                Err(e) => warn!("Failed to warm up model {}: {}", model_id, e),
            }
        }
        
        info!("Warmup completed in {:?}", start.elapsed());
        Ok(())
    }
}
```

### 7.2 缓存淘汰策略

```rust
impl ModelCache {
    /// 手动淘汰最近最少使用的模型
    pub fn evict_lru(&mut self) -> Option<String> {
        if let Some((model_id, _)) = self.sessions.pop_lru() {
            return Some(model_id);
        }
        None
    }
    
    /// 根据优先级淘汰
    pub fn evict_by_priority(&mut self, priority: impl Fn(&str) -> u32) -> Option<String> {
        let mut candidates: Vec<_> = self.sessions
            .iter()
            .map(|(id, _)| (id.clone(), priority(id)))
            .collect();
        
        candidates.sort_by_key(|(_, p)| *p);
        
        if let Some((model_id, _)) = candidates.first() {
            self.sessions.pop(model_id);
            return Some(model_id.clone());
        }
        
        None
    }
}
```

### 7.3 输入预处理缓存

```rust
/// 输入预处理缓存（可选优化）
pub struct InputPreprocessor {
    cache: Arc<RwLock<LruCache<String, PreprocessedInput>>>,
}

impl InputPreprocessor {
    pub async fn preprocess_or_cache(
        &self,
        input_key: &str,
        raw_input: InferenceInput,
    ) -> Result<HashMap<String, ort::Value>, CoreError> {
        {
            let cache = self.cache.read().await;
            if let Some(preprocessed) = cache.get(input_key) {
                return Ok(preprocessed.clone());
            }
        }
        
        let preprocessed = self.preprocess(raw_input)?;
        
        {
            let mut cache = self.cache.write().await;
            cache.put(input_key.to_string(), preprocessed.clone());
        }
        
        Ok(preprocessed)
    }
    
    fn preprocess(&self, input: InferenceInput) -> Result<HashMap<String, ort::Value>, CoreError> {
        // 预处理逻辑
        unimplemented!()
    }
}
```

## 8. 监控指标

```rust
use metrics::{counter, histogram, gauge};

impl InferenceEngine {
    pub async fn infer_with_metrics(
        &self,
        model_id: &str,
        model_path: &str,
        inputs: InferenceInput,
    ) -> Result<InferenceOutput, CoreError> {
        let start = Instant::now();
        
        // 尝试从缓存获取
        let cache_hit = {
            let cache = self.cache.read().await;
            cache.get(model_id).is_some()
        };
        
        if cache_hit {
            counter!("ferrinx_model_cache_hits_total").increment(1);
        } else {
            counter!("ferrinx_model_cache_misses_total").increment(1);
        }
        
        // 执行推理
        let result = self.infer(model_id, model_path, inputs).await?;
        
        // 记录延迟
        histogram!("ferrinx_inference_duration_seconds")
            .record(start.elapsed().as_secs_f64());
        
        // 记录并发数
        gauge!("ferrinx_sync_inference_concurrent_current")
            .set((self.max_concurrency - self.semaphore.available_permits()) as f64);
        
        Ok(result)
    }
}
```

## 9. 设计要点

### 9.1 CPU 密集任务隔离

- 使用 `spawn_blocking` 执行 ONNX 推理
- 不阻塞 tokio 运行时
- 合理配置 blocking 线程池大小

### 9.2 并发控制

- 使用 `Semaphore` 限制并发推理数
- 防止内存耗尽
- 超时保护

### 9.3 缓存策略

- LRU 缓存减少模型加载
- 缓存大小可配置
- 预加载热门模型

### 9.4 存储抽象

- 接口统一，后端可插拔
- Local/S3 通过 feature flag 切换
- 错误处理统一

## 10. 后续优化

### 10.1 批处理推理

```rust
impl InferenceEngine {
    /// 批处理推理（提高吞吐量）
    pub async fn infer_batch(
        &self,
        model_id: &str,
        batch_inputs: Vec<InferenceInput>,
    ) -> Result<Vec<InferenceOutput>, CoreError> {
        // 合并输入
        let batched_input = self.merge_inputs(batch_inputs)?;
        
        // 执行批量推理
        let batched_output = self.infer(model_id, &model_path, batched_input).await?;
        
        // 拆分输出
        let outputs = self.split_outputs(batched_output)?;
        
        Ok(outputs)
    }
}
```

### 10.2 模型优化

```rust
/// 模型优化器
pub struct ModelOptimizer {
    // ONNX Runtime 优化选项
}

impl ModelOptimizer {
    /// 模型量化
    pub fn quantize(&self, model_data: &[u8]) -> Result<Vec<u8>, CoreError> {
        // INT8 量化
        unimplemented!()
    }
    
    /// 图优化
    pub fn optimize_graph(&self, model_data: &[u8]) -> Result<Vec<u8>, CoreError> {
        // 图融合、常量折叠等
        unimplemented!()
    }
}
```

### 10.3 动态批处理

```rust
/// 动态批处理器
pub struct DynamicBatcher {
    queue: Arc<RwLock<VecDeque<PendingRequest>>>,
    batch_size: usize,
    timeout: Duration,
}

impl DynamicBatcher {
    pub async fn submit(&self, request: InferenceInput) -> Result<InferenceOutput, CoreError> {
        // 将请求加入队列
        // 等待批处理完成或超时
        // 返回结果
        unimplemented!()
    }
    
    pub async fn run_batch_loop(&self, engine: &InferenceEngine) {
        // 定期检查队列
        // 组装批次
        // 执行批推理
        // 分发结果
    }
}
```
