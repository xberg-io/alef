---
status: accepted
date: 2026-11-03
decision-makers: ["Project maintainers"]
---

# Raise the generated minimums for the other host languages

## Context and Problem Statement

Generated packages declared runtime floors at or near end of life: Ruby 3.2 (March 2026), PHP
8.2 (December 2026), Elixir 1.14 and 1.15, and macOS 13 and 14; Go and Rust floors trailed the
versions the maintainers build and test with. Several floors
were hardcoded in individual emitters, so scaffold and E2E manifests could disagree about the same
language.

## Decision Drivers

* Every advertised floor is a version its upstream still supports.
* One constant per language, so scaffold and E2E output cannot drift apart.

## Considered Options

* Keep the existing floors.
* Raise each floor to the oldest supported upstream release, routed through one constant.

## Decision Outcome

Chosen option: "Raise each floor to the oldest supported upstream release, routed through one
constant", because it removes end-of-life support claims and the duplicated literals at once.

The emitted floors are Node 22, Ruby 3.3, PHP 8.3, Elixir 1.17, Go 1.27, Java 25, R 4.4 and Rust 1.99.
Swift packages target macOS 26 and use `swift-tools-version: 6.2`, the first PackageDescription with
`.macOS(.v26)`; iOS stays at 16. Kotlin JVM 21, Android minSdk 24, Dart 3.13 and Zig 0.17 are
unchanged. Each value is a constant in `toolchain` in `src/core/template_versions.rs`; the existing
`required_ruby_version` and Swift `min_macos_version` overrides still take precedence. This extends
the Python decision in [ADR 0012](0012-raise-the-generated-python-minimum.md).

### Consequences

* Good, because generated manifests no longer promise support for end-of-life runtimes.
* Bad, because consumers on the old runtimes, including macOS 15 users of Swift packages, must stay on
  an older Alef release or use the overrides.

### Confirmation

Scaffold and E2E unit tests assert the emitted values; `template_versions_renovate_coverage` fails on
an unregistered version constant, and `msrv_declaration_consistency` ties the README to `rust-version`.

## More Information

* [ADR 0012: Raise the generated Python minimum](0012-raise-the-generated-python-minimum.md)
