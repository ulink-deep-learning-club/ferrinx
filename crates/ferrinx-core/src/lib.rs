pub mod error;
pub mod inference;
pub mod model;
pub mod storage;
pub mod transform;

pub use error::*;
pub use inference::*;
pub use model::*;
pub use storage::*;
pub use transform::*;

pub use inference::engine::{
    CacheEvictCallback, CacheLoadCallback, CacheStatus, ConcurrencyStatus, InferenceEngine,
};
pub use model::config::ModelConfig;
pub use model::loader::ModelLoader;
pub use storage::{LocalStorage, ModelStorage};
pub use transform::{PostprocessPipeline, PreprocessPipeline, TransformData, TransformError};

#[cfg(feature = "load-dynamic")]
pub fn init_onnxruntime(lib_path: &str) -> crate::error::Result<()> {
    ort::init_from(lib_path)
        .map_err(|e| crate::error::CoreError::SessionCreationFailed(e.to_string()))?
        .commit();
    Ok(())
}

#[cfg(not(feature = "load-dynamic"))]
pub fn init_onnxruntime(_lib_path: &str) -> crate::error::Result<()> {
    Ok(())
}
