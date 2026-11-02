---
status: accepted
date: 2026-11-03
decision-makers: ["Project maintainers"]
---

# Inspect Swift archives with the selected Rust toolchain's LLVM

## Context and Problem Statement

The pinned Swift consumer gate reads undefined symbols from a real Rust static archive before its
missing-callback link control. Run 41496139746 preserved Rust 1.99.1/LLVM 23.1.1 bitcode that Apple's
LLVM 21 nm cannot read: it exits unsuccessfully with an unknown attribute diagnostic. Its partial
stdout cannot establish the expected callback set. LLVM's default output also varies between native
Mach-O and bitcode members, and genuine section markers and Objective-C selectors extend the former
bare-C-symbol grammar.

[Org ADR-0004](https://github.com/xberg-io/agent-conventions/blob/main/adrs/0004-mise-toolchain-baseline.md)
keeps Rust in `rust-toolchain.toml` under the shared toolchain baseline. The archive reader must follow
the selected compiler without introducing an independently installed, ambient tool.

## Considered Options

* Keep using the ambient Apple nm.
* Pin and install a separate LLVM distribution.
* Use llvm-nm shipped with the exact selected Rust toolchain.

## Decision Outcome

Chosen option: "Use llvm-nm shipped with the exact selected Rust toolchain", because the compiler and
reader then share their LLVM version without another independently maintained toolchain pin.

The controller's `rust-toolchain.toml` declares `llvm-tools`. Guarded bootstrap activates that declaration
even when the selected historical CLI source omits the component. Before building, record the exact
compiler release, host, LLVM version, sysroot reader path, executable digest and controller declaration
digest; refuse missing, foreign or mismatched readers. Use explicit `--undefined-only` and
`--just-symbol-name` flags and reject unsuccessful commands before consuming their stdout.
Keep a closed grammar for observed Mach-O section markers, Objective-C selector symbols and ordinary
symbols, preserving rejection of typed/defined rows, malformed output and empty inventories.

### Consequences

* Good, because matching LLVM reads the producer's native objects and bitcode without partial-output admission.
* Good, because the existing Rust pin controls both compilation and inspection.
* Bad, because hosted bootstrap must install an additional declared Rust component.
* Bad, because new legitimate symbol forms still require evidence and reviewed parser changes.

### Confirmation

`scripts/consumer-swift/test_nm.py` checks a captured 109-name inventory with 39 Mach-O/Objective-C names
and 70 generated callback references, explicit reader flags and malformed/zero-work controls.
`scripts/consumer-swift/test_rust_symbols.py` checks bootstrap declaration ownership, version/path
provenance and missing, foreign, mismatched and failed-command refusals. The harness census is 78 tests.
The paired Rust reader successfully parses all 12,546 unique undefined names and all 70 callback
references from the 229,188,536-byte archive preserved by run 41496139746. This fast artifact proof
justifies the reader choice; it does not establish a hosted native qualification or substitute for
the actual negative link controls.

`.github/workflows/verify-xberg-swift.yaml` remains responsible for full hosted positive builds,
archive and linker-map checks, both safe-package suites and real negative controls. Those checks
must pass independently before reporting native qualification.

## More Information

* [Compiler-matched reader implementation](https://github.com/xberg-io/alef/commit/71f58e253568c1ed4d04dbf25820381af7fd6956)
* [Negative-control follow-up](https://github.com/xberg-io/alef/commit/e30931ea6aec225e199373b0515ba4730f742d3b)
* [Preserved native run](https://github.com/xberg-io/alef/actions/runs/41496139746)
