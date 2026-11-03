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

/// One registered provider: which component supplied it, for which contract.
struct Slot {
    component: String,
    contract: String,
    entry: Entry,
}

/// Several components provide the same contract and the caller asked for one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbiguousProvider {
    pub contract: String,
    /// Names of the components that provide `contract`, in registration order.
    pub components: Vec<String>,
}

impl core::fmt::Display for AmbiguousProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "contract `{}` is provided by several components ({}); select one explicitly",
            self.contract,
            self.components.join(", ")
        )
    }
}

/// Registry of contract providers, keyed by `(component, contract)`.
///
/// Each `(component, contract)` pair holds at most one provider; registering it
/// again replaces the previous provider. Different components providing the
/// same contract coexist.
pub struct ContractRegistry {
    entries: SpinMutex<Vec<Slot>>,
}

impl ContractRegistry {
    const fn new() -> Self {
        Self {
            entries: SpinMutex::new(Vec::new()),
        }
    }

    fn register<T: ?Sized + Send + Sync + 'static>(&self, component: &str, contract: &str, provider: Arc<T>) {
        let entry: Entry = Box::new(Erased(provider));
        let mut entries = self.entries.lock();
        if let Some(slot) = entries
            .iter_mut()
            .find(|slot| slot.component == component && slot.contract == contract)
        {
            slot.entry = entry;
        } else {
            entries.push(Slot {
                component: String::from(component),
                contract: String::from(contract),
                entry,
            });
        }
    }

    /// Every `(component, provider)` registered for `contract` with type `T`, in registration order.
    fn lookup_all<T: ?Sized + Send + Sync + 'static>(&self, contract: &str) -> Vec<(String, Arc<T>)> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|slot| slot.contract == contract)
            .filter_map(|slot| {
                slot.entry
                    .downcast_ref::<Erased<T>>()
                    .map(|erased| (slot.component.clone(), Arc::clone(&erased.0)))
            })
            .collect()
    }

    fn lookup_one<T: ?Sized + Send + Sync + 'static>(&self, component: &str, contract: &str) -> Option<Arc<T>> {
        let entries = self.entries.lock();
        entries
            .iter()
            .find(|slot| slot.component == component && slot.contract == contract)
            .and_then(|slot| slot.entry.downcast_ref::<Erased<T>>())
            .map(|erased| Arc::clone(&erased.0))
    }

    fn lookup_unique<T: ?Sized + Send + Sync + 'static>(
        &self,
        contract: &str,
    ) -> Result<Option<Arc<T>>, AmbiguousProvider> {
        let mut all = self.lookup_all::<T>(contract);
        match all.len() {
            0 => Ok(None),
            1 => Ok(all.pop().map(|(_, provider)| provider)),
            _ => Err(AmbiguousProvider {
                contract: String::from(contract),
                components: all.into_iter().map(|(component, _)| component).collect(),
            }),
        }
    }
}

/// The single process-wide contract registry.
static REGISTRY: ContractRegistry = ContractRegistry::new();

/// Register `provider` as `component`'s implementation of `contract`.
///
/// A second registration for the same `(component, contract)` replaces the
/// first; other components' providers for `contract` are untouched.
/// `T` is typically a trait object type such as `dyn Codec`.
pub fn register_provider<T: ?Sized + Send + Sync + 'static>(component: &str, contract: &str, provider: Arc<T>) {
    REGISTRY.register(component, contract, provider);
}

/// All providers registered for `contract` with trait object type `T`, as
/// `(component, provider)` pairs in registration order.
pub fn providers<T: ?Sized + Send + Sync + 'static>(contract: &str) -> Vec<(String, Arc<T>)> {
    REGISTRY.lookup_all(contract)
}

/// The provider `component` registered for `contract`, if any and if it was
/// registered with the same trait object type `T`.
pub fn provider_of<T: ?Sized + Send + Sync + 'static>(component: &str, contract: &str) -> Option<Arc<T>> {
    REGISTRY.lookup_one(component, contract)
}

