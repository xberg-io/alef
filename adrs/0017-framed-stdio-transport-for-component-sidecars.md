---
status: accepted
date: 2026-11-03
decision-makers:
  - Alef maintainers
amends: ["0015"]
---

# Framed stdio transport for component sidecars

## Context and Problem Statement

[Issue #842](https://github.com/xberg-io/alef/issues/842) requires component producers that run as executables rather than loaded libraries. ADR-0015 describes the native component ABI and signed distribution contract; executable producers need process isolation without a second artifact or trust model.

## Considered Options

* Framed stdio with the existing signed artifact and explicit host selection.
* Load every producer through the native C ABI.
* Introduce an independent RPC service, discovery mechanism and artifact format.

## Decision Outcome

Chosen option: "Framed stdio with the existing signed artifact and explicit host selection", because it supports independently managed runtimes while preserving lock verification, signatures and the content-addressed cache.

The host opts a component ID into stdio before loading it. The verified manifest's `library.file` names the executable. One child serves all contracts of that component, with serialized request/response exchanges. A frame is a nonzero, at most 16 MiB JSON body preceded by its four-byte little-endian unsigned length. The first operation is `describe`; subsequent `call` operations carry a contract, method and byte-array payload. Responses carry an integer ABI status and byte-array payload. Successful description payload bytes encode the descriptor JSON.

The handshake validates protocol version, ABI compatibility, component identity and version, feature hash and the complete provided-contract list against the verified manifest. Each handshake and call has a configurable deadline, defaulting to 30 seconds. A timeout, broken pipe or malformed protocol kills and reaps the child and invalidates the connection. A producer's nonzero application status returns an error while keeping the connection usable. A dead process cannot remain ready: status falls back to cached and subsequent ensure calls fail.

This is a bytes-only invocation transport. Native typed function tables, typed instances and host callback services are unsupported across the process boundary. Generated standalone Rust hosts can select IDs through `component_host.stdio_components`; contracts supplied only through stdio omit typed proxies. Existing native binding hosts retain the native loader.

This amends the transport description in ADR-0015 without changing native C layout, manifest schema or contract hashing. New public runtime error variants require the minor release prescribed there.

### Consequences

* Good, because executable producers reuse the existing signed installation and cache path.
* Good, because process failures are bounded and cannot leave a usable stale connection.
* Bad, because JSON byte arrays add copying and framing overhead, and calls within one component serialize.
* Bad, because existing native typed proxies and host callbacks cannot be reused over stdio.

### Confirmation

The runtime suite verifies framing, unsupported typed access and six signed real-process cases for shared-process reuse, handshake mismatches, recoverable status errors, deadlines, malformed frames, concurrent calls and child cleanup. Run:

```console
mise exec -- cargo test -p alef-component-runtime
mise exec -- cargo test -p alef --test e2e_component_stdio_host_test generated_stdio_host_round_trips_signed_sidecar_and_rejects_native_control -- --ignored --exact
```

`crates/alef-component-runtime/tests/stdio_sidecar.rs` supplies the real-process cases. `tests/e2e_component_stdio_host_test.rs` generates and compiles a standalone Rust host, then checks two binary payload round trips through a signed sidecar and a native control. Its support harness asserts three completed jobs and unchanged hashes for twelve input files, so an empty run or source repair cannot satisfy the check. The generated-host test requires a supported Unix host, Python 3 and the pinned Rust toolchain.
