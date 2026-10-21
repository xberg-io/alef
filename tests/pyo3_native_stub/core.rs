#[derive(Clone, Default)]
pub struct ContentConfig {
    pub label: String,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum CoreError {
    Rejected { vendor: String },
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("blocked")
    }
}

pub fn fail() -> Result<(), CoreError> {
    Err(CoreError::Rejected {
        vendor: "sample-vendor".into(),
    })
}

impl ContentConfig {
    pub fn default() -> Self {
        Self {
            label: "native-default".into(),
        }
    }

    pub fn validate(&self) {}
}
