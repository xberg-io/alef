---
status: accepted
date: 2026-11-06
decision-makers: ["Project maintainers"]
amends: ["0014"]
---

# Generate the oldest macOS Apple still supports

## Context and Problem Statement

[ADR 0014](0014-raise-the-generated-minimums-for-the-other-host-languages.md) set the generated Swift macOS minimum
to 26, the macOS of the build runners. Nothing in the generated code needs macOS 26, and repositories copy the value
into their release workflows, so their CLI, FFI libraries, extensions and Python wheels refuse macOS 15, which Apple
still supports. Which macOS should a generated `Package.swift` declare by default?

## Decision Drivers

* A generated package runs on every macOS that Apple still supports.
* The rule names a value without reading the generated code, so it does not move when a backend changes.
* The generated Swift API does not change to reach an older macOS.

## Considered Options

* Keep macOS 26, the macOS of the current build runners.
* Generate the oldest macOS Apple still supports.
* Generate the lowest macOS the generated code needs.

## Decision Outcome

Chosen option: "Generate the oldest macOS Apple still supports", because it is the only option that covers every
supported macOS without changing the generated Swift API.

As of 2026-11 that is macOS 15: `SWIFT_MIN_MACOS` in `src/core/template_versions.rs` is `"15.0"` and every emitted
`Package.swift` declares `.macOS(.v15)`. The value moves when Apple drops a release from support, not when Apple ships
a new one. `SWIFT_TOOLS_VERSION` stays `6.2`: consumers ship `staticLibrary` artifact bundles, which SwiftPM reads from
6.2 on, and PackageDescription 6.2 has `.v15`. A build runner label is not a deployment target, so the runners stay on
macOS 26. `[crates.swift] min_macos_version` still overrides the default. The iOS minimum and the other floors of
ADR 0014 are unchanged.

### Consequences

* Good, because generated packages and the binaries built against them install on macOS 15 again.
* Good, because the generated Swift API keeps `Duration` for a Rust `std::time::Duration`.
* Bad, because a maintainer must raise the constant by hand when Apple ends support for a release.

### Confirmation

The scaffold test (`src/scaffold/tests/language_swift_kotlin_gleam_zig.rs`) and
`tests/e2e_swift_macos_override.rs` assert `.macOS(.v15)` as the default and that a configured `min_macos_version`
replaces it. `tests/template_versions_renovate_coverage.rs` registers `SWIFT_MIN_MACOS` as a human-reviewed floor.

## Pros and Cons of the Options

### Keep macOS 26

* Good, because no change.
* Bad, because it refuses macOS 15, which Apple still supports, for no requirement of the code.

### Generate the lowest macOS the generated code needs

* Good, because packages would run back to macOS 10.15.
* Bad, because Swift's `Duration` needs macOS 13, so the generated API would change `Duration` to `TimeInterval`,
  a breaking change for every consumer that exposes a duration.
* Bad, because the floor would follow the generated code and move whenever a backend adopts a newer API.

## More Information

* [ADR 0014: Raise the generated minimums for the other host languages](0014-raise-the-generated-minimums-for-the-other-host-languages.md)
