use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budget {
    pub max_tokens: u32,
    pub optional_limit: Option<u32>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            optional_limit: Some(5),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(rename = "model_name")]
    pub model: String,
    #[serde(default)]
    pub budget: Budget,
    pub retries: u32,
    pub cache_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissive {
    pub model: String,
}
