#[derive(Debug)]
#[cfg_attr(alef, derive(thiserror::Error))]
#[non_exhaustive]
pub enum ConsumerError {
    #[cfg_attr(alef, error("denied: {reason}"))]
    Denied {
        reason: &'static str,
    },
    #[cfg_attr(alef, error("owned: {reason}"))]
    Owned {
        reason: String,
    },
    #[cfg_attr(alef, error("optional borrowed: {reason:?}"))]
    OptionalBorrowed {
        reason: Option<&'static str>,
    },
    #[cfg_attr(alef, error("optional owned: {reason:?}"))]
    OptionalOwned {
        reason: Option<String>,
    },
    #[cfg_attr(alef, error("timeout"))]
    Error,
    Future(String), // ~keep UNLISTED_FUTURE models an error added after extraction.
}

impl std::fmt::Display for ConsumerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied { reason } => write!(formatter, "denied: {reason}"),
            Self::Owned { reason } => write!(formatter, "owned: {reason}"),
            Self::OptionalBorrowed { reason } => write!(formatter, "optional borrowed: {reason:?}"),
            Self::OptionalOwned { reason } => write!(formatter, "optional owned: {reason:?}"),
            Self::Error => formatter.write_str("timeout"),
            Self::Future(message) => formatter.write_str(message), // ~keep UNLISTED_FUTURE
        }
    }
}

pub fn request(case: u8) -> Result<(), ConsumerError> {
    Err(match case {
        0 => ConsumerError::Denied {
            reason: "private_network",
        },
        1 => ConsumerError::Owned {
            reason: "owned payload".into(),
        },
        2 => ConsumerError::OptionalBorrowed { reason: None },
        3 => ConsumerError::OptionalBorrowed {
            reason: Some("borrowed payload"),
        },
        4 => ConsumerError::OptionalOwned { reason: None },
        5 => ConsumerError::OptionalOwned {
            reason: Some("owned optional payload".into()),
        },
        6 => ConsumerError::Future("new upstream failure".into()), // ~keep UNLISTED_FUTURE
        7 => ConsumerError::Error,
        _ => ConsumerError::Denied {
            reason: "fallback policy",
        },
    })
}
