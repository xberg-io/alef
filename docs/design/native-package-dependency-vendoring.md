# Design: Dependency vendoring for native-source language packages

**Status**: Draft
**Date**: 2026-05-24

## Problem

A binding package that compiles the generated Rust binding from source at the
consumer's install/build time ships a `Cargo.toml` whose dependencies on the core
library's workspace crates are expressed as workspace **path** dependencies, for example:

```toml
core-lib       = { path = "../../../crates/core-lib" }
core-lib-extra = { path = "../../../crates/core-lib-extra" }
```

Those relative paths point into the source workspace. Once the package is unpacked at
its install location (a gem cache, an sdist build dir, a hex package), the
`../../../crates/...` tree does not exist, so `cargo build` fails to resolve the
dependency and the install aborts:

```text
failed to load source for dependency `core-lib`
  unable to update /…/gems/…/crates/core-lib
  failed to read /…/crates/core-lib/Cargo.toml
```

This bites any consumer whose platform has no matching prebuilt artifact and therefore
falls back to a source build.

## Affected packages (by build model)

The defect applies to every package that runs `cargo build` on the shipped source at
the consumer's machine. It does **not** apply to packages that ship a precompiled
artifact (the consumer never invokes cargo).

| Build model | Languages | Affected |
|---|---|---|
| Compile Rust from shipped source at install/build | Ruby (rb_sys), Python sdist (maturin), Elixir (rustler source), PHP (source fallback), Swift (cargo build step), Zig (FFI from source) | **Yes** |
| Prebuilt artifact, no consumer-side cargo | Node (NAPI prebuilds), Go / Java / C# / Kotlin (prebuilt C-FFI lib), Dart (prebuilt FFI) | No |

Prebuilt-primary languages still have a latent source-fallback path; the fix should be
uniform so any source build is self-resolving.

**Version dependencies are sufficient for every affected language** — each ultimately
runs a normal `cargo build`, which resolves a registry (crates.io) dependency. No copy
is required when the core crates are published. This is the size-optimal outcome:
bundling the full transitive Rust tree would be tens of MB and strains registry limits
(for example pub.dev's 100 MB unpacked cap).

## Existing mechanism and gaps

alef already has packaging infrastructure (`src/publish/`, `src/core/config/publish.rs`):

- `VendorMode { CoreOnly, Full, None }` with a per-language `[publish.languages.<lang>]
  vendor_mode`. Defaults: Ruby/Elixir → `CoreOnly`, R → `Full`, else `None`.
- `vendor_core_only` (`src/publish/vendor.rs`): copies one `core_crate` directory into the package and inlines workspace inheritance (`workspace = true` package fields and dependency specs) into the copied crate's manifest.

Gaps that leave the affected packages broken:

1. **Not invoked by the release pipeline.** The consumer-facing package is built directly (a bare `gem build`, `maturin build`, etc.) without going through the
   `alef publish` vendoring/rewrite step, so the un-rewritten path-dep manifest ships.
2. **Single-crate assumption.** `vendor_core_only` copies one `core_crate` and inlines workspace deps into it. When the core library is split across several workspace crates and the binding depends on more than one of them directly, the binding package's own manifest path-deps to the other crates are never rewritten.
3. **Partial coverage.** Only `CoreOnly`-configured languages are handled. Other source-build languages default to `None` and ship raw path-deps.
4. **Copy vs registry.** `CoreOnly` copies crate source; for published core crates, rewriting to registry version-deps (copying nothing) is smaller and simpler.
5. **`Cargo.lock` and release ordering unaddressed.** A shipped lockfile with path sources breaks identically, and registry version-deps require the core crates to be published before the language packages.

## Proposed design

1. **Add `VendorMode::Registry`** (kebab `registry`): rewrite every workspace path-dependency in the binding package's shipped `Cargo.toml` to a registry version dependency (`<crate> = { version = "X.Y.Z", features = [...] }`), stripping `path` and copying nothing. Regenerate or scrub any shipped `Cargo.lock` so it carries no path sources. This is the default for source-build languages whose core crates are published.
2. **Generalize the rewrite to the full workspace-member closure.** Detect every dependency that resolves to a workspace member (from the root manifest's
   `[workspace.dependencies]` / member list) and rewrite all of them, not a single
   `core_crate`. Applies to both `Registry` and `CoreOnly`.
3. **Apply uniformly in `alef publish`.** Make the manifest rewrite a non-skippable step of package assembly for all native-source languages (Ruby, Python sdist, Elixir, PHP, Swift, Zig), and have the release workflows invoke `alef publish` package assembly rather than building the package directly, so the rewrite always runs on the artifact that ships.
4. **Keep `CoreOnly` / `Full` for offline / unpublished scenarios.** `Registry` is the size-optimal default when the core crates are published; `Full` (cargo vendor) remains for ecosystems that forbid network fetches at build (for example CRAN).
5. **Release ordering.** `alef publish` publishes the core Rust crates to the registry before the language packages, and asserts each referenced version exists on the registry as a precondition.
6. **Clean-room source-install smoke test.** In the publish pipeline, install the produced package from source in an environment with no workspace present and assert it builds. This reproduces exactly the failure class above and prevents regressions.

## Phasing

1. Generalize the path-dep rewrite to the full workspace-member closure; add
   `VendorMode::Registry`.
2. Wire the rewrite into `alef publish` package assembly and the release workflows for every source-build language; switch defaults for those languages to `Registry`.
3. Add the clean-room source-install smoke test to the pipeline.
4. Backfill: republish affected packages once the pipeline produces self-resolving manifests.

## References

- Config: `src/core/config/publish.rs` (`VendorMode`, `PublishLanguageConfig`)
- Vendoring: `src/publish/vendor.rs` (`vendor_core_only`, `vendor_full`),
  `src/publish/mod.rs` (per-language `default_vendor_mode`)
