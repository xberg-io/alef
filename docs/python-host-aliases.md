# Python host callback aliases

A Rust trait can add a fallible callback while keeping an existing Python protocol.
Declare the host method explicitly; Alef does not infer aliases from names or default bodies.
See [ADR 0019](https://github.com/xberg-io/alef/blob/6ab050d4e911067fb3919371d2d04d61cdd6c8fb/adrs/0019-configure-python-callback-host-aliases-explicitly.md).

```toml
[extensions.python_host_aliases.crates.xberg.traits.TokenizerBackend]
try_count_tokens = "count_tokens"
```

Aliases apply only to the named source crate; unknown crate keys are rejected against
the entire workspace before emission. Other crates keep their own callback contracts.

The configured Rust method must be a defaulted, synchronous `&self` method returning
`Result<T, E>`. Its target must be a required, infallible method on the same trait with
matching argument shapes and return type `T`. Use a qualified trait path as the table
key if its short name is ambiguous. The trait must have an active Python bridge.

The bridge first uses an explicit Python `try_count_tokens` implementation. If that
attribute is absent, it calls `count_tokens` through the fallible conversion path.
Host exceptions and return conversion errors retain their type/message text in the
Rust error; the original Python exception object and traceback are not preserved.
Descriptor failures propagate instead of selecting the fallback. The alias adds no
required Python protocol method.

Use the opt-in table form to keep the companion out of other generated host
bindings. These bridges inherit the existing Rust default for that method.

```toml
[extensions.python_host_aliases.crates.xberg.traits.TokenizerBackend]
try_count_tokens = { fallback = "count_tokens", python_only = true }
```

The opt-in companion remains available to PyO3 as an optional callback. Python
reference pages omit it to match the public protocol. The string form keeps the
original visibility in other bindings. Rust source and Rust reference documentation
retain the defaulted companion in either form.

The built-in extension validates aliases before emitting bindings. Normal generation
and read-only verification use the same configured emitter, and the raw extension
configuration participates in existing generation/cache input hashes. Rust callers
can opt in through `Pyo3Backend::with_host_aliases` using its already-scoped
`[traits.TokenizerBackend]` table. Constructing the existing unit
`Pyo3Backend` retains its behavior and public layout.

The existing `ffi_skip_methods` setting still controls explicit C vtable
exclusions. Keep it for C consumers whose configuration depends on that contract.
