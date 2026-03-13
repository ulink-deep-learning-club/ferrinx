use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use ort::session::Session;
use ort::value::{Tensor, TensorElementType, ValueType};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::{CoreError, Result};
use ferrinx_common::{ExecutionProvider, InferenceInput, InferenceOutput, OnnxConfig, Tensor as FerrinxTensor, TensorDataType};

pub type CacheEvictCallback = Arc<dyn Fn(Uuid) + Send + Sync>;
pub type CacheLoadCallback = Arc<dyn Fn(Uuid) + Send + Sync>;

type CachedSession = Arc<tokio::sync::Mutex<Session>>;

struct CacheState {
    cache: LruCache<Uuid, CachedSession>,
    loading: HashSet<Uuid>,
}

pub struct InferenceEngine {
    state: Arc<Mutex<CacheState>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    timeout: Duration,
    max_cache_size: usize,
    on_evict: Option<CacheEvictCallback>,
    on_load: Option<CacheLoadCallback>,
    execution_provider: ExecutionProvider,
    gpu_device_id: u32,
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

impl InferenceEngine {
    pub fn new(config: &OnnxConfig) -> Result<Self> {
        let cache_size = NonZeroUsize::new(config.cache_size).unwrap_or(NonZeroUsize::new(5).unwrap());
        
        Ok(Self {
            state: Arc::new(Mutex::new(CacheState {
                cache: LruCache::new(cache_size),
                loading: HashSet::new(),
            })),
            semaphore: Arc::new(tokio::sync::Semaphore::new(config.cache_size)),
            timeout: Duration::from_secs(30),
            max_cache_size: config.cache_size,
            on_evict: None,
            on_load: None,
            execution_provider: config.execution_provider.clone(),
            gpu_device_id: config.gpu_device_id,
        })
    }

    pub fn with_callbacks(
        mut self,
        on_evict: Option<CacheEvictCallback>,
        on_load: Option<CacheLoadCallback>,
    ) -> Self {
        self.on_evict = on_evict;
        self.on_load = on_load;
        self
    }

    pub async fn infer(
        &self,
        model_id: &Uuid,
        model_path: &str,
        inputs: InferenceInput,
    ) -> Result<InferenceOutput> {
        let start = Instant::now();

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| CoreError::ConcurrencyLimitReached)?;

        let model_id = *model_id;
        let model_path = model_path.to_string();
        let state = self.state.clone();
        let on_evict = self.on_evict.clone();
        let on_load = self.on_load.clone();
        let execution_provider = self.execution_provider.clone();
        let gpu_device_id = self.gpu_device_id;

