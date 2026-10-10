use std::sync::Arc;
#[derive(Debug, Clone)]
pub struct CoreError(pub String);
impl CoreError {
    pub fn new(message: String) -> Self {
        Self(message)
    }
}
impl std::fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for CoreError {}
#[async_trait::async_trait]
pub trait Worker: Send + Sync {
    async fn process(&self) -> Result<String, CoreError>;
}
pub async fn run_worker(worker: Arc<dyn Worker>) -> Result<String, CoreError> {
    worker.process().await
}
