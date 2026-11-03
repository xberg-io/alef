---
status: proposed
date: 2026-11-03
decision-makers:
  - Alef maintainers
---

# Apply fixture client options through configured JSON factories

## Context and Problem Statement

Issue [#822](https://github.com/xberg-io/alef/issues/822) requires generated client tests to apply fixture
`client_options`, including scalars, string arrays and number arrays. Liter's plain `create_client` accepts five
scalar parameters and cannot receive `blocked_networks`; its relevant bindings already configure
`client_factory_from_json`. Accepting a fixture while silently dropping its options would test different behavior.

## Considered Options

* Use each language's configured JSON factory and validate in Rust.
* Infer typed positional arguments for every host-language factory.
* Keep excluding option-bearing fixtures outside the existing Rust and Python support.

## Decision Outcome

Chosen option: "Use each language's configured JSON factory and validate in Rust", because the existing factory
contract passes all supported option values to the client's typed Rust deserializer without guessing host
signatures. Python retains its legacy plain-factory keyword-argument fallback. A generator may include an
option-bearing fixture only when its emitted client construction applies those options.

Preserve serialized fixture options as JSON text, including number tokens: parsing and re-encoding them through
JavaScript numbers or Go floating-point values can lose `u64` precision. Encode runtime credentials and mock URLs
with native JSON encoders, then combine them with the shared static options prefix before invoking the configured
factory. Static options must not override the runtime credential or mock endpoint.

Use Gleam's `gleam_json` encoder with `>= 3.1.0 and < 4.0.0`, managed through the central Hex dependency constant
and Renovate's widening rule. This avoids handwritten Gleam escaping and an OTP minimum increase solely for JSON
encoding. C's standard library has no JSON encoder; use a small, separately tested helper that preserves UTF-8 and
escapes JSON control characters, without adding a C dependency.

### Consequences

* Good, because the Rust deserializer owns option names, types and validation across bindings.
* Good, because opaque static JSON preserves large integer values and runtime encoders handle dynamic strings.
* Bad, because Gleam gains a dependency and C requires a narrowly scoped encoding helper.
* Bad, because generators without an applicable factory must still refuse the fixture. The `php_ext`, `brew` and
  `homebrew` packaging smoke generators do not execute fixtures and cannot establish client-option coverage.

### Confirmation

Before accepting this proposal, extend and run `tests/e2e_client_options.rs` to check scalar and array transport,
large integer preservation, escaped credentials and mock URLs, Python fallback, and refusal when no applicable
factory exists. Check generated client construction, not merely fixture names or inclusion decisions.

Compile and run representative generated clients against recording JSON factories, including Gleam and C, and
assert the exact decoded options and runtime fields. Exercise the C helper independently with UTF-8, quotes,
backslashes and control characters. Regenerate Liter's affected binding suites and run option-bearing network
fixtures against the mock server; report executed fixture counts separately from packaging smoke checks. These
compiled and native checks are required before acceptance; this proposed record does not claim they have run.
