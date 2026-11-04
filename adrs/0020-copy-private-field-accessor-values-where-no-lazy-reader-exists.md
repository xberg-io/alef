---
status: proposed
date: 2026-11-04
decision-makers:
  - Alef maintainers
amends: ["0019"]
---

# Copy private-field accessor values where no lazy reader exists

## Context and Problem Statement

A public `&self` accessor over a private core field (html-to-markdown's `NodeContext::attributes()`) returned an
empty value in every mirroring binding: the binding never held the private field, so the core value it rebuilt for
the call had an empty default (alef #865, html-to-markdown #969). ADR 0019 fixes this for Python visitor callbacks
with an opt-in, callback-scoped lazy reader and lists copying the value into the binding object as a rejected
option. That reader exists only for Python, only for a configured visitor context, and only during the callback. Every
other binding, Python without the opt-in, and every binding-to-core round trip outside a callback has no reader and
still returns the empty value.

## Considered Options

* Copy the accessor's value into a binding field wherever no lazy reader serves it, and keep the lazy reader where it
  is configured.
* Use only the lazy reader, and add an equivalent reader to each backend.
* Copy the value everywhere, including where a lazy reader is configured.

## Decision Outcome

Chosen option: "Copy the accessor's value into a binding field wherever no lazy reader serves it, and keep the lazy
reader where it is configured", because it returns the real value in every binding today while keeping 0019's deferred
evaluation where a consumer asked for it.

This amends the rejected copy option of 0019: copying is used where no lazy reader exists. A copied accessor becomes a
binding field read through the accessor in the core-to-binding conversion and passed back through the public
constructor the binding-to-core conversion already calls. It is copied only when that constructor takes a by-value
parameter of the accessor's name and type and the private field is not serde-skipped; otherwise the method keeps its
current behavior and alef warns. A Python visitor context accessor listed in `[crates.python.visitor_context_accessors]`
is not copied: the scoped reader of 0019 serves it, and the binding class has no field of that name.

### Consequences

* Good, because every backend, and Python without the opt-in, returns the real value instead of an empty one.
* Good, because an opted-in Python visitor keeps 0019's behavior: no map is materialized for a callback that does not
  call the getter.
* Bad, because a copied value is materialized on every conversion from the core, even when the host never reads it.
* Bad, because Go and Swift expose the copied value as a field where they previously had a method, and the generated
  constructors gain an optional parameter.
* Bad, because an opted-in Python visitor context used outside a visitor callback, such as one returned from a
  function or passed back into the core, still yields an empty value: it has no copied field to fall back on.

### Confirmation

The tests in `src/codegen/private_accessors_tests.rs` check each backend's copy, the constructor and type checks, and
that an opted-in Python visitor context emits the scoped reader without a copied field while a context without the
opt-in copies the value (`opted_in_python_visitor_context_keeps_the_lazy_reader_without_a_carried_copy`,
`python_visitor_context_without_the_opt_in_carries_the_value`). The exclusion is applied by
`with_private_accessor_fields_except` in `src/codegen/private_accessors.rs` from
`scoped_context::lazy_readers` in `src/backends/pyo3/trait_bridge/scoped_context.rs`.
