---
status: accepted
date: 2026-05-23
decision-makers: ["Na'aman Hirschfeld"]
---

# One root crate for the generator

## Context and Problem Statement

alef started as a Cargo workspace of 16 crates (56033e56c, 2026-04-01) and grew to about 30: a core crate, a
codegen crate, an e2e crate, a CLI crate and one `alef-backend-<lang>` crate per language. Code crossed crate
boundaries as `alef_<crate>::` paths, and each crate carried its own `Cargo.toml`.

## Considered Options

* One root crate `alef`, binary and library, with each former crate as a module under `src/`
* A workspace with one crate per stage and per language backend

## Decision Outcome

Chosen option: "One root crate `alef`, binary and library, with each former crate as a module under `src/`",
because 85f3cacdc (2026-05-23) "merged 30 Cargo.toml files into a single root Cargo.toml" and rewrote the
`alef_<crate>::` paths to `crate::<module>::`. The commit records the mechanics, not a rationale beyond them.

* 85f3cacdc moved each `crates/alef-*/src/` to `src/<module>/`, with backends under `src/backends/<lang>/`, and
  renamed the `alef-cli` package to `alef` with the binary name unchanged.
* ef8e1eee8 (2026-06-13) folded the last separate crate, `alef-core`, into `src/core/`, dropped the workspace table
  and exposed `run_with_extensions` from the library.
* The workspace that exists today holds only `crates/alef-component-abi` and `crates/alef-component-runtime`, added
  in 5ed953732 (2026-10-10). They are runtime support that generated bindings depend on, not part of the generator.

### Consequences

* Good, because the generator is one package with one version to publish and install.
* Good, because a change that touches the IR and several backends is one crate's change, with no cross-crate
  re-exports to maintain.
* Bad, because the whole generator compiles as one unit, so a change to any backend rebuilds the crate.

### Confirmation

* `Cargo.toml` declares one `[package]` named `alef` with `[lib] path = "src/lib.rs"` and
  `[[bin]] name = "alef"`, and `[workspace] members` lists only the two component crates (`Cargo.toml:51`).
* `.ai-rulez/context/architecture.md` describes the pipeline as one root-flat crate with modules under `src/`.
