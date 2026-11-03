# Component distribution

Reference for the `alef component` workflow beyond the overview in the README.

## Components-only crates

A crate that declares `[[crates.components]]` does not need binding languages. When `languages`
is empty on the crate and in `[workspace]`, `alef generate` / `alef scaffold` emit only the
component producer crates (`crates/<core>-<component>-component`); no backend runs and no binding
lock is staged. `alef component build|package|verify|lock` work unchanged and write the lock to
`--output` only.

```toml
[workspace]

[[crates]]
name = "poly"
sources = ["src/lib.rs"]
# no `languages`
```

A workspace-level `languages` list is still inherited. To opt a crate out of it, set
`languages = ["rust"]`: Rust has no backend, so generation skips it.

## Targets

By default a component is built for five targets: `x86_64`/`aarch64` Linux GNU, `x86_64`/`aarch64`
macOS and `x86_64` Windows MSVC, filtered by the crate's `[targets]` table and `bundled_on`. An
explicit `targets` list may name any well-formed Rust triple, for example
`aarch64-pc-windows-msvc`. Alef checks the syntax only; the triple must be buildable with your
toolchain, and the runtime only selects a lock entry whose target equals the host's.

`alef_component_runtime::target::host_target()` returns the host triple (`None` on hosts that
cannot download components). It recognises the five defaults plus `aarch64-pc-windows-msvc`.

## URL and artifact names

`url_template` must start with `https://` (or `file://` for local dev locks) and contain
`{artifact}`. Optional placeholders: `{crate}`, `{component}`, `{version}`, `{target}`,
`{feature_hash}`, `{contract_hash}`.

```toml
[component_distribution]
url_template = "https://github.com/owner/repo/releases/download/v{version}/{artifact}"
artifact_name_template = "poly-{component}-{version}-{target}.tar.gz"
```

`artifact_name_template` defaults to `{crate}-{component}-{version}-{fh16}-{ch16}-{target}.tar.gz`
(`fh16`, `ch16`: first 16 hex characters of the feature and contract hashes). It must contain
`{component}` and `{target}`, and the rendered name may use only `[A-Za-z0-9._-]`. Include
`{crate}` if several crates write to one output directory. The same template must be in effect for
`package`, `verify` and `lock`.

## Dev locks

```console
alef component package --unsigned --library target/debug/libpoly_ruff_component.so
alef component lock --unsigned --url-template "file://$PWD/dist/components/{artifact}"
```

`lock --unsigned` checks archive hashes but not signatures, and writes a lock with empty
`public_keys` and empty `key_id`s. The runtime does not verify signatures against such a lock, so
never ship it. `--url-template` overrides the configured template and is required when the crate
has no `component_distribution`.

## Signing keys

`alef component package` takes the Ed25519 private key from exactly one source, always with
`--key-id`:

| Flag | Source |
| --- | --- |
| `--signing-key PATH` | key file: PKCS#8 PEM or DER, or a raw 32-byte seed |
| `--signing-key -` | stdin: PKCS#8 PEM or DER |
| `--signing-key-env NAME` | env var: PKCS#8 PEM text, or base64 of PKCS#8 DER / a 32-byte seed |

```console
alef component package --signing-key-env RELEASE_SIGNING_KEY --key-id release-2026
```

The key is read once before any archive is written, is never logged, and a missing or empty
variable is an error. Prefer the env or stdin forms in CI so the secret never touches disk.

### Rotation

`public_keys` holds any number of trusted keys, and every signed manifest carries one signature
by one of them. To rotate:

1. Add the new public key next to the old one and release; clients now trust both.
2. Sign new releases with the new key (`--key-id` of the new entry).
3. Once old releases are no longer served, remove the old public key.

Binding locks staged by `alef component lock` embed only the keys their artifacts reference.

## Shared target directory

By default each component builds into `.alef/components/build/<crate>/<component>`, so every
dependency is compiled once per component. Pass one directory to share the compilation:

```console
alef component build --target-dir target/components
alef component package --target-dir target/components --signing-key-env RELEASE_SIGNING_KEY --key-id release-2026
```

