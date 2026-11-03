# Component stdio transport and Go producers

The [component distribution reference](components.md) covers signing, locks and cache installation. Executable sidecars reuse that distribution path. Go c-shared producers use the existing native C ABI. The design choices are recorded in [ADR-0017](../../adrs/0017-framed-stdio-transport-for-component-sidecars.md) and [ADR-0018](../../adrs/0018-go-c-shared-component-producers.md).

## Choose a transport

| Producer | Host invocation | Constraints |
| --- | --- | --- |
| Native Rust or Go shared library | Existing typed C ABI and native binding hosts | Go c-shared supports one Go runtime per process |
| Executable sidecar | `LoadedComponent::invoke_bytes` | Serialized bytes-only calls; no typed C tables or host callbacks |

To use several Go producers in one application, isolate them in executable/stdio processes. The Go c-shared generator does not generate those executables.

## Load an executable sidecar

Package a prebuilt executable with the existing single-profile, single-target override:

```console
alef component package --component codec --target aarch64-apple-darwin --library ./bin/codec-sidecar --signing-key-env RELEASE_SIGNING_KEY --key-id release-2026
alef component verify
alef component lock
```

Use the target triple matching the executable. `component lock` needs the usual distribution URL configuration. The signed manifest's `library.file` points to the executable payload; there is no alternate stdio artifact schema or signature format. Sidecar selection belongs to the host.

For a generated standalone Rust host:

```toml
[crates.component_host]
output = "src/components.rs"
stdio_components = ["codec"]
```

Run `alef generate` after updating the existing crate configuration. The generated manager selects these IDs as sidecars. A contract with only stdio providers has no generated typed proxy. Native binding backends continue to use native libraries.

For direct runtime access, select the component before first use. This fragment assumes a parsed `ComponentLock`, a cache directory and the host target string:

```rust
use alef_component_abi::AlefHostApiV1;
use alef_component_runtime::ComponentManager;
use std::time::Duration;

let manager = ComponentManager::from_lock(lock, cache_dir, target, AlefHostApiV1::EMPTY)?
    .with_stdio_component("codec")
    .with_sidecar_timeout(Duration::from_secs(5));
let codec = manager.ensure_contract("codec", "codec", b"alef_component_entry_v1_codec\0")?;
let output = codec.invoke_bytes("transform", b"hello")?;
```

The entry-symbol argument is retained for the common native/stdio API; stdio dispatch uses the contract name. The default deadline is 30 seconds. One long-lived child serves every contract of the component, with calls serialized across them.

## Implement the stdio protocol

Stdin and stdout carry frames only; write diagnostics to stderr. Each frame is a four-byte little-endian unsigned body length followed by that many JSON bytes. Valid lengths are 1 through 16,777,216 bytes. Flush each response. Payloads are JSON arrays of byte values, including descriptor JSON encoded as bytes.

The host first sends:

```json
{"op":"describe"}
```

Respond with `{"status":0,"payload":[...]}`, where the payload decodes to a descriptor of this shape:

```json
{
  "protocol_version": 1,
  "abi_major": 1,
  "abi_minor": 0,
  "component_id": "codec",
  "component_version": "1.0.0",
  "feature_set_hash": "<64 hex characters matching the manifest>",
  "provides": [
    {
      "contract": "codec",
      "interface_version": 1,
      "contract_hash": "<64 hex characters matching the manifest>",
      "implementation": "Codec"
    }
  ]
}
```

Replace the example identities and hashes with the signed manifest values. The complete `provides` list, including its order and implementation fields, must match. The host checks protocol and ABI compatibility before making the connection ready.

A call and successful response look like:

```json
{"op":"call","contract":"codec","method":"transform","payload":[104,105]}
```

```json
{"status":0,"payload":[105,104]}
```

Answer exactly once per request. A nonzero ABI status carries error-message bytes and leaves the process usable. Invalid JSON, invalid frame lengths, truncated frames, EOF and deadline expiry invalidate the connection and terminate/reap the child. Later ensure calls fail and status reports cached rather than ready. Dropping the last owner also reaps the child.

