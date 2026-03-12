use crate::model::config::{LabelMapping, PostprocessOp, PreprocessOp};
use image::GenericImageView;
use ndarray::{ArrayD, IxDyn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum TransformError {
    InvalidInput(String),
    InvalidBase64,
    ImageError(String),
    ShapeMismatch {
        expected: Vec<usize>,
        actual: Vec<usize>,
    },
    NoLabels,
    UnsupportedOperation(String),
    JsonError(String),
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransformError::InvalidInput(s) => write!(f, "Invalid input: {}", s),
            TransformError::InvalidBase64 => write!(f, "Invalid base64 encoding"),
            TransformError::ImageError(s) => write!(f, "Image error: {}", s),
            TransformError::ShapeMismatch { expected, actual } => {
                write!(
                    f,
                    "Shape mismatch: expected {:?}, got {:?}",
                    expected, actual
                )
            }
            TransformError::NoLabels => write!(f, "No labels loaded"),
            TransformError::UnsupportedOperation(s) => write!(f, "Unsupported operation: {}", s),
            TransformError::JsonError(s) => write!(f, "JSON error: {}", s),
        }
    }
}

impl std::error::Error for TransformError {}

impl From<serde_json::Error> for TransformError {
    fn from(e: serde_json::Error) -> Self {
        TransformError::JsonError(e.to_string())
    }
}

