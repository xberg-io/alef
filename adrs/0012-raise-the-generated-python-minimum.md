---
status: accepted
date: 2026-11-02
decision-makers: ["Project maintainers"]
---

# Raise the generated Python minimum

## Context and Problem Statement

Generated Python package requirements, stable ABI features, type-checker settings and E2E packages
must advertise the same runtime floor. Main has adopted Python 3.11 across these surfaces; publishing
that change removes Python 3.10 compatibility and needs an explicit migration boundary.

## Considered Options

* Keep the generated Python 3.10 default.
* Raise the generated Python floor consistently to 3.11.
* Add a configurable minimum to each consumer.

## Decision Outcome

Chosen option: "Raise the generated Python floor consistently to 3.11", because it implements the
maintainers' current runtime baseline without adding an unnecessary configuration API.

Generated package metadata requires Python 3.11 or newer, classifiers omit Python 3.10, PyO3 and
maturin request abi3-py311, and type checking and E2E manifests use the same floor. No optional
minimum-version field is introduced. The development interpreter remains a separate toolchain choice.

Publish this compatibility change in Alef 0.117.0. Consumers regenerate owned manifests and bindings,
rebuild wheels and update their runtime matrices to Python 3.11 or newer. Existing Python 3.10
consumers must retain an older qualified Alef release until they can migrate. Canonical owners
regenerate outputs; hand patches to generated manifests are not the migration path.

### Consequences

* Good, because package requirements and the native stable ABI describe the same runtime support.
* Good, because the current baseline does not require a new consumer configuration field.
* Bad, because regenerated products drop Python 3.10 and consumers must rebuild and migrate.

### Confirmation

Five real-producer qualifications check package metadata, Cargo and maturin ABI flags, type checking,
local and registry E2E output, and the registered template using its existing context. Acceptance
requires actual collection and execution, complete emitted-output evidence, current version/schema
checks and verification of all three release packages; test presence alone is not qualification.

## More Information

* [ADR 0000: Record architecture decisions](0000-record-architecture-decisions.md)
* [ADR 0006: A project-agnostic generator](0006-a-project-agnostic-generator.md)
