---
status: accepted
date: 2026-04-01
decision-makers: ["Na'aman Hirschfeld"]
---

# Extract the Rust API into an IR with syn

## Context and Problem Statement

alef generates bindings for many languages from one Rust crate. Every backend needs the same facts about that crate's
public API: its types, fields, functions, methods, enums, errors and traits. The first commit, 56033e56c ("feat:
initial skif workspace with 16 crates and full CLI", 2026-04-01), described the tool as one that "Extracts pub API
via syn, generates idiomatic bindings for 10 languages".

## Considered Options

* Parse the crate's source with `syn` into one intermediate representation (`ApiSurface`) that every language
  backend consumes through a shared `Backend` trait

No alternatives were recorded.

## Decision Outcome

Chosen option: "Parse the crate's source with `syn` into one intermediate representation (`ApiSurface`) that every
language backend consumes through a shared `Backend` trait", because one extraction then serves every target
language, and a backend only maps the IR to its language.

The pipeline is `extract → core → codegen → backends::<lang>` (`.ai-rulez/context/architecture.md`):

* `src/extract/` parses the configured source files with `syn`, follows `mod` declarations and `pub use`
  re-exports, and returns an `ApiSurface`.
* `src/core/ir/` defines the IR, and `src/core/backend.rs` the `Backend` trait whose `generate_bindings` takes an
  `ApiSurface`.
* `alef extract` writes the IR as JSON, so the input every backend sees can be inspected on its own.

### Consequences

* Good, because adding a language means adding one backend over an IR that already exists.
* Good, because extraction reads source text and needs no compilation of the consumer crate.
* Bad, because `syn` sees syntax, not resolved types. Macro-generated items, cfg-gated variants and re-exports have
  to be resolved by alef itself, and `#[cfg_attr(alef, alef(skip))]` attributes in the consumer steer what is
  extracted.

### Confirmation

* `Cargo.toml:95` depends on `syn` with the `full`, `parsing` and `visit` features.
* `src/extract/extractor/mod.rs:56` defines `pub fn extract(...) -> Result<ApiSurface>`.
* `src/core/ir/surface.rs:137` defines `ApiSurface`, and `src/core/backend.rs:401` the `Backend` trait, whose
  `generate_bindings` (`:412`) takes `&ApiSurface`.
