---
status: proposed
date: 2026-11-04
decision-makers:
  - Alef maintainers
---

# Scope borrowed Python visitor context accessors

## Context and Problem Statement

A Python visitor receives an owned binding object while the core visitor receives a borrowed context. Private lazy fields can be lost in the conversion even when the published binding exposes an accessor for them. Cloning the core context can materialize those fields before Python has asked for them.

## Considered Options

* Copy accessor values when constructing each binding object.
* Keep a callback-scoped reader keyed by the binding object's identity.
* Change each consumer to retain an owned lazy attribute source.

## Decision Outcome

Chosen option: "Keep a callback-scoped reader keyed by the binding object's identity", because the existing Python class can read the borrowed core data during synchronous dispatch without materializing it during construction.

Python consumers opt in with `[crates.python.visitor_context_accessors]`. The initial implementation accepts a visible, synchronous, no-argument string-map method on a generated visitor context class. Alef constructs that class from borrowed public fields. The callback retains the Python object while its matching reader is active on the dispatch thread. The getter clones the returned map only when called. A context saved beyond the callback falls back to ordinary binding behavior. Nested callbacks keep their own reader and can still access any live outer context by its identity.

### Consequences

* Good, because visitors that skip the getter skip its map materialization and cloning.
* Good, because the Python context class and its public method name stay the same.
* Bad, because saved callback contexts can only keep the attributes by reading and saving the returned map during dispatch.

### Confirmation

The emitter test in `src/backends/pyo3/trait_bridge/visitor_bridge/tests.rs` checks borrowed construction, deferred getter evaluation, and field name collisions. The tests in `src/codegen/visitor_context_scope_tests.rs` check unused reads, nested dispatch, expired scopes, cross-thread access, and unwinding. The html-to-markdown Python visitor tests verify returned attributes and escaped contexts on an actual wheel.
