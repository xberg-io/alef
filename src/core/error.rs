use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlefError {
    #[error("Config error: {0}")]
    Config(String),
    #[error("Extraction error: {0}")]
    Extraction(String),
    #[error("Generation error for {language}: {message}")]
    Generation { language: String, message: String },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
