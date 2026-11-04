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

## Bundled (static-link) fallback

`bundled_on` marks targets where downloading component code is unsupported; alef records them in
the lock (`mode = bundled`, no artifact) but no backend generates the bundled delivery. Static
musl binaries cannot `dlopen`, so a Rust host that ships such a target links the implementation
in itself:

1. Add the producer's implementation crate as an optional dependency of the host, enabled by a
   cargo feature (for example `bundled = ["dep:my-impl"]`) with the component's `features`
   forwarded.
2. Build the contract's provider directly from the implementation type (the `implementation`
   path in `provides`) instead of going through `ComponentManager`, under
   `#[cfg(feature = "bundled")]`. Gate the `component_manager` path with
   `#[cfg(not(feature = "bundled"))]`.
3. Build the musl release with `--features bundled`; glibc, macOS and Windows builds keep the
   downloadable component and its small core.

The generated host module does not do this for you: it cannot know how a given host selects between the
two paths, so the switch stays a few lines of host code.

## Root certificates

Downloads verify TLS against the Mozilla roots compiled into the binary by default. Hosts behind a
TLS-inspecting proxy (`HTTPS_PROXY` is honoured) need the operating system trust store instead:
enable the `platform-verifier` feature of `alef-component-runtime` and call
`DownloadOptions::with_root_certs(RootCerts::PlatformVerifier)`. Requesting it without the feature
fails the download with an error naming the feature, never a silent fallback to the Mozilla roots.
Archive integrity is unchanged, since size, SHA-256 and signature come from the lock.

## Extra producer dependencies and contract identity

`extra_dependencies` on a component adds Cargo dependencies to its generated producer crate, so an
implementation type can live in a crate other than the core crate:

```toml
[[crates.components]]
name = "pdf"
# ...
[crates.components.extra_dependencies]
pdf-engine = { version = "2", features = ["fast"] }
serde_json = "1"
```

Each value is a Cargo dependency spec (a version string or an inline table) emitted verbatim into
the producer `Cargo.toml`. Names must be valid Cargo package names and cannot override
`alef-component-abi`, `futures` or the core crate. The producer pins `alef-component-abi` to the
exact alef version (`=X.Y.Z`).

## Source crate and producer location

Two optional per-component fields place the implementation and the producer outside the core
crate's workspace; both default to the layout above, so omitting them changes nothing:

```toml
[[crates.components]]
name = "web"
source_manifest = "components/crates/x-web/Cargo.toml"  # default: the [[crates]] core crate
producer_dir = "components/crates/x-web-component"      # default: crates/<core>-<name>-component
features = ["web"]
```

- `source_manifest` is the `Cargo.toml` of the crate holding the `implementation` types. The
  producer keeps its path dependency on the core crate (the contract trait home, whose
  `trait_path`s are emitted verbatim) and additionally depends on the source crate, named by that
  manifest's `[package] name`; both paths are computed relative to `producer_dir`. The component's
  `features` go on the source crate, `default_features` applies to both dependencies, and the core
  dependency carries no features. If the manifest names the core crate itself, a single dependency
  with the features is emitted as before. Not valid for Go producers.
- `producer_dir` is where the producer crate is generated and built:
  `alef component build` runs `cargo build --manifest-path <producer_dir>/Cargo.toml`, so cargo
  resolves the workspace and `Cargo.lock` that own that directory (the build's `--target-dir` is
  unchanged). The package name stays `<core>-<name>-component`. It must not contain `.` or `..`
  segments, must be unique per component (defaults included), and must not be, contain or sit
  inside the core crate directory or equal the `source_manifest` directory, so generation never
  overwrites real files. Changing or removing `producer_dir` leaves the previously generated
  producer crate where it was; delete it by hand.

Both are resolved relative to the directory containing `alef.toml`, not the working directory, so
`alef --config ../alef.toml ...` finds the same files. Every other path in the config, including
`sources` and the core crate location the producer depends on, remains relative to the working
directory; run alef from the project root for a producer in a second workspace to be consistent.

The contract hash no longer includes `trait_path`: it covers the contract name, interface version,
method signatures and the record and enum shapes they reference. A contract trait re-exported
through a different crate path therefore has the same hash, and one contract crate can be
implemented by several engine crates without an aggregator. This changes every previously
computed contract hash (see ADR 0015); relock and rebuild components.
