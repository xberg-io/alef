use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("{0}")]
    Callback(String, #[source] Option<Box<dyn std::error::Error + Send + Sync>>),
    #[error("worker failed: {0}")]
    Wrapped(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("native failure")]
    NativeFailure,
}

#[async_trait::async_trait]
pub trait Worker: Send + Sync {
    fn initialize(&self) -> Result<(), WorkerError>;
    fn shutdown(&self) -> Result<(), WorkerError>;
    fn work(&self) -> Result<String, WorkerError>;
    async fn work_async(&self) -> Result<String, WorkerError>;
}

pub struct Registry(Option<Arc<dyn Worker>>);

impl Registry {
    pub fn register(&mut self, worker: Arc<dyn Worker>) -> Result<(), WorkerError> {
        worker.initialize()?;
        self.0 = Some(worker);
        Ok(())
    }
}

pub fn workers() -> &'static RwLock<Registry> {
    static WORKERS: RwLock<Registry> = RwLock::new(Registry(None));
    &WORKERS
}

pub mod plugins {
    use super::{WorkerError, workers};

    pub fn unregister_worker(name: &str) -> Result<(), WorkerError> {
        if name == "native-only" {
            return Err(WorkerError::NativeFailure);
        }
        clear_workers()
    }

    pub fn clear_workers() -> Result<(), WorkerError> {
        let worker = workers().write().0.take();
        worker.map_or(Ok(()), |worker| worker.shutdown())
    }
}

fn registered_worker() -> Arc<dyn Worker> {
    workers().read().0.clone().unwrap()
}

pub fn invoke_worker() -> Result<String, WorkerError> {
    registered_worker().work()
}

pub async fn invoke_worker_async() -> Result<String, WorkerError> {
    registered_worker()
        .work_async()
        .await
        .map_err(|error| WorkerError::Wrapped(Box::new(error)))
}

pub fn reject_natively() -> Result<String, WorkerError> {
    Err(WorkerError::NativeFailure)
}
