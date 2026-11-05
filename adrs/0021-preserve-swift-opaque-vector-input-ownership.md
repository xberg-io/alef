---
status: accepted
date: 2026-11-05
decision-makers: ["Alef maintainers"]
---

# Preserve Swift opaque vector input ownership

## Context and Problem Statement

A Swift facade array can contain the same opaque wrapper more than once. Passing each reference directly to swift-bridge's owned vector insertion transfers one native allocation repeatedly, even when the core type supports `Clone` and the caller expects to retain the original wrapper.

## Decision Drivers

* Preserve existing facade signatures and the core type's declared ownership capabilities.
* Give each owned vector insertion one native owner without guessing `Clone` support.
* Keep borrowed vector elements, DTOs, primitives, scalar arguments and constructors unchanged.

## Considered Options

* Continue transferring every opaque wrapper directly.
* Clone declared `Clone` values through a borrow and check transfers of other values.
* Remove owned opaque vectors from the Swift facade.

## Decision Outcome

Chosen option: "Clone declared `Clone` values through a borrow and check transfers of other values", because it supports ordinary Swift aliasing for cloneable inputs while preserving the transfer contract of non-cloneable values.

The policy applies to facade free functions that materialize owned opaque vector elements. Each insertion of a type whose IR declares `Clone` obtains a fresh native wrapper through a borrowed Rust shim; `Copy` values use copying. The original Swift wrapper remains owned and reusable. A non-cloneable value retains its single-transfer semantics, with an ownership precondition checked before insertion. The raw RustBridge vector ABI remains ownership-transferring.

### Consequences

* Good, because repeated cloneable Swift references become distinct native owners and existing function signatures remain stable.
* Good, because non-cloneable values are never cloned implicitly and an already-transferred wrapper is rejected before another facade insertion.
* Bad, because cloning incurs the cost and sharing semantics of the core type's `Clone` implementation for every insertion.
* Bad, because invalid non-cloneable reuse is a precondition failure rather than a recoverable error; adding throwing signatures would change the public API.

### Confirmation

`src/backends/swift/gen_rust_crate/owned_inputs_tests.rs` verifies generated clone shims, guard placement and unchanged borrowed, DTO and primitive conversions. Its native fixture runs only valid inputs: distinct owners, repeated synchronous and asynchronous cloneable inputs, exact values and drop counts, and the ownership-state transition after one non-cloneable transfer. Invalid transfers are not executed.

Run generator controls with `mise run test:run -- --lib owned_inputs_tests`. Qualify the actual generated Rust bridge and Swift facade with `mise run test:run -- --lib owned_vector_inputs_support_repeated_clone_handles_and_checked_moves -- --ignored --nocapture --test-threads=1` using a matching Swift compiler and SDK. The fixture accepts an explicitly selected absolute `SWIFT_EXEC` without changing the process-wide environment allowlist.
