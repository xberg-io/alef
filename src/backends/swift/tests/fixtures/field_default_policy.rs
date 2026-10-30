use serde::{Deserialize, Serialize};

fn default_retries() -> u32 {
    3
}

fn default_limit() -> Option<u32> {
    Some(5)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarOptions {
    pub model: String,
    #[serde(default = "default_retries")]
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionalOptions {
    pub model: String,
    #[serde(default = "default_limit")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyOptions {
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_limit")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedOptions {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_limit")]
    pub limit: Option<u32>,
}
