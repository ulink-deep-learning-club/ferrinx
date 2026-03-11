use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub meta: ModelMeta,
    pub model: ModelFile,
    #[serde(default)]
    pub inputs: Vec<InputConfig>,
    #[serde(default)]
    pub outputs: Vec<OutputConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelFile {
    pub file: String,
    #[serde(default)]
    pub labels: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InputConfig {
    pub name: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub shape: Vec<i64>,
    #[serde(default = "default_dtype")]
    pub dtype: String,
    #[serde(default)]
    pub preprocess: Vec<PreprocessOp>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputConfig {
    pub name: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub shape: Vec<i64>,
    #[serde(default = "default_dtype")]
    pub dtype: String,
    #[serde(default)]
    pub postprocess: Vec<PostprocessOp>,
}

fn default_dtype() -> String {
    "float32".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum PreprocessOp {
    #[serde(rename = "resize")]
    Resize { size: Vec<u32> },

    #[serde(rename = "grayscale")]
    Grayscale,

    #[serde(rename = "normalize")]
    Normalize { mean: Vec<f32>, std: Vec<f32> },

    #[serde(rename = "to_tensor")]
    ToTensor {
        #[serde(default = "default_dtype")]
        dtype: String,
        #[serde(default)]
        scale: Option<f32>,
    },

    #[serde(rename = "transpose")]
    Transpose { axes: Vec<usize> },

    #[serde(rename = "squeeze")]
    Squeeze {
        #[serde(default)]
        axes: Vec<usize>,
    },

    #[serde(rename = "unsqueeze")]
    Unsqueeze { axes: Vec<usize> },

    #[serde(rename = "reshape")]
    Reshape { shape: Vec<i64> },

    #[serde(rename = "center_crop")]
    CenterCrop { size: Vec<u32> },

    #[serde(rename = "pad")]
    Pad {
        padding: Vec<u32>,
        #[serde(default)]
        value: Option<f32>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum PostprocessOp {
    #[serde(rename = "softmax")]
    Softmax,

    #[serde(rename = "sigmoid")]
    Sigmoid,

    #[serde(rename = "argmax")]
    Argmax {
        #[serde(default)]
        keep_prob: bool,
    },

    #[serde(rename = "top_k")]
    TopK { k: usize },

    #[serde(rename = "threshold")]
    Threshold { value: f32 },

    #[serde(rename = "slice")]
    Slice {
        #[serde(default)]
        start: usize,
        #[serde(default)]
        end: usize,
    },

    #[serde(rename = "map_labels")]
    MapLabels,

    #[serde(rename = "nms")]
    Nms {
        iou_threshold: f32,
        score_threshold: f32,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelMapping {
    pub labels: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ModelConfig {
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    pub fn from_toml_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_toml(&content)?)
    }

    pub fn load_labels(&self, base_path: &Path) -> Option<LabelMapping> {
        self.model.labels.as_ref().and_then(|label_file| {
            let label_path = base_path.join(label_file);
            std::fs::read_to_string(label_path)
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
        })
    }

    pub fn input_by_name(&self, name: &str) -> Option<&InputConfig> {
        self.inputs
            .iter()
            .find(|i| i.name == name || i.alias.as_deref() == Some(name))
    }

    pub fn output_by_name(&self, name: &str) -> Option<&OutputConfig> {
        self.outputs
            .iter()
            .find(|o| o.name == name || o.alias.as_deref() == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_model_config() {
        let toml_content = r#"
[meta]
name = "lenet-mnist"
version = "1.0"
description = "MNIST digit classification"

[model]
file = "lenet.onnx"
labels = "labels.json"

[[inputs]]
name = "input.1"
alias = "image"
shape = [-1, 1, 28, 28]
dtype = "float32"

[[inputs.preprocess]]
type = "resize"
size = [28, 28]

[[inputs.preprocess]]
type = "normalize"
mean = [0.1307]
std = [0.3081]

[[outputs]]
name = "output.1"
alias = "prediction"
shape = [-1, 10]
dtype = "float32"

[[outputs.postprocess]]
type = "softmax"

[[outputs.postprocess]]
type = "argmax"
keep_prob = true
"#;

        let config = ModelConfig::from_toml(toml_content).unwrap();

        assert_eq!(config.meta.name, "lenet-mnist");
        assert_eq!(config.meta.version, "1.0");
        assert_eq!(config.model.file, "lenet.onnx");
        assert_eq!(config.inputs.len(), 1);
        assert_eq!(config.outputs.len(), 1);

        let input = &config.inputs[0];
        assert_eq!(input.name, "input.1");
        assert_eq!(input.alias, Some("image".to_string()));
        assert_eq!(input.shape, vec![-1, 1, 28, 28]);
        assert_eq!(input.preprocess.len(), 2);

        let output = &config.outputs[0];
        assert_eq!(output.name, "output.1");
        assert_eq!(output.postprocess.len(), 2);
    }

    #[test]
    fn test_parse_label_mapping() {
        let json_content = r#"
{
    "labels": ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"],
    "description": "MNIST digits"
}
"#;

        let mapping: LabelMapping = serde_json::from_str(json_content).unwrap();
        assert_eq!(mapping.labels.len(), 10);
        assert_eq!(mapping.description, Some("MNIST digits".to_string()));
    }

    #[test]
    fn test_input_by_name() {
        let config = ModelConfig {
            meta: ModelMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
                description: String::new(),
            },
            model: ModelFile {
                file: "test.onnx".to_string(),
                labels: None,
            },
            inputs: vec![InputConfig {
                name: "input.1".to_string(),
                alias: Some("image".to_string()),
                shape: vec![-1, 3, 224, 224],
                dtype: "float32".to_string(),
                preprocess: vec![],
            }],
            outputs: vec![],
        };

        assert!(config.input_by_name("input.1").is_some());
        assert!(config.input_by_name("image").is_some());
        assert!(config.input_by_name("unknown").is_none());
    }
}
