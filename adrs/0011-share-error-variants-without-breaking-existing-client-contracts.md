---
status: proposed
date: 2026-11-01
decision-makers: ["Project maintainers"]
---

# Share error variants without breaking existing client contracts

## Context and Problem Statement

Different Rust error families can expose the same variant class name with different payloads and
introspection methods. Emitting one class must preserve membership in each family, while keeping
existing client constructors, catches, names and the public Rust generator helpers usable.
Commit `56d61d484db9c81c061db5c20871dcea89477cf3` implements shared classes but reserves new
marker names and adds overloads and helper arguments that can break existing clients (issue #771).

## Considered Options

* Prefix every shared variant with its family and require client migration.
* Keep unqualified shared classes and add membership and payload metadata compatibly.
* Reject every shared variant or user type whose name resembles an added marker.

## Decision Outcome

Chosen option: "Keep unqualified shared classes and add membership and payload metadata compatibly",
because existing clients already construct and catch the original unqualified classes.

A shared class keeps its first family's superclass and the union of payload and introspection fields.
Python uses multiple bases; Ruby uses family modules and family-class case matching. Java, C# and PHP
use marker interfaces for additional family membership. These mechanisms do not promise multiple
class inheritance in Ruby, Java, C# or PHP. Existing first-family catches and constructors remain valid.

Added marker names yield to backend-projected user types, trait interfaces and bridge companions,
enums and configured exception classes. PHP also reserves the Rust companions created by
`#[php_interface]`, separately from PHP's case-insensitive class namespace. Allocation is
deterministic for an API and target language. Source-derived potential trait companions are reserved
even when their configuration is inactive, so enabling a bridge later does not change a marker name.
This conservative reservation only suffixes newly added markers; it never rejects or renames a
user-owned symbol. Preferred names stay unchanged when unreserved, otherwise
`Family` and numbered suffixes avoid occupied names. PHP allocation respects case-insensitive names.
Java's added cross-family introspection values use a named factory and a distinct internal constructor;
existing public constructor signatures retain overload resolution, including an untyped null cause.
Public Rust helpers keep their original signatures, with additive named all-error variants for callers
that supply whole-API context. Core serialized IR fields remain unchanged.

### Consequences

* Good, because shared variants retain membership and the union of values without renaming user types.
* Good, because old Rust calls and generated Java constructor calls continue to compile.
* Bad, because marker names can differ across API models when user-owned or potential companion names
  occupy preferred names, including an inactive companion.
* Bad, because each target language requires native qualification of its distinct membership mechanism.

### Confirmation

`tests/error_generator_legacy_api.rs` compiles the original public Rust calls.
`tests/error_family_compatibility.rs` checks unique and shared marker-name collisions and compiles/runs
old Java null-cause and payload-constructor clients. `tests/error_class_family_collision.rs`,
`tests/error_class_family_introspection.rs` and `tests/error_class_family_lineage.rs` cover membership,
union payloads and inherited introspection. Acceptance additionally requires recorded native
Python, Ruby, PHP, Java and C# execution with real collection counts; generated-string tests or
optional early returns alone do not qualify a target language.
