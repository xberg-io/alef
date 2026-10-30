use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub max_bytes: u32,
    pub max_items: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_bytes: 512,
            max_items: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub model: String,
    #[serde(default)]
    pub budget: Budget,
    pub retries: u32,
}
