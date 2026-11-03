---
status: accepted
date: 2026-11-03
decision-makers: ["Project maintainers"]
---

# ABI and runtime stability policy for component crates

## Context and Problem Statement

Alef's component system lets a Rust library ship selected trait implementations as separately built,
downloadable dynamic libraries. Three artifacts make it up:

* `alef-component-abi` (`no_std`): the `repr(C)` types a host and a component exchange
  (`AlefHostApiV1`, `AlefComponentV1`, `AlefTaskV1`, `AlefStr`, `AlefOwnedBuffer`), the entry symbol
  `alef_component_entry_v1`, and `ABI_MAJOR_V1` / `ABI_MINOR_V1`.
* `alef-component-runtime`: the manifest, lock and cache types (`ComponentManifest`, `ComponentLock`,
  `COMPONENT_MANIFEST_SCHEMA`, `COMPONENT_ABI_VERSION`), download and verification (SHA-256, ed25519
  manifest signature, verified stamps) and the dynamic loader.
* The `alef` CLI (`alef component build|package|lock`), which derives a contract IR from a Rust trait
  (`ComponentContractIr` in `src/codegen/component.rs`), hashes it with BLAKE3 and generates the
  producer crate.

All three version in lockstep with alef (0.117.x; nine minor releases in three weeks). Nothing records
what a consumer may rely on across those releases, and the recent #826 change (the provider registry
keyed by (component, contract), `register_provider` taking the component name) shows that the Rust API
of these crates moves. The first external consumer, poly, embeds the runtime in a released binary and
loads components built by a separately released toolchain, so a rule is needed for which skews are
safe. No ADR describes the component system itself either.

## Decision Drivers

* A host and a component are built at different times by different toolchains; a layout mismatch is
  undefined behavior, not an error.
* The compatibility check must be mechanical (an integer or a hash comparison at load time), not
  dependent on the crate version string.
* Consumers must be able to upgrade the Rust API of the runtime deliberately, without it changing the
  set of components they can load.

## Considered Options

* Treat the crate version as the compatibility signal and use caret ranges everywhere.
* Version three independent surfaces (binary ABI, on-disk formats, Rust API) with separate rules and
  pin consumers exactly.
* Freeze everything until a 1.0 and forbid changes.

## Decision Outcome

Chosen option: "Version three independent surfaces with separate rules and pin consumers exactly",
because the load-time checks already compare integers and hashes, and the crate version only describes
the Rust API.

### Architecture

A component is a cdylib exporting `alef_component_entry_v1(requested_abi_major, host, out_component)`.
It returns a pointer to a static `AlefComponentV1` carrying `abi_major`, `abi_minor`, the component id
and version, the `contract_hash` and `feature_set_hash`, a typed function table (`contract`,
`contract_size`) and `create` / `destroy`. The host passes an `AlefHostApiV1`. Distribution is a
`component.json` manifest, the library, and an ed25519 `component.sig`; a `ComponentLock` lists
artifacts by identity (crate, component, version, target, feature hash, contract hash) with URL,
SHA-256, size and manifest SHA-256. The runtime installs into a content-addressed cache under
`sha256/<digest>` and refuses anything that does not match the lock.

### ABI crate (`alef-component-abi`)

* The binary contract is identified by `ABI_MAJOR_V1` and the entry symbol suffix `_v1`, not by the
  crate version. The entry symbol and every `...V1` struct are frozen at the `repr(C)` layout in
  force at 0.117.0.
* Every ABI struct starts with `struct_size`. Within a major, structs evolve append-only: a new field
  goes at the end, `struct_size` grows, and a reader treats fields beyond the size it received as
  absent. Fields are never reordered, retyped or removed, and `AlefStatus` stays an open integer (new
  codes allowed, unknown codes treated as failure).
* An append-only change bumps `ABI_MINOR_V1`. The loader accepts a component whose `abi_major` equals
  the host's and whose `abi_minor` is not greater than the host's, so a newer host loads older
  components and an older host refuses newer ones.
* Anything else is a layout break: a new major, new entry symbol (`alef_component_entry_v2`) and new
  structs (`AlefHostApiV2`, `AlefComponentV2`). `V1` types are not edited in place for a break. A host
  that supports both majors tries the highest entry symbol first. A new major also bumps
  `COMPONENT_ABI_VERSION` and the crate's semver major.
* Rust-level conveniences in the crate (`HostApi`, `FromHost`, `ContractRegistry`, `register_provider`)
  are not part of the binary contract. They follow the Rust API rule below.

### Contract hash

`contract_hash` is BLAKE3 over `serde_json::to_vec` of `ComponentContractIr`. It is a promise that two
sides agree on the typed function table. It changes when any of these change: the contract name,
`interface_version`, the resolved trait path, a method's name, order, `is_async`, parameter names,
order, types and `borrowed` flags, result type, `fallible` or `error_type`, a referenced record's
fields, or a referenced enum's variant names or order. It does not change with doc comments,
implementation code, component features (resolution rejects a feature set that changes a contract's
hash) or the alef version.

