---
status: accepted
date: 2026-08-13
decision-makers: ["Na'aman Hirschfeld"]
---

# Generational handles across the C ABI

## Context and Problem Statement

The generated C FFI crate passed each opaque Rust value to the host as a raw pointer (`*mut Type`), which the host
held, passed back into every call and freed with a generated destructor. A raw pointer carries no type and no
lifetime, so the FFI layer could not tell a valid pointer from a freed one or from a pointer to another type. The Go
backend already kept its own handle registry for trait-bridge lifetimes (28c1095c0, 2026-05-30).

## Considered Options

* Scalar, generational `AlefHandle` tokens resolved through a typed registry in the FFI crate
* Raw opaque pointers to the Rust value

## Decision Outcome

Chosen option: "Scalar, generational `AlefHandle` tokens resolved through a typed registry in the FFI crate", because
that is what 91cb7d321 and the 0.61.0 release adopted; no commit or CHANGELOG entry records a rationale. The generated
registry's `HandleError` cases show what it rejects: a zero handle, an unknown slot, a stale generation, the wrong type,
a handle in use, and the same handle passed twice.

* 91cb7d321 (2026-08-13) added the typed generational handle registry, and 298656162 (2026-08-13) moved Go onto it.
* The 0.61.0 release (2026-08-18) records it as breaking: "ordinary opaque values now cross the C ABI as scalar,
  generational `AlefHandle` tokens with zero as the invalid sentinel", and every host binding and C consumer must be
  regenerated.
* A handle is a `u64`: the low 32 bits hold the slot index plus one and the high 32 bits the slot's generation. The
  generation advances when the value is removed, so a handle kept after its value is freed resolves to
  `StaleGeneration`.

### Consequences

* Good, because use after free, double free and type confusion across the C ABI become errors instead of undefined
  behavior.
* Bad, because every call resolves its handle through a locked registry.
* Bad, because the change broke every pointer-shaped C consumer and required regenerating all host bindings.

### Confirmation

* `src/backends/ffi/templates/handle_registry.rs.jinja:1` defines `type AlefHandle = u64` and the `HandleError`
  cases.
* `src/backends/ffi/gen_bindings/tests/handle_registry.rs:5` asserts the generated registry uses typed generational
  tokens and rejects a stale generation.
* `src/backends/ffi/gen_bindings/tests/constructors.rs:410` asserts that an opaque constructor returns a
  generational handle.
* `CHANGELOG.md:12937` records the breaking change.