Rust producers can use the runtime's public `stdio::{StdioDescriptor, StdioRequest, StdioResponse, read_frame, write_frame}` helpers. The helpers frame JSON bytes; producers still serialize envelopes and implement dispatch. Typed instance creation, typed contract-table access and host callback services are unsupported over this transport.

## Generate a Go c-shared producer

Continue defining the contract as a Rust trait. For example, `demo::Codec` can expose a synchronous `transform(&self, input: &[u8]) -> Vec<u8>` method. Add these entries to the existing crate configuration:

```toml
[[crates.component_contracts]]
name = "codec"
trait_path = "demo::Codec"
interface_version = 1

[[crates.components]]
name = "codec"
producer_language = "go"
features = []
targets = ["aarch64-apple-darwin"]
provides = [{ contract = "codec", implementation = "NewCodec" }]
```

Omitting `producer_language` preserves Rust producer generation. A Go implementation must name a plain constructor identifier. The constructor returns the generated Go interface, such as `Codec`. Run `alef generate`, then implement the methods in `crates/<core>-codec-component/implementation.go`. `implementation.go` and `go.mod` are create-once user-owned seeds; maintain module dependencies there. Other bridge files and contract headers are regenerated. Cargo `extra_dependencies` are unsupported for Go components.

For the example infallible bytes method, the implementation has this shape:

```go
func NewCodec() Codec { return &CodecImplementation{} }

func (implementation *CodecImplementation) Transform(arg0 []byte) []byte {
    result := append([]byte(nil), arg0...)
    for left, right := 0, len(result)-1; left < right; left, right = left+1, right-1 {
        result[left], result[right] = result[right], result[left]
    }
    return result
}
```

Use the pinned Go toolchain and a CGO-compatible C compiler on the target platform:

```console
alef component build --component codec --target aarch64-apple-darwin
alef component package --component codec --target aarch64-apple-darwin --signing-key-env RELEASE_SIGNING_KEY --key-id release-2026
alef component verify
alef component lock
```

The build command sets `CGO_ENABLED=1`, maps the Rust target to `GOOS`/`GOARCH`, and invokes `go build -buildmode=c-shared`. Native Linux GNU, macOS and Windows targets are supported; unsupported target mappings and cross-compilation are rejected. Select your native triple instead of the macOS example. Existing `--library` packaging also accepts an explicitly built native payload.

Supported methods are synchronous and use booleans, fixed-width integers, floats, UTF-8 strings and bytes, with unit results and fallible returns supported. Async methods and complex wire types are rejected with method context. Returned bytes live in C-owned allocations with C deallocation callbacks; implementation identity uses `runtime/cgo.Handle`. Recovered Go panics become component failures at the ABI boundary.

New `go.mod` seeds follow Alef's shared Go language floor. Existing user-owned modules retain their settings across regeneration.

## Local development loop

`component build` and `component package` default to release artifacts. Pass `--debug` to both commands to build and package a debug artifact; `package --library PATH` takes precedence over the debug/release lookup. If you use `--target-dir`, pass the same directory to build and package.

For a local unsigned loop on the example target:

```console
alef component build --component codec --target aarch64-apple-darwin --debug
alef component package --component codec --target aarch64-apple-darwin --debug --unsigned
alef component lock --target aarch64-apple-darwin --unsigned --output dev.components.lock.json --url-template "file://$PWD/dist/components/{artifact}"
```

`component lock --target` accepts repeated flags or comma-separated triples. It requires records for each component profile's intersection with the requested targets; profiles with no matching target are skipped. Every requested triple must be configured by at least one profile. Without `--target`, the lock requires the complete configured target matrix.

Explicit `lock --unsigned --output PATH` writes an isolated development lock and preserves existing host and binding locks. Omitting `--output` writes `components.lock.json` and stages unsigned locks into configured hosts and binding packages. Signed locks still stage when `--output` is explicit.

Unsigned locks contain no public keys and skip signature verification; archive hashes are still checked. Use them only for development. A distribution configuration without public keys can retain custom artifact names and URLs for this loop, but package and lock require explicit `--unsigned`. Their default signed modes fail without the configured signing keys.
