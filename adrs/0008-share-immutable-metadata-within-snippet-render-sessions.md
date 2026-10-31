---
status: accepted
date: 2026-10-31
decision-makers: ["Na'aman Hirschfeld"]
---

# Share immutable metadata within snippet render sessions

## Context and Problem Statement

Documentation verification renders many fixture/language cells from the same immutable crate IR. Each cell
reconstructed crate-wide field, enum and collection metadata; cloning the resolver and changing an iteration root
also copied or rebuilt these maps. Issue #753 records this construction path; it does not establish a measured
end-to-end speedup for a particular cache design.

## Decision Drivers

* Share the underlying facts across cells, optional-path enrichment and iteration roots.
* Preserve per-call aliases, optionality, PHP getters, Swift accessors and snippet bytes.
* Keep custom `E2eCodegen` implementations and existing one-shot entry points compatible.
* Bound metadata lifetime to one immutable model and render run.

## Considered Options

* Pass an explicit per-report session holding shared facts and local root cursors.
* Keep owned metadata maps and cache their clones for each cell.
* Use a global or thread-local cache keyed by a model address or fingerprint.

## Decision Outcome

Chosen option: "Pass an explicit per-report session holding shared facts and local root cursors", because the
report driver already owns the lifetime of its borrowed IR and all fixture/language cells. `SnippetRenderSession`
borrows that model, builds common metadata lazily, and separates shared facts from each resolver's root and optional
path overlay. Result-field facts are cached by the binding's optionality rule; declared call facts use the semantic
core lookup name. Caller configuration and presentation operations are not cached by root identity.

The additive `E2eCodegen::render_snippet_body_in_session` method defaults to the existing functions-aware method.
Built-in renderers opt into session reuse; custom implementations retain their own rendering. Existing owned map
builders and one-shot presentation APIs remain available. A new model or render run requires a new session, with no
global cache, pointer-identity key or whole-model serialization on the request path.

### Consequences

* Good, because changing a call or iteration root can reuse the same immutable map storage.
* Good, because aliases, caller-supplied getter maps and per-fixture operations remain local.
* Good, because dropping the report session releases its metadata without cache invalidation rules.
* Bad, because the render boundary and built-in snippet adapters must carry an explicit session.
* Bad, because one-shot callers still construct their own short-lived session, and unrelated backend rendering work
  remains outside this cache.

### Confirmation

`src/e2e/codegen/presentation/metadata_cache_tests.rs` counts events at the actual fact constructors and checks
storage identity across repeated requests and changed element roots.
`src/e2e/field_access/metadata_storage_tests.rs` checks that optional enrichment keeps shared facts and isolated
overlays. `src/e2e/codegen/presentation/session_render_tests.rs` exercises the real report's fixture/language census
and the default functions-aware forwarding path for a borrowed custom implementation.

Independent review, model-isolation and alias/getter controls, and exact operation and template-byte comparisons
passed for supported built-in renderers.

## Pros and Cons of the Options

### Pass an explicit per-report session holding shared facts and local root cursors

* Good, because borrowed IR and report ownership make the cache boundary explicit.
* Bad, because built-in renderer adapters need an additional forwarding path.

### Keep owned metadata maps and cache their clones for each cell

* Good, because existing owned-map consumers need fewer internal changes.
* Bad, because deep copies retain work proportional to the crate size for every cell or root.

### Use a global or thread-local cache keyed by a model address or fingerprint

* Good, because callers would not need an explicit session parameter.
* Bad, because addresses can be reused and immutable models/configuration can differ between runs.
* Bad, because fingerprinting a whole model repeatedly adds work and still requires lifetime/invalidation policy.

## More Information

* [Alef issue #753](https://github.com/xberg-io/alef/issues/753)
