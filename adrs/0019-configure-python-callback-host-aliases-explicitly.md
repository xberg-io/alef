---
status: accepted
date: 2026-11-03
decision-makers:
  - Alef maintainers
---

# Configure Python callback host aliases explicitly

## Context and Problem Statement

A Python implementation of an infallible Rust callback can raise an exception, but that Rust signature has no error channel. The current bridge warns and substitutes the return type's default. Adding a defaulted fallible Rust method does not recover the error when its default calls that infallible bridge. This masks failures as valid values, as demonstrated by [Xberg tokenizer issue #2374](https://github.com/xberg-io/xberg/issues/2374).

Alef's IR retains whether a trait method has a default implementation, not its body. Inferring aliases from matching `try_` names and signatures could therefore bypass arbitrary Rust default logic. Adding fields to existing public configuration or IR structs would also break callers that construct them with Rust struct literals.

## Considered Options

* Declare Python host-method aliases explicitly through the existing extension configuration.
* Infer fallible companions from method names and signatures.
* Add callback aliases to public configuration or IR structs.
* Patch generated bridges after generation.

## Decision Outcome

Chosen option: "Declare Python host-method aliases explicitly through the existing extension configuration", because it preserves existing host protocols and public Rust layouts while making error routing a validated generation input.

A stateless built-in extension accepts aliases scoped to the configured source crate and trait:

```toml
[extensions.python_host_aliases.crates.xberg.traits.TokenizerBackend]
try_count_tokens = "count_tokens"
```

The alias source must be a defaulted, fallible Rust method; the target must be a required, infallible method of the same trait. Both must have compatible ordered arguments and the same successful return type. The initial supported shape is synchronous `&self` callbacks with owned results. Unknown or ambiguous crate, trait and method keys, excluded methods and unsupported signatures fail validation before binding output is written. Crate keys are checked against the whole workspace, then each Python backend receives only its source crate's aliases.

The generated bridge first uses an explicit host override named after the fallible Rust method. If that attribute is genuinely absent, it calls the configured legacy host method through the existing fallible conversion path. Lookup and conversion errors propagate, including `AttributeError` raised by an existing descriptor; `inspect.getattr_static` distinguishes that case from absence. Required Python protocol methods remain unchanged. Calls retain copied context variables and cached backend names. Other Rust defaults that call an aliased method must still forward through this bridge.

Private configured-backend state and an additive constructor carry aliases without changing existing public config, IR, unit-backend or bridge-generator layouts. The direct backend constructor accepts the already crate-selected trait table. Normal generation and readonly verification use the same configured emitter. The existing whole-`alef.toml` cache input includes aliases; no mutable global alias state or post-generation rewriting is introduced.

This mapping affects Python callback emission. It does not change other host backends or C vtable generation. A consumer adding a new defaulted Rust callback must separately use existing `ffi_skip_methods` where necessary to retain its external callback layout. Error conversion retains the original exception type and message as text; it does not preserve the original Python exception object or traceback identity.

### Consequences

* Good, because existing Python implementations can serve a fallible Rust hook without adopting a second required host method.
* Good, because configuration validation, cache invalidation and file ownership remain in the normal generation pipeline.
* Good, because public Rust struct-literal compatibility and unrelated backend output are preserved.
* Bad, because consumers must declare the mapping and maintain compatible Rust method signatures explicitly.
* Bad, because infallible callbacks without a configured fallible route retain their existing default-substitution behavior.

### Confirmation

`src/extensions/python_host_aliases.rs` validates alias shapes; its workspace module checks source-crate scope. The PyO3 configured backend and trait-bridge emitter select the host method before rendering. Shared default-delegate tests verify that unrelated Rust defaults still reach alias forwarding.

`tests/python_host_alias_cache.rs` exercises real CLI generation, source-crate isolation and single-crate readonly verification after warming, changing and removing alias configuration. Native fixture probes check legacy-only hosts, explicit fallible overrides, nested default calls, conversion failures and descriptor errors. Qualification must execute the same strict probes with aliases disabled to demonstrate the swallowed-error failure, then with aliases configured to retain the original cause. These checks supplement the ordinary producer release gates.
