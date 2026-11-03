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
