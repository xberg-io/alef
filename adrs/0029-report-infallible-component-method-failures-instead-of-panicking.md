---
status: proposed
date: 2026-11-15
decision-makers:
  - Alef maintainers
---

# Report infallible component method failures instead of panicking

## Context and Problem Statement

`alef` generates a `<Contract>Proxy` that implements a contract trait by calling a native component's C function table. Every C method returns an `AlefStatus`, but a trait method may be infallible in Rust (`fn name(&self) -> String`). The proxy panicked when such a method returned a non-OK status, and it looked each function up with `.expect(...)` on every call, panicking on a null slot. A component that is buggy or built against a different table therefore aborts the host process (or unwinds through whatever the host was doing) instead of failing one call.

## Decision Drivers

* A misbehaving component must not take the host down.
* The contract trait is the consumer's API; the proxy cannot change an infallible signature.
* Failures must stay visible: silently returning a value would hide broken components.
* A broken table should fail activation, where the host already handles errors, not the first call.

## Considered Options

* Validate slots in `Proxy::new`; on an infallible failure, report it through a process-wide hook and return the result type's `Default`.
* Keep panicking, and document that infallible methods abort on component failure.
* Refuse to generate a proxy for an infallible method, forcing every contract method to be fallible.

## Decision Outcome

Chosen option: "Validate slots in `Proxy::new`; report infallible failures and return `Default`", because it keeps every existing contract signature, turns the null-slot case into an activation error, and leaves the host in control of how loud a failure is.

`Proxy::new` returns `Result<Self, alef_component_runtime::ComponentError>`. It reads the contract table, fails with `ComponentError::MissingContractMethod(name)` for the first null slot before any instance is created, and stores the function pointers, so calls no longer touch `Option`s. When an infallible method's call returns a non-OK status, the proxy takes the error message, calls `alef_component_runtime::report_infallible_failure` with an `InfallibleCallFailure` (component id, method, status, message), and returns `Default::default()` for the result type (an empty string or byte vector, zero, `false`, `()`). The runtime writes the failure to stderr unless the host installed a handler with `set_infallible_failure_hook`; a host that prefers to abort can panic from its hook. Fallible methods keep returning the contract's error type.

### Consequences

* Good, because a failing component degrades one call's result instead of crashing the host.
* Good, because a table with missing methods is reported by activation, with an error code.
* Bad, because a caller of an infallible method cannot tell a default value from a real one without a hook; contracts whose results matter should declare the method fallible.
* Bad, because `Proxy::new`'s error type changed from `String` to `ComponentError`, which breaks code that calls it directly.

### Confirmation

`src/codegen/component_proxy.rs` tests assert the generated proxy contains no `panic!` or `.expect(`, checks every slot before instantiating, routes infallible failures through the hook, and documents every `unsafe` block. `crates/alef-component-runtime/src/proxy_support.rs` tests the hook.

## Pros and Cons of the Options

### Report and return `Default`

* Good, because no signature changes are needed.
* Bad, because the fallback value is indistinguishable from a real result without the hook.

### Keep panicking

* Good, because failures cannot go unnoticed.
* Bad, because one component can abort the host.

### Require every method to be fallible

* Good, because the error reaches the caller through the type system.
* Bad, because it rejects existing contracts and forces `Result` on methods that cannot fail in-process.

## More Information

* [ADR 0015: ABI and runtime stability policy for component crates](0015-abi-and-runtime-stability-policy-for-component-crates.md)
