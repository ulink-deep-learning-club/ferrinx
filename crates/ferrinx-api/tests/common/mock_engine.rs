use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ferrinx_common::{InferenceInput, InferenceOutput};
use ferrinx_core::error::{CoreError, Result};
use tokio::sync::{Mutex, Semaphore};

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

pub struct MockInferenceEngine {
    semaphore: Arc<Semaphore>,
    responses: Arc<Mutex<HashMap<String, InferenceOutput>>>,
    should_fail: Arc<Mutex<bool>>,
    delay_ms: Arc<Mutex<u64>>,
    max_concurrency: usize,
}

impl MockInferenceEngine {
    pub fn new(cache_size: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(cache_size)),
            responses: Arc::new(Mutex::new(HashMap::new())),
            should_fail: Arc::new(Mutex::new(false)),
            delay_ms: Arc::new(Mutex::new(0)),
            max_concurrency: cache_size,
        }
    }

    pub fn with_default_response() -> Self {
        let engine = Self::new(5);
        engine.set_default_response();
        engine
    }

    pub fn set_default_response(&self) {
        let output = InferenceOutput {
            outputs: HashMap::from([("output".to_string(), serde_json::json!([1.0, 2.0, 3.0]))]),
            latency_ms: 10,
        };
        let mut responses = futures::executor::block_on(self.responses.lock());
        responses.insert("default".to_string(), output);
    }

    pub async fn set_response(&self, model_id: &str, output: InferenceOutput) {
        let mut responses = self.responses.lock().await;
        responses.insert(model_id.to_string(), output);
    }

    pub async fn set_should_fail(&self, fail: bool) {
        let mut should_fail = self.should_fail.lock().await;
        *should_fail = fail;
    }

    pub async fn set_delay_ms(&self, delay: u64) {
        let mut delay_ms = self.delay_ms.lock().await;
        *delay_ms = delay;
    }

    pub async fn infer(
        &self,
        model_id: &str,
        _model_path: &str,
        _inputs: InferenceInput,
    ) -> Result<InferenceOutput> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| CoreError::ConcurrencyLimitReached)?;

        let delay = *self.delay_ms.lock().await;
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        let should_fail = *self.should_fail.lock().await;
        if should_fail {
            return Err(CoreError::InferenceFailed("Mock error".to_string()));
        }

        let responses = self.responses.lock().await;
        if let Some(output) = responses.get(model_id) {
            Ok(output.clone())
        } else if let Some(output) = responses.get("default") {
            Ok(output.clone())
        } else {
            Ok(InferenceOutput {
                outputs: HashMap::from([("output".to_string(), serde_json::json!([0.0]))]),
                latency_ms: 1,
            })
        }
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

    pub async fn preload_models(&self, _models: &[(String, String)]) -> Result<()> {
        Ok(())
    }

    pub async fn clear_cache(&self) {}
}