        let outputs = tokio::time::timeout(self.timeout, async move {
            let session = Self::get_or_load_session(
                state.clone(),
                model_id,
                model_path,
                on_evict,
                on_load,
                execution_provider,
                gpu_device_id,
            ).await?;

            let input_tensors = prepare_inputs(&session, &inputs).await?;
            
            let mut session_guard = session.lock().await;
            let ort_inputs: HashMap<String, ort::value::Value> = input_tensors
                .into_iter()
                .map(|(k, v)| (k, v.into_dyn()))
                .collect();
                
            let ort_outputs = session_guard.run(ort_inputs)
                .map_err(|e| CoreError::InferenceFailed(e.to_string()))?;
                
            parse_outputs(ort_outputs)
        })
        .await
        .map_err(|_| CoreError::InferenceTimeout)??;

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceOutput {
            outputs,
            latency_ms,
        })
    }

    pub async fn preload_models(&self, models: &[(Uuid, String)]) -> Result<()> {
        info!("Preloading {} models into cache", models.len());
        
        for (model_id, model_path) in models {
            match self.preload_model(*model_id, model_path).await {
                Ok(_) => info!("Preloaded model {} from {}", model_id, model_path),
                Err(e) => warn!("Failed to preload model {}: {}", model_id, e),
            }
        }
        
        Ok(())
    }

    async fn preload_model(&self, model_id: Uuid, model_path: &str) -> Result<()> {
        let mut state_guard = self.state.lock().await;
        
        if state_guard.cache.contains(&model_id) {
            debug!("Model {} already in cache, skipping preload", model_id);
            return Ok(());
        }
        
        if state_guard.loading.contains(&model_id) {
            debug!("Model {} already being loaded, skipping preload", model_id);
            return Ok(());
        }

        if state_guard.cache.len() >= state_guard.cache.cap().get() {
            let (evicted_id, _) = state_guard.cache.pop_lru().unwrap();
            debug!("Evicting model {} from cache to make room for preload", evicted_id);
            if let Some(ref callback) = self.on_evict {
                callback(evicted_id);
            }
        }

        state_guard.loading.insert(model_id);

        let model_path = model_path.to_string();
        let on_load = self.on_load.clone();
        let execution_provider = self.execution_provider.clone();
        let gpu_device_id = self.gpu_device_id;
        
        drop(state_guard);
        
        let load_result = tokio::task::spawn_blocking(move || {
            let builder = Session::builder()
                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?;

            let mut builder = match execution_provider {
                ExecutionProvider::CPU => builder,
                ExecutionProvider::CUDA => {
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    {
                        builder
                            .with_execution_providers([
                                ort::ep::CUDA::default()
                                    .with_device_id(gpu_device_id)
                                    .build()
                            ])
                            .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                    }
                    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                    {
                        let _ = gpu_device_id;
                        builder
                    }
                }
                ExecutionProvider::TensorRT => {
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    {
                        builder
                            .with_execution_providers([
                                ort::ep::TensorRT::default()
                                    .with_device_id(gpu_device_id)
                                    .build()
                            ])
                            .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                    }
                    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                    {
                        let _ = gpu_device_id;
                        builder
                    }
                }
                ExecutionProvider::CoreML => {
                    #[cfg(target_os = "macos")]
                    {
                        builder
                            .with_execution_providers([
                                ort::ep::CoreML::default().build()
                            ])
                            .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        builder
                    }
                }
                ExecutionProvider::ROCm => {
                    #[cfg(target_os = "linux")]
                    {
                        builder
                            .with_execution_providers([
                                ort::ep::ROCm::default()
                                    .with_device_id(gpu_device_id)
                                    .build()
                            ])
                            .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        let _ = gpu_device_id;
                        builder
                    }
                }
            };

            builder
                .commit_from_file(&model_path)
                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))
        })
        .await
        .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))?;

        let mut state_guard = self.state.lock().await;
        state_guard.loading.remove(&model_id);
        
        match load_result {
            Ok(session) => {
                let session = Arc::new(tokio::sync::Mutex::new(session));
                state_guard.cache.put(model_id, session);
                
                if let Some(ref callback) = on_load {
                    callback(model_id);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn clear_cache(&self) {
        let mut state_guard = self.state.lock().await;
        let evicted_count = state_guard.cache.len();
        
        if let Some(ref callback) = self.on_evict {
            for (model_id, _) in state_guard.cache.iter() {
                callback(*model_id);
            }
        }
        
        state_guard.cache.clear();
        state_guard.loading.clear();
        info!("Cleared {} models from cache", evicted_count);
    }

    pub async fn cache_status(&self) -> CacheStatus {
        let state_guard = self.state.lock().await;
        CacheStatus {
            loaded_models: state_guard.cache.len(),
            max_size: state_guard.cache.cap().get(),
        }
    }

    pub fn concurrency_status(&self) -> ConcurrencyStatus {
        ConcurrencyStatus {
            available_permits: self.semaphore.available_permits(),
            total_permits: self.max_cache_size,
        }
    }

    pub async fn get_cached_model_ids(&self) -> Vec<Uuid> {
        let state_guard = self.state.lock().await;
        state_guard.cache.iter().map(|(id, _)| *id).collect()
    }

    async fn get_or_load_session(
        state: Arc<Mutex<CacheState>>,
        model_id: Uuid,
        model_path: String,
        on_evict: Option<CacheEvictCallback>,
        on_load: Option<CacheLoadCallback>,
        execution_provider: ExecutionProvider,
        gpu_device_id: u32,
    ) -> Result<CachedSession> {
        loop {
            let mut state_guard = state.lock().await;
            
            if let Some(session) = state_guard.cache.get(&model_id) {
                debug!("Cache hit for model {}", model_id);
                return Ok(Arc::clone(session));
            }
            
            if state_guard.loading.contains(&model_id) {
                drop(state_guard);
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            
            debug!("Cache miss for model {}, loading from {}", model_id, model_path);
            
            state_guard.loading.insert(model_id);
            
            let old_evicted = if state_guard.cache.len() >= state_guard.cache.cap().get() {
                let (evicted_id, _) = state_guard.cache.pop_lru().unwrap();
                debug!("Evicting model {} from cache", evicted_id);
                if let Some(ref callback) = on_evict {
                    callback(evicted_id);
                }
                Some(evicted_id)
            } else {
                None
            };
            
            drop(state_guard);
            
            let load_result = tokio::task::spawn_blocking(move || {
                let builder = Session::builder()
                    .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?;

                let mut builder = match execution_provider {
                    ExecutionProvider::CPU => builder,
                    ExecutionProvider::CUDA => {
                        #[cfg(any(target_os = "linux", target_os = "windows"))]
                        {
                            builder
                                .with_execution_providers([
                                    ort::ep::CUDA::default()
                                        .with_device_id(gpu_device_id)
                                        .build()
                                ])
                                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                        }
                        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                        {
                            let _ = gpu_device_id;
                            builder
                        }
                    }
                    ExecutionProvider::TensorRT => {
                        #[cfg(any(target_os = "linux", target_os = "windows"))]
                        {
                            builder
                                .with_execution_providers([
                                    ort::ep::TensorRT::default()
                                        .with_device_id(gpu_device_id)
                                        .build()
                                ])
                                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                        }
                        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                        {
                            let _ = gpu_device_id;
                            builder
                        }
                    }
                    ExecutionProvider::CoreML => {
                        #[cfg(target_os = "macos")]
                        {
                            builder
                                .with_execution_providers([
                                    ort::ep::CoreML::default().build()
                                ])
                                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            builder
                        }
                    }
                    ExecutionProvider::ROCm => {
                        #[cfg(target_os = "linux")]
                        {
                            builder
                                .with_execution_providers([
                                    ort::ep::ROCm::default()
                                        .with_device_id(gpu_device_id)
                                        .build()
                                ])
                                .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            let _ = gpu_device_id;
                            builder
                        }
                    }
                };

                builder
                    .commit_from_file(&model_path)
                    .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))
            })
            .await
            .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))?;
            
            let mut state_guard = state.lock().await;
            state_guard.loading.remove(&model_id);
            
            return match load_result {
                Ok(session) => {
                    let session = Arc::new(tokio::sync::Mutex::new(session));
                    state_guard.cache.put(model_id, Arc::clone(&session));
                    
                    if let Some(ref callback) = on_load {
                        callback(model_id);
                    }
                    
                    info!(
                        "Loaded model {} into cache (evicted: {:?}, cache size: {}/{})",
                        model_id,
                        old_evicted,
                        state_guard.cache.len(),
                        state_guard.cache.cap().get()
                    );
                    
                    Ok(session)
                }
                Err(e) => Err(e),
            };
        }
    }
}

