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

## Incremental releases: fingerprints, plans and lock reuse

Rebuilding every component for every target each release is wasteful when nothing a component is
built from changed. alef fingerprints each build, plans which builds can be skipped, and carries
the previous artifacts' lock entries over.

```console
alef component plan --previous-lock components.lock.json --extra image=manylinux_2_28 -o plan.json
alef component build --plan plan.json --extra image=manylinux_2_28
alef component package --plan plan.json --extra image=manylinux_2_28 --signing-key-env KEY --key-id release-2026
alef component lock --reuse components.lock.json --plan plan.json
```

**Fingerprint.** A SHA-256 hex digest (`input_hash`) over, in this order: the fingerprint format
version, alef version, `<crate>/<component>`, target, profile (`release` or `debug`), the sorted
feature list and `default_features`; each provided contract's hash; the distribution settings
(`url_template`, `artifact_name_template`, `public_keys`); the compiler identity; the build
environment; the implicit configuration files; the workspace root manifest tables; the `--extra`
pairs; and the producer's dependency closure.

- *Compiler.* The `release:`, `commit-hash:` and `LLVM version:` lines of `rustc -vV`. The `host:`
  line is left out, so the same compiler on two build machines agrees.
- *Environment.* `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `CARGO_BUILD_RUSTC`,
  `CARGO_BUILD_RUSTC_WRAPPER`, `CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER`, `CARGO_BUILD_TARGET`,
  `CARGO_PROFILE_*`, `CARGO_TARGET_*_RUSTFLAGS`, the target's `CARGO_TARGET_<T>_LINKER`, `RUSTC`,
  `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`, `MACOSX_DEPLOYMENT_TARGET`,
  `IPHONEOS_DEPLOYMENT_TARGET`, `SDKROOT`, and `CC`, `CXX`, `AR`, `CFLAGS`, `CXXFLAGS`, `LDFLAGS` in
  their cc-rs spellings (`<NAME>`, `<NAME>_<target>`, `<NAME>_<target_with_underscores>`,
  `TARGET_<NAME>`, `HOST_<NAME>`). `--env NAME` (repeatable on `plan`, `build` and `package`) adds
  any other variable a build script reads.
- *Native toolchain.* Only when the host OS family matches the target's: for `*-apple-*` targets on
  a macOS host, the SDK version and SDK build version from
  `xcrun --sdk <sdk> --show-sdk-version` / `--show-sdk-build-version` and the first line of
  `xcrun --sdk <sdk> clang --version` (Apple clang drives the link); `<sdk>` follows the target OS
  (`macosx` for darwin and macabi, `iphoneos`, `iphonesimulator` for `-sim` and x86_64 iOS,
  `appletvos`/`appletvsimulator`, `watchos`/`watchsimulator`, `xros`/`xrsimulator`). For
  `*-windows-msvc` targets on a Windows host, the Visual Studio `catalog_productDisplayVersion` and
  `installationVersion` from `vswhere` under `%ProgramFiles(x86)%\Microsoft Visual Studio\Installer`
  (the variable is matched case-insensitively and defaults to `C:\Program Files (x86)`); the
  developer-shell variables `VCToolsVersion` and `WindowsSDKVersion` are not used, since a plan from
  a plain shell and a build in a developer shell would disagree. A missing tool hashes the
  marker `absent`. Like `rustc -vV`, this identity is the build machine's, so for Apple and MSVC
  targets `plan` must run on the same OS family as the build; from another family it records
  nothing. Each plan row records the planning `host_family` (`macos`, `windows`, `other`), and
  `build` / `package` reject an Apple or MSVC row planned on a different family with a clear error. The C compiler version is not probed:
  put it in `--extra`, for example `--extra cc=$(cc --version | head -1)`.
- *Configuration files.* The SHA-256 of every `.cargo/config.toml`, `.cargo/config`,
  `rust-toolchain.toml` and `rust-toolchain` found walking up from the working directory and from
  the producer directory all the way to the filesystem root (not stopping at `.git`), and of
  `$CARGO_HOME/config.toml` (`~/.cargo` by default). Records are labeled by scope and depth, never
  by absolute path, so the checkout location does not change the digest.
- *Manifest tables.* From the producer workspace root: `[profile]`, `[patch]`, `[replace]`,
  `[workspace.package]` without its `version` key, `[workspace.lints]` and
  `[workspace.dependencies]` (a path dependency's `version` is ignored there). A release can reuse
  an artifact when a version bump leaves all fingerprinted inputs unchanged. If regeneration
  changes a producer's dependency declaration or another hashed file, the plan rebuilds it.
  `env!("CARGO_PKG_VERSION")` inside a reused artifact reports the version it was built at.
- *Closure.* `cargo metadata --manifest-path <producer>/Cargo.toml --locked`, so a producer in
  another workspace uses that workspace's `Cargo.lock`, and a stale or missing lock is an error.
  There is no `--filter-platform`: the closure is a sound superset that includes host-only build
  dependencies. Metadata unifies features across the whole workspace, so a feature change in an
  unrelated member can cause a rebuild; that is over-sensitive, never stale. Normal and build
  dependencies are followed, each with its resolved feature set. Registry and git packages enter as
  name, version, source and `Cargo.lock` checksum. Path packages (the producer, the core crate, any
  engine crate) enter by a digest of their files, without their version; workspace members outside
  the closure do not matter.

The path-package digest uses git when the directory is inside a repository: the blob ids of the
tracked files (`git ls-files -s`) and of untracked files that are not ignored
(`git ls-files -o --exclude-standard`, skipping `target` directories), with modified and untracked
files hashed from their current content, so an uncommitted edit or a new source file counts and
line-ending conversion does not. If git cannot list a directory that has a `.git` ancestor, alef logs
a warning and hashes by walking it instead. Outside git it hashes the content of every regular file
except `target` and `.git` directories. A symlink to a file is hashed by the content of its target in
both modes; a symlink to a directory is not followed. The same tree hashes differently with and
without git, which only costs a rebuild. Files outside the package directory, such as
`include_str!("../../data.json")`, are not covered: keep them inside a hashed package or pass them
through `--extra`.

Pass the same `--extra` and `--env` values to `plan`, `build` and `package`. Only Rust producers are
fingerprinted, so the release-plan commands reject Go components. The standalone build, package
and lock commands still support them.

**Record and lock.** `package --plan` stores the planned fingerprint as `input_hash` in the artifact
record; without `--plan` nothing is recorded. `lock` copies it into each lock entry written to
`--output`. It is optional: older records and locks parse unchanged, and an entry without it is never
reused. It is not covered by the manifest signature. Runtimes older than this field reject a lock
that carries it (the lock structs deny unknown fields), so `lock` strips it from the locks it stages
into binding packages; only the lock at `--output`, the one you keep for the next `plan`, has it.

**Plan.** `plan` prints (or writes with `-o`) `{"schema_version": 1, "debug": false, "rows": [...]}`.
A row records the crate, component, target, configured release version, full input fingerprint,
checkout input fingerprint, and action (`build` or `reuse`).
`previous` is the lock entry to carry over and is present only for `reuse`. A row is `reuse` only
when the previous lock has an entry with an equal `input_hash`, `Download` mode and the same crate,
component, target, feature hash and contract hash; the version may differ. If more than one entry
matches, `plan` fails and names them. Without `--previous-lock` every row is `build`. `plan`,
`build` and `package` must agree on `--debug`.

**Build and package.** `build --plan` and `package --plan` handle only the `build` rows (still
narrowed by `--component` and `--target`). A selected component and target that has no row is an
error (`not in plan`); one that has a `reuse` row is skipped with an info log. Both recompute the
fingerprint and refuse to continue if the inputs changed since the plan. After a successful
`build --plan`, the library gets a sidecar `<library path>.input-hash` holding the planned
fingerprint, the library SHA-256, and the planned version. A rebuild first removes any old sidecar.
`package --plan` checks all three values before packaging the library.

**Lock reuse.** `lock --reuse <previous-lock> --plan <plan>` copies each `reuse` row's entry from
the previous lock verbatim (URL, hashes, signature, key id) without downloading or uploading
anything, and merges it with the freshly packaged records in `--input`, which hold the `build` rows
only. Before writing the lock, it checks every selected row against the current checkout inputs,
including project-local Cargo and rustup configuration. A fresh record must match the planned
version and input fingerprint. It refuses a `--url-template` (reused entries keep their URL); a `reuse` row for a component
or target the crate does not configure or that `--component` / `--target` do not select; an entry that
differs from the one in the previous lock; an entry whose `key_id` is not in the current
`component_distribution.public_keys` (or whose key there differs from the previous lock's); a signed
entry in an `--unsigned` lock; a fresh record with no row in the plan, with a fingerprint that
differs from the plan, or for a pair the plan reuses. Reused entries count toward the
configured-matrix completeness check. When every row is `reuse` no archives are built, so `--input`
may be absent or empty; a `build` row of the plan with no record in `--input` is an error.

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
  segments and must not overlap another producer, a core crate directory, or a `source_manifest`
  directory. These checks include default producer paths and compare directories without regard
  to case, even on case-sensitive filesystems, so a configuration cannot introduce a collision
  when used on a case-insensitive checkout. Changing or removing `producer_dir` leaves the
  previously generated producer crate where it was; delete it by hand.

Both are resolved relative to the directory containing `alef.toml`, not the working directory, so
`alef --config ../alef.toml ...` finds the same files. Every other path in the config, including
`sources` and the core crate location the producer depends on, remains relative to the working
directory; run alef from the project root for a producer in a second workspace to be consistent.

The contract hash no longer includes `trait_path`: it covers the contract name, interface version,
method signatures and the record and enum shapes they reference. A contract trait re-exported
through a different crate path therefore has the same hash, and one contract crate can be
implemented by several engine crates without an aggregator. This changes every previously
computed contract hash (see ADR 0015); relock and rebuild components.