impl From<image::ImageError> for TransformError {
    fn from(e: image::ImageError) -> Self {
        TransformError::ImageError(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum TransformData {
    Image(image::DynamicImage),
    TensorF32(ArrayD<f32>),
    TensorI64(ArrayD<i64>),
    Json(serde_json::Value),
    Scalar(ScalarValue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScalarValue {
    Index(usize),
    Float(f32),
    Int(i64),
}

pub struct PreprocessPipeline {
    ops: Vec<PreprocessOp>,
}

impl PreprocessPipeline {
    pub fn new(ops: Vec<PreprocessOp>) -> Self {
        Self { ops }
    }

    pub fn run(&self, input: TransformData) -> Result<TransformData, TransformError> {
        let mut data = input;
        for op in &self.ops {
            data = self.apply_op(op, data)?;
        }
        Ok(data)
    }

    fn apply_op(
        &self,
        op: &PreprocessOp,
        data: TransformData,
    ) -> Result<TransformData, TransformError> {
        match op {
            PreprocessOp::Resize { size } => {
                let img = data.into_image()?;
                let resized =
                    img.resize_exact(size[0], size[1], image::imageops::FilterType::Triangle);
                Ok(TransformData::Image(resized))
            }

            PreprocessOp::Grayscale => {
                let img = data.into_image()?;
                let gray = img.into_luma8();
                Ok(TransformData::Image(gray.into()))
            }

            PreprocessOp::Normalize { mean, std } => {
                let tensor = data.into_tensor_f32()?;
                let normalized = normalize_tensor(&tensor, mean, std);
                Ok(TransformData::TensorF32(normalized))
            }

            PreprocessOp::ToTensor { dtype: _, scale } => {
                let img = data.into_image()?;
                let tensor = image_to_tensor(&img, scale.unwrap_or(1.0))?;
                Ok(TransformData::TensorF32(tensor))
            }

            PreprocessOp::Transpose { axes } => {
                let tensor = data.into_tensor_f32()?;
                let transposed = tensor.permuted_axes(axes.as_slice());
                Ok(TransformData::TensorF32(transposed))
            }

            PreprocessOp::Squeeze { axes } => {
                let tensor = data.into_tensor_f32()?;
                let mut shape: Vec<usize> = tensor.shape().to_vec();
                let (data, _offset) = tensor.into_raw_vec_and_offset();

                for &axis in axes.iter().rev() {
                    if axis < shape.len() && shape[axis] == 1 {
                        shape.remove(axis);
                    }
                }

                let new_tensor = ArrayD::from_shape_vec(IxDyn(&shape), data)
                    .map_err(|e| TransformError::InvalidInput(e.to_string()))?;
                Ok(TransformData::TensorF32(new_tensor))
            }

            PreprocessOp::Unsqueeze { axes } => {
                let tensor = data.into_tensor_f32()?;
                let mut shape: Vec<usize> = tensor.shape().to_vec();
                let (data, _offset) = tensor.into_raw_vec_and_offset();

                for &axis in axes {
                    if axis <= shape.len() {
                        shape.insert(axis, 1);
                    }
                }

                let new_tensor = ArrayD::from_shape_vec(IxDyn(&shape), data)
                    .map_err(|e| TransformError::InvalidInput(e.to_string()))?;
                Ok(TransformData::TensorF32(new_tensor))
            }

            PreprocessOp::Reshape { shape } => {
                let tensor = data.into_tensor_f32()?;
                let (data, _offset) = tensor.into_raw_vec_and_offset();
                let new_shape: Vec<usize> = shape
                    .iter()
                    .map(|&d| if d < 0 { 1usize } else { d as usize })
                    .collect();
                let new_tensor = ArrayD::from_shape_vec(IxDyn(&new_shape), data)
                    .map_err(|e| TransformError::InvalidInput(e.to_string()))?;
                Ok(TransformData::TensorF32(new_tensor))
            }

            PreprocessOp::CenterCrop { size } => {
                let img = data.into_image()?;
                let (w, h) = (size[0], size[1]);
                let (img_w, img_h) = img.dimensions();

                let x = (img_w.saturating_sub(w)) / 2;
                let y = (img_h.saturating_sub(h)) / 2;

                let cropped = img.crop_imm(x, y, w, h);
                Ok(TransformData::Image(cropped))
            }

            PreprocessOp::Pad { padding, value } => {
                let img = data.into_image()?;
                let p = padding;
                let padded = pad_image(
                    &img,
                    p[0],
                    p[1],
                    p.get(2).copied().unwrap_or(p[0]),
                    p.get(3).copied().unwrap_or(p[1]),
                    value.unwrap_or(0.0),
                );
                Ok(TransformData::Image(padded))
            }
        }
    }
}

pub struct PostprocessPipeline {
    ops: Vec<PostprocessOp>,
    labels: Option<LabelMapping>,
}

impl PostprocessPipeline {
    pub fn new(ops: Vec<PostprocessOp>, labels: Option<LabelMapping>) -> Self {
        Self { ops, labels }
    }

    pub fn run(&self, input: TransformData) -> Result<serde_json::Value, TransformError> {
        let mut data = input;
        for op in &self.ops {
            data = self.apply_op(op, data)?;
        }
        data.to_json()
    }

    fn apply_op(
        &self,
        op: &PostprocessOp,
        data: TransformData,
    ) -> Result<TransformData, TransformError> {
        match op {
            PostprocessOp::Softmax => {
                let tensor = data.into_tensor_f32()?;
                let softmax = compute_softmax(&tensor);
                Ok(TransformData::TensorF32(softmax))
            }

            PostprocessOp::Sigmoid => {
                let tensor = data.into_tensor_f32()?;
                let sigmoid = tensor.mapv(|v| 1.0 / (1.0 + (-v).exp()));
                Ok(TransformData::TensorF32(sigmoid))
            }

            PostprocessOp::Argmax { keep_prob } => {
                let tensor = data.into_tensor_f32()?;
                let (idx, val) = find_argmax(&tensor);

                if *keep_prob {
                    Ok(TransformData::Json(serde_json::json!({
                        "class_index": idx,
                        "probability": val
                    })))
                } else {
                    Ok(TransformData::Scalar(ScalarValue::Index(idx)))
                }
            }

            PostprocessOp::TopK { k } => {
                let tensor = data.into_tensor_f32()?;
                let top_k = find_top_k(&tensor, *k);
                Ok(TransformData::Json(serde_json::to_value(&top_k)?))
            }

            PostprocessOp::Threshold { value } => {
                let tensor = data.into_tensor_f32()?;
                let thresholded = tensor.mapv(|v| if v >= *value { v } else { 0.0 });
                Ok(TransformData::TensorF32(thresholded))
            }

            PostprocessOp::Slice { start, end } => {
                let tensor = data.into_tensor_f32()?;
                let (data, _offset) = tensor.into_raw_vec_and_offset();
                let end_idx = if *end == 0 { data.len() } else { *end };
                let sliced = data[*start..end_idx].to_vec();
                Ok(TransformData::TensorF32(
                    ArrayD::from_shape_vec(IxDyn(&[sliced.len()]), sliced).unwrap(),
                ))
            }

            PostprocessOp::MapLabels => {
                let labels = self.labels.as_ref().ok_or(TransformError::NoLabels)?;
                match data {
                    TransformData::Scalar(ScalarValue::Index(idx)) => {
                        let label = labels.labels.get(idx).cloned().unwrap_or_default();
                        Ok(TransformData::Json(serde_json::json!({
                            "label": label,
                            "class_index": idx
                        })))
                    }
                    TransformData::Json(ref json) => {
                        if let Some(idx) = json.get("class_index").and_then(|v| v.as_u64()) {
                            let label =
                                labels.labels.get(idx as usize).cloned().unwrap_or_default();
                            let mut result = json.clone();
                            if let serde_json::Value::Object(ref mut map) = result {
                                map.insert("label".to_string(), serde_json::Value::String(label));
                            }
                            Ok(TransformData::Json(result))
                        } else {
                            Ok(data)
                        }
                    }
                    _ => Err(TransformError::InvalidInput(
                        "Expected index or json for map_labels".to_string(),
                    )),
                }
            }

            PostprocessOp::Nms {
                iou_threshold,
                score_threshold,
            } => Err(TransformError::UnsupportedOperation(format!(
                "NMS with iou={}, score={} not yet implemented",
                iou_threshold, score_threshold
            ))),
        }
    }
}

fn normalize_tensor(tensor: &ArrayD<f32>, mean: &[f32], std: &[f32]) -> ArrayD<f32> {
    tensor.mapv(|v| {
        let c = mean.len();
        let _total: usize = tensor.shape().iter().product();
        let flat_idx = 0;
        let channel_idx = flat_idx % c;
        (v - mean[channel_idx]) / std[channel_idx]
    })
}

fn image_to_tensor(img: &image::DynamicImage, scale: f32) -> Result<ArrayD<f32>, TransformError> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut data = Vec::with_capacity((w * h * 3) as usize);

    for pixel in rgb.pixels() {
        for c in pixel.0.iter() {
            data.push(*c as f32 / scale);
        }
    }

    let tensor = ArrayD::from_shape_vec(IxDyn(&[h as usize, w as usize, 3]), data)
        .map_err(|e| TransformError::InvalidInput(e.to_string()))?;

    Ok(tensor)
}

fn compute_softmax(tensor: &ArrayD<f32>) -> ArrayD<f32> {
    let max = tensor.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: f32 = tensor.iter().map(|v| (v - max).exp()).sum();
    tensor.mapv(|v| (v - max).exp() / exp_sum)
}

fn find_argmax(tensor: &ArrayD<f32>) -> (usize, f32) {
    let mut max_idx = 0;
    let mut max_val = f32::NEG_INFINITY;

    for (i, &v) in tensor.iter().enumerate() {
        if v > max_val {
            max_val = v;
            max_idx = i;
        }
    }

    (max_idx, max_val)
}

fn find_top_k(tensor: &ArrayD<f32>, k: usize) -> Vec<(usize, f32)> {
    let mut indexed: Vec<(usize, f32)> = tensor.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    indexed.into_iter().take(k).collect()
}

fn pad_image(
    img: &image::DynamicImage,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    value: f32,
) -> image::DynamicImage {
    let (w, h) = img.dimensions();
    let new_w = w + left + right;
    let new_h = h + top + bottom;

    let mut padded = image::RgbImage::new(new_w, new_h);
    let fill_value = (value * 255.0) as u8;

    for pixel in padded.pixels_mut() {
        *pixel = image::Rgb([fill_value, fill_value, fill_value]);
    }

    let img_rgb = img.to_rgb8();
    image::imageops::overlay(&mut padded, &img_rgb, left as i64, top as i64);
    image::DynamicImage::ImageRgb8(padded)
}

impl TransformData {
    pub fn into_image(self) -> Result<image::DynamicImage, TransformError> {
        match self {
            TransformData::Image(img) => Ok(img),
            _ => Err(TransformError::InvalidInput(
                "Expected image data".to_string(),
            )),
        }
    }

    pub fn into_tensor_f32(self) -> Result<ArrayD<f32>, TransformError> {
        match self {
            TransformData::TensorF32(t) => Ok(t),
            _ => Err(TransformError::InvalidInput(
                "Expected float tensor".to_string(),
            )),
        }
    }

    pub fn to_json(&self) -> Result<serde_json::Value, TransformError> {
        match self {
            TransformData::TensorF32(t) => {
                let data: Vec<f32> = t.iter().cloned().collect();
                Ok(serde_json::to_value(data)?)
            }
            TransformData::TensorI64(t) => {
                let data: Vec<i64> = t.iter().cloned().collect();
                Ok(serde_json::to_value(data)?)
            }
            TransformData::Json(v) => Ok(v.clone()),
            TransformData::Scalar(s) => Ok(serde_json::to_value(s)?),
            TransformData::Image(_) => Err(TransformError::InvalidInput(
                "Cannot convert image to json directly".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::arr1;

    #[test]
    fn test_softmax() {
        let tensor = arr1(&[1.0, 2.0, 3.0]).into_dyn();
        let softmax = compute_softmax(&tensor);

        let sum: f32 = softmax.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_argmax() {
        let tensor = arr1(&[0.1, 0.5, 0.3, 0.9, 0.2]).into_dyn();
        let (idx, val) = find_argmax(&tensor);

        assert_eq!(idx, 3);
        assert!((val - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_top_k() {
        let tensor = arr1(&[0.5, 0.9, 0.3, 0.7, 0.1]).into_dyn();
        let top3 = find_top_k(&tensor, 3);

        assert_eq!(top3.len(), 3);
        assert_eq!(top3[0].0, 1);
        assert_eq!(top3[1].0, 3);
        assert_eq!(top3[2].0, 0);
    }

    #[test]
    fn test_preprocess_pipeline_empty() {
        let pipeline = PreprocessPipeline::new(vec![]);
        let tensor = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1.0, 2.0, 3.0]).unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                assert_eq!(t.len(), 3);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_postprocess_pipeline_softmax_argmax() {
        let pipeline = PostprocessPipeline::new(
            vec![
                PostprocessOp::Softmax,
                PostprocessOp::Argmax { keep_prob: true },
            ],
            None,
        );

        let tensor = arr1(&[1.0, 5.0, 2.0]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let obj = result.as_object().unwrap();
        assert_eq!(obj.get("class_index").unwrap().as_u64().unwrap(), 1);
        assert!(obj.get("probability").unwrap().as_f64().unwrap() > 0.5);
    }

    #[test]
    fn test_map_labels() {
        let labels = LabelMapping {
            labels: vec!["cat".to_string(), "dog".to_string(), "bird".to_string()],
            description: None,
        };

        let pipeline = PostprocessPipeline::new(vec![PostprocessOp::MapLabels], Some(labels));

        let result = pipeline
            .run(TransformData::Scalar(ScalarValue::Index(1)))
            .unwrap();

        let obj = result.as_object().unwrap();
        assert_eq!(obj.get("label").unwrap().as_str().unwrap(), "dog");
        assert_eq!(obj.get("class_index").unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn test_postprocess_sigmoid() {
        let pipeline = PostprocessPipeline::new(vec![PostprocessOp::Sigmoid], None);
        let tensor = arr1(&[0.0, 1.0, -1.0]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let values: Vec<f32> = serde_json::from_value(result).unwrap();
        assert!((values[0] - 0.5).abs() < 1e-6);
        assert!(values[1] > 0.5);
        assert!(values[2] < 0.5);
    }

    #[test]
    fn test_postprocess_threshold() {
        let pipeline =
            PostprocessPipeline::new(vec![PostprocessOp::Threshold { value: 0.5 }], None);
        let tensor = arr1(&[0.3, 0.7, 0.5, 0.2]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let values: Vec<f32> = serde_json::from_value(result).unwrap();
        assert_eq!(values[0], 0.0);
        assert_eq!(values[1], 0.7);
        assert_eq!(values[2], 0.5);
        assert_eq!(values[3], 0.0);
    }

    #[test]
    fn test_postprocess_slice() {
        let pipeline =
            PostprocessPipeline::new(vec![PostprocessOp::Slice { start: 1, end: 3 }], None);
        let tensor = arr1(&[1.0, 2.0, 3.0, 4.0, 5.0]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let values: Vec<f32> = serde_json::from_value(result).unwrap();
        assert_eq!(values, vec![2.0, 3.0]);
    }

    #[test]
    fn test_postprocess_slice_to_end() {
        let pipeline =
            PostprocessPipeline::new(vec![PostprocessOp::Slice { start: 2, end: 0 }], None);
        let tensor = arr1(&[1.0, 2.0, 3.0, 4.0]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let values: Vec<f32> = serde_json::from_value(result).unwrap();
        assert_eq!(values, vec![3.0, 4.0]);
    }

    #[test]
    fn test_preprocess_normalize() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Normalize {
            mean: vec![0.5, 0.5, 0.5],
            std: vec![0.5, 0.5, 0.5],
        }]);

        let tensor = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1.0, 0.0, -1.0]).unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                let normalized: Vec<f32> = t.iter().cloned().collect();
                assert!((normalized[0] - 1.0).abs() < 1e-6);
                assert!((normalized[1] - (-1.0)).abs() < 1e-6);
                assert!((normalized[2] - (-3.0)).abs() < 1e-6);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_preprocess_squeeze() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Squeeze { axes: vec![0, 2] }]);

        let tensor =
            ArrayD::from_shape_vec(IxDyn(&[1, 3, 1, 2]), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
                .unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                assert_eq!(t.shape(), &[3, 2]);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_preprocess_unsqueeze() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Unsqueeze { axes: vec![0, 2] }]);

        let tensor =
            ArrayD::from_shape_vec(IxDyn(&[3, 2]), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                assert_eq!(t.shape(), &[1, 3, 1, 2]);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_preprocess_transpose() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Transpose { axes: vec![1, 0] }]);

        let tensor =
            ArrayD::from_shape_vec(IxDyn(&[2, 3]), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                assert_eq!(t.shape(), &[3, 2]);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_preprocess_reshape() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Reshape { shape: vec![2, 3] }]);

        let tensor =
            ArrayD::from_shape_vec(IxDyn(&[6]), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        match result {
            TransformData::TensorF32(t) => {
                assert_eq!(t.shape(), &[2, 3]);
            }
            _ => panic!("Expected tensor"),
        }
    }

    #[test]
    fn test_transform_data_into_image() {
        let img = image::DynamicImage::new_rgb8(10, 10);
        let data = TransformData::Image(img);
        let result = data.into_image();
        assert!(result.is_ok());
    }

    #[test]
    fn test_transform_data_into_tensor_f32() {
        let tensor = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1.0, 2.0, 3.0]).unwrap();
        let data = TransformData::TensorF32(tensor);
        let result = data.into_tensor_f32();
        assert!(result.is_ok());
    }

    #[test]
    fn test_transform_data_into_tensor_f32_wrong_type() {
        let img = image::DynamicImage::new_rgb8(10, 10);
        let data = TransformData::Image(img);
        let result = data.into_tensor_f32();
        assert!(result.is_err());
    }

    #[test]
    fn test_transform_data_to_json_tensor_f32() {
        let tensor = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1.0, 2.0, 3.0]).unwrap();
        let data = TransformData::TensorF32(tensor);
        let result = data.to_json().unwrap();
        let values: Vec<f32> = serde_json::from_value(result).unwrap();
        assert_eq!(values, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_transform_data_to_json_tensor_i64() {
        let tensor = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1i64, 2, 3]).unwrap();
        let data = TransformData::TensorI64(tensor);
        let result = data.to_json().unwrap();
        let values: Vec<i64> = serde_json::from_value(result).unwrap();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn test_transform_data_to_json_image_error() {
        let img = image::DynamicImage::new_rgb8(10, 10);
        let data = TransformData::Image(img);
        let result = data.to_json();
        assert!(result.is_err());
    }

    #[test]
    fn test_preprocess_grayscale() {
        let pipeline = PreprocessPipeline::new(vec![PreprocessOp::Grayscale]);
        let img = image::DynamicImage::new_rgb8(10, 10);
        let result = pipeline.run(TransformData::Image(img)).unwrap();

        match result {
            TransformData::Image(gray_img) => {
                assert!(gray_img.as_luma8().is_some());
            }
            _ => panic!("Expected image"),
        }
    }

    #[test]
    fn test_postprocess_argmax_without_prob() {
        let pipeline =
            PostprocessPipeline::new(vec![PostprocessOp::Argmax { keep_prob: false }], None);

        let tensor = arr1(&[0.1, 0.9, 0.3]).into_dyn();
        let result = pipeline.run(TransformData::TensorF32(tensor)).unwrap();

        let idx = result.get("Index").unwrap().as_u64().unwrap();
        assert_eq!(idx, 1);
    }
}
