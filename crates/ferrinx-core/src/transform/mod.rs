mod pipeline;

pub use pipeline::{PostprocessPipeline, PreprocessPipeline, TransformData, TransformError};

use ndarray::{ArrayD, IxDyn};

#[derive(Debug, Clone)]
pub enum TransformInput {
    Image(image::DynamicImage),
    Tensor(Vec<f32>),
    Json(serde_json::Value),
    Raw(Vec<u8>),
}

impl TransformInput {
    pub fn from_base64_image(data: &str) -> Result<Self, TransformError> {
        let decoded = base64_decode(data)?;
        let img = image::load_from_memory(&decoded)?;
        Ok(TransformInput::Image(img))
    }

    pub fn from_json_value(value: serde_json::Value) -> Self {
        TransformInput::Json(value)
    }

    pub fn into_data(self) -> Result<TransformData, TransformError> {
        match self {
            TransformInput::Image(img) => Ok(TransformData::Image(img)),
            TransformInput::Tensor(v) => {
                let shape = IxDyn(&[v.len()]);
                let tensor = ArrayD::from_shape_vec(shape, v)
                    .map_err(|e| TransformError::InvalidInput(e.to_string()))?;
                Ok(TransformData::TensorF32(tensor))
            }
            TransformInput::Json(v) => Ok(TransformData::Json(v)),
            TransformInput::Raw(v) => Ok(TransformData::Json(serde_json::Value::Array(
                v.iter()
                    .map(|&b| serde_json::Value::Number(b.into()))
                    .collect(),
            ))),
        }
    }
}

fn base64_decode(data: &str) -> Result<Vec<u8>, TransformError> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let data = if data.starts_with("data:") {
        data.split(',')
            .nth(1)
            .ok_or(TransformError::InvalidBase64)?
    } else {
        data
    };

    STANDARD
        .decode(data)
        .map_err(|_| TransformError::InvalidBase64)
}
