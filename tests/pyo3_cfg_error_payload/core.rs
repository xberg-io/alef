#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("blocked")]
    Rejected {
        reason: String,
        #[cfg(feature = "http")]
        status: u16,
    },
}

pub fn fail() -> Result<(), CoreError> {
    Err(CoreError::Rejected {
        reason: "condition".into(),
        #[cfg(feature = "http")]
        status: 429,
    })
}

pub fn http_enabled() -> bool {
    cfg!(feature = "http")
}
