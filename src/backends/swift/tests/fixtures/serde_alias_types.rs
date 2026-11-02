use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    #[default]
    Auto,
    Cpu,
    Cuda,
    Metal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReaderVariant {
    #[default]
    BasePrinted,
    LargePrinted,
    BaseHandwritten,
    LargeHandwritten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisionTask {
    #[default]
    Ocr,
    Table,
    Formula,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutTask {
    #[default]
    Ocr,
    Table,
    Formula,
    Chart,
    Caption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    WholePage,
    Paired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeightPrecision {
    #[default]
    Auto,
    F32,
    F16,
    Bf16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReaderOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<ReaderVariant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(alias = "cache-dir", skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
    #[serde(alias = "hf-revision", alias = "revision", skip_serializing_if = "Option::is_none")]
    pub hf_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VisionOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<VisionTask>,
    #[serde(alias = "model-path", skip_serializing_if = "Option::is_none")]
    pub model_path: Option<String>,
    #[serde(alias = "model-id", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(alias = "hf-revision", alias = "revision", skip_serializing_if = "Option::is_none")]
    pub hf_revision: Option<String>,
    #[serde(alias = "cache-dir", skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<LayoutTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(alias = "layout-mode", skip_serializing_if = "Option::is_none")]
    pub layout_mode: Option<LayoutMode>,
    #[serde(alias = "enable-chart-understanding", skip_serializing_if = "Option::is_none")]
    pub enable_chart_understanding: Option<bool>,
    #[serde(alias = "cache-dir", skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DecoderOptions {
    #[serde(alias = "model-path", skip_serializing_if = "Option::is_none")]
    pub model_path: Option<String>,
    #[serde(alias = "model-id", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(alias = "hf-revision", alias = "revision", skip_serializing_if = "Option::is_none")]
    pub hf_revision: Option<String>,
    #[serde(alias = "cache-dir", skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dtype: Option<WeightPrecision>,
}

fn default_tokens() -> u32 {
    512
}
fn default_optional_limit() -> Option<u32> {
    Some(5)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredOptions {
    #[serde(rename = "model_name", alias = "legacy_model", alias = "legacy-model")]
    pub model: String,
    #[serde(default = "default_tokens", alias = "max-tokens")]
    pub max_tokens: u32,
    #[serde(
        default = "default_optional_limit",
        alias = "optional-limit",
        skip_serializing_if = "Option::is_none"
    )]
    pub optional_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissiveOptions {
    #[serde(alias = "legacy_model")]
    pub model: String,
}
