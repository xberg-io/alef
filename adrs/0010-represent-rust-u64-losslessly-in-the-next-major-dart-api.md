---
status: proposed
date: 2026-10-31
decision-makers: ["Project maintainers"]
---

# Represent Rust u64 losslessly in the next major Dart API

## Context and Problem Statement

The FRB backend mapped Rust `u64` through signed `i64` and exposed Dart `int`, losing the full unsigned domain.
PR #766 preserves `0` through `18446744073709551615`, but changes generated public Dart types and callers.
This proposal defines that next-major API contract; it does not authorize inclusion in maintenance release
0.115.11, select a new version number, or change an existing published package.

ADR 0002 governs structured values crossing the C FFI as JSON. This decision concerns the separate FRB Dart
surface and does not replace that JSON contract.

## Decision Drivers

* Preserve every Rust `u64` value without signed wrapping or floating-point rounding.
* Use the same public numeric meaning for scalars, DTOs, containers, callbacks and streams.
* Reject invalid arguments before native entry and transport callback serializer failures without hanging.
* Make the breaking migration and unqualified platform/SDK boundaries explicit.

## Considered Options

* Use BigInt scalars and FRB generalized Uint64List in the next major Dart API.
* Retain int and restrict the supported unsigned range.
* Expose unsigned values as decimal strings.

## Decision Outcome

Chosen option: "Use BigInt scalars and FRB generalized Uint64List in the next major Dart API", because it preserves
Rust's entire unsigned domain while retaining typed integer operations and FRB's unsigned codec. This remains a
proposal until the confirmation requirements below are met and maintainers accept the breaking-release plan.

The contract is specific to `u64`. Narrower integers, signed `i64`, `usize`/`isize` and `Vec<u8>` retain their existing
mappings; this is not a claim that every Rust integer is lossless on every Dart target. Rust core signatures remain
unchanged. Generated Rust facades, Dart declarations and FRB codecs must be regenerated and distributed together.

### Migration contract

| Rust shape | Previous Dart surface | Proposed Dart surface | Caller migration |
|------------|-----------------------|-----------------------|------------------|
| `u64` | `int` | `BigInt` | Use exact BigInt values for parameters, results and DTO fields. |
| `Option<u64>` | `int?` | `BigInt?` | Preserve null; convert only present values. |
| `Vec<u64>` | `Int64List` | FRB generalized `Uint64List` | Rebuild lists with the FRB BigInt-aware carrier. |
| Nested containers, enums and newtypes containing `u64` | Nested int carriers | Corresponding BigInt carriers | Update nested fields, map keys/values and wrapper construction. |
| Callbacks and streams carrying `u64` | int parameters/results/events | BigInt parameters/results/events | Update implementations, listeners and typed collections together. |

For exact constants, use `BigInt.parse('18446744073709551615')`; use `BigInt.zero`/`BigInt.one` for small identities.
Do not round-trip a wide value through `double`, or use `.toInt()` merely to satisfy an old signature. An application
that intentionally narrows a value must establish its range and the target platform's representability first.

The `Uint64List` here is FRB's generalized collection, not a drop-in substitution of `dart:typed_data.Uint64List`.
The emitted callers use the `flutter_rust_bridge_for_generated.dart` support library's `Uint64List.fromList` with
BigInt elements. A consumer release must document that import or provide an intentional public re-export. Values
already truncated by construction of a typed list cannot be recovered or rejected afterward.

Scalar and recursive DTO argument guards and the unsigned SSE writer reject negative or oversized BigInt values.
Callback serialization errors use the existing FRB error transport; infallible native trait callbacks retain their
existing warning/default-value fallback. This does not turn an infallible callback into a fallible API or justify
uncaught asynchronous serializer errors.

String-taking Rust JSON constructor helpers retain numeric JSON value fields and their existing object-key rules.
`BigInt` is not directly encodable by Dart's default JSON encoder; converting it to a JSON string changes the schema.
Applications using those helpers need an exact numeric JSON strategy, rather than automatic `.toInt()` conversion
or decimal-string substitution.

Adoption requires an explicitly designated breaking generator release after 0.115.11. Each consumer adopting the
changed public Dart API needs its own breaking-release/migration assessment, regenerated declarations and codecs,
and updated handwritten callers. Existing published tags and SDKs remain immutable. No product version is changed
by this ADR.

### Consequences

* Good, because the public representation can retain the entire unsigned 64-bit range.
* Good, because invalid unsigned values can fail before native core calls or complete callback error transport.
* Bad, because existing int callers, DTO constructors, typed lists, callbacks and stream consumers stop compiling.
* Bad, because the generalized list exposes an FRB support-library dependency that consumers must manage.
* Bad, because JSON integration requires an explicit exact-number strategy, and runtime/platform proof costs more
  than checking emitted source strings.

### Confirmation

Existing PR source supplies scalar/container tests in `tests/backends_dart_u64_bigint.rs`, recursive and newtype
cases in `tests/backends_dart_newtype_functions.rs` and `tests/backends_dart_nested_map_keys.rs`, and source-bound
caller checks in `src/e2e/codegen/dart/u64_callers_tests.rs`. These references describe available checks, not an
independent claim that this proposed contract has passed them.

Before acceptance, independently run the actual ignored native probes in `tests/backends_dart_u64_bigint/native.rs`,
`tests/backends_dart_u64_canonical_native.rs` and `src/e2e/codegen/dart/u64_callers_native.rs`. Require their nonzero
case/receipt inventories, full-width boundary round trips, signed/narrow controls, typed-argument failures before
native core entry, JSON constructor failures at the Rust deserialization boundary, callback completion/error checks,
and the poisoned integer caller's intended compilation failure.
Do not substitute skipped tests, analyzer-only output or an author-reported native run for these results.

The verification pin is Dart 3.13.7 with FRB 2.13.0; it is not a new minimum-SDK policy. The default generated SDK
constraint at this PR is `>=3.13.0 <4.0.0`. Qualification at that minimum, and any lower minimum explicitly retained
by a consumer, remains required or needs a separately reviewed support-policy change. Author-reported native
macOS checks do not establish Windows, Linux, mobile or browser codec support. Each supported target needs direct
qualification; browser BigInt/list/callback behavior is unproved here, not implicitly retired or certified.

Shared extractor changes to reference placement and newtype container metadata also affect non-Dart backends.
Run the full extraction/backend regression gates and representative non-Dart generation comparisons, or separate
those changes for their own qualification. No new build, test or cross-platform proof is asserted by this ADR.

## Pros and Cons of the Options

### Use BigInt scalars and FRB generalized Uint64List in the next major Dart API

* Good, because exact integer arithmetic and unsigned FRB carriers preserve Rust's declared domain.
* Bad, because a coordinated breaking migration is required and the carrier depends on FRB's generated support API.

### Retain int and restrict the supported unsigned range

* Good, because many existing callers would retain their source types.
* Bad, because the restriction would change Rust's exposed value contract and require platform-sensitive limits.

### Expose unsigned values as decimal strings

* Good, because decimal text can preserve the full range across runtimes.
* Bad, because every arithmetic operation and native call needs parsing and validation, and string fields would
  blur the distinction between a numeric API and a separately specified JSON/text transport.

## More Information

* [Alef PR #766](https://github.com/xberg-io/alef/pull/766)
* [Dart BigInt](https://api.dart.dev/dart-core/BigInt-class.html)
* [Dart JSON encoding](https://api.dart.dev/dart-convert/jsonEncode.html)
* Repository ADR 0000 governs this proposed record; ADR 0002 remains unchanged.
