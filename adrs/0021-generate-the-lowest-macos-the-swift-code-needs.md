---
status: proposed
date: 2026-11-05
decision-makers:
  - Alef maintainers
amends: ["0014"]
---

# Generate the lowest macOS that the Swift code needs

## Context and Problem Statement

[ADR 0014](0014-raise-the-generated-minimums-for-the-other-host-languages.md) raised the generated macOS minimum from 13 to 26 and the iOS minimum to 16. Nothing in the generated code needs macOS 26. The value followed the macOS of the build runners, and the runners build for older deployment targets without problems. Repositories copy the value into their release workflows, so their CLI, FFI libraries, extensions and Python wheels refuse every macOS before 26, including macOS 15, which Apple still patches.

The generated Swift uses two features that set a floor:

* async/await, in every generated package. It runs on macOS 10.15 and iOS 13 and later.
* Swift's `Duration`, for a Rust `std::time::Duration` in the public API, such as a `retryAfter` error value. It needs macOS 13 and iOS 16.

## Decision Drivers

* A generated package runs on every macOS that its code can run on.
* The deployment target is a property of the code, not of the build runner.
* One Swift type stands for a Rust `Duration` everywhere that alef emits one.

## Considered Options

* Keep macOS 26, the macOS of the current build runners.
* Generate the oldest macOS that Apple still patches (macOS 15).
* Generate the lowest macOS that the generated code needs, and give a Rust `Duration` to Swift as a `TimeInterval`.

## Decision Outcome

Chosen option: "Generate the lowest macOS that the generated code needs", because it is the only option that meets the first two drivers. Apple's patch window is a reason to test, not a reason to refuse an older macOS that the code runs on.

`SWIFT_MIN_MACOS` is `10.15` and `SWIFT_MIN_IOS` is `13.0`, the async/await floor. SwiftPM names 10.x releases by major and minor, so the platform case renders as `.macOS(.v10_15)`. `SWIFT_TOOLS_VERSION` stays `6.2`: published packages consume a `staticLibrary` artifact bundle, which SwiftPM reads from 6.2 on (SE-0482). The tools version sets the toolchain that builds the package, not the macOS that the package runs on. `[crates.swift] min_macos_version` and `min_ios_version` still override the defaults.

A Rust `Duration` is a `TimeInterval` in seconds in the generated Swift. That is the Foundation type for a time span on every macOS, and the trait bridge already used it. The alternative keeps `Duration` and raises the floor to macOS 13 for every package that exposes one.

The deployment target of a binary that a repository builds from Rust is the repository's own setting. The lowest values are macOS 10.13 for Intel, the floor of tree-sitter's Swift package, and macOS 11.0 for Apple Silicon.

### Consequences

* Good, because generated packages run back to macOS 10.15 and iOS 13.
* Good, because the floor has a test: the error probe type-checks the generated Swift with `-target <arch>-apple-macos<SWIFT_MIN_MACOS>`, so an API above the floor fails the build.
* Good, because a Rust `Duration` has one Swift type in the bindings, the trait bridge and the docs.
* Bad, because `TimeInterval` does not carry its unit in the type, and callers that use the `Clock` APIs convert with `.seconds(value)`.
* Bad, because the change of `retryAfter` from `Duration` to `TimeInterval` breaks Swift callers that use the old type.

### Confirmation

The scaffold test and `tests/e2e_swift_macos_override.rs` assert `.macOS(.v10_15)`, `.iOS(.v13)` and `swift-tools-version: 6.2`. The override test asserts that a configured minimum replaces the default. The `swiftpm_version_case` test covers the 10.x rendering. The error duration probe compiles against the floor and checks the `TimeInterval` values.

## More Information

This amends the macOS and iOS parts of ADR 0014. The other language floors of ADR 0014 are unchanged. See #892.