/// The single provider for `contract`: `Ok(None)` when none is registered (or
/// none with type `T`), `Err` when more than one component provides it.
pub fn try_provider<T: ?Sized + Send + Sync + 'static>(contract: &str) -> Result<Option<Arc<T>>, AmbiguousProvider> {
    REGISTRY.lookup_unique(contract)
}

/// Like [`try_provider`], but reports ambiguity as `None` as well. Prefer
/// [`try_provider`], [`providers`] or [`provider_of`] when several components
/// may provide the same contract.
pub fn provider<T: ?Sized + Send + Sync + 'static>(contract: &str) -> Option<Arc<T>> {
    REGISTRY.lookup_unique(contract).ok().flatten()
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
        registry.register::<dyn Greeter>("a", "greeter", Arc::new(Hello));
        let found = registry
            .lookup_unique::<dyn Greeter>("greeter")
            .unwrap()
            .expect("provider registered");
        assert_eq!(found.greet(), "hello");
    }

    #[test]
    fn lookup_of_unregistered_contract_is_absent() {
        let registry = ContractRegistry::new();
        assert!(registry.lookup_unique::<dyn Greeter>("greeter").unwrap().is_none());
    }

    #[test]
    fn lookup_with_mismatched_type_is_absent_not_a_panic() {
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("a", "shared-name", Arc::new(Hello));
        assert!(
            registry
                .lookup_unique::<dyn OtherContract>("shared-name")
                .unwrap()
                .is_none()
        );

        registry.register::<dyn OtherContract>("a", "other", Arc::new(OtherImpl));
        assert_eq!(
            registry
                .lookup_unique::<dyn OtherContract>("other")
                .unwrap()
                .unwrap()
                .other(),
            7
        );
    }

    #[test]
    fn registering_the_same_component_and_contract_twice_replaces_the_provider() {
        struct Goodbye;
        impl Greeter for Goodbye {
            fn greet(&self) -> &str {
                "goodbye"
            }
        }
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("a", "greeter", Arc::new(Hello));
        registry.register::<dyn Greeter>("a", "greeter", Arc::new(Goodbye));
        let found = registry.lookup_unique::<dyn Greeter>("greeter").unwrap().unwrap();
        assert_eq!(found.greet(), "goodbye");
    }

    #[test]
    fn process_wide_helpers_round_trip_through_the_shared_static() {
        register_provider::<dyn Greeter>("comp", "process-wide-greeter", Arc::new(Hello));
        let found = provider::<dyn Greeter>("process-wide-greeter").expect("provider registered");
        assert_eq!(found.greet(), "hello");
        assert!(provider::<dyn OtherContract>("process-wide-greeter").is_none());
    }

    #[test]
    fn two_components_providing_one_contract_coexist() {
        struct Goodbye;
        impl Greeter for Goodbye {
            fn greet(&self) -> &str {
                "goodbye"
            }
        }
        let registry = ContractRegistry::new();
        registry.register::<dyn Greeter>("first", "greeter", Arc::new(Hello));
        registry.register::<dyn Greeter>("second", "greeter", Arc::new(Goodbye));

        let all = registry.lookup_all::<dyn Greeter>("greeter");
        let names: Vec<&str> = all.iter().map(|(component, _)| component.as_str()).collect();
        assert_eq!(names, ["first", "second"]);
        assert_eq!(
            registry.lookup_one::<dyn Greeter>("first", "greeter").unwrap().greet(),
            "hello"
        );
        assert_eq!(
            registry.lookup_one::<dyn Greeter>("second", "greeter").unwrap().greet(),
            "goodbye"
        );

        let error = registry
            .lookup_unique::<dyn Greeter>("greeter")
            .err()
            .expect("ambiguous");
        assert_eq!(error.contract, "greeter");
        assert_eq!(error.components, ["first", "second"]);

        // Re-activating one component replaces only its own slot.
        registry.register::<dyn Greeter>("first", "greeter", Arc::new(Goodbye));
        assert_eq!(
            registry.lookup_one::<dyn Greeter>("first", "greeter").unwrap().greet(),
            "goodbye"
        );
        assert_eq!(registry.lookup_all::<dyn Greeter>("greeter").len(), 2);
    }
}
