# Python host callback aliases

A Rust trait can add a fallible callback while keeping an existing Python protocol.
Declare the host method explicitly; Alef does not infer aliases from names or default bodies.

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
required Python protocol method and changes no other language's callback behavior.

The built-in extension validates aliases before emitting bindings. Normal generation
and read-only verification use the same configured emitter, and the raw extension
configuration participates in existing generation/cache input hashes. Rust callers
can opt in through `Pyo3Backend::with_host_aliases` using its already-scoped
`[traits.TokenizerBackend]` table. Constructing the existing unit
`Pyo3Backend` retains its behavior and public layout.

An added Rust trait method can still affect C callback layouts. Configure the existing
`ffi_skip_methods` for a method that should inherit its Rust default across C FFI;
Python host aliases do not silently change or repair that ABI contract.
