use std::collections::HashMap;

#[derive(Clone)]
pub struct Client;

#[derive(Clone)]
pub struct UsizeRecord {
    pub count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("stream failed")]
    Failed,
}

pub fn create_client() -> Client {
    Client
}

impl Client {
    pub async fn usize_record(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<UsizeRecord, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(UsizeRecord { count: 42 })]))
    }

    pub async fn usize_scalar(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<usize, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(42)]))
    }

    pub async fn usize_optional(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Option<usize>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(Some(42)), Ok(None)]))
    }

    pub async fn usize_vector(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Vec<usize>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(vec![0, 42, 1 << 40])]))
    }

    pub async fn usize_nested(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Vec<Option<usize>>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(vec![Some(42), None])]))
    }

    pub async fn usize_map(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<HashMap<usize, u64>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(HashMap::from([(7, u64::MAX)]))]))
    }

    pub async fn u64_scalar(&self) -> Result<impl futures_util::Stream<Item = Result<u64, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(u64::MAX)]))
    }

    pub async fn u64_vector(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<Vec<u64>, StreamError>>, StreamError> {
        Ok(futures_util::stream::iter([Ok(vec![u64::MAX])]))
    }
}