async fn prepare_inputs(
    session: &CachedSession,
    inputs: &InferenceInput,
) -> Result<HashMap<String, ort::value::Value>> {
    let session_guard = session.lock().await;
    let mut input_tensors = HashMap::new();
    let session_inputs = session_guard.inputs();

    if session_inputs.len() == 1 && inputs.inputs.len() == 1 {
        let input_info = &session_inputs[0];
        let actual_name = input_info.name().to_string();
        let (_, input_data) = inputs.inputs.iter().next().unwrap();
        
        let tensor = value_to_tensor(input_data.clone(), input_info.dtype())?;
        input_tensors.insert(actual_name, tensor);
    } else {
        for (input_name, input_data) in &inputs.inputs {
            let input_info = session_inputs
                .iter()
                .find(|i| i.name() == input_name)
                .ok_or_else(|| CoreError::InputNotFound(input_name.clone()))?;

            let tensor = value_to_tensor(input_data.clone(), input_info.dtype())?;
            input_tensors.insert(input_name.clone(), tensor);
        }
    }

    Ok(input_tensors)
}

fn value_to_tensor(
    value: serde_json::Value,
    value_type: &ValueType,
) -> Result<ort::value::Value> {
    match value_type {
        ValueType::Tensor { ty, shape, .. } => match ty {
            TensorElementType::Float32 => {
                let (data, input_shape) = extract_f32_data(&value, shape)?;
                let tensor: Tensor<f32> = Tensor::from_array((input_shape, data.into_boxed_slice()))?;
                Ok(tensor.into())
            }
            TensorElementType::Int8 => {
                let (data, input_shape) = extract_i8_data(&value, shape)?;
                let tensor: Tensor<i8> = Tensor::from_array((input_shape, data.into_boxed_slice()))?;
                Ok(tensor.into())
            }
            TensorElementType::Int64 => {
                let (data, input_shape) = extract_i64_data(&value, shape)?;
                let tensor: Tensor<i64> = Tensor::from_array((input_shape, data.into_boxed_slice()))?;
                Ok(tensor.into())
            }
            _ => Err(CoreError::UnsupportedTensorType),
        },
        _ => Err(CoreError::UnsupportedInputType),
    }
}

