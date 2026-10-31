#[derive(Clone)]
pub struct Client;

#[derive(Clone)]
pub struct Chunk {
    pub text: String,
}

pub mod errors {
    #[derive(Debug, thiserror::Error)]
    pub enum StreamError {
        #[error("future failure")]
        FutureFailure,
        #[error("authentication failed: {message}")]
        Authentication { message: String, status: u16 },
        #[error("rate limited: {message}")]
        RateLimited {
            message: String,
            status: u16,
            retry_ms: u64,
        },
        #[error("content policy: {message}")]
        ContentPolicy { message: String, status: u16 },
    }
}
use errors::StreamError;

pub fn create_client() -> Client {
    Client
}

impl Client {
    pub async fn records(
        &self,
        mode: u32,
    ) -> Result<impl futures_util::Stream<Item = Result<Chunk, StreamError>> + Send + 'static, StreamError> {
        if mode == 1 {
            return Err(StreamError::Authentication {
                message: "bad key".into(),
                status: 401,
            });
        }
        if mode == 3 {
            return Err(StreamError::ContentPolicy {
                message: "blocked".into(),
                status: 403,
            });
        }
        if mode == 4 {
            return Err(StreamError::FutureFailure);
        }
        let mut items = vec![Ok(Chunk { text: "first".into() }), Ok(Chunk { text: "second".into() })];
        if mode == 2 {
            items.push(Err(StreamError::RateLimited {
                message: "slow down".into(),
                status: 429,
                retry_ms: 7000,
            }));
            items.push(Ok(Chunk {
                text: "must not follow the error".into(),
            }));
        }
        Ok(futures_util::stream::iter(items))
    }
}

impl Client {
    pub async fn scalar_records(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<u64, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(42), Ok(u64::MAX)]))
    }
    pub async fn optional_records(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Option<u64>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(Some(42)), Ok(None)]))
    }
    pub async fn vector_records(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Vec<u32>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(vec![1, 2])]))
    }
    pub async fn float_records(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Vec<f32>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(vec![1.5, 2.5])]))
    }
    pub async fn unit_records(&self) -> Result<impl futures_util::Stream<Item = Result<(), StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(())]))
    }
}
