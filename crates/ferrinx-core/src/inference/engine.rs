use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ort::session::Session;
use ort::value::{Tensor, TensorElementType, ValueType};
use tokio::sync::Semaphore;
use tracing::info;

use crate::error::{CoreError, Result};
use ferrinx_common::{InferenceInput, InferenceOutput, OnnxConfig};

pub struct InferenceEngine {
    semaphore: Arc<Semaphore>,
    timeout: Duration,
    max_concurrency: usize,
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
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.cache_size)),
            timeout: Duration::from_secs(30),
            max_concurrency: config.cache_size,
        })
    }

    pub async fn infer(
        &self,
        _model_id: &str,
        model_path: &str,
        inputs: InferenceInput,
    ) -> Result<InferenceOutput> {
        let start = Instant::now();

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| CoreError::ConcurrencyLimitReached)?;

        let model_path_owned = model_path.to_string();
        let outputs = tokio::time::timeout(self.timeout, async move {
            tokio::task::spawn_blocking(move || {
                let mut session = Session::builder()
                    .and_then(|mut b| b.commit_from_file(&model_path_owned))
                    .map_err(|e| CoreError::ModelLoadFailed(e.to_string()))?;

                let input_tensors = prepare_inputs(&session, inputs)?;
                
                let ort_inputs: HashMap<String, ort::value::Value> = input_tensors
                    .into_iter()
                    .map(|(k, v)| (k, v.into_dyn()))
                    .collect();
                    
                let ort_outputs = session.run(ort_inputs)
                    .map_err(|e| CoreError::InferenceFailed(e.to_string()))?;
                    
                parse_outputs(ort_outputs)
            })
            .await
            .map_err(|e| CoreError::BlockingTaskFailed(e.to_string()))?
        })
        .await
        .map_err(|_| CoreError::InferenceTimeout)??;

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceOutput {
            outputs,
            latency_ms,
        })
    }

    pub async fn preload_models(&self, _models: &[(String, String)]) -> Result<()> {
        info!("Model preloading not implemented in simplified version");
        Ok(())
    }

    pub async fn clear_cache(&self) {
        info!("Model cache not implemented in simplified version");
    }

    pub async fn cache_status(&self) -> CacheStatus {
        CacheStatus {
            loaded_models: 0,
            max_size: self.max_concurrency,
        }
    }

    pub fn concurrency_status(&self) -> ConcurrencyStatus {
        ConcurrencyStatus {
            available_permits: self.semaphore.available_permits(),
            total_permits: self.max_concurrency,
        }
    }
}

fn prepare_inputs(
    session: &Session,
    inputs: InferenceInput,
) -> Result<HashMap<String, ort::value::Value>> {
    let mut input_tensors = HashMap::new();

    for (input_name, input_data) in inputs.inputs {
        let input_info = session
            .inputs()
            .iter()
            .find(|i| i.name() == input_name)
            .ok_or_else(|| CoreError::InputNotFound(input_name.clone()))?;

        let tensor = value_to_tensor(input_data, input_info.dtype())?;
        input_tensors.insert(input_name, tensor);
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
                let data: Vec<f32> = serde_json::from_value(value)?;
                let shape_vec: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
                let tensor: Tensor<f32> = Tensor::from_array((shape_vec, data.into_boxed_slice()))?;
                Ok(tensor.into())
            }
            TensorElementType::Int64 => {
                let data: Vec<i64> = serde_json::from_value(value)?;
                let shape_vec: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
                let tensor: Tensor<i64> = Tensor::from_array((shape_vec, data.into_boxed_slice()))?;
                Ok(tensor.into())
            }
            _ => Err(CoreError::UnsupportedTensorType),
        },
        _ => Err(CoreError::UnsupportedInputType),
    }
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
        let data: Vec<f32> = tensor.1.to_vec();
        return Ok(serde_json::to_value(data)?);
    }

    if let Ok(tensor) = value.try_extract_tensor::<i64>() {
        let data: Vec<i64> = tensor.1.to_vec();
        return Ok(serde_json::to_value(data)?);
    }

    Ok(serde_json::Value::Null)
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
}
