---
status: accepted
date: 2026-11-03
decision-makers:
  - Alef maintainers
---

# Go c-shared component producers

## Context and Problem Statement

[Issue #843](https://github.com/xberg-io/alef/issues/843) requires Go implementations of component contracts consumed through Alef's native ABI. Rust traits already define contract extraction and hashes; duplicating the contract in a Go-specific schema would introduce a second compatibility authority.

## Considered Options

* Generate Go c-shared producers from existing Rust trait contracts.
* Require a handwritten C ABI adapter for every Go producer.
* Support Go exclusively as an executable stdio producer.

## Decision Outcome

Chosen option: "Generate Go c-shared producers from existing Rust trait contracts", because it reuses native hosts, contract hashes, packaging and trust while keeping implementation code in Go.

`producer_language = "go"` explicitly selects generation; omission continues to mean Rust and preserves existing serialized defaults. Each provided contract's `implementation` is a plain Go constructor identifier such as `NewCodec`, rather than an expression or Rust type path. Alef generates the Go/C bridge and contract headers. `go.mod` and `implementation.go` are create-once, user-owned seeds: developers maintain Go dependencies and method bodies there. Cargo `extra_dependencies` are rejected. New module seeds use the shared Go language floor rather than a separately pinned version.

The bridge uses `runtime/cgo.Handle` for implementation identity and C-owned allocation for ABI contexts and returned buffers. Go pointers do not escape into long-lived C memory; result buffers carry C deallocation callbacks. Panics are recovered at the exported boundary and become component failures. Supported methods are synchronous, with scalar, UTF-8 and byte-array parameters and results, unit results and fallible returns. Async and complex wire types are rejected with component, contract and method context.

`alef component build` uses `CGO_ENABLED=1` and `go build -buildmode=c-shared`, mapping native Rust target triples to Go OS/architecture pairs. It requires a native CGO-compatible C compiler and rejects unsupported target and cross-compilation requests. Packaging and loading retain the existing native ABI and signed distribution format.

Only one Go c-shared runtime is supported per host process. Applications needing several Go producers must use executable/stdio isolation rather than load multiple Go shared libraries. This does not imply that the c-shared generator also emits a stdio executable.

### Consequences

* Good, because Go implementations use the same ABI and contract identity as Rust producers.
* Good, because user-owned module and implementation seeds survive regeneration.
* Bad, because CGO adds a native compiler requirement and cross-compilation remains unsupported.
* Bad, because supported wire types and the single Go runtime limit constrain eligible applications.

### Confirmation

`tests/component_go_producer_test.rs` checks Go dispatch, generated bridge and buffer ownership, constructor/lifecycle naming, async rejection, the shared module floor and seed ownership. The native conformance test builds seven real shared libraries: one valid producer and six distinct mutants. A language-neutral runtime driver checks invocation, contract hashes, result bytes, buffer deallocation, error statuses, required methods and ABI descriptors, with an assertion that all seven artifacts were checked. Run with the pinned Go and Rust toolchains and a CGO-compatible C compiler:

```console
mise run test:run -- --test component_go_producer_test
mise run test:run -- --test component_go_producer_test generated_go_producer_passes_language_neutral_native_conformance_and_negative_controls -- --ignored --exact
mise run test:run -- --lib go_component
mise run test:run -- --test e2e_component_dev_loop_test
```

The native driver and mutations live in `tests/support/component_conformance/{main,native}.rs`. Public CLI tests in `tests/e2e_component_dev_loop_test.rs` preserve signed defaults, debug/release selection, target filtering and isolated unsigned output. Existing Rust producer checks remain part of the generator suite.
