# Component stdio protocol 2: binary frames

This is the wire specification for stdio sidecars that speak protocol 2: binary frames with raw payload bytes and request ids, optionally pipelined. Protocol 1 (JSON envelopes, one call at a time) is described in [Component stdio transport and Go producers](component-stdio-and-go.md#implement-the-stdio-protocol) and remains supported. The decision is recorded in [ADR-0025](../../adrs/0025-pipeline-binary-frames-over-stdio-sidecar-connections.md). The reference implementation is `alef-component-runtime`'s `stdio::frame` module.

## Negotiation

1. The host starts the sidecar with the environment variable `ALEF_STDIO_PROTOCOLS` set to the comma-separated protocol versions it speaks. A host that speaks protocol 2 sets `1,2`. An older host does not set it.
2. The host sends the protocol-1 describe frame, exactly as before: a `u32` little-endian length followed by the JSON `{"op":"describe"}`. A sidecar must ignore unknown fields in this request.
3. The sidecar answers with a protocol-1 response frame, `{"status":0,"payload":[...]}`, whose payload is the descriptor JSON. To select protocol 2 it sets `"protocol_version": 2` and may add `"max_in_flight"`:

   ```json
   {
     "protocol_version": 2,
     "max_in_flight": 8,
     "abi_major": 1,
     "abi_minor": 0,
     "component_id": "codec",
     "component_version": "1.0.0",
     "feature_set_hash": "<64 hex characters matching the manifest>",
     "provides": [ ... ]
   }
   ```

   A sidecar must declare protocol 2 only when `ALEF_STDIO_PROTOCOLS` lists `2`; otherwise it declares protocol 1 and omits `max_in_flight`, because older hosts reject both. `max_in_flight` is an integer from 1 to 4294967295 and defaults to 1; the host rejects it on a protocol-1 descriptor. All other descriptor fields and their validation are unchanged.
4. Every frame after the describe response, in both directions, is a protocol-2 frame. The host sends nothing between the describe request and its response, so the switch is unambiguous.

A sidecar that supports both protocols serves old hosts with protocol 1 and new hosts with protocol 2 from the same executable.

## Frame layout

All integers are little-endian. Every frame is a 20-byte prefix, then `header_len` header bytes, then `payload_len` payload bytes. There is no other framing, padding or trailer.

| Offset | Size | Field | Rule |
| --- | --- | --- | --- |
| 0 | 1 | `magic` | Always `0xA2`. |
| 1 | 1 | `kind` | `1` = call (host to sidecar), `2` = response (sidecar to host). |
| 2 | 2 | `flags` | Reserved, must be `0`. |
| 4 | 8 | `request_id` | `u64`, never `0`. |
| 12 | 4 | `header_len` | `u32`, at most 65,536. |
| 16 | 4 | `payload_len` | `u32`, at most 16,777,216 (16 MiB). |
| 20 | `header_len` | header | Kind-specific, below. |
| 20 + `header_len` | `payload_len` | payload | Raw bytes, never encoded. |

The payload limit applies to the payload bytes themselves; a call or response may carry a full 16 MiB payload. A zero-length payload is valid.

### Call header (`kind = 1`)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | `contract_len`, `u16` |
| 2 | `contract_len` | contract name, UTF-8 |
| 2 + `contract_len` | rest of header | method name, UTF-8 |

The payload is the method's input bytes.

### Response header (`kind = 2`)

The header is exactly 4 bytes: the `AlefStatus` code as an `i32`. `0` is success and the payload is the result. Any other value is an error status (`1` invalid argument, `3` internal error, or a vendor code) and the payload is a UTF-8 error message. An error response leaves the connection usable.

A worked example: a call with id 1 to `engine.echo` carrying the two bytes `68 69`:

```text
a2 01 00 00  01 00 00 00 00 00 00 00  0c 00 00 00  02 00 00 00
06 00 65 6e 67 69 6e 65 65 63 68 6f  68 69
```

and its successful response:

```text
a2 02 00 00  01 00 00 00 00 00 00 00  04 00 00 00  02 00 00 00
00 00 00 00  68 69
```

## Ordering and concurrency

* The host assigns ids, starting at 1 and increasing; an id is never reused on a connection.
* The sidecar answers every call exactly once, with a response frame carrying the call's id. It must never send a response for an id it did not receive, and never send a call frame.
* Responses may be sent in any order.
* The host keeps at most `max_in_flight` calls unanswered. A sidecar that declares more than 1 must accept that many concurrent calls, and must write each response frame as one unit: frames from concurrent handlers must never interleave on stdout.
* The sidecar must keep reading stdin while calls are running, so a large request never blocks behind a large response.
* The host closes stdin to end the connection; the sidecar exits on EOF. Diagnostics go to stderr only.

## Errors

| Condition | Host behavior |
| --- | --- |
| Response with a nonzero status | That call fails with the status and message; the connection stays up. |
| A call exceeds its deadline | The id is abandoned: the call fails with a timeout and a later response for that id is read and dropped. The slot stays occupied until that response arrives. The connection stays up, unless the sidecar sent no response at all during the deadline, or every slot is held by an abandoned call: then the sidecar is considered hung and is killed. |
| EOF, bad `magic`, unknown `kind`, nonzero `flags`, id `0`, a length over its limit, a truncated frame, a response header that is not 4 bytes, a call frame from the sidecar, or a response for an id that is not outstanding | A protocol violation: every in-flight call fails at once and the child is killed and reaped. |
| The sidecar exits or a write to it fails | Every in-flight call fails at once. |

After a fatal failure the next call respawns the sidecar from the verified executable, within the manager's restart budget, and runs the describe handshake and the host's session setup again ([ADR-0024](../../adrs/0024-respawn-stdio-sidecars-after-a-fatal-connection-failure.md)).

On the sidecar side, a frame it cannot parse is a protocol violation: it should write a message to stderr and exit, which the host reports as a failed connection. An unknown contract or method, or a failed call, is a response with a nonzero status.

## Implementing a sidecar

A Go sidecar needs `encoding/binary`'s `LittleEndian` and three changes to a protocol-1 implementation:

* At start-up, read `ALEF_STDIO_PROTOCOLS`. Serve the describe frame in protocol 1 as before, adding `"protocol_version": 2` and `max_in_flight` only when the list contains `2`.
* After the describe response, loop over `io.ReadFull` of the 20-byte prefix, then the header and payload, rejecting any violation of the rules above.
* For each call, run the handler (on a goroutine when `max_in_flight > 1`) and write the 20-byte prefix, the 4-byte status and the payload under one mutex, with one `Write` of a single buffer or a locked `bufio.Writer` flushed per frame.

State shared between calls (registrations, caches) must be safe for concurrent use once `max_in_flight > 1`. Rust sidecars can use `alef_component_runtime::stdio::{read_binary_frame, write_binary_frame, decode_call_header, encode_response_header}`.
