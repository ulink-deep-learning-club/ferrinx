#[cfg(all(
    feature = "api-17",
    any(
        feature = "api-18",
        feature = "api-19",
        feature = "api-20",
        feature = "api-21",
        feature = "api-22",
        feature = "api-23",
        feature = "api-24",
    )
))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled.\n\
     Available: api-17, api-18, api-19, api-20, api-21, api-22, api-23, api-24\n\
     Default: api-23 (ONNX Runtime 1.23, last version supporting NVIDIA Pascal/Volta)"
);

#[cfg(all(
    feature = "api-18",
    any(
        feature = "api-19",
        feature = "api-20",
        feature = "api-21",
        feature = "api-22",
        feature = "api-23",
        feature = "api-24",
    )
))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

#[cfg(all(
    feature = "api-19",
    any(
        feature = "api-20",
        feature = "api-21",
        feature = "api-22",
        feature = "api-23",
        feature = "api-24",
    )
))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

#[cfg(all(
    feature = "api-20",
    any(
        feature = "api-21",
        feature = "api-22",
        feature = "api-23",
        feature = "api-24",
    )
))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

#[cfg(all(
    feature = "api-21",
    any(feature = "api-22", feature = "api-23", feature = "api-24",)
))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

#[cfg(all(feature = "api-22", any(feature = "api-23", feature = "api-24",)))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

#[cfg(all(feature = "api-23", feature = "api-24"))]
compile_error!(
    "Multiple ONNX Runtime API versions selected. Only one api-* feature can be enabled."
);

pub mod error;
pub mod inference;
pub mod model;
pub mod storage;
pub mod transform;

pub use error::*;
pub use inference::*;
pub use model::*;
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
    let path = std::path::Path::new(lib_path);

    if !path.exists() {
        return Err(crate::error::CoreError::OnnxRuntimeLibraryNotFound(
            format!("Path does not exist: {}", lib_path),
        ));
    }

    if !path.is_file() {
        return Err(crate::error::CoreError::OnnxRuntimeLibraryNotFound(
            format!("Path is not a file: {}", lib_path),
        ));
    }

    ort::init_from(lib_path)
        .map_err(|e| crate::error::CoreError::SessionCreationFailed(e.to_string()))?
        .commit();

    tracing::info!("ONNX Runtime initialized: {}", ort::info());

    Ok(())
}

#[cfg(not(feature = "load-dynamic"))]
pub fn init_onnxruntime(_lib_path: &str) -> crate::error::Result<()> {
    Ok(())
}

#[cfg(all(test, feature = "load-dynamic"))]
mod tests {
    use super::*;

    #[test]
    fn test_init_onnxruntime_nonexistent_path() {
        let result = init_onnxruntime("/nonexistent/path/libonnxruntime.so");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::error::CoreError::OnnxRuntimeLibraryNotFound(_)
        ));
        assert!(err.to_string().contains("Path does not exist"));
    }

    #[test]
    fn test_init_onnxruntime_directory_path() {
        let result = init_onnxruntime("/tmp");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::error::CoreError::OnnxRuntimeLibraryNotFound(_)
        ));
        assert!(err.to_string().contains("Path is not a file"));
    }
}
