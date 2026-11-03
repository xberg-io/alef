---
status: accepted
date: 2026-11-03
decision-makers: ["Project maintainers"]
amends: ["0012"]
---

# Configure Python runtime minimums through managed versions

## Context and Problem Statement

A consumer needs Python 3.11 without coupling its migration to unrelated changes in every binding.
The newer generator validates custom Swift codecs more strictly, and existing consumers may need
separate compatibility work for eleven custom-codec owners before full regeneration succeeds.
Silently changing the 0.115 maintenance default or adding a public Rust configuration field would
create another compatibility break. Main already defaults to Python 3.11 and must retain that floor.

## Considered Options

* Require a full newest-generator migration before changing the Python floor.
* Add a public Python configuration field.
* Configure the runtime through the existing managed dependency-version registry.

## Decision Outcome

Chosen option: "Configure the runtime through the existing managed dependency-version registry",
because a supported runtime is a versioned dependency and the existing map preserves exhaustive
Rust configuration literals and public helper signatures.

Consumers select a supported major/minor minimum through the existing configuration shape:

```toml
[crates.python.dependency_versions]
"runtime:python" = "3.11"
```

One private validated policy supplies package requirements, classifiers, type-checker versions,
PyO3/maturin stable-ABI flags, local and registry E2E manifests, and the registered E2E template.
Patch versions, raw requirement fragments, unsupported minors and values below the release line's
default fail configuration validation. An omitted Python object, null object or empty configuration
retains the registered template's default behavior.

The 0.115 maintenance line preserves its complete default 3.10 output and may opt in to 3.11.
Main retains its 3.11 default and rejects 3.10. This amends only ADR 0012's conclusion that an
override is unnecessary; it does not lower Main's floor or introduce a new public field or enum variant.
The separately planned next-major Dart API migration remains independent.

Maintenance releases still require source-bound release verification and must explicitly opt out
of GitHub Latest through immutable tagged metadata. Existing versions and tags are never reused.

### Consequences

* Good, because consumers can migrate Python independently while retaining their other binding contracts.
* Good, because existing public configuration literals and helper call sites continue to compile.
* Bad, because this managed-version key needs semantic validation and each release line must qualify
  its own omitted-output behavior.

### Confirmation

Producer tests in `src/scaffold/languages/python/minimum_version_tests.rs` and
`src/e2e/codegen/python/minimum_version_tests.rs` exercise actual output and the registered helper.
Validation tests reject unsupported minima, and `tests/python_runtime_minimum_compatibility.rs`
compiles the original exhaustive Rust literal and unchanged public helper calls. Complete omitted
output is compared with the qualified maintenance baseline. Consumer acceptance additionally
requires a real 3.11 wheel, metadata and ABI checks, import, extraction and lock resolution.

## More Information

* [ADR 0006: A project-agnostic generator](0006-a-project-agnostic-generator.md)
* [ADR 0012: Raise the generated Python minimum](0012-raise-the-generated-python-minimum.md)