fn extract_f32_data(value: &serde_json::Value, expected_shape: &[i64]) -> Result<(Vec<f32>, Vec<usize>)> {
    let tensor: FerrinxTensor = serde_json::from_value(value.clone())
        .map_err(|e| CoreError::InvalidInput(format!("Expected Tensor format, got: {}", e)))?;
    
    if tensor.dtype != TensorDataType::Float32 {
        return Err(CoreError::InvalidInput(format!(
            "Expected float32 tensor, got {:?}",
            tensor.dtype
        )));
    }
    
    let data = tensor.decode_f32()
        .map_err(|e| CoreError::InvalidInput(format!("Failed to decode tensor: {}", e)))?;
    
    let expected_shape_usize: Vec<usize> = expected_shape.iter().map(|&d| d as usize).collect();
    let tensor_shape_usize: Vec<usize> = tensor.shape.iter().map(|&d| d as usize).collect();
    
    if tensor_shape_usize != expected_shape_usize {
        return Err(CoreError::InvalidInput(format!(
            "Shape mismatch: model expects {:?}, but tensor has {:?}",
            expected_shape_usize, tensor_shape_usize
        )));
    }
    
    let expected_len: usize = expected_shape_usize.iter().product();
    if data.len() != expected_len {
        return Err(CoreError::InvalidInput(format!(
            "Data size mismatch: expected {} elements for shape {:?}, got {}",
            expected_len, expected_shape_usize, data.len()
        )));
    }
    
    Ok((data, expected_shape_usize))
}

fn extract_i8_data(value: &serde_json::Value, expected_shape: &[i64]) -> Result<(Vec<i8>, Vec<usize>)> {
    let tensor: FerrinxTensor = serde_json::from_value(value.clone())
        .map_err(|e| CoreError::InvalidInput(format!("Expected Tensor format, got: {}", e)))?;
    
    if tensor.dtype != TensorDataType::Int8 {
        return Err(CoreError::InvalidInput(format!(
            "Expected int8 tensor, got {:?}",
            tensor.dtype
        )));
    }
    
    let data = tensor.decode_i8()
        .map_err(|e| CoreError::InvalidInput(format!("Failed to decode tensor: {}", e)))?;
    
    let expected_shape_usize: Vec<usize> = expected_shape.iter().map(|&d| d as usize).collect();
    let tensor_shape_usize: Vec<usize> = tensor.shape.iter().map(|&d| d as usize).collect();
    
    if tensor_shape_usize != expected_shape_usize {
        return Err(CoreError::InvalidInput(format!(
            "Shape mismatch: model expects {:?}, but tensor has {:?}",
            expected_shape_usize, tensor_shape_usize
        )));
    }
    
    let expected_len: usize = expected_shape_usize.iter().product();
    if data.len() != expected_len {
        return Err(CoreError::InvalidInput(format!(
            "Data size mismatch: expected {} elements for shape {:?}, got {}",
            expected_len, expected_shape_usize, data.len()
        )));
    }
    
    Ok((data, expected_shape_usize))
}

