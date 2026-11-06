---
status: proposed
date: 2026-11-06
decision-makers:
  - Alef maintainers
amends: ["0017"]
---

# Executable component producers

## Context and Problem Statement

ADR-0017 lets a host load a component as a child process, but alef can only produce one that is a Rust cdylib or a Go c-shared bridge. A consumer whose sidecar is an independently built executable (for example a pure-Go binary cross-compiled with `CGO_ENABLED=0`) must declare a Go producer, which makes `alef generate` emit a c-shared bridge nobody builds, cannot be planned or reused, and is packaged and locked outside alef's flow, so its lock has to be merged by hand with the Rust components' lock.

## Decision Drivers

* A sidecar is built by the consumer's own toolchain; alef cannot and should not know how.
* One signed lock must cover native and executable components, without an external merge.
* Release planning must reuse an unchanged executable like an unchanged Rust build.

## Considered Options

* A third `producer_language`, `executable`, with a consumer build command and a declared-input fingerprint.
* Keep declaring Go producers and build the executable outside alef.
* Let `plan` hash the producer directory only, with no build command or toolchain identity.

## Decision Outcome

Chosen option: "A third `producer_language`, `executable`", because it keeps the signed artifact, plan, reuse and lock path of ADR-0015 and ADR-0017 and moves only the build step to the consumer.

An executable component declares `producer_dir` and an `executable` table: `build` (a `sh -c` command run in `producer_dir` with `{target}`, `{out}` and `{profile}` substituted, `{out}` required), optional `file` and `target_files` (the payload name, `.exe` appended for Windows targets by default), `inputs` (globs, default `producer_dir`) and `toolchain_version_cmd`. Alef generates no producer or bridge for it, `alef verify` has nothing to compare, and host generation selects it as a stdio component without listing it in `component_host.stdio_components`. The contract is still the Rust trait named in `provides`; `implementation` is an opaque name recorded in the manifest.

`alef component plan` fingerprints the contract hashes, distribution settings, target, profile, payload name, build command template, the SHA-256 of every input file, the trimmed output of `toolchain_version_cmd`, `--env` variables and `--extra` pairs. The alef version is not an input, because alef does not shape the bytes of a command it only runs. Cross-compilation is the command's business, so a plan for any configured target can be built on any host. `build`, `package` and `lock` follow the existing plan flow, so one lock holds both kinds of component.

This amends ADR-0017 by adding a way to produce the executable a stdio host loads; the transport, framing and trust model are unchanged.

### Consequences

* Good, because a pure-Go or otherwise non-Rust sidecar ships through the same signed lock, plan and reuse path as native components.
* Good, because no unused bridge is generated and no lock merge is needed.
* Bad, because the fingerprint is only as complete as the declared inputs and toolchain command; an undeclared input (a network fetch, an undeclared file) can leave a stale build reused.
* Bad, because the build command runs through `sh`, so Windows build hosts need one.

### Confirmation

Unit tests cover the configuration validation, the fingerprint and the host selection. `tests/e2e_component_executable_test.rs` drives the `alef` binary over a shell-script producer built by `cp`: it plans, builds for two targets, packages and signs, locks next to a Rust component, reuses an unchanged build, rebuilds after a source or toolchain change and rejects a build that writes nothing:

```console
mise exec -- cargo test --test e2e_component_executable_test --test component_executable_producer_test
```

## Pros and Cons of the Options

### A third `producer_language`, `executable`

* Good, because it reuses every downstream step and needs no new artifact format.
* Bad, because alef runs a consumer-supplied shell command.

### Keep declaring Go producers and build the executable outside alef

* Good, because it needs no change to alef.
* Bad, because it generates an unused bridge, bypasses planning and reuse, and splits the lock.

### Let `plan` hash the producer directory only

* Good, because it is simpler.
* Bad, because a changed build command or compiler would reuse a stale executable.

## More Information

* [ADR-0015](0015-abi-and-runtime-stability-policy-for-component-crates.md)
* [ADR-0017](0017-framed-stdio-transport-for-component-sidecars.md)
* [ADR-0018](0018-go-c-shared-component-producers.md)
