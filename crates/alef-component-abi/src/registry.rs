//! Process-wide contract-provider registry.
//!
//! The core crate a component set was extracted from does not depend on
//! `alef-component-runtime` (that dependency would defeat the point of
//! shrinking the core crate), so it cannot hold a `LoadedComponent` directly.
//! Instead, the host binding activates a downloaded component and registers a
//! generated proxy here under the contract's name; the core crate looks the
//! provider back up by the same name and the trait object type it expects.
//!
//! Storage is a single, explicitly named `static` (`REGISTRY`) rather than a
//! scattering of per-contract statics, keeping the process-wide state in one
//! place with one documented API instead of ad hoc globals.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// A minimal spinlock so this crate can stay `#![no_std]` and dependency-free.
///
/// Registration and lookup are both O(1)-ish, infrequent (once per component
/// activation, once per call site cache miss), so a spinlock is adequate and
/// avoids pulling in a platform-specific mutex or an external crate.
struct SpinMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is only ever granted through `lock`, which holds
// exclusive access for the lifetime of the returned guard. ~keep
unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    fn lock(&self) -> SpinMutexGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinMutexGuard { lock: self }
    }
}

struct SpinMutexGuard<'a, T> {
    lock: &'a SpinMutex<T>,
}

impl<T> core::ops::Deref for SpinMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: holding the guard proves exclusive access to `value`. ~keep
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> core::ops::DerefMut for SpinMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: holding the guard proves exclusive access to `value`. ~keep
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// A type-erased `Arc<T>` that can be stored behind `dyn Any` even when `T` is
/// an unsized trait object: `Erased<T>` itself is always `Sized` because
/// `Arc<T>` is a plain (possibly fat) pointer.
struct Erased<T: ?Sized + 'static>(Arc<T>);

type Entry = Box<dyn Any + Send + Sync>;

/// Registry of contract providers, keyed by contract name.
///
/// Each contract name holds at most one provider; registering again under the
/// same name replaces the previous provider.
pub struct ContractRegistry {
    entries: SpinMutex<Vec<(String, Entry)>>,
}

impl ContractRegistry {
    const fn new() -> Self {
        Self {
            entries: SpinMutex::new(Vec::new()),
        }
    }

    fn register<T: ?Sized + Send + Sync + 'static>(&self, contract: &str, provider: Arc<T>) {
        let boxed: Entry = Box::new(Erased(provider));
        let mut entries = self.entries.lock();
        if let Some(slot) = entries.iter_mut().find(|(name, _)| name == contract) {
            slot.1 = boxed;
        } else {
            entries.push((String::from(contract), boxed));
        }
    }

    fn lookup<T: ?Sized + Send + Sync + 'static>(&self, contract: &str) -> Option<Arc<T>> {
        let entries = self.entries.lock();
        entries
            .iter()
            .find(|(name, _)| name == contract)
            .and_then(|(_, entry)| entry.downcast_ref::<Erased<T>>())
            .map(|erased| Arc::clone(&erased.0))
    }
}

/// The single process-wide contract registry.
static REGISTRY: ContractRegistry = ContractRegistry::new();

/// Register `provider` as the implementation of `contract`.
///
/// A second registration for the same `contract` name replaces the first.
/// `T` is typically a trait object type such as `dyn Codec`.
pub fn register_provider<T: ?Sized + Send + Sync + 'static>(contract: &str, provider: Arc<T>) {
    REGISTRY.register(contract, provider);
}

/// Look up the provider registered for `contract`, if any and if it was
/// registered with the same trait object type `T`.
///
/// Returns `None` both when no provider was registered for `contract` and
/// when a provider was registered under that name with a different type;
/// callers cannot distinguish the two from this API alone, mirroring `Any`.
pub fn provider<T: ?Sized + Send + Sync + 'static>(contract: &str) -> Option<Arc<T>> {
    REGISTRY.lookup(contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Greeter: Send + Sync {
        fn greet(&self) -> &str;
    }

    struct Hello;

    impl Greeter for Hello {
        fn greet(&self) -> &str {
            "hello"
        }
    }

    trait OtherContract: Send + Sync {
        fn other(&self) -> u32;
    }

    struct OtherImpl;

    impl OtherContract for OtherImpl {
        fn other(&self) -> u32 {
            7
        }
    }

    #[test]
    fn register_then_lookup_returns_the_same_provider() {
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("greeter", Arc::new(Hello));
        let found = registry.lookup::<dyn Greeter>("greeter").expect("provider registered");
        assert_eq!(found.greet(), "hello");
    }

    #[test]
    fn lookup_of_unregistered_contract_is_absent() {
        let registry = ContractRegistry::new();
        assert!(registry.lookup::<dyn Greeter>("greeter").is_none());
    }

    #[test]
    fn lookup_with_mismatched_type_is_absent_not_a_panic() {
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("shared-name", Arc::new(Hello));
        assert!(registry.lookup::<dyn OtherContract>("shared-name").is_none());

        registry.register::<dyn OtherContract>("other", Arc::new(OtherImpl));
        assert_eq!(registry.lookup::<dyn OtherContract>("other").unwrap().other(), 7);
    }

    #[test]
    fn registering_twice_replaces_the_previous_provider() {
        struct Goodbye;
        impl Greeter for Goodbye {
            fn greet(&self) -> &str {
                "goodbye"
            }
        }
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("greeter", Arc::new(Hello));
        registry.register::<dyn Greeter>("greeter", Arc::new(Goodbye));
        let found = registry.lookup::<dyn Greeter>("greeter").unwrap();
        assert_eq!(found.greet(), "goodbye");
    }

    #[test]
    fn process_wide_helpers_round_trip_through_the_shared_static() {
        register_provider::<dyn Greeter>("process-wide-greeter", Arc::new(Hello));
        let found = provider::<dyn Greeter>("process-wide-greeter").expect("provider registered");
        assert_eq!(found.greet(), "hello");
        assert!(provider::<dyn OtherContract>("process-wide-greeter").is_none());
    }
}