The hash must stay stable across alef releases for an unchanged contract. Consequently:

* Changing the IR serialization (adding, renaming or reordering a `ComponentContractIr` field, a
  `WireType` variant tag, or the serde representation) changes every hash and is a breaking change: it
  needs the "Changed (BREAKING)" changelog heading, a bump of `interface_version` semantics documented
  in the release notes, and a golden-hash test update in the same commit.
* Adding a `WireType` variant does not change existing hashes, so it is allowed in a minor release.
* A contract author bumps `interface_version` for intentional semantic changes that leave the
  signature unchanged.

### Manifest and lock formats (`alef-component-runtime`)

* `COMPONENT_MANIFEST_SCHEMA` (currently 2) versions the manifest, the record and the lock; the lock
  carries it as `schema_version`. `COMPONENT_ABI_VERSION` (currently 1) is the manifest's `abi_version`
  and tracks the ABI major.
* The structs use `deny_unknown_fields`, so adding a field is a schema bump, not a compatible
  extension. Today the runtime accepts exactly the current schema and ABI version and reports
  `UnsupportedLockSchema` otherwise.
* Policy for future bumps: the runtime of release N reads the schema of release N and, for one minor
  release series after a bump, the previous schema (a read-only upgrade path: parse the old form, never
  write it). Producers and `alef component lock` always write the current schema. Dropping a schema
  version is a breaking change of the runtime. Because locks are reviewed and committed in consumer
  repositories, a schema bump ships with a changelog entry that says whether `alef component lock`
  must be rerun.
* The cache layout (`sha256/<digest>`, `<digest>.verified` stamp, `<digest>.lock`) is private to the
  runtime. A layout or stamp-format change must invalidate rather than misread old entries: an
  unreadable or non-matching stamp falls back to full re-verification, and the cache can always be
  discarded and refetched. Verification inputs (lock hashes, manifest signature, `ed25519` as the only
  algorithm) are never weakened by a minor release.

### Versioning and pinning

* The three crates and the CLI keep one workspace version (lockstep). The version numbers describe the
  Rust API and the CLI surface; binary and on-disk compatibility is defined only by the ABI major, the
  manifest schema and the contract hash above. A lockstep bump therefore does not by itself make older
  components unloadable.
* While the version is `0.x`, a minor bump may break the Rust API of `alef-component-runtime` and the
  non-`repr(C)` parts of `alef-component-abi` (as #826 did); a patch bump may not. The CLI's flags and
  config keys follow the same rule. After 1.0 these become ordinary semver.
* Consumers that embed the runtime or implement a host pin both `alef-component-runtime` and
  `alef-component-abi` with exact `=` requirements, upgrade them together, and rebuild their
  components with the matching `alef` CLI minor. Producer crates generated by `alef` currently depend
  on `alef-component-abi = "<alef version>"` (`src/codegen/component_producer.rs`), a caret
  requirement; it is tightened to `=<alef version>` so a producer cannot silently resolve a newer
  patch of the ABI crate than the CLI that generated it.
* MSRV: the crates declare `rust-version = "1.99"`, the same as the rest of alef. An MSRV raise is a
  minor release in `0.x`, is announced under "Changed" in the changelog, and is never made in a patch
  release. `alef-component-abi` stays `no_std` and keeps its dependency set minimal, because it is
  linked into every component.

### Consequences

* Good, because load-time compatibility is decided by values that can be asserted in tests, not by
  reading version strings.
* Good, because poly can take runtime API changes on its own schedule while its already-published
  locks keep working within the schema window.
* Good, because append-only structs and a separate V2 for breaks leave existing components loadable by
  newer hosts.
* Bad, because the hash and schema rules freeze the IR serialization; improving it later costs a
  breaking release.
* Bad, because the one-series read window for old schemas is code the runtime must carry and test;
  today it is exact-match only, so the window is a commitment for the next bump, not a present
  behavior.
* Bad, because exact `=` pins mean consumers must upgrade the runtime, the ABI crate and the CLI
  together.

### Confirmation

`crates/alef-component-abi/tests/layout_test.rs` pins the sizes and field offsets of the `V1`
structs; a layout change must update it and the changelog. A golden test for
`ComponentContractIr::hash` in `src/codegen/component.rs` (alongside
`canonical_contract_hash_is_deterministic` and `adding_an_enum_variant_changes_the_contract_hash`)
guards hash stability, and the runtime's loader and manager tests cover the schema and ABI version
rejection paths. Follow-ups, tracked with #845: tighten the producer template's `alef-component-abi`
requirement to `=`, add the golden-hash test if absent, and implement the previous-schema read path
with the next schema bump.

## More Information

* [ADR 0002: Structured values cross the C FFI as serde JSON](0002-structured-values-cross-the-c-ffi-as-serde-json.md)
* [ADR 0007: Generational handles across the C ABI](0007-generational-handles-across-the-c-abi.md)
* [ADR 0014: Raise the generated minimums for the other host languages](0014-raise-the-generated-minimums-for-the-other-host-languages.md)
* Issue #845