fn extract_i64_data(value: &serde_json::Value, expected_shape: &[i64]) -> Result<(Vec<i64>, Vec<usize>)> {
    let tensor: FerrinxTensor = serde_json::from_value(value.clone())
        .map_err(|e| CoreError::InvalidInput(format!("Expected Tensor format, got: {}", e)))?;
    
    if tensor.dtype != TensorDataType::Int64 {
        return Err(CoreError::InvalidInput(format!(
            "Expected int64 tensor, got {:?}",
            tensor.dtype
        )));
    }
    
    let data = tensor.decode_i64()
        .map_err(|e| CoreError::InvalidInput(format!("Failed to decode tensor: {}", e)))?;
    
    let expected_shape_usize: Vec<usize> = expected_shape.iter().map(|&d| d as usize).collect();
    let tensor_shape_usize: Vec<usize> = tensor.shape.iter().map(|&d| d as usize).collect();
    
    if tensor_shape_usize != expected_shape_usize {
        return Err(CoreError::InvalidInput(format!(
            "Shape mismatch: model expects {:?}, but tensor has {:?}",
            expected_shape_usize, tensor_shape_usize
        )));
    }
    
    let expected_len: usize = expected_shape_usize.iter().product();
    if data.len() != expected_len {
        return Err(CoreError::InvalidInput(format!(
            "Data size mismatch: expected {} elements for shape {:?}, got {}",
            expected_len, expected_shape_usize, data.len()
        )));
    }
    
    Ok((data, expected_shape_usize))
}

fn parse_outputs(
    outputs: ort::session::SessionOutputs,
) -> Result<HashMap<String, serde_json::Value>> {
    let mut result = HashMap::new();

    for (output_name, output_value) in outputs.iter() {
        let json_value = tensor_to_json(&output_value)?;
        result.insert(output_name.to_string(), json_value);
    }

    Ok(result)
}

fn tensor_to_json(value: &ort::value::Value) -> Result<serde_json::Value> {
    if let Ok(tensor) = value.try_extract_tensor::<f32>() {
        let shape: Vec<i64> = tensor.0.iter().map(|&d| d as i64).collect();
        let data: Vec<f32> = tensor.1.to_vec();
        let ferrinx_tensor = FerrinxTensor::new_f32(shape, &data);
        return Ok(serde_json::to_value(ferrinx_tensor)?);
    }

    if let Ok(tensor) = value.try_extract_tensor::<i8>() {
        let shape: Vec<i64> = tensor.0.iter().map(|&d| d as i64).collect();
        let data: Vec<i8> = tensor.1.to_vec();
        let ferrinx_tensor = FerrinxTensor::new_i8(shape, &data);
        return Ok(serde_json::to_value(ferrinx_tensor)?);
    }

    if let Ok(tensor) = value.try_extract_tensor::<i64>() {
        let shape: Vec<i64> = tensor.0.iter().map(|&d| d as i64).collect();
        let data: Vec<i64> = tensor.1.to_vec();
        let ferrinx_tensor = FerrinxTensor::new_i64(shape, &data);
        return Ok(serde_json::to_value(ferrinx_tensor)?);
    }

    Err(CoreError::UnsupportedTensorType)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrinx_common::ExecutionProvider;

    #[tokio::test]
    async fn test_concurrency_status() {
        let config = OnnxConfig {
            cache_size: 3,
            preload: vec![],
            execution_provider: ExecutionProvider::CPU,
            gpu_device_id: 0,
        };

        let engine = InferenceEngine::new(&config).unwrap();

        let status = engine.concurrency_status();
        assert_eq!(status.available_permits, 3);
        assert_eq!(status.total_permits, 3);
    }

    #[tokio::test]
    async fn test_cache_status() {
        let config = OnnxConfig {
            cache_size: 3,
            preload: vec![],
            execution_provider: ExecutionProvider::CPU,
            gpu_device_id: 0,
        };

        let engine = InferenceEngine::new(&config).unwrap();

        let status = engine.cache_status().await;
        assert_eq!(status.loaded_models, 0);
        assert_eq!(status.max_size, 3);
    }
}
