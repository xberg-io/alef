---
status: proposed
date: 2026-11-15
decision-makers:
  - Alef maintainers
amends: ["0017", "0024"]
---

# Pipeline binary frames over stdio sidecar connections

## Context and Problem Statement

ADR-0017's stdio frames carry payloads as JSON arrays of byte values, so content grows about fourfold and the 16 MiB frame limit admits only about 4 MiB of payload. Calls are serialized across a component, so a host that calls one sidecar from a thread pool (poly, from rayon) queues every worker behind one process ([poly issue #189](https://github.com/Goldziher/polylint/issues/189)). Already published sidecars must keep working.

## Decision Drivers

* Payload bytes cross the pipe as bytes, and the frame limit bounds real content.
* Concurrent callers are not serialized by the transport.
* A sidecar that keeps state per connection (poly's `Init` and config registrations) must see every call.
* A slow request must not take down the connection for others; a hung sidecar must still be restarted.
* Old sidecars keep working under the new host, and new sidecars can still serve old hosts.

## Considered Options

* Protocol 2: binary frames with request ids, pipelined over one process, negotiated in the describe handshake.
* A pool of N protocol-1 processes per sidecar.
* Binary frames only, calls still serialized.

## Decision Outcome

Chosen option: "Protocol 2", because it fixes both problems on the one connection a stateful sidecar needs, and the negotiation keeps protocol 1 untouched for every existing sidecar.

The host starts every child with `ALEF_STDIO_PROTOCOLS=1,2`. The handshake is unchanged: a protocol-1 `{"op":"describe"}` frame. A sidecar that sees `2` in that variable may answer with `"protocol_version": 2` and an optional `"max_in_flight"` (default 1); a sidecar that does not keeps answering protocol 1, which the host still speaks exactly as before. After a protocol-2 describe response, both directions switch to binary frames: a 20-byte little-endian prefix (magic `0xA2`, kind, reserved flags, a nonzero `u64` request id, header and payload lengths), a small header, and raw payload bytes of at most 16 MiB. The layout is specified in `docs/design/component-stdio-binary-protocol.md`.

The host keeps at most `max_in_flight` requests unanswered, writes frames from one writer thread and reads responses on one reader thread that dispatches them by id; no lock is held while a caller waits. The sidecar may answer in any order. A request whose deadline passes is abandoned and its late response dropped; that timeout is recoverable unless the sidecar answered nothing during the whole deadline or every slot is held by an abandoned request, in which case the sidecar counts as hung and the connection is stopped. EOF, a malformed frame, a failed write or a response for an id that is not outstanding fails every in-flight call at once and stops the connection; ADR-0024's respawn on the next call applies unchanged. Protocol-1 connections keep ADR-0017's serialized calls and fatal timeouts, and ADR-0024's no-dispatch-before-setup gate now lives in `SidecarProcess::exchange` (formerly `exchange_gated`).

A pool of processes was rejected because each process would hold a separate session: a config registered on one process is unknown to the others, so stateful sidecars would break, and the pool would multiply memory and start-up cost. Binary frames alone would leave the serialization that motivated the change.

### Consequences

* Good, because a payload uses its real size against the frame limit, and the host encodes it with one copy and no JSON.
* Good, because one sidecar serves concurrent callers, up to a limit the sidecar chooses.
* Good, because protocol-1 sidecars and hosts keep working; a sidecar can support both protocols by reading the environment variable.
* Bad, because a sidecar that declares `max_in_flight > 1` must handle concurrent requests and serialize whole-frame writes itself.
* Bad, because an abandoned request keeps its slot until its late response arrives, so a sidecar that never answers some requests loses concurrency until it is restarted.
* Bad, because a protocol-2 timeout no longer kills the child while the sidecar keeps answering others, so the abandoned work runs to completion inside it.

### Confirmation

`crates/alef-component-runtime/src/stdio/frame.rs` unit-tests the byte layout, round trips at the payload limit, oversized and truncated frames and invalid prefixes; `src/stdio/binary_tests.rs` drives the pipelined connection over in-memory pipes (out-of-order dispatch, the in-flight bound, recoverable and fatal timeouts, a dropped late response, EOF and unknown-id failures). `tests/stdio_binary_sidecar.rs` runs a signed Python sidecar that negotiates protocol 2 only when advertised: negotiation and protocol-1 fallback, a 16 MiB payload, eight concurrent half-second calls finishing in well under the four seconds serialization would take, the declared bound, timeout isolation, a stalled sidecar's restart, process death mid-flight, protocol violations and session replay. `tests/stdio_sidecar.rs` keeps every protocol-1 case on the unchanged fixture.

```console
mise exec -- cargo test -p alef-component-runtime
```

## Pros and Cons of the Options

### Protocol 2: binary frames with request ids, pipelined over one process

* Good, because it keeps one session per component.
* Bad, because the transport needs reader and writer threads and per-request bookkeeping.

### A pool of N protocol-1 processes per sidecar

* Good, because it needs no protocol change.
* Bad, because per-process sessions break stateful sidecars, and payloads stay inflated.

### Binary frames only, calls still serialized

* Good, because it is the smallest change.
* Bad, because rayon workers still queue on one process.

## More Information

* [ADR-0017](0017-framed-stdio-transport-for-component-sidecars.md)
* [ADR-0024](0024-respawn-stdio-sidecars-after-a-fatal-connection-failure.md)
* [Wire format](../docs/design/component-stdio-binary-protocol.md)
