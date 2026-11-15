//! Support for the contract proxies `alef` generates over native components.
//!
//! A contract method that cannot fail in Rust (its trait signature returns a plain value) still
//! crosses a C ABI that returns a status. When a component reports a failure there, the generated
//! proxy cannot return an error, and panicking would take the host down for one misbehaving
//! component. It reports the failure through [`report_infallible_failure`] and returns the
//! result type's `Default` value instead (empty string, empty bytes, zero, `false`, `()`).
//!
//! Hosts observe or escalate these failures with [`set_infallible_failure_hook`]; without a hook
//! each one is written to stderr. ADR-0029 records the decision.

use std::sync::{Arc, PoisonError, RwLock};

/// A native component reported a failure from a contract method whose Rust signature cannot
/// return an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InfallibleCallFailure<'a> {
    /// The component id from its descriptor.
    pub component: &'a str,
    /// The contract method, in the trait's snake_case spelling.
    pub method: &'a str,
    /// The non-OK `AlefStatus` code the component returned.
    pub status: i32,
    /// The component's error message, lossily decoded as UTF-8.
    pub message: &'a str,
}

impl<'a> InfallibleCallFailure<'a> {
    /// Describe one failure; generated proxies call this.
    #[must_use]
    pub fn new(component: &'a str, method: &'a str, status: i32, message: &'a str) -> Self {
        Self {
            component,
            method,
            status,
            message,
        }
    }
}

type Hook = Arc<dyn Fn(&InfallibleCallFailure<'_>) + Send + Sync>;

static HOOK: RwLock<Option<Hook>> = RwLock::new(None);

/// Replace the process-wide handler for [`InfallibleCallFailure`]s. The handler runs on the
/// calling thread, before the proxy returns the default value; it may log, count, or panic if the
/// host prefers to abort. It must not call back into the failing proxy.
pub fn set_infallible_failure_hook(hook: impl Fn(&InfallibleCallFailure<'_>) + Send + Sync + 'static) {
    *HOOK.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
}

/// Report a failure of an infallible contract method: to the hook set with
/// [`set_infallible_failure_hook`], or to stderr when none is set. Generated proxies call this and
/// then return the method's default value.
pub fn report_infallible_failure(failure: &InfallibleCallFailure<'_>) {
    let hook = HOOK.read().unwrap_or_else(PoisonError::into_inner).clone();
    match hook {
        Some(hook) => hook(failure),
        None => eprintln!(
            "alef component `{}`: method `{}` failed with status {}: {}; returning its default value",
            failure.component, failure.method, failure.status, failure.message
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_hook_receives_each_failure() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            set_infallible_failure_hook(move |failure| {
                seen.lock().unwrap().push((
                    failure.component.to_owned(),
                    failure.method.to_owned(),
                    failure.status,
                    failure.message.to_owned(),
                ));
            });
        }
        report_infallible_failure(&InfallibleCallFailure::new("demo", "name", 5, "broken"));
        assert!(
            seen.lock()
                .unwrap()
                .contains(&("demo".to_owned(), "name".to_owned(), 5, "broken".to_owned()))
        );
    }
}
