---
status: accepted
date: 2026-04-01
decision-makers: ["Na'aman Hirschfeld"]
---

# Structured values cross the C FFI as serde JSON

## Context and Problem Statement

Go, Java (Panama), C#, Zig and Dart in `ffi` style bind a library through the generated C FFI crate. A C ABI carries
scalars, pointers and byte buffers, not Rust structs, enums or collections, so every structured value the core accepts
or returns needs a representation that both sides can build and read. The first FFI backend in 56033e56c (2026-04-01)
generated a `<prefix>_<type>_from_json` function per struct: "Create a `{type_name}` from a JSON string. Returns null on
failure."

## Considered Options

* Serialize structured values to and from JSON strings with `serde_json` at the C ABI, using the core types' serde
  derives

No alternatives were recorded.

## Decision Outcome

Chosen option: "Serialize structured values to and from JSON strings with `serde_json` at the C ABI, using the core
types' serde derives", because a type that derives serde already has a wire shape, so the FFI layer needs no
per-type C struct layout.

* For each structured type the FFI backend emits `<prefix>_<type>_from_json` and `_to_json`; enums get the same
  pair, and the host binding passes the JSON string across the ABI.
* Trait bridges use the same route in the other direction: a host callback returns a JSON string, and the generated
  bridge parses it with `serde_json::from_str`. When the string does not parse, the bridge logs a warning and returns
  `Default::default()`.
* Since 2026-08-13 the Rust value lives behind a generational handle rather than a raw pointer (ADR 0007), but values
  still enter and leave it as JSON.

### Consequences

* Good, because any type with serde derives crosses the C ABI without hand-written marshaling code.
* Good, because the JSON names follow `serde_rename` and `serde_rename_all`, so every C-ABI language sees the same
  wire names as the core's own serialization.
* Bad, because every structured call pays for serialization and parsing on both sides.
* Bad, because a trait method's return type must be `Default + Serialize + Deserialize`, and a failing host
  callback is reported only as a warning and a default value.

### Confirmation

* `src/backends/ffi/templates/type_from_json.jinja:9` declares the generated `<prefix>_<type>_from_json`, and
  `src/backends/ffi/templates/enum_from_json.jinja:24` parses with `serde_json::from_str`.
* `src/backends/ffi/templates/ffi_serde_from_str_default.jinja:1` holds the trait-bridge return parse with its
  `Default::default()` fallback, rendered from `src/backends/ffi/trait_bridge/call_body.rs:472`.