Both commands must receive the same value.

### Producer crates in the host workspace

Producer crates are standalone manifests without a `[workspace]` table. When the producer
directories sit under a host workspace, add them to its `members` (or `exclude` them) so Cargo
accepts the layout. As members they inherit the workspace `[profile.release]` (for example
`lto = "thin"`, `panic = "unwind"`) and share its `Cargo.lock` and target directory.

```toml
[workspace]
members = ["crates/poly-ruff-component", "crates/poly-oxc-component"]

[profile.release]
lto = "thin"
panic = "unwind"
```

## Host services

A component can call back into its host through an optional service table appended to
`AlefHostApiV1`: `services_context`, `service_call` and `service_notify`. The table is
bytes-only: the host defines which service names exist and how request and response bytes are
encoded.

```rust
// Host: implement the trait, register it before loading components.
struct Services;
impl alef_component_runtime::HostServices for Services {
    fn call(&self, service: &str, request: &[u8]) -> Result<Vec<u8>, String> { /* ... */ }
    fn notify(&self, service: &str, payload: &[u8]) { /* ... */ }
}
let manager = manager.with_host_services(std::sync::Arc::new(Services));

// Component: keep the HostApi from `FromHost::from_host` and use it later.
let reply = host.call_service("lint.run", &request)?;
host.notify("progress", b"3/10")?;
```

- **Versioning.** The fields before `services_context` are the original layout. `struct_size`
  says whether a host has the service fields; `HostApi::from_raw` copies only the declared
  bytes, so an older host still loads (logging works) and service calls return
  `HostServiceError::Unsupported`, as they do when a callback pointer is null.
- **Ownership.** A `call` response is an `AlefOwnedBuffer` allocated by the host. The component
  copies it and releases it through the buffer's own `free` callback, which `call_service` does
  before returning. On a non-`OK` status the host may put a UTF-8 error message in the same
  buffer; it surfaces as `HostServiceError::Failed { status, message }`. `notify` has no
  response buffer; the host copies the payload before returning.
- **Threading.** `service_call` and `service_notify` may be invoked concurrently from any
  thread until the instance is destroyed, so `HostServices` implementations must be
  thread-safe. `HostApi` is `Copy` and may be stored in the implementation; a stored copy is
  valid until the component's `destroy`.
- **Panics.** The runtime's trampolines catch panics in `HostServices` and return
  `INTERNAL_ERROR`; nothing unwinds across the FFI boundary.
- **Registration scope.** `with_host_services` applies to components loaded afterwards.
  Without it the table is present but empty, so components see `Unsupported`.

## Pure-Rust hosts

A host that links the core crate and loads components itself, with no ffi or pyo3 binding in
between, can have alef emit its glue:

```toml
[crates.component_host]
output = "crates/host/src/components_gen.rs"
lock_path = "components.lock.json" # optional, relative to the module; this is the default
```

`alef generate` then writes one Rust module holding, for every contract the crate's components
provide: the `#[repr(C)]` table with its `AlefContract::CONTRACT_HASH`, the `<Contract>Proxy`
that implements the contract trait, `<CONTRACT>_ENTRY_SYMBOL` (NUL-terminated, for
`ComponentManager::ensure_contract`) and `<CONTRACT>_CONTRACT_HASH_HEX`. It also holds
`COMPONENT_IDS`, `host_target()` (alef-component-runtime's detection) and

```rust
pub fn component_manager(cache_root: PathBuf, offline: bool) -> Result<ComponentManager, ComponentError>
```

over the embedded lock. The cache root and offline flag come from the caller; the module reads
no environment variables. The host crate depends on `alef-component-runtime`,
`alef-component-abi` and `serde_json`. `alef component lock` stages the lock at the configured
`lock_path`, and `alef generate` seeds an empty one when it is missing.

To check a committed hash against the lock in CI, run `alef component contract-hash [--crate X]`;
it prints one `component contract hash crate=… component=… contract=… hash=…` line per provided
contract, computed the same way as the lock's contract hash.
