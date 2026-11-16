---
status: proposed
date: 2026-11-16
decision-makers: [xberg-io/alef maintainers]
---

# Preserve host callback exceptions in Rust error sources

## Context and Problem Statement

Generated trait bridges currently turn callback exceptions into Rust error
messages. An application calling back into the same host runtime needs the
original exception object, including its type and traceback. A Rust operation
may move between threads, wrap the error, or handle it before another error
reaches the host.

## Considered Options

* Preserve the native exception in the Rust error source chain.
* Keep an exception in call-local or global state alongside a string error.

## Decision Outcome

Chosen option: "Preserve the native exception in the Rust error source chain",
because ownership follows the error through Rust, including across tasks and
through wrappers that expose their underlying error.

### Consequences

* Each backend uses a reference owned by its host runtime and obeys that
  runtime's thread and lifetime rules.
* A consumer opts in through the optional `{source}` placeholder in
  `error_constructor`. It receives an optional boxed, thread-safe Rust error
  source. A consumer can preserve that source alongside its existing message.
* A Rust wrapper on the return path must expose its underlying error through
  `std::error::Error::source` for the binding to recover the exception.
* Runtimes without a suitable retained reference continue to receive a
  message. Backend support can be added as its object lifetime rules allow.

For example, an error enum variant with a `String` message and a
`#[source]` field of type `Option<Box<dyn std::error::Error + Send + Sync>>`
can use this configuration in its `[[crates]]` entry.

```toml
error_constructor = "core_crate::Error::Plugin { message: {msg}, source: {source} }"
```

Rust-side conversion and transport errors supply `None`. This also keeps the same
constructor usable by backends that provide message-only callback errors.

PyO3 uses generated typed error converters to recover the exception from
public Rust results. Node and JNI inspect the configured bridge error and
projected Rust error enums at their native return boundary. Opt-in plugin
lifecycle wrappers also look for a host exception before converting a
Rust-only error. PHP supports lifecycle-only trait bridge bindings. An R
condition would need ownership and rethrow support on the R runtime thread.

### Confirmation

The constructor tests in `src/core/config/resolved/imports.rs` check that
ordinary bridge errors supply no source. Each participating backend tests
the generated callback and return conversions with the same host exception.
