use std::sync::atomic::{AtomicU32, Ordering};

static SCALAR_CALLS: AtomicU32 = AtomicU32::new(0);
static CALLBACK_STARTED: AtomicU32 = AtomicU32::new(0);
static CALLBACK_FINISHED: AtomicU32 = AtomicU32::new(0);

pub fn raw_scalar(value: u64) -> u64 {
    SCALAR_CALLS.fetch_add(1, Ordering::SeqCst);
    value
}

pub fn echo_signed(value: i64) -> i64 {
    SCALAR_CALLS.fetch_add(1, Ordering::SeqCst);
    value
}

pub fn scalar_calls() -> u32 {
    SCALAR_CALLS.load(Ordering::SeqCst)
}
pub fn callback_started() -> u32 {
    CALLBACK_STARTED.load(Ordering::SeqCst)
}
pub fn callback_finished() -> u32 {
    CALLBACK_FINISHED.load(Ordering::SeqCst)
}

pub fn reset_counts() {
    SCALAR_CALLS.store(0, Ordering::SeqCst);
    CALLBACK_STARTED.store(0, Ordering::SeqCst);
    CALLBACK_FINISHED.store(0, Ordering::SeqCst);
}

pub fn install_warning_capture() -> bool {
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber).is_ok()
}

pub mod plugins {
    pub trait WideCallback: Send + Sync {
        fn value(&self, input: u64) -> u64;
    }

    #[doc(hidden)]
    pub mod registry {
        use super::WideCallback;
        use parking_lot::RwLock;
        use std::sync::{Arc, OnceLock};

        #[doc(hidden)]
        pub struct WideRegistry {
            callback: Option<Arc<dyn WideCallback>>,
        }

        impl WideRegistry {
            pub fn register(&mut self, callback: Arc<dyn WideCallback>) -> Result<(), String> {
                self.callback = Some(callback);
                Ok(())
            }

            pub fn current(&self) -> Arc<dyn WideCallback> {
                self.callback.as_ref().expect("registered source callback").clone()
            }
        }

        #[doc(hidden)]
        pub fn get_registry() -> &'static RwLock<WideRegistry> {
            static REGISTRY: OnceLock<RwLock<WideRegistry>> = OnceLock::new();
            REGISTRY.get_or_init(|| RwLock::new(WideRegistry { callback: None }))
        }
    }
}

pub fn invoke_callback(input: u64) -> u64 {
    let callback = plugins::registry::get_registry().read().current();
    CALLBACK_STARTED.fetch_add(1, Ordering::SeqCst);
    let output = callback.value(input);
    CALLBACK_FINISHED.fetch_add(1, Ordering::SeqCst);
    output
}
