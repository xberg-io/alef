# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (BREAKING)

- **BREAKING (regenerate required):** the embedded `alef:hash:` stamp is now computed over each generated file's own content alone. Previously `inputs_hash` -- a whole-tree fingerprint over every parsed Rust source plus the canonicalized `alef.toml` -- was folded into every individual file's stamp, so a one-line change anywhere restamped the entire generated tree (measured at 99.3% noise in one consumer regen, and one commit elsewhere that rewrote 8,418 files with exactly one insertion and one deletion each). The generation-inputs fingerprint is now recorded once per crate in a committed `.alef-generation.toml`, and `alef verify` performs two checks instead of one: a content walk for hand-edit detection and a crate-scoped fingerprint comparison for stale-tree detection. `CODEGEN_FORMAT_VERSION` moves 2 -> 3, so every file re-stamps once on the first generate after upgrading; land the pin bump and the regen in the same commit and no consumer observes a red `alef verify`.

### Fixed

- fix(process): kill the real process tree on Windows. `kill_process_tree` was only tree-aware on Unix; the non-Unix arm called `Child::kill`, which ends one process, so a `cmd /C` wrapper died at its deadline while every grandchild it started kept running and kept alef's stdout and stderr pipes open -- the bounded drain in `process::capture` then waited its full 5s `OUTPUT_DRAIN_GRACE` instead of ending at the timeout, on every timeout-driven kill path including hooks, backend build commands, test-app processes and the e2e mock-server shutdown. Windows children are now assigned to a job object at spawn and torn down with `TerminateJobObject`. The process-tree tests are no longer `unix`-gated -- that gap is why the Windows arm shipped as a direct-child kill wearing a tree-kill name -- and now run on both platforms against a real grandchild, with a sabotage control proving a direct-child kill leaves it alive
- fix(snippets): find a Windows FFI library in the zig manifest probe. `directory_has_ffi_library` prepended `lib` to every candidate including `.dll`, a filename no Windows toolchain produces and zig never searches for: its own linker diagnostic names `{name}.dll`, `{name}.lib`, `lib{name}.a`. The probe could therefore never see a real Windows FFI library, so the debug-profile fallback it gates never fired there. The probe now offers the names this host actually links, and the test fixture that had pinned `lib{name}.dll` -- which made the probe and the fixture agree with each other while `zig build` could link neither -- writes the real one
- fix(wasm): decide whether a cfg-gated enum variant exists at generation time instead of emitting a `#[cfg(...)]` attribute on it. `wasm_bindgen` builds its enum AST from the raw item with no cfg awareness and then generates cast clauses naming every variant it saw, so cfg-stripping the declaration afterwards leaves the macro's own output referencing a variant that no longer exists (`E0599`, reported against the line that declares it). Mirroring the conversion arm's cfg onto the declaration remains correct for napi, whose macro never rewrites the item
- fix(codegen): derive "which cfg-gated enum variants exist" from one authority. Two generators answered differently and failed in opposite directions: a foreign (dependency) cfg-gated variant was always assumed maybe-reachable, so a conversion catch-all was emitted even when the binding's configured feature set provably rules the gate out (an unreachable pattern under `-D warnings`), while the napi and wasm wrapper-type generators never read a variant's `cfg` at all, so a host-owned variant's arm carried a guard the wrapper declaration did not mirror (a non-exhaustive match once the feature was on). Both now consult `enum_variant_declaration`, which reuses the existing `cfg_feature_satisfied` evaluator rather than parsing `cfg` a second time
- fix(swift): JSON-bridge a 64-bit `Ok` type instead of panicking. `swift-bridge-ir` has a match arm for every integer width except `u64`/`i64`, which fall through to `todo!()`, and every `Result<Ok, String>` alef emits for Swift reaches it -- so a fallible function returning `Result<u64, _>` crashed `alef generate` outright. Applies in `Result` position only; a bare `u64` return keeps its native type
- fix(python): convert a plain function's return value, not only an adapter's. The converter lookup was widened to the union of public input dataclasses and return-only `TypedDict`s on the adapter path only, so a plain function returning a public input dataclass got the correct annotation and import alongside an unconverted native value -- `api.py` declared the public type and returned the private one
- fix(e2e/python): emit Python that survives a consumer's lint pass untouched. Generated e2e Python was dirty under any ruff config selecting `UP` and `RUF`, so `lint --fix` rewrote it after alef stamped it and the file drifted off its `alef:hash` on every regen, with no stable state. Beyond the deprecated `typing` import, the bulk was ~150 redundant `noqa: S101` directives on emitted asserts; per-statement imports are now hoisted rather than suppressed, and a real isort ordering bug that made `ruff --select I` rewrite the file every run is fixed
- fix(snippets): recognize the missing-library form Apple's current linker emits -- `ld: library '<name>' not found`, which names no `-l` and so matched none of the GNU/classic-Apple/lld patterns. Measured against a real consumer tree, this moved a Go snippet corpus from 140 failed / 0 unavailable to 6 failed / 134 unavailable: a not-yet-built native library is a build-ordering problem, not a broken snippet. The 6 that still fail are the check working -- widening the predicate must never launder a genuine failure into a skip, so `file not found` and an undefined symbol alongside the quoted line both keep their failure
- fix(python): adopt the shared trait-bridge emit gates in the pyo3 backend, the one target of 17 that never did. A bridge with `register_fn` but no `registry_getter` -- a configuration alef warns about but permits -- put a `wrap_pyfunction!` in the `#[pymodule]` body for a `#[pyfunction]` that was never written, so the generated `-py` crate failed to compile; and `exclude_languages` was a silent no-op, because the only filter asked for the `"pyo3"` spelling that `exclude_languages` never accepts. Module init, the `api.py` facade, `.pyi` stubs, marker classes and the reported registration surface now resolve every bridge symbol through one helper
- fix(go,zig): derive C type names through `c_consumer::export_type_prefix` -- cbindgen's real `[export] prefix` shouty-snake-casing -- instead of a local `.to_uppercase()`. For any `[ffi] prefix` carrying an internal word boundary the two disagree (`SAMPLE_CORE` vs `SAMPLECORE`), so Go emitted `C.SAMPLECOREWidget` against a header declaring `SAMPLE_COREWidget`: a cgo build failure across `binding.go`, `visitor.go` and the service API, with Zig failing equivalently on its trait-bridge vtable type
- fix(snippets): recognize LLVM `lld`'s `unable to find library -l<name>` as a missing build artifact, alongside the GNU and Apple linker phrasings already handled, so a consumer linking with `-fuse-ld=lld` no longer sees a not-yet-built library reported as a real snippet failure. The bucketing is now pinned through `finalize_result` rather than the predicate alone, with a genuine compile error asserted to stay a failure
- fix(cli): kill the e2e mock-server's whole process tree on shutdown, not just the direct process, so a descendant it backgrounds can no longer survive `alef test-apps run` as an orphan; the mock server is now spawned into its own process group and registered for Ctrl-C forwarding, matching the tree-kill behavior `setup`/`build` already use
- refactor(cache): drop the `inputs_hash` parameter that the content-only stamp split left dead on `is_lang_cached`/`is_stage_cached`/`stamped_outputs_agree_with_disk`. A parameter named for a fingerprint that no longer participates is how two tests came to assert an already-removed contract, so the signature now states what the code does; six call sites that hashed every source file and `alef.toml` purely to feed the dead argument no longer do that work. Cache staleness is unchanged -- `compute_lang_hash` already folds the config, and tree-staleness lives in the recorded generation fingerprint `alef verify` reads
- test(extract): pin `error_type` resolution in a crate declaring two unrelated error enums, for both an explicit `Result<T, SecondError>` return and a `Result<T>` alias hint declared in a sibling module; the backend tests construct `FunctionDef.error_type` by hand, so extraction was never exercised for this shape
- Dart's post-build native-library staging (`PostBuildStep::StageDartNatives`) now resolves the built artifact through the same profile-aware `find_built_artifact` resolver as FFI staging, instead of hardcoding `release` with no profile parameter -- a debug-only `alef build` could previously stage nothing or, worse, silently stage a stale `release` copy left over from an earlier run.
- Consolidated four independent, hand-rolled "where is the built native library" resolvers (Ruby, Node/napi, PHP, Elixir packagers under `alef publish`) onto `crate::publish::package::find_built_artifact` / `find_built_artifact_with_extra_dirs` / `find_built_artifact_any_with_extra_dirs`, so future fixes to artifact resolution (profile handling, `deps/` diagnostics) apply to every language instead of drifting independently per backend.
- Added `crate::publish::package::PREFERRING_RELEASE_ORDER` as the single declared "release wins, debug falls back" ordering, now shared by `ffi_stage::stage_ffi_preferring_release`/`ffi_artifact_built_preferring_release` and the new `dart_native::stage_dart_native_libraries_preferring_release`.
- fix(wasm): `Vec<Option<T>>` now renders as `(T | undefined)[]` in generated `.d.ts` union types for untagged data enums, instead of the syntactically-wrong `T | undefined[]` (which TypeScript parses as `T | (undefined[])`)
- fix(cli): teach `alef adopt`'s marker table about `.npmrc` (`;` INI comments) so a drifted or pre-existing `.npmrc` is stamped in place instead of silently falling back to the presence-only `.alef-ownership.toml` record (#509)
- fix(php): write a visitor trait bridge's PHP interface to `{TraitName}Interface.php`, matching the `{TraitName}Interface` class the file actually declares -- the previous `{TraitName}.php` filename could never be resolved by a PSR-4 autoloader (#485)
- fix(php): embed alef's ownership marker in both trait-bridge PHP interface files (visitor and registration), which previously carried none at all
- fix(python): forward `#[cfg(feature = "...")]`-referenced feature names into the generated pyo3 crate's own `Cargo.toml` `[features]` table (mirroring ruby/elixir/node/php), fixing `unexpected_cfgs` errors under `-D warnings` when the crate's manifest declared only `extension-module`
- fix(codegen): stop emitting an unreachable `_ => Default::default()` catch-all in enum `From<...>` conversions when every cfg-gated variant is host-owned (its match arm and the source variant share the identical `#[cfg(...)]` gate, so they always compile in or out together); the catch-all is now emitted only when a variant is genuinely absent from the matched type (excluded variants, or a foreign-crate cfg-gated variant whose arm is dropped) — fixes `unreachable_patterns` errors under `-D warnings` in generated pyo3, napi, and php crates
- fix(rustler): stop masking the same unreachable catch-all with a per-arm `#[allow(unreachable_patterns)]` in the flat-enum `From<core>` conversion; the underlying condition is fixed instead of suppressed
- fix(extendr): apply the same host-ownership-aware catch-all condition to the enum conversion generator that landed with an over-broad `has_cfg_variants` check (alef commit eb78151ac)
- refactor(scaffold): centralize the Cargo `[features]` default/forwarding-line formula (`codegen::cfg::cfg_default_and_forwarding_lines`) shared by ruby, elixir, node, php, and (newly) python scaffolds, and share the enum conversion catch-all decision (`codegen::conversions::enum_conversion_needs_catch_all`) between the shared enum conversion generator and the independent napi, php, rustler, and extendr reimplementations of the same rule, so they can't drift apart again
- Fix `FieldResolver::is_array` to fall back to the IR-anchored collection classification (`ir_collection::is_collection_path`) when `[e2e].fields_array` does not name the field, mirroring `is_optional`'s existing IR fallback. Fixes generated Rust/Go e2e assertions calling `.len()`/`*field` directly against a still-wrapped `Option<Vec<T>>` field known only through the IR (e.g. `result.results[0].chunks.len() >= 2`).
- Wire `FieldResolver::with_ir_result_fields` into the Dart, PHP, and Swift e2e generators' per-call resolver construction, matching Kotlin/C#/Java/TypeScript. Without it, `with_anchored_optional_paths` and `is_optional`'s IR fallback were silent no-ops for these three backends, so a leaf `Option<Vec<T>>` field known only through the IR emitted a bare `.length`/`.count` against a nullable collection instead of the optional-safe form.
- Fix `swift_count_target` (Swift e2e generator) to fall back to `is_array`/`is_collection_root` when `SwiftFirstClassMap` has no vec-field data for a leaf, instead of assuming any unrecorded leaf is a bridged scalar. Fixes generated Swift `count_min`/`count_equals` assertions silently counting the characters of a stringified debug dump instead of the collection's real element count.
- Add a positive JSON-bridge check (`FieldResolver::leaf_is_json_bridged_via_swift_map`) that a genuinely JSON-bridged field wins over the IR's `is_array`/`is_collection_root` classification in the Swift e2e generator's `not_empty`/`is_empty` gate. Fixes generated Swift calling `.isEmpty` directly on a bridged `RustString` leaf ("value of type 'RustString' has no member 'isEmpty'").
- `GoValidator::is_dependency_error` did not recognize the linker's missing-shared-library shape (`ld: cannot find -l<name>` / Apple ld's `library not found for -l<name>`), so any Go snippet validated without a prior `alef build` producing the FFI artifact reported as a real `Fail` instead of `Unavailable`. In one consumer's no-build CI run this misclassified 141 of 146 Go snippets as genuine failures. `is_dependency_error` now recognizes both linker shapes (the accompanying `collect2: error: ld returned 1 exit status` summary line is deliberately not matched, since it carries no root-cause signal on its own) but excludes `undefined reference to` / `Undefined symbols for architecture`, which mean the library was found and a symbol inside it is wrong -- a real link defect that must stay `Fail`.
- Fix the TypeScript e2e generator (both the napi/node object-literal path and the wasm setter path) emitting a struct field's wire/serde name where the host public identifier belongs. When a fixture keys a field by its wire spelling and that field's `#[serde(rename)]` diverges from its napi/wasm-bindgen `js_name`, the generated test emitted the wire name and failed at runtime with `Missing field '<jsName>'`. Both paths now resolve the public key through `codegen::naming::to_node_name`, the same formula the backends use to emit `js_name`, instead of running the fixture key through a generic `snake_to_camel`.
- Add `FieldResolver::with_result_is_byte_payload`, the single oracle `is_valid_for_result` / `result_field_oracle_knows` now consult to reject every field path against a call whose declared Rust return type is a raw byte payload (`bytes::Bytes` / `Vec<u8>` / `[u8]` / `[u8; N]`), and wire it into the Rust and Go e2e generators. A byte-returning call's anchored root type is `None` for the same reason it is `None` when no IR was wired in at all, so both oracles' permissive default previously accepted the fixture's declared struct field path against a value that has no fields -- generating `result.audio.is_some()` against `Bytes` (E0609) and `result.Content` against `[]byte`.
- Remove an unused `PathBuf` import in `src/publish/package/php.rs` that failed the build under `-D warnings`.
- fix(wasm): stop `ApiSurface::with_deduped_functions` from tautologically re-enabling a cfg-gated free function's call. Dedup collapses a same-named real-impl/stub pair (mutually-exclusive `feature = "X"` / `not(feature = "X")` gates) into one entry whose `cfg` is the OR of both, which for such a pair is always true -- so the merged entry survived the wasm backend's cfg exclusion regardless of which feature the binding actually enabled, while still calling the *real* variant's deep, feature-gated `rust_path`. `WasmBackend::generate_bindings` now drops any function whose own cfg gate is unsatisfied by the binding's configured features *before* dedup runs, so at most one mutually-exclusive variant ever reaches it.
- fix(cli): `run_command_captured_with_env` -- the runner every per-language backend build command (`cargo build`, `wasm-pack build`, `napi build`, `maturin develop`, ...) goes through -- used `Command::output()`, which blocks reading each pipe to end-of-stream. A leaked descendant that outlives the direct child (a background job started with `&` and never waited on) kept the pipe open indefinitely, so a build that failed after minutes of real compiler output could surface nothing at all. It now streams both pipes on background threads, mirroring each chunk to alef's own stderr live and draining for at most `OUTPUT_DRAIN_GRACE` after the direct child exits, matching the fix already applied to post-build `RunCommand` steps.
- refactor(e2e/go): extract the per-call `FieldResolver` construction out of `test_function.rs` into `test_function/call_resolver.rs`, keeping the over-cap file below its ratchet ceiling.
- fix(zig): a function whose own error type fails to name-match a declared error set no longer silently resolves to the *first* declared set. With two unrelated error enums in a crate, whichever was declared first became the answer for every mismatch involving the other, and the generated decoder then collapsed all of the real error codes into `UnknownFfiError`. A mismatch now resolves to an anonymous `error{OutOfMemory,UnknownFfiError}` rather than claiming a specific wrong identity.
- fix(codegen): enum conversion initializers no longer emit `Self::Variant { field: field }` where the field needs no conversion, which failed `clippy::redundant_field_names` under `-D warnings` (129 occurrences in one generated crate). A third site in the reverse conversion direction had the same root cause; all three now share one helper.
- fix(cli): warn when a declared `workspace.sync.text_replacements` target is left unwritten -- by an ownership refusal, a git-ignored path, an unreadable file, an invalid regex, or a write failure. Previously only the ownership case logged anything, and only a generic "carries no alef marker" line that never connected the refusal to the unfulfilled version-sync contract; the repo was left internally inconsistent in a way that failed a release gate much later, far from the cause.
- fix(cli): the orphan-reclaim bookkeeping diagnostic ("kept output this run, zero manifest entries last run") now fires for every swept root. It previously inspected only roots passed via `disk_scan_roots`, which the scaffold and README/docs/e2e/test-apps sweeps deliberately leave empty, so those sweeps never reported the condition for any cause.
- fix(extract): `exclude.*` entries naming public items that are generic -- and therefore never extracted -- no longer warn that they "did not match any extracted item". That wording implied a typo and invited deleting correct, defensive config, at 76 log lines per run. Entries indistinguishable from a genuine typo still warn.
- feat(cli): `[workspace] auto_update_alef_version` (default `false`) lets a regen update the `alef_version` pin, but only from a clean release build and only upward. The pin is a CI install target that `install-alef` resolves to a release asset, so a regen from a dirty tree never rewrites it.

## [0.71.0] - 2026-08-27

### Fixed

- **Four independent trait-bridge stub defects, one per backend.** Go emitted a bare identifier as
  an enum's default return, which is valid only for a constant-backed enum — a sealed-interface or
  struct-shaped enum has no such constant, so the identifier named a *type* and the compiler
  rejected it as "not an expression"; the default is now constructed according to the enum's real
  Go representation. Java computed its excluded-type set with an empty enum registry, which cannot
  tell a real enum from one the crate's own `exclude_types` marshals as `String`; it now passes the
  IR enum names minus the configured exclusions. Swift skipped default-body methods when stubbing,
  but the production backend declares every trait method as a required protocol member (alef cannot
  carry a Rust default body through the IR), so the stub never conformed — and its
  `import Foundation` heuristic matched only a constructor call, missing a bare type annotation.
  TypeScript gated every import on a plain substring test, so an enum whose name is a *prefix* of
  another correctly-used name was spuriously imported; import gating is now word-boundary aware.

- **Zig snippet validation could not reach a debug-profile FFI library.** Zig snippets are built
  through `zig build` against the consumer's real `build.zig`, whose `ffi_path` build option
  defaults to the release profile. The synthesized snippet build only ever threaded
  `.target`/`.optimize` into its `b.dependency("binding", .{…})` call, and a top-level `-D` cannot
  set an option on a `.path` dependency — so there was no mechanism at all to redirect that path.
  With `alef build` producing a debug artifact, every Zig snippet failed with `unable to find
  dynamic system library`. The validator now resolves the library itself — release preferred,
  debug fallback, never crediting a `deps/`-only copy — and splices the resolved option into the
  dependency call. When neither profile holds a real library it returns no override rather than
  guessing a path.

- **An `Iterate` docs operation resolved its per-item fields against the call's result type
  instead of the collection's element type.** When a per-item field name was *also* reachable as a
  nested path from the result — e.g. `content`, reachable as `results[].content` — the
  result-anchored resolver reproduced that whole path underneath the already-peeled loop variable,
  emitting `for result in result.results.iter() { println!("{}", result.results[0].content) }`.
  That does not compile in Rust and does not typecheck in TypeScript. Because every backend shares
  one presentation layer, the same wrong accessor shipped in Rust, Python, TypeScript and WASM.
  Per-item fields now resolve through an element-anchored resolver.

- **A TypeScript trait-bridge stub declared `Promise<string>` for a struct-typed method.** A stub
  returning a non-enum struct annotated itself `string` and returned a bare `"{}"`, so it could not
  satisfy the interface it was passed to regardless of its body. The stub now names the real struct,
  casts through `as unknown as`, and the import sweep reaches struct names referenced only by that
  cast.

- **`alef build` staged an FFI library from a profile it never built.** `find_built_artifact`
  hardcoded the `release` profile, so a plain `alef build` — which runs `cargo build` and produces
  the *debug* artifact — could not find what it had just built. It instead fell through to
  `target/release/deps/`, whose contents come from whatever other cargo invocation happened to
  compile that crate, under that invocation's feature unification. The staged library was fresh but
  feature-incomplete: linking a consumer program against it failed with dozens of undefined symbols.
  `find_built_artifact` now takes an explicit `BuildProfile` and searches only the two uplifted,
  profile-scoped directories; `deps/` is consulted solely to *name* a rejected copy in the error.
  `StageFfiLibrary` passes the profile the current invocation actually built. Callers with no build
  of their own (`alef generate`'s post-build pass, `alef test`'s e2e staging) try release then debug
  explicitly. Packaging passes `Release`, matching its always-release contract.

- **A snippet run could report success while most of the corpus was never checked at the level it
  asked for.** `RunSummary` now tracks `fully_verified` — results that reached their requested level
  with no downgrade or capability cap — and the summary leads with `Checked at requested level:
  N/Total (P%)`. `alef snippets check` now fails, unconditionally rather than only under `--strict`,
  when not one single result reached its requested level; `alef docs`/`alef all` warn loudly on the
  same condition instead of bailing, because that pipeline cannot guarantee a build ran in the same
  invocation. Related: the Java validator no longer counts a `package does not exist` symbol cascade
  as real failures when the package was simply never built, the "no snippet session configured"
  diagnostic now names the session *target* rather than the language, and Python `typecheck` runs
  the interpreter's own compile check ahead of `pyrefly`, so a hard `IndentationError` can no longer
  pass.

- **Four generated-snippet type defects.** Python field access now narrows an `Optional` before
  subscripting it instead of indexing it bare; the C free-function call site routes omitted optional
  arguments through `resolve_optional_sentinel`, so an IR-declared handle parameter gets the `0`
  sentinel rather than `NULL`; an `iterate` operation with an empty `fields` list renders a fallback
  `print(item)` rather than an empty — and therefore syntactically invalid — Python loop body; and a
  Python adapter wrapper now converts its native return value when `options.py` publishes that type
  only as a return-only `TypedDict`, matching the wrapper's own annotation.

- **Generated output that is not formatter-canonical by construction.** Three emitters produced
  output a consumer's own formatter rewrites: `binding.go`'s stdlib import block was assembled by
  manual insert-position juggling that mis-ordered one real combination (declared errors, no sync
  functions or non-static methods); e2e `main_test.go` had an unsorted import block, two one-line
  `if err != nil { panic(err) }` checks, a one-line `go func() { for … } }()` drain, and
  gofmt-incorrect `+` spacing; and the Elixir `GenServer` template carried a double blank line and a
  pre-joined `when` clause one column past `mix format`'s limit. Each drift let a consumer's
  `gofmt -w` or `mix format` rewrite the file *after* alef hashed and stamped it, permanently
  stranding it outside alef's ownership. All three now round-trip byte-identically through the real
  formatter, asserted by tests that invoke `gofmt`/`mix format` and self-skip when absent.

- **Ownership markers alef itself refused to recognise.** The PHP `install.sh`, R `install.R`, and
  Node/napi e2e `.npmrc` emitters hand-spelled an `alef-generated` marker string that alef's own
  `content_has_alef_marker` guard does not match. All three are `generated_header: false`, so the
  hand-written text was the *only* ownership signal — these files were permanently stranded as
  unowned. They now source their marker from `hash::header`/`hash::STANDARD_HEADER_LINE`, and each
  has a test asserting through the real guard rather than a copied literal.

- **`package_dir` no longer leaks a trailing slash into every path built from it.**
  `ResolvedCrateConfig::package_dir` returned a user's configured `[crates.output]`/`scaffold_output`
  string verbatim, so a trailing `/` produced double-slash paths that `alef adopt` could never match
  against the real on-disk file. Fixed at the source, which protects the ~35 `format!("{pkg_dir}/…")`
  call sites across `scaffold/languages/` and `publish/`; `scaffold_license_files` also now builds its
  `LICENSE` path with `Path::join`.

- **Trait-bridge test stubs now satisfy the interfaces they claim to implement, in four
  backends.** Four independent causes, each a generator re-deriving or hardcoding a fact another
  part of the pipeline already had:
  - Go dropped all four required super-trait methods whenever the super trait was declared in a
    private module and re-exported, because the lookup matched on the extracted `rust_path` rather
    than the configured, publicly visible path. Java had already hit and fixed this; Go now has the
    same synthetic fallback.
  - Java forced every enum-returning method to `String`, because the exclusion helper it used
    cannot see enums (they live in a separate registry). It now uses the enum-aware helper, as C#
    already did.
  - Kotlin-Android fell back to a hardcoded, project-specific variant table and otherwise called a
    bare constructor, which is invalid for both of Kotlin's enum lowerings. It now reads the real
    enum registry and emits `Type.CONSTANT` or `Type.Variant` as appropriate.
  - Dart took the first variant unconditionally and naively lowercased it, producing a constructor
    tear-off when that variant carried fields and the wrong casing regardless.
  The dispatcher also passed an empty enum slice to the Dart and Swift emitters, so their existing
  enum-default lookups could never succeed at all. Where no fieldless variant exists, the stub now
  warns naming the type and language instead of guessing a value the target compiler rejects.

- **Snippet session locks are keyed by fingerprint, not by config name.** `alef.toml` can point two
  differently-named sessions (a language fallback such as `typescript` and an explicit
  binding-package target such as `node`) at the same `cwd` and manifest. They resolve to one
  physical workspace directory but each name got its own `Mutex`, so two batch groups that both
  believed they held the session lock wrote into the same `snippet_batch_N.ts` files concurrently.
  The corruption was worse than the lost work it caused: a file cut mid-token silences `tsc`'s
  semantic diagnostics for **every other file in the same program**, so unrelated real failures
  were reported as passes. Any TypeScript snippet count taken before this fix understates the
  failures.

- **TypeScript snippet checks no longer require `@types/node` to read a file.** The generated
  `await (await import("node:fs/promises")).readFile(...)` form is emitted into every TypeScript
  target, but `tsc` degrades an unresolvable `node:`-prefixed dynamic import to a bare-identifier
  lookup and reports `TS2591: Cannot find name 'node:fs/promises'`. A browser/WASM package with no
  `@types/node` in its graph therefore failed every byte-payload snippet. The validator now writes
  a minimal self-contained ambient declaration into each check, which merges cleanly with a real
  `@types/node` when one is present.

- **Generated wasm TypeScript now constructs the classes wasm-bindgen actually exports.** The wasm
  backend lowers every struct with fields to a JS class with a positional constructor, never a
  plain interface, but four places in the shared node/wasm e2e generator still assumed the NAPI
  object shape: array-typed `json_object` arguments fell through to a bare object literal; the
  transitive nested-class import walk was seeded only from a call's `options_type` and missed a
  class reachable solely through an argument's own fields; trait-bridge stub enum return types and
  casts used the unprefixed IR name the wasm package does not export; and an `Iterate` presentation
  path split on `results[0].` spliced its tail segment in verbatim, referencing a snake_case member
  against a binding that only exports the camelCased one (that last one affected node identically).

- **`alef build` now restages the FFI shared library it just built.** Staging into the Go, Java and
  C# native-library directories only ever ran from `alef test --e2e` and `alef publish`; `alef
  build` rebuilt the cdylib, never copied it, and reported success, so the staged artifact rotted
  silently until a consumer's cgo link failed on symbols that had been added weeks earlier. A
  missing built artifact is now a `tracing::warn!` naming the destination instead of a silent
  no-op. Separately, `find_built_artifact` (FFI staging plus Zig/Go/C#/CLI/C-FFI packaging) now
  also searches each candidate directory's `deps/` subdirectory, because a crate compiled only as
  another crate's path-dependency is never uplifted to `target/release/` and was therefore
  reported absent while sitting in `target/release/deps/`.

- **Generated Go docs snippets now name the error type the Go binding actually declares.** The
  snippet generator used the raw Rust-side `[crate] error_type` value, while the Go backend's own
  error generator strips a leading case-insensitive match of the package name from that same value
  to avoid revive's stutter lint — so a snippet referenced `pkg.SampleCrateError` against a binding
  that declares `pkg.Error`. Both now derive the name through `go_error_type_name` in
  `src/codegen/naming.rs`, alongside a new `go_package_name_from_module` whose empty-module-path
  fallback is now reachable (the previous `split(..).next_back().unwrap_or("binding")` could never
  return `None`, so an empty module path yielded an empty package name).

- **Generated TypeScript no longer splices a raw fixture string or array literal into a
  `Uint8Array` field.** Two call sites each lowered a `bytes` fixture value independently and both
  got the string case wrong: the napi object-literal builder wrapped any value in
  `Uint8Array.from(...)`, which rejects a `string`, and the WASM `default()`+setter builder had no
  string branch at all and emitted a bare quoted string. Both now ask one shared classifier, which
  lowers a file path, inline text, base64 or a number array to the right expression. WASM
  array-of-object arguments with a known IR element type also route through the typed builder as
  node already did, so their elements construct real wasm-bindgen class instances instead of plain
  object literals.

- **Generated Rust docs snippets no longer move out of a plain collection field, and no longer
  `Display`-format a field that does not implement it.** The `Iterate` template appended a borrow
  adapter only when the collection was `Option`-wrapped, so a plain `Vec` field behind an index
  expression was moved out of (E0507); it now borrows in both cases. Separately the per-item
  `println!` chose `{}` vs `{:?}` from the operation-level `display` flag with no reference to the
  field's own type, so a field such as `Vec<Vec<String>>` was formatted with `Display`. Per-item
  fields are now checked individually against an allowlist of `String`/`char`/numeric/`bool`
  primitives and fall back to `{:?}` otherwise.

- **`VerifyFrbBridgeCoverage` no longer passes silently on a gate naming an undeclared
  feature.** A `#[cfg(feature = "...")]` whose feature the sibling `Cargo.toml` never declares at
  all was treated exactly like one declared but left out of `default`, so the function was
  excluded from coverage and the build passed. That is the alef #135 scenario itself: the
  ownership guard refuses to write a forwarding `[features]` entry into a pre-marker-convention
  manifest, the facade gains a gated function the manifest can never activate, and the coverage
  failure was the only signal that would have surfaced the refused write. An inactive gate is now
  excluded only when the manifest declares every feature it names; an undeclared one stays a
  coverage candidate, and the diagnostic names the undeclared feature and the manifest and points
  at `alef adopt <path>` rather than at a stale bridge.

- **Dart FRB `#[cfg]` gates now attach across intervening attributes.** `cfg_gated_free_functions`
  associated a gate with its function only when the `pub fn` sat on the very line after the
  `#[cfg(...)]`, but the generated facade always emits `#[frb]` (or `#[frb(opaque)]`) in between,
  so the gate was never recorded. In a real facade only 3 of ~70 gated free functions were
  followed directly by a signature, leaving ~96% of gates invisible. Two consequences are fixed
  together: `missing_bridge_functions` no longer reports a gated-and-disabled function as a
  missing bridge entry and fails the build, and `CarryFrbCfgGates` now carries the gate into
  `frb_generated.rs`'s wire wrapper and dispatch arm. The scan now skips further attribute lines
  (single- or multi-line) and doc comments, and still declines to attach to an `impl`, a `struct`,
  a private `fn`, or anything past a blank line.

- **`[e2e].fields_optional` is no longer blamed for optionality the IR derived.**
  `with_ir_fields` deliberately merges IR-derived `Option<T>` names into the optional set, but
  `declaring_config_key` then reported `fields_optional` as the source for those names too — so
  the docs-snippet diagnostic told consumers to correct or delete a config entry that was never
  in their `alef.toml`. Config-declared provenance is now tracked in its own set that the merge
  never touches.

- **A fixture path extending past a `fields_method_calls`-covered tagged union now resolves.**
  `result_field_oracle_knows` refused any path crossing a tagged-union field without consulting
  `fields_method_calls`, so a path like `metadata.format.excel.sheet_count` was dropped from every
  generated snippet even though the bindings expose it and the consumer had declared exactly how
  to cross that union. Such a path now resolves against the variant's own payload type. A path
  with no covering entry still refuses, and a segment the IR cannot judge still abstains.

- **A host-owned `#[cfg]`-gated enum variant keeps its match arm and gains a matching
  `#[cfg(...)]` guard.** Generated Rust glue named such a variant unconditionally, so a build with
  the feature off failed with `E0599`. The shared
  `codegen::conversions::{gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding_cfg}`
  hard-coded `cfg => Option::<&str>::None` on every arm even though the
  `enum_from_binding_to_core` / `enum_from_core_to_binding` templates already accepted a per-arm
  gate, which broke napi, magnus, rustler and wasm at once. The same omission is fixed in napi's
  `gen_tagged_enum_binding_to_core` / `gen_tagged_enum_core_to_binding`, rustler's
  `gen_rustler_flat_data_enum_from_core` / `_to_core`, php's `gen_flat_data_enum_from_impls` and
  `gen_string_to_enum_expr`, and pyo3's data-enum `#[getter]` accessors and `#[staticmethod]`
  variant factories in `codegen::generators::enums` — pyo3 needs its own fix because
  `enum_has_data_variants` short-circuits data enums out of the shared conversions path. The
  trait-bridge visitor glue had the same hole one level down: `VisitorResultVariant` carried no
  `cfg` field at all, so the magnus, napi, pyo3, rustler, wasm and php `visitor_method` templates
  emitted an unguarded reference to a gated callback-result variant. Fallback selection is
  corrected with it — a `_ =>` default arm and php's no-default-variant fallback no longer elect a
  cfg-gated variant as the always-available stand-in, and a catch-all arm is now emitted whenever
  any variant is gated so the match stays exhaustive with the feature off.
- **A `#[cfg]`-gated enum variant merged in from a `[[crates.source_crates]]` crate has its arm
  dropped entirely instead of gated.** The generated binding crate never declares a Cargo feature
  for a foreign crate's cfg — `codegen::cfg::collect_cfg_gates` deliberately skips a non-host
  `rust_path` when it builds the passthrough `[features]` table — so re-emitting the gate verbatim
  produces `unexpected cfg condition value` for a feature the consumer cannot activate. Worse, a
  gate of the `any(test, feature = "testkit")` shape is satisfied by `cfg(test)` under
  `cargo clippy --all-targets`, so the arm still compiles and then fails `E0599` on a variant the
  foreign crate was never built with; both were observed in a consumer's PHP crate.
  `codegen::cfg::is_host_owned_rust_path` is the single authority that decides host versus foreign,
  and every emitter now asks it rather than re-deriving the comparison: dart's `From<Mirror>` and
  `From<CoreType>` enum impls, wasm's tagged-enum From impls (which already gated but never asked
  about ownership), php's string-to-enum match, the shared `codegen::conversions::enums` arms, and
  napi, rustler and the visitor-result metadata walk. pyo3 drops both shapes — the accessor arm,
  covered by the existing `_ => None` fallback, and the whole `#[staticmethod]` factory, which has
  no arm to gate around. Every drop is announced through `tracing::warn!`, and php no longer
  advertises a dropped variant as an accepted string value.
- **A type or enum wholly gated behind a Cargo feature carries that gate onto every generated item
  that names its host path.** Dart's `rust_from_core_enum_open`, `rust_from_core_struct_open`,
  `rust_from_mirror_enum_open`, `rust_from_mirror_struct_open`, `rust_opaque_wrapper_struct` and
  `rust_from_json_bridge_fn` templates were each passed a `source_cfg` and each ignored it, so a
  build excluding the feature hit `E0433` on a module path that does not exist. The mirror struct
  and enum declarations stay unconditional, since their fields are widened FRB-native types rather
  than the host path; only the impls and functions that name `core_ty` verbatim are gated. On the
  FFI side, `gen_enum_free`, `gen_enum_to_json`, `gen_enum_to_string`, `gen_enum_from_json` and the
  private `from_i32_rs` reconstruction helper never threaded `EnumDef::cfg` into their templates
  the way `gen_type_free` / `gen_type_new` already threaded `TypeDef::cfg`, so an enum defined
  inside a gated module got unconditional accessors — `E0433` for exactly the consumer that
  declares the feature via `[crates.ffi].extra_features` without enabling it by default.
- **A stripping Jinja tag no longer welds the following generated line onto a `//` comment.**
  `trim_blocks` eats the newline after a tag and `{%-` eats the one before it, so a source line
  followed directly by a stripping tag in `generators/enums/enum_definition.jinja` lost its line
  ending and the next emitted line was appended to it. Where that line was a comment, the comment
  swallowed an entire `if let ... {`, leaving its closing brace unmatched, and a consumer's
  generated PyO3 crate did not parse. Every expected fragment was still textually present, just
  commented out, so no `contains()` assertion could see the defect; the regression test parses the
  output with `syn` instead.
- **Generated e2e tests and doc snippets unwrap an `Option<Vec<T>>` field reached through an
  array-projected path.** `FieldResolver::ir_field_sets` only ever proves a *bare* field name
  optional, by unanimity across every declaration of that name in the crate, while the
  `_with_optionals` accessor renderers key their per-segment unwrap check by the full cumulative
  path walked so far. A bare name therefore never matched once the path crossed more than one
  segment, so `entries[0].sections[0]` and `entries[0].sections.len()` rendered against the
  `Option` unguarded — `E0608` and a missing method in Rust, an unguarded `.first()`/`.size` on a
  nullable receiver in Kotlin, and the equivalent in the other backends. Every per-call resolver
  now calls `with_anchored_optional_paths` over the fixture's own assertion field paths, resolving
  them through the IR's real `(owner_type, field_name)` walk the way `presentation.rs` already did
  for doc snippets: rust, dart, kotlin, php, csharp, java, swift, typescript and zig. Kotlin needed
  a second wire as well — its resolver never called `with_ir_result_fields`, leaving
  `ir_result_field_map.root_type` at `None`, which makes `with_anchored_optional_paths` an
  unconditional no-op whatever paths it is handed.
- **Swift trait-bridge protocols are visible to code that imports only the umbrella module.**
  `Swift{Trait}Bridge` protocols are emitted into `Sources/RustBridge/`, so a doc snippet that
  wrote `class Foo: SwiftEmbeddingBackendBridge` after `import <Umbrella>` alone failed with
  "cannot find type ... in scope". `gen_bridge_registration_overloads_file` now emits a
  `public typealias Swift{Trait}Bridge = RustBridge.Swift{Trait}Bridge` per configured bridge,
  following the same per-symbol re-export idiom the main module file already uses for opaque handle
  types rather than a blanket `@_exported import`. The SwiftPM compile gate gained a third
  `DocsSnippet` target that depends only on the umbrella module, reproducing the failure under a
  real `swift build`.
- **wasm resolves a core type's real module path for static and instance calls.** `gen_method`
  composed `{core_import}::{type_name}` from the bare IR name, which only works for a type
  re-exported at the core crate root; a type living under a private module produced
  `{core_import}::T::default()` and rustc rejected it with "cannot find `T`", even with the type's
  gating feature enabled. It now uses `core_type_path`, the existing shared authority that walks
  `TypeDef::rust_path`.
- **magnus no longer synthesizes an `impl Default` it cannot satisfy.**
  `gen_struct_default_impl_explicit` emitted a whole-struct `Default` as soon as any one field
  carried its own default (a single `#[serde(default)]` was enough), then filled every remaining
  required field through the untyped `default_value_for_field` fallback, which renders
  `{Type}::default()` for a `Named` field whether or not that type implements `Default`. A struct
  with a required field of a non-`Default` type failed to compile with "no function or associated
  item named `default` found". The already-computed `default_types` set is now consulted per field,
  and the whole impl is skipped when a required field cannot be satisfied.
- **The Java e2e stub always implements a super-trait bridge's `name()` and `version()`.**
  `trait_interface.jinja` declares both abstract unconditionally whenever a bridge configures
  `super_trait`, but the e2e stub derived them by matching `TraitBridgeConfig::super_trait` against
  the super-trait `TypeDef`'s `rust_path` and silently emitted neither when the lookup missed — as
  it does for a super-trait declared in a private module and re-exported via `pub use`, whose
  `rust_path` need not equal the configured value. Both sides now read the same
  `trait_bridge_naming::SUPER_TRAIT_REQUIRED_METHODS` list.
- **C doc snippets derive trait-bridge register/unregister/clear symbols even for a
  fixture-level-skipped fixture.** `resolve_fixture_call_info` gated symbol derivation on
  `fixture.skip.languages`, but that directive opts a fixture out of the executable harness only —
  the docs-snippet generator renders a skipped fixture regardless — so the naive,
  already-populated `call.function` config text was left uncorrected and the snippets called a
  pluralized symbol the generated header never declares, without its trailing `out_error` param.
  Derivation is now gated on the call-level `skip_languages`, the same authority the harness and
  the docs generator's own inclusion filter already use for "this language cannot represent this
  call at all".
- **wasm doc snippets import nested classes the snippet body reaches only through the IR.** The
  standalone snippet import builder considered only the manually configured `nested_types` map,
  unlike `render_test_file`'s builder, which also derives nested classes transitively via
  `collect_transitive_nested_types_for_wasm`. A call with no `nested_types` override — the common
  case — could still emit a nested `SomeClass.default()` construction through
  `ts_builder_expression_inner`'s own IR-derived lookup, leaving the snippet referencing an
  undeclared symbol and failing to typecheck.
- **Hand-authored `docs.shows` and `docs.presentation.operations` paths are validated against the
  IR.** Only assertion-derived paths went through the existing existence check (`shows_on_result`),
  so a stale or misspelled field name in authored docs config reached every snippet backend's
  compiler identically. An iterate block's per-item fields are now checked against the collection's
  own element type, resolved through a newly anchored `ir_collection_map`, rather than against the
  call's result type, and `result_field_oracle_knows` refuses a path that continues past a field
  the IR knows it cannot walk into as a struct (the tagged-union/enum shape) instead of falling
  through to a permissive flat check. A field with no `Named` resolution at all — a
  `serde_json::Value` or other scalar, where continuing the path is unjudgeable rather than
  impossible — is tracked separately and still accepted, so a `document.payload.anything` accessor
  keeps deriving as before. Only the IR may refute an authored path: the
  `[e2e].result_fields` allow-list is incomplete by construction, so a `has_ir_result_evidence`
  gate keeps it from dropping the deliberately-documented virtual and namespaced paths an author
  writes `docs.shows` for in the first place.
- **A failed pipeline command reports its own output, not just its exit status.**
  `run_run_command` (post-build `RunCommand` steps, including the Swift `cargo build` step) and
  `run_shell` (the per-language e2e format override, including the default rust `cargo fmt --all`)
  both reported a bare exit code, so `'cargo' exited with status 101` gave no hint that the real
  cause was a macOS linker fixup error and an e2e formatter failure carried no diagnostic at all.
  `run_run_command` now tees both streams through `process::capture::output_reader_tee`, which
  mirrors each chunk live so a long build still looks alive while capturing it, and quotes the last
  ~4KB of each stream on failure; `run_shell` moves from `Command::status()` to `Command::output()`
  and quotes both streams the same way `run_command_captured_with_env` already does.
- **A snippet session key spelled exactly like its language wins claim resolution when another
  candidate corroborates it as a deliberate alias.** `resolve_session_claim` reported a target-less
  snippet as ambiguous whenever its language had a genuinely different-directory second session,
  even when one candidate was a bare-language-named key aliased onto another, already-present
  candidate's own working directory. `alias_default_claim` lets the exact name win only when a
  differently-named candidate already shares its directory; a standalone exactly-named candidate
  still resolves as ambiguous, unchanged.
- **The Dart FRB bridge-coverage check no longer reports a `#[cfg]`-gated facade function as a
  stale bridge.** `flutter_rust_bridge_codegen` expands against the dart Rust crate's own default
  features, so a facade function behind a feature that is not in `default` is correctly absent from
  the generated bridge — but `missing_bridge_functions` was a plain line scan with no `cfg`
  awareness and counted every one of them as a function frb had failed to bridge, failing the dart
  post-build stage on a bridge that was in fact freshly and correctly generated. It now filters on
  `cfg_feature_satisfied` against the feature set read from the facade's sibling `Cargo.toml`
  through the existing `codegen::cfg::read_default_enabled_cargo_features` seam, and falls back to
  the old unfiltered check when that manifest cannot be read rather than suppressing coverage
  silently.
- **That check's failure message states what was observed instead of asserting one cause.** It
  previously claimed `flutter_rust_bridge_codegen did not (re)generate this bridge`, which is one
  of several explanations and was the wrong one in the case above. It now reports the facade
  functions that have no bridge counterpart and lists the causes it cannot distinguish between.
- **Hand-authored `docs.shows`/`docs.presentation.operations` field paths are now validated against
  the IR before rendering**, matching the check already applied to paths derived from `assertions`.
  A fixture-authored typo or stale field name now drops the operation (falling back to
  assertion-derived shows when every authored operation is dropped) instead of emitting a
  non-compiling accessor identically across every generator that shares the snippet/e2e
  presentation layer (Rust, Dart, Java, Swift, Kotlin, TypeScript, WASM, and the rest).
- **A docs/e2e presentation path that continues past a field the IR can confirm is not a struct it
  can walk further into (the tagged-union/enum shape) is now refused** instead of silently falling
  through to a permissive flat check that let the accessor renderer emit a plain field access into
  an enum variant.
- **An `Iterate` operation's per-item `fields` are now validated against the collection's own
  element type**, resolved from the IR, instead of the call's result type. A per-item field name
  that does not exist on the iterated element (e.g. a renamed struct field) is now dropped from the
  operation instead of reaching every backend's snippet compiler.

- **C#, Zig, Dart (`style = "ffi"`) and Kotlin/Native now consult the result-presence companion.**
  A scalar `Option` return crosses the C ABI as a bare scalar, so absence and a legitimate zero are
  the same bytes. C# matched `TypeRef::Optional(_)` unconditionally and emitted
  `if (nativeResult == 0) { return null; }` against an `int64_t`, which also shadowed the wrapper's
  error check so a genuine FFI failure surfaced as `null`. Zig and Kotlin/Native passed the raw C
  value through and let the language coerce it into an optional as non-null. Dart's `dart:ffi`
  typedef declared `Pointer<Void>` where the FFI crate exported `int64_t` — and read `Option<bool>`,
  which crosses as `i32`, as an 8-byte pointer. The `ConsumesCabiNotYetWired` ledger in
  `backends::result_presence_stance_tests` is now empty. The default Dart `frb` style is unaffected.
- **Go calls the trait-bridge register symbol the FFI backend actually exports.** The FFI backend
  names it `{prefix}_{register_fn}` from the bridge's configured `register_fn`; Go composed
  `{prefix}_register_{trait_snake}` from the trait name, so any bridge whose `register_fn` spelled
  anything else linked against a symbol exported nowhere.
- **A return of `Option<Option<SomeType>>` declares a handle on the C side.** The FFI backend
  declared `*mut c_char` for that shape while its own emitted body handed back `insert_handle(..)`
  and its absent branch handed back the handle-shaped `0` — three answers to one question, of which
  only the declaration reached the header and the consuming backends.
- **Go names the trait registry in the configured unregister wrapper.** It rendered
  `unregister_c_call.jinja` without `trait_snake`, so the undefined value resolved to the empty
  string and the template emitted a bare `Registry.delete(name)` — an identifier the generated
  package never declares.
- **napi, wasm and php honour `exclude_languages` for trait bridges.** All three emitted the bridge
  wrapper struct and the register/unregister/clear entry points regardless, so a consumer writing
  `exclude_languages = ["wasm"]` still got the bridge. Each backend's emitter, options-field wiring
  and reported registration surface now read one shared predicate. The napi gate honours both
  `"node"` and `"napi"`.
- **php, magnus and rustler no longer emit host wrappers for a trait absent from the API surface.**
  The Rust-side bridge emitter skips such a bridge; the host-side pass did not, so PHP emitted
  wrapper methods forwarding to `crate::<register_fn>`, magnus emitted
  `define_module_function("<register_fn>", …)`, and rustler emitted Elixir delegates calling
  `<AppModule>.Native.<fn>` — each naming a symbol no pass generated. `native.ex` likewise declared
  NIF stubs for those bridges. All now ask the same lookup.
- **php and magnus type stubs no longer declare bridge entry points the bindings skip.** The
  `.stub.php` and Ruby RBS emitters listed a bridge's methods off `trait_bridges` alone.
- **extendr's `extendr_module!` no longer registers a registration function that was never
  generated.** `collect_trait_bridge_functions` wired `register_fn` into the module macro without
  checking `registry_getter`, while `gen_registration_fn` writes no `#[extendr] pub fn` without one
  — a Rust compile error.
- **pyo3: a public wrapper is annotated with the return type `options.py` publishes**, and converts
  the native value into it, instead of being annotated `-> _rust.<Name>` — the private extension
  module's `#[pyclass]`. Under the `typed-dict` output style the wrapper's annotation named a
  different type than the one a consumer imports under the same word.
- **pyo3: the keyword-omission unpack is no longer emitted for a field `options.py` never nulls.**
  A bare `#[serde(default)]` enum field renders as a literal default and can never be absent, so the
  unpack was dead — and a type checker resolves an unpacked keyword against every remaining
  parameter, costing one error per pair (three such unpacks in one constructor call produced six).
- **`alef setup` and `alef build` kill a timed-out command's whole process group**, not just the
  `sh` wrapper, so a `sh -> gradlew -> daemon` tree no longer outlives its deadline and reparents to
  PID 1. The drain that follows is bounded by a 5s grace rather than reading to end of stream, which
  a descendant holding the inherited pipes never reaches; the captured helper also drains
  concurrently with the wait, so a command that fills the OS pipe buffer no longer can only end by
  timing out.
- **Generated Rust no longer trips `redundant_field_names`, `collapsible_if` or
  `vec_init_then_push`** under a consumer's deny-level clippy. Struct literals use field-init
  shorthand where the value is exactly the field identifier; the FFI `*_free` wrappers, the pyo3 DTO
  alias helper and enum discriminant branch, the napi/wasm/extendr/php visitor result branches and
  the Dart FRB loader build script use let-chains; the extendr and rustler visitor-context pair
  lists are `vec![]` literals. `extra_clippy_allows` remains available for consumer-owned code.
- **e2e validator diagnostics are reported once per crate** rather than once per render pass.

### Changed

- **Timed pipeline commands are spawned into their own process group** and registered for
  termination forwarding, so Ctrl-C still tears the whole tree down. Forwarding delivers `SIGKILL`,
  matching the snippet validators. Untimed pipeline commands stay in the foreground group and are
  unaffected. The process-group lifecycle moved from `src/snippets/validators/` to a crate-level
  `src/process/` module so both paths share one implementation.
- **Service-API and trait-bridge C symbols are spelled in one place, `codegen::c_consumer`.** Both
  the FFI emitters that export them and the Go cgo call sites that consume them derive their names
  from it. Templates receive whole symbols rather than fragments to interpolate. The service
  family's two derivations agreed on every input, so that half is a drift guard, not a behaviour
  change.
- **`register_fn` without `registry_getter` warns at config resolution.** Every backend's
  registration emitter needs a registry and emits nothing without one (the C FFI backend panics), so
  the combination silently produced no registration function anywhere.
- **A trait bridge skipped because its trait is absent logs a `WARN`**; one skipped because
  `exclude_languages` names the target logs at `DEBUG`, since that is an honoured request rather
  than degradation.

### Removed

- **The unreachable `KotlinJvmBridgeGenerator`.** A Kotlin/JVM consumer calls the generated Java
  bridge class directly, so it emitted nothing reachable.

## [0.70.0] - 2026-08-26

### Changed (BREAKING)

- **`E2eCodegen::render_snippet_body_with_functions` no longer has a default implementation.** The
  default discarded the function registry, and the one backend that never overrode it
  (`kotlin_android`) silently dropped every field of a call's result as a result. Making the method
  required turns forgetting it into a compile error. **Migration:** any out-of-tree `E2eCodegen`
  implementation must now implement this method explicitly; the previous default's body is
  equivalent to ignoring the `functions` argument.
- **Generated Go and Java wrappers change shape for a function returning `Option<scalar>`.** They
  now consult a presence companion before trusting the returned value (see below). Go call sites
  that relied on the returned pointer being non-`nil` will now correctly receive `nil` for an
  absent result; Java call sites will now correctly receive `Optional.empty()`. Code that treated
  the old always-present value as meaningful was already reading a fabricated zero.

### Fixed

- **A direct `Option<scalar>` return can now distinguish `None` from `Some(0)`.** Such a return
  crosses the C ABI as a bare scalar, so absence and a legitimate zero were the same bytes. The C
  ABI now exports a `{fn}_has_result` presence companion, and Go, Java and Kotlin/JVM consult it
  before trusting the value. Go's defect was worse than ambiguity: its wrapper built a pointer that
  could never be `nil`, so every `None` arrived as a real `Some(0)`; Java's `Optional.of(result)`
  could never be empty, so `None` reached the facade as `Optional[0]`. Whether a companion exists
  is asked of `ffi::type_map::result_presence_companion_exists` — the same predicate that decides
  whether the symbol is exported — so a host binding can never reference a companion the FFI crate
  never emitted. This includes the deliberate owned-receiver exclusion, where the companion's second
  call would find the handle already consumed. **C#, Zig and the opt-in Dart `ffi` style are
  audited but not yet wired**; a stance ledger over every `Language` now pins that set so a new
  backend cannot compile without declaring where it stands.
- **A foreign crate's `#[cfg]` no longer leaks into generated code.** `codegen::cfg` deliberately
  excludes foreign-crate features from Cargo feature forwarding — forwarding them names a feature
  the core crate does not define and breaks resolution — while variant emission copied the same cfg
  verbatim, so the two halves disagreed. Both now ask one authority,
  `is_host_owned_rust_path`. A foreign-owned cfg-gated variant's arm is dropped with a named
  warning rather than emitted behind an undeclared feature. Host-owned variants keep their gate
  unchanged. Swift's `__alef_{enum}_from_swift_string` helper turned out to have carried no cfg
  guard at all, for host or foreign variants — an unguarded reference to a variant that may not
  exist — and now gates host variants and drops foreign ones.
- **alef built only on Rust 1.95 or newer while advertising 1.85 and declaring 1.88.** Three e2e
  assertion emitters used `if let` guards, stable only from 1.95, and `rust-toolchain.toml` pins a
  far newer toolchain, so no CI job ever compiled the crate at its declared floor. Installing
  0.68.0 from crates.io failed with `E0658` on any toolchain below 1.95. The guards are rewritten
  (each was the first arm of its match with a `_` pattern and a returning body, so an early
  `if let` is equivalent), the README now matches `Cargo.toml`, and a new `msrv` CI job compiles at
  the version it reads from the manifest.
- **Ownership records that predate the committed manifest are now migrated when queried, not only
  when written.** A path whose ownership predates the committed `.alef-ownership.toml` and whose
  content never changed could live only in the gitignored `.alef/scaffold-owned-paths.manifest`
  indefinitely; clearing `.alef` then lost the record and alef refused to regenerate the file,
  having no durable proof it had ever written it. One consumer hit this for 48 outputs. Every
  ownership-gated write and `alef verify`'s frozen-file scan already query every unmarkable managed
  path, so an ordinary run now migrates the whole legacy ledger before the cache can be cleared out
  from under it.
- **Compile validation is bounded.** A `before` hook that wandered into a pathological state ran
  until a human killed it — one consumer interrupted a Gradle build after 34 minutes. Diagnostics
  are now truncated head-and-tail with an explicit dropped count rather than streamed in full;
  Swift's module-path resolution, which spawned `swift build --show-bin-path` with no deadline and
  no process-group teardown while every sibling subprocess had both, is now bounded like the rest;
  and a new `docs.snippets.before_timeout_secs` lets a package build have its own budget instead of
  sharing one number with every individual snippet compile. Truncation is never silent.
- **`alef adopt` no longer costs more than the problem it recovers from.** Recovering 48 paths
  emitted ~124k tokens. Target matching compiled a fresh glob per candidate path, classification
  and diff rendering repeated per target, and the ownership manifest was read-modify-written once
  per target. One session is now shared across every target of an invocation, and under
  `--converged-only` — where a drifted file cannot be adopted at all — diff bodies are bounded, with
  the withheld files still named and counted. Without `--converged-only` nothing is bounded, because
  there the diff is the consent document for a write the command performs. Which files are adopted
  and which create-once seeds are refused is unchanged.
- **Go generated the wrong C symbol for any name that defeats snake-casing.** Go composed
  `{prefix}_{type_snake}_{method_snake}` while the FFI backend exports through
  `c_consumer::method_symbol`, which leaves the method component verbatim — so `parseURLPath`,
  `utf8Length` and `_Internal` linked against symbols that are exported nowhere. Go now asks
  `c_consumer`. Separately, `Option<Duration>` had no arm at all in the return lowering and fell to
  a catch-all emitting `unmarshalU64`, a helper the generated package never declares, and
  `Option<Option<T>>` declared one more pointer level than its expression could produce.

### Added

- **Every language reference page now documents how to register a trait-bridge plugin.** `Backend`
  gained `trait_bridge_registration_surface`, implemented by 16 backends, and the docs layer asks
  the backend rather than restating naming. Six templates were parameterised so the emitted name
  and the documented name come from one place and cannot drift; each backend has a test asserting
  the reported surface names a symbol the generated output actually declares. Kotlin/JVM and JNI
  deliberately report nothing — the former emits no registration function at all today, and the
  latter's `Java_..._nativeRegister*` shims are an ABI the Kotlin/Java side links against rather
  than an API a consumer calls.
- **Docs-only e2e fixtures**, for documentation content with no single-call shape — configuration
  discovery, standalone pipelines, multi-step handoffs. Every API reference in such a fixture is
  resolved against the real surface, so a renamed field fails the run, but the fixture is never
  executed or generated into test code. A docs-only fixture is structurally unable to be counted as
  runtime coverage: it is a separate type with no conversion into `Fixture`, and it publishes under
  its own slug.
- The generated Android project enables the Gradle build cache. The configuration cache is emitted
  commented out, with the reason: the generated `buildAndroidJniLibs` task reads `gradle.taskGraph`
  from inside `onlyIf` and assigns `System.err` to `Exec.errorOutput`, both of which Gradle 9
  rejects by failing the build rather than degrading.

### Removed

- Four enum conversion arm functions with no callers.

## [0.69.0] - 2026-08-26

### Fixed

- `alef docs` parsed `docs.snippets.required_languages` through a fence-tag-only parser while `alef snippets gaps` parsed the same key through a session-target-aware one. An entry of `node`, `wasm` or `kotlin_android` was therefore accepted by one command and rejected by the other, so `alef all` aborted with `unknown language: node` on a config its own sibling command had already validated. The resolver is now a single authority in `snippets::types` that both call sites use. The abort also short-circuited docs/snippet validation, so it was masking every finding behind it.
- **napi, wasm: a payload-bearing variant on a default-representation enum silently dropped its payload.** An enum with no `#[serde(tag/content/untagged)]` — for example `enum Label { A, B, Custom(String) }` — was emitted by napi as a `#[napi(string_enum)]` with a bare `Custom,` variant and by wasm as a plain C-style `Custom = 1,`, discarding the field in both. Both backends now route such enums through the same discriminated-object emitter an explicitly tagged enum already used, so the payload round-trips in both directions. pyo3 was already correct, which is why Python preserved payloads while Node and WASM did not.
- **Generated DTOs for pyo3, napi, magnus, rustler and extendr now deserialize a container-level `#[serde(from/into/try_from/transparent)]` struct by delegating to the core type's own `Deserialize` and converting via `Into`.** The derived field-by-field object `Deserialize` silently disagreed with the real wire shape, which is commonly a positional array. Delegation makes no positional assumption; a struct whose fields cannot round-trip (an unwrapped opaque field, a sanitized non-`Cow` field, a cfg-gated field) falls back to the derived impl rather than guessing. wasm and the seven FFI-derived backends were already unaffected. Serialize symmetry is deliberately unchanged.
- **The C ABI now emits a `has_<field>` presence companion for every optional struct field whose return type has no null representation** — `Option<i32/u64/f32/f64/bool/…>` and `Option<Duration>`. Previously `None` and a legitimate zero-valued `Some` both returned the same `0`/`0.0` sentinel with no way to tell them apart, so a field meaning "explicitly disabled" was indistinguishable from one meaning "apply the default". Pointer-shaped types already had a real null and are unchanged.
- **PHP now throws, naming both the field and the offending value, when a string-backed enum field does not match a known variant.** It previously substituted the default or first variant, which a consumer's own core-side `validate()` then ran against and could not detect — an unknown value became a real, plausible, wrong one.
- **Swift's generated enum-from-string helper returns `Result` instead of panicking.** An unrecognised wire string used to `panic!` inside `__alef_<enum>_from_swift_string`, unwinding across the swift-bridge FFI boundary, which is undefined behaviour. Every call site now propagates the error, forcing an otherwise-infallible wrapper's return type to `Result<_, String>` so the failure has somewhere to go.
- Generated binding↔core conversions no longer silently drop `Vec` elements that fail to (de)serialize. A failed element keeps its slot instead of vanishing and shifting every later index — a shrinking collection is undetectable from the output shape, while a preserved slot is at least positionally honest.
- `alef snippets check` no longer tells you to run `alef build` for a language that has no `docs.snippets.sessions` target configured at all. That advice was false — no build could change the result — and it was why running `alef build` and then `alef snippets check` produced byte-identical unresolved-dependency counts. The rollup now separates "No session configured" from the real build-ordering case, with distinct remediation for each.
- Fixed-size arrays of primitives (`[u8; N]`, `[f64; N]`) now resolve losslessly to `Bytes`/`Vec<T>` instead of being sanitized to a lossy `String` placeholder. `resolve_type` had no `syn::Type::Array` arm at all, so every fixed array fell through to a stringified `Named` type.
- Sanitized public-API diagnostics are now driven by the sanitizer's recorded rewrite rather than by pattern-matching the `String` placeholder, so a field genuinely declared `String` is never conflated with one rewritten to `String`. Sanitized parameters also record `original_type` for the first time, which several backends already gated on and which had therefore been inert.
- The Zig reference pages document `[]const u8`/`[]u8` at the wrapper boundary instead of a DTO's Rust type name, by asking the Zig emitter rather than re-deriving the shape. The same mapping error had Zig strings documented as `[:0]const u8` — a sentinel-terminated slice that appears nowhere in generated output — for parameters, returns and struct fields alike.
- **A trait implemented from a foreign crate no longer contributes methods to the binding surface.** An `impl SomeFramework::Trait for Config { fn schema() … }` — written to serve OpenAPI generation, a serializer, or any other tool — had its methods lifted into the public binding API, where they sanitized lossily and aborted generation. The trait filter was a denylist of std traits, so every other trait passed. A fully-qualified trait path rooted in a crate that is neither the one being extracted nor any crate contributing a type to the surface is now foreign. A single-segment path (an imported trait) is deliberately unchanged: resolving it depends on module visit order and would drop real methods.
- **The C ABI `from_i32` reconstruction helper now carries a variant's `#[cfg]`.** A variant behind `#[cfg(feature = "…")]` does not exist in a build without that feature, so an ungated match arm naming it was a hard compile error in the consumer's crate. Discriminants stay reserved, so numbering is stable across feature subsets.
- **C# no longer declares a scalar optional return as `IntPtr`.** A function returning `Option<u64>` is exported by the FFI crate as a raw `u64`; C# declared the same symbol as `IntPtr`, read the integer bit pattern as a UTF-8 string pointer and passed it to `FreeString` — an arbitrary-address free. The pointer-vs-scalar decision now has one owner in `ffi::type_map` that C# asks; the private copy it replaced had already drifted on `Option<Option<Duration>>`. **A direct `Option<scalar>` return still cannot distinguish `None` from `Some(0)`; a presence channel for that position is not yet implemented.**
- `kotlin_android` documentation snippets no longer silently drop every field of a call's result. It was the one language backend without a `render_snippet_body_with_functions` override, so it fell to a trait default that discards the function registry; the call then anchored to an unrelated struct sharing its name and the field oracle correctly rejected everything beneath it.
- Generated reference pages no longer corrupt a URL followed by punctuation. `Proxy URL (e.g. "http://proxy:8080", …)` rendered as `<http://proxy:8080\",>` because quotes were not excluded from the autolink match. Balanced parentheses and trailing slashes are still treated as part of the URL.
- Cross-page reference links are no longer hardcoded to a `.md` suffix. New `[docs].reference_link_style` selects `suffixed` (default, unchanged) or `extensionless` for documentation sites that route without file extensions.
- A fenced code block naming a language alef does not generate bindings for — `astro`, `mdx`, `hcl` — no longer fails documentation validation. It is reported at warning severity instead, naming the tag, so a typo like `pythn` stays visible rather than passing silently. A tag that claims a real binding target and still fails to resolve remains an error.
- **MCP prompts and resources constructed at runtime can now be declared in configuration.** `docs.mcp.declared` covers surfaces built through calls like `Prompt::new(…)` rather than declared by attribute, which attribute extraction cannot see and therefore reported as nothing missing. Attribute-derived surfaces win on a name collision, and every dropped duplicate is reported once with a count. Consumers who declare nothing see identical output.
- Sanitized method and function parameters now record `original_type`, mirroring the field path, so a diagnostic can name the original Rust type. Several backends already gated behaviour on that value and had been inert for parameters.

### Added

- A regression test proving that the functions `alef generate` actually consults to decide skip-vs-regenerate report a cache miss for byte-identical inputs once the compiled-in alef version changes. Prior coverage only compared two cache keys in isolation and never tied that difference to the real on-disk read path.

## [0.68.0] - 2026-08-25

### Changed (BREAKING)

- **Generated bindings for a `&mut` DTO parameter now return the updated value.** A core function with a `&mut T` parameter on a non-opaque (serde DTO) type was emitted as an owned by-value parameter returning void: the binding converted the caller's object into an owned intermediate, mutated the intermediate, and dropped it. The call compiled, raised nothing, and silently did nothing observable, in Python, Node, PHP, Go, Java, Kotlin, Dart and Swift. The wrapper now returns the mutated value in all eight.
- **Migration.** For a core signature `fn tag_record(record: &mut Record)`, assign the result back over the value you passed in — Python `tag_record(record)` becomes `record = tag_record(record)`; Node `tagRecord(record)` becomes `record = tagRecord(record)`; PHP `tagRecord($record)` becomes `$record = tagRecord($record)`; Go `err := TagRecord(record)` becomes `record, err := TagRecord(record)`, where `record` is now a `*Record`; Java and Kotlin `tagRecord(record)` become `record = tagRecord(record)`; Dart `await tagRecord(record)` becomes `record = await tagRecord(record)`; Swift `try tagRecord(record: record)` becomes `record = try tagRecord(record: record)`. A call site that ignored the previously-void result was already silently broken and needs the assignment added. No call shape keeps working unchanged.
- **Generation now fails, naming the function**, for the two `&mut` DTO shapes a binding has no room to express: more than one `&mut` parameter, and a `&mut` parameter on a function that already returns a value. Both previously emitted a binding that accepted the argument and discarded the mutation. Change the core signature to return the updated value itself, or fold both results into one returned type.
- Unchanged by design: a `&mut` parameter on an opaque handle type still mutates through the handle, which was already correct; and `&mut` on `String`, `Vec<T>` or a scalar still surfaces as a compile error in the generated Rust rather than a silent no-op. Neither shape was ever silently lossy.

### Added

- `alef build --strict`: fail the run when a language was skipped because its toolchain is not on `PATH`, naming each skipped language and the precondition that failed. Off by default (a missing local toolchain still leaves the rest of the build clean); pass it in CI so a skipped-and-never-built language surfaces as a non-zero exit instead of a log line nobody read.
- Added unit tests pinning `is_valid_for_result`'s intentional permissive/anchored asymmetry directly at the `FieldResolver` layer (`src/e2e/field_access/resolver/classify.rs`), so a future accidental over-anchoring of the permissive check is caught by `cargo test --lib` without needing the full presentation-layer suite.
- `alef e2e snippets-migrate` and its coverage driver gained regression coverage for two `curated_snippets` path-resolution edge cases: an `existing_root` equal to the configured `snippets.output` now compares correctly, and a bare `*` glob that crosses a `/` into alef's own generated output is refused by name.
- Kotlin e2e assertions now lower a tagged-union field path (`<union>.<variant>.<field>`) for ANY single-payload variant the IR resolves, not only the one hand-maintained fixture shape, narrowing via a real `when (val v = …) { is <Union>.<Variant> -> { … } }` block on both the `kotlin` and `kotlin_android` targets. The payload property name is computed from the IR through `kotlin_field_name_with_type` — the same helper the Kotlin binding backend itself uses — so it can never drift from the emitted binding. Detection reuses `FieldResolver::tagged_union_split`, the generic primitive Gleam/Dart/Swift already consult; `FieldResolver::union_variant_payload` is new.
- `FieldSkip::UnionTraversalNotImplementedForKotlin` (`GeneratorGap`): a tagged-union boundary Kotlin detects but cannot yet lower (a multi-field variant, or a union type the IR never anchored) now emits a loud, named, counted skip instead of silently falling through to a flat accessor chain against a sealed class — code that does not compile.
- `TypeDef` now records a struct's container-level serde conversion (`serde_container_conversion`, holding `from`/`into`/`try_from`/`transparent`), read from `#[serde(...)]` including through `#[cfg_attr(...)]`. These attributes were previously parsed for no purpose — extraction read only `serde_rename_all` — so a struct with a hand-written wire shape (commonly a tuple or array for a small value type) generated an object-shaped binding DTO that silently failed to round-trip at runtime.
- New `ValidationCode::SerdeContainerConversionUnsupported`: a struct carrying any of those attributes now raises a named diagnostic instead of quietly generating a binding whose JSON shape disagrees with the core type's real one. Deliberately `Warning` severity — it never aborts a build, because the remedy today is to exclude the type — and scoped to the languages it actually affects (pyo3, napi, magnus, wasm, rustler, extendr, which re-derive their own local binding struct). The FFI-derived backends (Go, Java, C#, Dart, Swift, Kotlin, Zig) deserialize through the core type's own serde impl and are unaffected, so it does not fire for a consumer targeting only those.
- A declared `since` that names a release newer than the crate's own version now raises a Warning (`since_newer_than_crate_version`) naming the item, the declared `since`, and the crate version it exceeds; an unparseable `since` raises a distinct `since_version_unparseable` rather than passing silently. Both `#[alef(since = "...")]` and `#[deprecated(since = "...")]` are checked, across all seven item kinds that carry version metadata. Comparison uses `semver::Version::cmp_precedence`, not the derived `Ord` — the latter orders build metadata (`1.2.0+build > 1.2.0`), which the SemVer spec forbids from affecting precedence.
- Added `codegen::mut_writeback`, the single policy module every backend consults to decide whether a `&mut` parameter needs writing back, which type the binding must return in place of `()`, and which `&mut` shapes are unsupported. Backends no longer each answer that question their own way; the generated Rust reference asks it too, so the docs cannot describe a signature the binding does not emit.

### Changed

- `validate_call_arg_signatures` (unknown fixture arg / missing required parameter) is now `Severity::Error` and aborts e2e generation; a consumer-fleet survey found zero legitimate call sites it would have flagged.
- `validate_call_module_overrides`'s Go check (a bare-word `overrides.go.module`/`module` is never a resolvable Go import path) is now `Severity::Error` and aborts e2e generation. The equivalent Java check (a `module` override that looks like a class, not a package) stays `Severity::Warning`, since no consumer in the surveyed fleet currently sets that field.
- `kotlin/discriminated.rs::render_discriminated_union_assertion` takes the sealed-class variant's payload property name as a parameter instead of assuming a literal name; existing callers pass the previous literal unchanged, so behavior for the hand-maintained fixture shape is identical.
- The Python `api.py` facade and the `<module>.pyi` stub now derive parameter existence, order and optionality from one shared decision (`backends::pyo3::py_signature`) instead of re-deriving it independently, so the two artifacts cannot drift apart. New agreement tests render both from one fixture and assert identical parameters in identical order, in both the required and the defaulted direction.
- Extracted the free-function delegation predicate that the WASM, NAPI and shared function generators each need into `codegen::generators::can_auto_delegate_function_with_named_let_bindings`, replacing two byte-identical private copies.
- A `String`/`Bytes` parameter the source declared by value is no longer documented as `&str`/`&[u8]` on the Rust page; the borrow forms are emitted only when the IR records a borrow.
- Added `backends::php::layout::{php_class_output_dir, php_psr4_target}` as the single authority for where the PHP userland classes live. The php backend, the scaffolded root `composer.json` and the e2e `composer.json` now all read it instead of each re-deriving the directory, so the root and e2e PSR-4 targets cannot name two different trees. The root manifest also honours `[crates.php.stubs] output`, which it previously ignored while the backend wrote the classes there.
- `FieldResolver::result_relative_path` returns `Cow<'_, str>` instead of `&str`: the envelope projection it can now prepend is a computed path, not a slice of its input.

### Fixed

- `.ai-rulez/skills/binding-audit/SKILL.md` and the (now-removed, folded into that skill) `binding-audit-pattern` rule documented a grep for intentional binding-removal attributes that matched only `#[alef::skip]` and `#[doc(hidden)]`. The extractor (`src/extract/extractor/helpers/attributes.rs:304-333`) accepts three spellings — `#[alef::skip]`, the list form `#[alef(skip)]`, and either nested in `#[cfg_attr(...)]` (the form in common use, e.g. `#[cfg_attr(alef, alef(skip))]`) — so the documented grep missed the dominant real-world spelling and would misclassify a correctly-excluded item as a binding gap. The grep now matches all three spellings.
- Ruby e2e snippet and spec generation no longer double-prefixes an `options_type` (or adapter `request_type`) that already names a module. Both generators share one constructor builder (`ruby/args.rs::build_args_and_setup`), which unconditionally prepended the call's module regardless of whether the configured name already carried one, turning e.g. `"Sample::DocumentRequest"` into `Sample::Sample::DocumentRequest.new(...)` in both outputs identically. `values::qualify_ruby_type` now prepends the module only when the name has no `::` already, matching how `csharp`/`go` take `options_type` verbatim.
- Fixed a false-positive `field's #[serde(default)] value disagrees with its #[derive(Default)]/impl Default value` warning that fired on genuinely agreeing enum-typed fields whenever the enum's declaring source file was extracted after the struct's manual `impl Default`; the disagreement check now runs once, after the whole crate is extracted, instead of inline per source file.
- Fixed spurious `impl Default body is neither a struct literal nor a constant-foldable delegation; field defaults are unresolved` warnings for zero-field types whose `impl Default` delegates to an argument-free `Self::new()` returning a bare `Self`; the resolver now recognizes this shape directly.
- Suppressed the same unresolved-default warning for types already excluded from every binding surface (`#[alef(skip)]` in any recognized spelling, or `#[doc(hidden)]`), since an excluded type's fields never reach codegen and the warning was pure regen-log noise. Non-excluded types with a genuinely unfoldable default still warn.
- `alef e2e snippets-migrate`: fixed `--config` pointing at a config file outside the project making every project-root-relative `curated_snippets` glob resolve against the wrong directory; the project root is now the process working directory, matching how the same globs are already resolved at generation time.
- **pyo3**: the `api.py` constructor-call converter now derives keyword-argument names by calling `resolve_param_ident` (the same function the `.pyi` `__init__` stub and the real `#[new]` signature use) instead of re-deriving them separately; a field carrying `#[serde(rename = "type")]` (a keyword in both Rust and Python) previously emitted `_rust.T(type_=...)` in `api.py` while the native stub and constructor both used bare `type`, causing a `pyrefly` `[unexpected-keyword]` error.
- **pyo3**: `Vec<StructType>` fields now convert element-wise in `api.py`'s `_to_rust_*` converters instead of passing the raw list straight through to the native pyclass constructor, which `pyrefly` flagged as `[bad-argument-type]`.
- **pyo3**: an already-`Option<Enum>` field no longer routes through the kwarg-omission trick in `api.py`'s converters, which made `pyrefly` cross-assign the two enum argument types between the two parameters when a constructor had two such fields; it now passes `None` directly via a ternary. The omission trick remains for fields that are non-optional in the native binding and rely on a real, non-`None` Rust-computed default. Test coverage for all three converter fixes above was added to the `pyrefly_generated_package_tests` fixture, closing the gap that let them ship with the `pyrefly` gate reporting clean.
- `validate_call_args`'s `binding_excluded` skip was language-blind: it skipped argument-signature validation for any function/method the IR marked `binding_excluded`, even when a resolved language (Rust) still emits a real, positionally-bound call against it, making a wrong arg name on such a call structurally undetectable. The validator now skips only when every resolved language in the current run agrees the call is excluded.
- `validate_call_module`'s Go precedence check missed a rung the generator actually consults: a named call with no override of its own falls back to the base `[e2e.call]`'s Go override before falling to `[go].module`, so a bad base override could be silently used by every named call's generated snippet while the check, run per named call, reported nothing. The validator's doc comment describing the Go resolution order had the same gap and was corrected alongside the fix.
- Fixed the e2e snippet coverage driver treating every adapter-handled method (`[[crates.adapters]]`) as excluded from every non-Rust language's documentation snippets, even though every backend that consumes adapters still binds the method in every language except the ones the adapter's own `skip_languages` names. Coverage now re-derives the per-language answer from the adapter config instead of trusting the language-blind exclusion flag, so an adapter-handled method with an unaffected language stays in `coverage.expected` and still renders.
- Generated Java `pom.xml` is now emitted in poly's canonical XML style (2-space indent; a multi-attribute root `<project>` tag wrapped one attribute per line), ending the `alef generate` → `poly fmt` oscillation that rewrote the file on every regen. The `<developers>` and capsule `<dependency>` blocks moved out of raw Rust `format!` string assembly into `java_pom.xml.jinja` loops over structured context values, per the jinja-templates rule.
- Generated FFI `*-ffi-config.cmake` likewise matches poly's canonical CMake fixed point, and its body moved out of a raw multiline `format!` into a new `ffi_config.cmake.jinja` template. The canonical shape was verified empirically against already-canonical consumer files rather than assumed.
- Pinned `poly_would_reformat`'s probe subprocess to a stable cwd via `spawn_from_stable_dir` instead of a bare `Command::new("poly")`, fixing a flaky `post_build_format_order_tests` failure under full-suite parallel `cargo test --lib`. `poly` resolves its config/repo-root context from the process's ambient current directory, not from its argument paths, and this crate's tests share one process-wide cwd via `CwdGuard` around tempdirs they delete — so an unpinned spawn could inherit an already-deleted directory and report "would reformat" for content that is genuinely canonical.
- Anchor the four `chunks_have_content` / `chunks_have_embeddings` / `chunks_have_heading_context` / `first_chunk_starts_with_heading` synthetic e2e assertion handlers at the call's configured `result_fields` envelope prefix (with an IR-confirmed index hop for a `Vec<T>` prefix) instead of hardcoding `{result_var}.chunks`. A result type that wraps its documents behind an envelope had every chunks-recipe assertion silently dropped across all eleven backends implementing the recipe, because the oracle backing the handlers only ever asked whether the bare `chunks` name resolved directly on the call's own root type. `FieldResolver::anchor_leaf` (`src/e2e/field_access/leaf_anchor.rs`) is new and strictly additive: it tries every `result_fields` prefix the IR confirms reaches the leaf before agreeing with a refusal, and the existing refusal for a genuinely unreachable field is unchanged and still fires.
- Fixed the WASM backend emitting `compile_error!` stubs for free functions whose only non-delegatable parameters are non-opaque `&Named` or `&[Named]` references, producing a generated `lib.rs` that could not compile. Such functions now delegate to the real core call, matching the binding the NAPI backend already generated from the same IR — WASM was gated on a predicate stricter than its own delegation body requires. The deliberate `compile_error!` fallback for genuinely non-delegatable functions is untouched and still fires.
- Fixed the generated Python `api.py` facade silently reordering parameters. A parameter whose type derives `Default` was promoted to `= None` and moved behind every required parameter, so the facade's positional order no longer matched the Rust source, the native `#[pyo3(signature = ...)]`, the `.pyi` stub, or the generated docs — a positional call bound its arguments to the wrong parameters, with no type error. The facade now preserves declaration order and grants the extra default only when every later parameter is already defaulted.
- Render the Rust API reference from canonical Rust instead of the binding-normalized IR. A `&T` or `&mut T` parameter is now documented as a borrow in the signature, the parameter table and the generated example, and a method's receiver comes from `MethodDef::receiver` instead of an unconditional `&self` — so a `&mut self` method is no longer documented as `&self` with a non-compiling example. The IR already carried `is_ref`/`is_mut`/`receiver`; the renderer was ignoring them. Binding pages are unchanged.
- Preserve the declared Rust type of a struct or enum-variant field whose named type is not part of the binding surface. `sanitize_unknown_types` previously rewrote such a leaf to `String` with no record of what it was, so a Rust-only excluded field was documented as `Option<String>`; the pre-sanitization type is now recorded in `FieldDef::original_type` and rendered on the Rust page.
- Fixed the generated `e2e/php/composer.json` (and the registry-mode `test_apps/php` manifest) autoloading a `src/` subdirectory of the PHP class tree that no alef stage writes. The e2e generator appended a fixed `/src/` to the resolved package root, so any layout whose `[crates.output] php` path did not already end in `src` sent Composer to an unmanaged directory — which only resolved while a duplicate copy of the class tree was kept beside the managed one.
- Resolve a qualified `Result` path in a return type against the alias it names rather than against the module's `use` statements. A function returning `crate::Result<T>` from a module that also imports `anyhow::Result` for its internal helpers resolved its `error_type` to `anyhow::Error`, and the Zig backend then fell back to the first declared error set and emitted the wrong error union. `crate::`, `self::`, `super::` and uniform paths into a locally declared module now select the alias they name; a qualified path naming another crate resolves to that crate's error instead of borrowing the local one. An unqualified `Result<T>` is unchanged. A parameterised alias with a default (`type Result<T, E = CrateError>`) now resolves to the default instead of recording the bare parameter name `E`.
- Replaced consumer-specific fixture identifiers in the rustler backend's tests (`sync_functions.rs`, `cfg_dedup/tests.rs`) with neutral ones (`score_pair`, `SampleVector`, `sample_crate::vector_ops`, `scoring`/`scoring-presets`), matching the neutral convention the sibling wasm regression test already uses. Test-only rename; no behaviour change.
- Fixed e2e accessors dropping a real nested struct hop on an envelope-shaped result root. With a `result_fields`-declared projection (`results: Vec<Document>`), `FieldResolver::result_relative_path` read a genuinely nested `metadata.output_format` as a virtual namespace label and emitted `result.output_format` — a member the root does not declare. The generic path now asks `FieldResolver::anchor_leaf` (the prefix search added for the synthetic `chunks` handler) where an envelope-rooted path lives instead of carrying its own copy, so both paths resolve one fixture field to one place.
- `alef publish`'s PHP manifest validation derived its expected PSR-4 targets from hardcoded `src/` and `packages/php/src/` literals — a fourth independent opinion on a directory `backends::php::layout` is the authority for — and so rejected any correctly-configured non-default PHP output layout. Both expectations now come from the authority (`php_psr4_target` for the root manifest, the new `php_package_psr4_target` for the package-local one, which relativizes against the manifest's own directory as Composer does). The pre-existing test for this validator was itself asserting a target that disagreed with what the scaffolder generates for its own config, and was corrected.
- A struct field, method parameter, or return type declared as a fixed-size array of a named type the binding surface already carries (`[Point; 4]`) now lowers to the same typed list shape `Vec<Point>` produces, instead of being sanitized to a JSON `String` and failing the run on `lossy_sanitized_surface`. Serde gives a fixed array and a `Vec` the identical sequence wire form, so the rewrite is lossless and the declared length is preserved in `original_type`. The existing fallbacks are untouched: a fixed array of a type outside the binding surface still sanitizes to `String`, and the `[(K, V); N]` tuple-array shape the wasm backend reconstructs from still takes its JSON-string path.
- The C# reference page described an API shape the emitted binding does not have: free-function examples called a bare, un-suffixed method name, when every C# free function is emitted as a `public static` member of the generated wrapper class and every async member carries an `Async` suffix. Examples now call the wrapper class through `codegen::naming::csharp_wrapper_class_name`, and `docs::naming::csharp_async_member_name` is the single rule both `docs::signatures` and `docs::examples` consult, replacing two independently duplicated copies.
- The C/FFI reference page documented `Vec<T>` as a typed handle array and `HashMap<K, V>`/`serde_json::Value` as `void*`. The C ABI declares one JSON `const char*` for all three regardless of element type (`FfiParamMapper`/`FfiReturnMapper` in `backends/ffi/type_map.rs`), so a batch-of-handles parameter was documented as something the header never takes. Two existing tests had pinned the wrong `void*` spelling and were corrected.
- The Rust reference page now shows a field's real declared type whenever the sanitizer recorded one in `original_type`, instead of only when the rewrite was also lossy. Without this, the lossless fixed-size-array lowering above (`[Point; 4]` to `Vec<Point>`) would have reintroduced the "binding shape, not canonical Rust" defect on the one page that exists to avoid it.
- Enum variant conversions that JSON-round-trip a sanitized or `Json`-typed field — rustler and magnus data-carrying binding enums, pyo3 and extendr variant constructors — swallowed a parse failure and substituted `Default::default()` with no diagnostic at all. They now emit a `tracing::warn!` naming the field, the variant, and the offending value first. The conversion stays infallible, so every existing `.into()` call site is unaffected.
- `alef snippets check --strict` failed on generated reference pages with `unknown fenced code language: no_run`. Bare `no_run`/`ignore`, multi-attribute combinations such as `rust,no_run,should_panic`, and `rust,edition2021` are all valid rustdoc fence shorthand, but `is_rust_code_block` and `audit_fences` each matched the fence info string against a fixed set of literal combinations rather than parsing it, so anything outside that set fell through unrecognised and leaked the raw fence into generated pages. `Language::from_fence_info` is now the single parser both call sites use: split on commas, strip the documented rustdoc attribute vocabulary, and treat the remainder as Rust when it is empty or `rust`.

### Removed

- Removed the static `enforce_build_dependency` pre-flight gate from `alef docs`/`alef all`'s snippet validation, which bailed under `strict` whenever a language had no configured `docs.snippets.sessions.<target>.before` step, even when that language's own validator builds the snippet from source without needing one. `alef snippets check` never called this gate, so the two commands could reach opposite pass/fail verdicts for the same corpus; both now rely solely on the same empirical validation results (`enforce_snippet_summary`). The gate's strict-bail message also pointed operators to "run `alef build` first", advice the gate's own doc admitted could never change its verdict since it read only static session config; that dead end is gone with the gate.

## [0.67.6] - 2026-08-25

### Added

- `Codegen::cfg::expand_configured_features`, which resolves a configured feature list through the core crate's own `[features]` table (transitively, skipping `dep:` and `crate/feature` tokens) and falls back to the list verbatim when the core manifest cannot be read. The JNI shim generator uses it for both its default-target feature set and each per-target override, so gate evaluation agrees with the manifest alef itself scaffolds.
- `BuildAndroidJniLibs` derives its target list from `[crates.kotlin_android] abis` (the same list that scaffolds the `jniLibs/<abi>/` directories) and its manifest from `[crates.jni] crate_dir`, so the directories alef creates and the directories alef fills cannot name two different sets.
- Added: `buildAndroidJniLibs` derives its target list from `[crates.kotlin_android] abis` (the same list that scaffolds the `jniLibs/<abi>/` directories) and its manifest from `[crates.jni] crate_dir`, so the directories alef creates and the directories alef fills cannot name two different sets.
- Added: `codegen::cfg::expand_configured_features`, which resolves a configured feature list through the core crate's own `[features]` table (transitively, skipping `dep:` and `crate/feature` tokens) and falls back to the list verbatim when the core manifest cannot be read. The JNI shim generator uses it for both its default-target feature set and each per-target override, so gate evaluation agrees with the manifest alef itself scaffolds.
- Add `[e2e.call(s).*.overrides.java] module` validation: warns when the value's last dot-segment starts with an uppercase letter (looks like a Java class, not a package) — the reported regression that produced `import io.xberg.Xberg.*;` in generated snippets.
- Add `[e2e.call(s).*] module` / `overrides.go.module` validation: warns when the effective Go import path (override, then `[go].module`, then the base field) is a bare word with no `.` or `/`, since only the standard library resolves that way and this field never names it.
- Add fixture `args` vs. IR signature validation: warns when a fixture's effective `args` (its own, or its resolved call's) name a parameter the Rust function/method signature does not declare, or omit a required parameter with no default. Resolves through the same `CallIr`/`TargetParams` seam e2e codegen already uses for argument type lowering, so it silently no-ops when the call is unresolvable or the resolved function is `binding_excluded` rather than claiming a false positive.
- Both new checks land as warnings only, not errors — see `src/e2e/validate_call_module.rs` and `src/e2e/validate_call_args.rs` doc comments for the consumer-fleet measurements behind that choice.
- `alef snippets audit` accepts `--config` and gained a curated-versus-generated accounting pass: a snippet under an audited root that no coverage ledger records as generated and no `curated_snippets` declaration claims is reported as `UnaccountedSnippet` (warning), a declared file is reported positively as curated, and a declaration that claims a path alef generates is an error. The pass is named as skipped, rather than silently omitted, when `--config` is unset or no coverage ledger records anything as generated.
- `alef snippets check` carries the same accounting through its configured audit pass.
- Added `[crates.e2e.snippets].curated_snippets`: glob patterns (relative to `output`) declaring hand-authored snippet files as curated on purpose rather than alef-generated. Resolved into `SnippetGenerationReport::curated_paths` and into `migration::MigrationEntry::curated`, so both the generation report and `alef e2e snippets-migrate` can distinguish a declared, intentional absence of a generated equivalent from a genuine coverage gap.
- Implemented `render_snippet_body` for the brew (shell) e2e code generator: documentation snippets for CLI-based bindings now render a single `binary subcommand "<url>" --flags` line, built from the same call-config resolution the executable brew e2e suite already uses.
- Add `[crates.verify].ignore_ephemeral`, a glob-pattern opt-out so `alef verify` never reports intentionally ephemeral, gitignored generated output (e.g. registry-mode `test_apps/`) as a permanent "missing generated files" failure; every excluded path is still counted and reported in `alef verify`'s coverage output.
- Added `[crates.e2e.snippets].sample_base_url`: the public base URL generated documentation snippets bind for a fixture's `mock_url` / `mock_url_list` arguments. It is documentation-only — the executable e2e suite keeps binding the per-fixture mock server — so a project can publish snippets a reader can actually run without changing what its tests talk to. Relative fixture paths (`"/pdf/report.pdf"`) resolve against the mock server for tests and against the configured host for docs, from the same fixture, with no per-fixture edit. An explicit `$mock_url` placeholder resolves against it too.
- Add `[crates.node].excluded_default_features`; `scaffold_node_cargo` now drops excluded names from both the wrapper's own `[features] default = [...]` array and the core dependency's explicit `features = [...]` line, matching the fix already shipped for Ruby/Swift/Dart. Same defect: a `target_dep_overrides` entry excluding a feature for one cfg target was defeated by the wrapper's own unconditional default forwarding.
- Add `[crates.elixir].excluded_default_features`; `scaffold_elixir_cargo` fixed the same way.
- Add `[crates.php].excluded_default_features`; `scaffold_php_cargo` fixed the same way. The function-gated feature set PHP must always request (`php_function_gated_core_features_to_add`) is deliberately NOT filtered against the exclusion -- those are hard compile-time requirements of an unconditionally-emitted function, not a default-features convenience.
- Add `[crates.ffi].excluded_default_features`; `effective_ffi_default_features` (the single derivation `scaffold_ffi` and `warn_on_ffi_feature_drift` both read) now excludes these names from both the FFI crate's own `[features] default = [...]` list and the core dependency's explicit `features = [...]` line, while still declaring them so `cargo build --features <name>` keeps working.
- Added: a warning when a field path declared in `[e2e].fields`, `fields_optional`, `fields_array`, `fields_method_calls` or `result_fields` is refused for a target because that target's result type declares no such member. The warning names the field, the target language and the config key that declares it. Paths nobody declared — assertion groupings, streaming pseudo-fields, virtual namespace prefixes — stay silent, and no target fails its build over a per-target shape difference.
- `alef snippets gaps` now prints a gap-coverage report on every run — snippet roots and files discovered, documentation roots and pages actually opened, references found versus supplied by configuration, and required languages against snippet groups compared — so a "No gaps found." result can no longer read as a wider claim than the check made. A consumer that omitted `required_languages`, `docs_dirs` and `include_base_paths` from its `alef.toml` previously read a clean gap report for a run in which the language-parity check never executed and not one documentation page was opened.
- `alef snippets gaps` now names every unset input (`docs_dirs`/`--docs`, `required_languages`/`-L`, `include_base_paths`/`--include-base-path`) together with the check class its absence disables.
- `alef snippets gaps` gained `--strict`, which fails the run when an unset input left a check class with nothing to compare, so a CI job whose purpose is gap detection cannot go green by being unconfigured. An unset `include_base_paths` is reported but deliberately not strict-fatal: it makes include targets over-report rather than manufacture a false clean.
- **`alef verify` now reports its own coverage on every run.** Every finding verify produces is a negative claim, so a green result was indistinguishable from a run that examined nothing -- and consumer CI reads it under job names like "Alef-generated bindings freshness" as a whole-tree freshness gate. It is a far narrower claim: only files carrying an alef marker on disk are held to a hash; markerless generated output (`.json`, `.jar`, lockfiles) is checked for PATH PRESENCE only, so a present-but-wrong file passes; and anything outside the ownership walk's scan set is never opened at all. Each run now prints the managed surface split into content-verified / present-but-not-content-verified / absent, the files opened versus never examined, unmarked create-once seeds, and marked files the surface does not claim. Follows the `alef snippets audit` precedent of naming the check class a run skipped instead of printing a bare clean result.
- Added: `[crates.ruby].excluded_default_features`, mirroring `SwiftConfig`/`DartConfig`. `scaffold_ruby_cargo` previously forwarded every `collect_cfg_features` name into the generated wrapper crate's `[features] default = [...]` array unconditionally, which re-enabled a feature a `[crates.ruby].target_dep_overrides` entry excluded for a specific `cfg` target one layer down (Cargo unions feature requests across every dependency edge to the same resolved package regardless of target). The excluded name stays declared (so `cargo build --features <name>` keeps working) but is dropped from `default` and from the core dependency's own explicit `features = [...]` line.
- The gating itself is unchanged, and is now pinned by tests rather than argued from doc comments. For an unmarkable seed (`LICENSE`, `mvnw`, `gradlew`, `.gitkeep` -- paths `marker_comment_style` answers `None` for), `alef adopt --write --clobber-create-once-seeds` writes no byte of the file: `stamp_for` yields `None`, so the entire adoption is one entry in the committed `.alef-ownership.toml`. That entry is precisely what `write_scaffold_files_report` accepts as proof of ownership for an unmarkable path (`owned = has_marker || (!is_markable && is_owned_by_ownership_record(..))`), so the adoption is what clears the guard for the next overwriting write. Five tests in `cli::commands::adopt::tests::create_once_seeds` measure the bytes on both sides of the adoption and both sides of the write, including a control proving the identical `overwrite: true` write refuses when the adoption did not happen.
- Added regression coverage: `tests/cache_stage_hit_requires_intact_outputs.rs` (mirrors the existing per-language tamper-detection test for the stage cache) and `bin_cli::all_commands::tests::all_a_cache_hit_run_does_not_delete_its_own_manifested_binding_output` (drives `alef all` twice over the same fixture and asserts the second, cache-hit run does not delete the first run's binding output).
- Add `the_emitted_cgo_preamble_defines_exactly_the_effective_ffi_default_features`, which reads the `-D` tokens back out of the Go file the backend actually writes and compares them against `effective_ffi_default_features`, so a re-introduced Go-local derivation fails a test instead of shipping a preamble that disagrees with the library. Its fixture exercises a configured passthrough feature, a feature discovered only from a `#[cfg(feature = ...)]` gate, and a declare-only `extra_features` name, each pinned by a control assertion.
- regression coverage: `src/e2e/codegen/assertion_recipes.rs` and `src/e2e/codegen/rust/assertions.rs` add an `Envelope { results: Vec<Document> }` / `Document { chunks }` fixture and assert both directions — a root type that genuinely declares `chunks` still renders the real assertion, and a root type that only reaches `chunks` through a different IR type does not.
- Add `generate_formats_a_scaffold_manifest_changed_by_a_config_only_edit` to `src/bin_cli/core_commands/format_scope_tests.rs`, proven to fail against pre-fix code and pass against the fix.
- **e2e/java**: added javac-backed regression coverage proving a generated doc snippet whose fixture carries a plain (non-`json_object`) `string` argument over the JVM's 65535-byte `CONSTANT_Utf8` cap actually compiles, not just renders without one long literal. The underlying fix (`java_string_literal` chunking, task #180) was already merged; this closes the gap where only rendered-text assertions existed for the call-argument path, and includes a sanity test proving javac itself rejects an unchunked 100,000-byte literal so a pass is evidence the compiler ran.
- Added `tests/java_kotlin_generate_build_dir_agreement_test.rs`, which runs the real `JavaBackend`/`KotlinBackend` generators and the real `build_command_config_for_language` resolution and asserts the build command's target directory is an ancestor of the directory sources actually landed in, for both the unconfigured default and a `[crates.output]` that moves the tree outside `packages/<lang>`.
- Add snippet/e2e cross-generator agreement tests plus direct unit tests (declared key survives, undeclared key refused, `serde_flatten` types exempted) for the new filter.
- Added regression tests in `src/e2e/codegen/typescript/test_file/json_object_field_agreement_tests.rs` asserting a declared nested key survives and an undeclared nested key is refused identically by both `render_snippet_body` and `render_test_case`.
- Added: `src/bin_cli/tree_state.rs`, the classifier, compiled by `build.rs` via `#[path]` and by the crate normally, so `cargo test --lib` exercises the shipped code instead of a second copy of it. Its tests assert both directions — a checkout dirtied only by untracked files reports clean, and a tracked modification, deletion, or staged addition still reports dirty.
- Re-audited the three still-suppressed pyrefly codes on generated `api.py` (`bad-argument-count`, `not-iterable`, `missing-attribute`, alef-334); extended the `pyrefly_generated_package_tests` fixture with a multi-arg native constructor, a `Vec<enum>` field, a nested options dataclass, and a thiserror-derived error enum to exercise each code's most plausible generated shape, and confirmed via hand-corruption that the gate genuinely detects each one (not a vacuous pass) while finding no live defect under those shapes.
- Added a dedicated integration test (`bin_cli::all_commands::pyrefly_generated_package_tests`) that runs `alef all` against a real fixture and then runs real `pyrefly check` over the actual generated `packages/python` output — the first alef-side check that points pyrefly at generated package output rather than only doc snippets (`snippets::validators::python` remains snippet-only). Skips cleanly when `pyrefly` is not on `PATH`, matching the existing convention in `snippets::validators::python`'s own pyrefly-backed tests.
- Added a canonical `~keep` rule documenting which `Named`-containing shapes transport per-element (`Vec<Named>`) versus as one JSON blob (`Map<_, _>`, bare `Named`, `Optional<Named>`) across the Swift trait bridge, in `gen_rust_crate::plugin_inbound`.
- Added (swift): a two-target SwiftPM compile regression that generates the trait-bridge and box files into a fixture package with the real `Client -> RustBridge` dependency direction and runs `swift build` on it. Every previous Swift trait-bridge test asserted on emitted strings, which is how #258 shipped twice; this gate fails on macOS when the Swift toolchain is missing and skips loudly elsewhere rather than reporting a silent pass.
- Extended the two-target SwiftPM compile gate with DTO types nested in containers — `Vec<Named>` in return and parameter position, and `Option<Named>` in return position.

### Changed

- The Android ABI cross-compile is gated at execution, not configuration, so no build that works today starts requiring an NDK. It runs only for `assembleRelease`/`publishAndReleaseToMavenCentral` task graphs, only for ABIs whose library is not already staged (a publish workflow that unpacks prebuilt libraries keeps working on a runner with no NDK), and never under `-Palef.skipAndroidJni=true`. A missing `cargo-ndk` fails with a message naming the tool, the install command and the opt-out instead of a bare exec failure.
- Fold the Go backend's cgo `-D` feature-macro derivation onto `codegen::cfg::effective_ffi_default_features`. `backends::go::cgo_features` carried a private `ffi_default_features` that duplicated the centralized derivation line for line, so an edit to one silently left the other behind and the emitted cgo preamble could stop describing the cdylib it links. Only the Go-specific half stays local: the cbindgen macro-name mangling and the `-D` formatting.
- Add `the_emitted_cgo_preamble_defines_exactly_the_effective_ffi_default_features`, which reads the `-D` tokens back out of the Go file the backend actually writes and compares them against `effective_ffi_default_features`, so a re-introduced Go-local derivation fails a test instead of shipping a preamble that disagrees with the library.
- Changed: the Android ABI cross-compile is gated at execution, not configuration, so no build that works today starts requiring an NDK. It runs only for `assembleRelease`/`publishAndReleaseToMavenCentral` task graphs, only for ABIs whose library is not already staged (a publish workflow that unpacks prebuilt libraries keeps working on a runner with no NDK), and never under `-Palef.skipAndroidJni=true`. A missing `cargo-ndk` fails with a message naming the tool, the install command and the opt-out instead of a bare exec failure.
- `docs.snippets` validation now fails fast, before any toolchain runs, when a language needs a compiled artifact (`compile`/`typecheck`/`run`) but has no configured session that could plausibly have produced one yet -- no session at all, an ambiguous session, or a session with an empty `before` list. Warns always; under `strict`, bails immediately instead of spending an hour validating snippets that were doomed from the start (GH #256).
- `alef snippets check --lang <language>` (and any other filtered `run_validation` call) now prepares only the configured sessions its filtered snippet set actually needs, instead of running every configured `before` build hook regardless of the filter -- a single-language diagnostic no longer pays for every other language's build. Sessions sharing a working directory with a needed one are still prepared together, so the scratch sweep never treats a cohabiting session's live build cache as abandoned.
- Changed: languages served from the cache are now reported at the default verbosity (`<lang>: unchanged since the last run by this alef build, skipping`). A fully cached run previously printed only `Generated 0 files`, which reads as "nothing needed changing" when it means "nothing was looked at".
- Changed: every `.alef/` cache key that can skip work is now a `CacheKey`, constructible only inside `cli::cache_identity`, where each constructor folds in the alef build identity. The `is_ir_cached` / `is_lang_cached` / `is_stage_cached` predicates accept nothing else, so a future cache cannot gate a skip on a key that forgot the salt — it does not compile. This is the durable half: the IR cache drifted out of step with its two siblings precisely because the salt was a convention each call site had to remember.
- Note for consumers: the first `alef generate` after upgrading to this release re-extracts the IR instead of replaying the previous release's, and regenerates any language whose outputs no longer match their stamps. Nothing is re-stamped by this change — `compute_inputs_hash` and `CODEGEN_FORMAT_VERSION` are untouched, so no file's embedded `alef:hash:` moves and there is no mass invalidation. The visible cost is one extraction. The visible diff is whatever real output difference a stale cache had been hiding, which on a repo that has been replaying an old surface for several releases can be large. That diff is the correct output finally landing, not a regression introduced here.
- Fold the Go backend's cgo `-D` feature-macro derivation onto `codegen::cfg::effective_ffi_default_features`. `backends::go::cgo_features` carried a private `ffi_default_features` that duplicated the centralized derivation line for line, so an edit to one silently left the other behind and the emitted cgo preamble could stop describing the cdylib it links. Only the Go-specific half stays local: the cbindgen macro-name mangling and the `-D` formatting.
- `[crates.e2e.snippets].curated_snippets` globs now resolve against the project root (the directory holding `alef.toml`), not against `[crates.e2e.snippets].output`. Hand-authored snippets sit beside the generated tree rather than inside it, so an `output`-relative pattern could not name them at all; measured across three consumer trees, all 113 hand-authored snippets lived outside `output`. **BREAKING**: an existing `output`-relative pattern must be rewritten with its full project-root path (`docker/*.md` becomes `docs/snippets/docker/*.md`); a pattern that no longer matches fails loudly with an error naming it, and the error explains the new base.
- A curated glob is now refused if it escapes the project root or is absolute, and `snippets-migrate` fails when the migrated root does not lie beneath the project root, rather than silently reinterpreting every pattern in a different key space.
- A curated declaration is resolved by walking only each pattern's literal directory prefix, so project-root-relative patterns do not cost a walk of the repository.
- A `curated_snippets` pattern that matches zero files, or that matches a path alef itself generates, now fails the run instead of being silently accepted.
- Changed: a snippet run that publishes the unconfigured `https://example.com` fallback now warns once, naming the affected fixtures and the config key that fixes it, and records them on `SnippetGenerationReport::placeholder_sample_url_fixtures`. Generated output is unchanged when `sample_base_url` is unset. An unusable `sample_base_url` (empty, whitespace-bearing, or scheme-less) fails generation instead of silently falling back.
- Changed: `render_cargo_toml` in the Rust e2e generator takes a `CargoTomlInputs` struct instead of twelve positional arguments, six of them adjacent `bool`s behind a `too_many_arguments` allow.
- Changed: the build-time working-tree classifier now asks `git diff --quiet HEAD` — tracked paths only, index and working tree both, so a staged addition or a deletion still counts as dirty. Untracked files no longer count: reaching the compiler requires a `mod`/`include!` chain rooted at a tracked `src/lib.rs`, so untracked source that actually affects the build drags a tracked modification along with it. A denylist would have covered `.cargo-ok` and then waited for the next tool's marker file.
- Changed: a repository with no commit yet now stamps `unknown` instead of `clean`. There is no `HEAD` to call the tree clean relative to, and `clean` reads as a provenanced build.
- **Consumers should expect a large one-time diff on their next `alef all`.** Every generated file whose emitted shape was not already canonical is reformatted in that run. The change is formatting-only: file bodies are re-derived from the same generation inputs, `alef verify` is clean afterwards, and consecutive runs are byte-identical (verified across 831 files on the fixture). Review the diff once and commit it; it does not recur.
- One consequence worth knowing: for a language whose generator emits non-canonical bytes, `alef all` now rewrites those files on every run (raw content written, formatter canonicalises it back to the same result), so the per-language "up to date (skipping)" cache no longer fires for them and the run's changed-file count no longer settles at zero. On-disk content is unaffected and stable; the cost is extra writes, not drift.
- Changed the Swift JSON-bridged leaf verdict to be answered in one place for both generators: `FieldResolver::swift_json_bridged_traversal_prefix` and the new `swift_json_bridged_iteration_prefix` are two framings of one walk, and the snippet presentation resolver now asks it instead of re-deriving nothing at all.
- Changed (BREAKING, swift): alef no longer emits a default-implementation extension on `Swift{Trait}Bridge`. The IR records that a Rust trait method has a default body but never the body itself, so every stub alef wrote was a guess — 0.67.5's `unit_enum_default_case` picked the first fieldless variant rather than the Rust `Default`, defaulted `Bool` methods returned `true` where Rust returned `false`, and `Named` returns got a `"{}"` literal that no DTO deserialises from. Because the inbound Rust wrapper calls the Swift shim for defaulted methods too, those guesses replaced the real Rust default at runtime instead of sitting unused. Conformers must now implement defaulted methods; each such protocol method carries a doc comment explaining why the stub is absent and what value to supply. Swift bridge conformers that relied on the generated defaults will need to add those methods.
- Changed: the two TypeScript-family accessor renderings are now one derivation parameterized by map lowering (`TypescriptMapAccess`), so `node` and `wasm` can only disagree where the bindings genuinely differ — a NAPI `HashMap` is an object index, a wasm-bindgen one is a JS `Map` `get`. An optional receiver before a `get` renders `?.get(...)`, not the element form's `?.[`.
- Extract `scaffold::core_dep_features_excluding` as the shared, generalized filter (Node/Elixir/PHP/FFI all reuse it) instead of copying a per-language `<lang>_core_dep_features` helper four more times.
- Regenerate `schemas/alef.schema.json` for the four new config fields.
- Correct `language_excludes`'s doc comment and the `LedgerExpectations`/`function_excluded_for_language` comments in `src/snippets/gaps.rs` — they referenced a nonexistent `[crates.skipped]` config table, a nonexistent `[workspace.crates."<name>"]` per-crate override, and a nonexistent `#[alef::exclude]`/`#[alef::opaque]` attribute pair. Only `#[alef::skip]`/`#[doc(hidden)]` exist, and they act via the extraction-time `binding_excluded` IR flag, honored separately from and uniformly across `language_excludes`, not folded into it.
- Clarified the `[crates.docs.snippets].strict` and `.deny_unclassified` doc comments and the `alef snippets check --strict` CLI help to state the unified semantics; regenerated `schemas/alef.schema.json` accordingly.
- Split `scaffold_license_files`/`scaffold_gitattributes` out of `src/scaffold/mod.rs` into a new `src/scaffold/generated_files.rs` module to restore the file-size ratchet.
- Split the Java checkstyle/javadoc scaffold tests out of `src/scaffold/tests/ffi_go_java_ruby.rs` into a new `src/scaffold/tests/java_checkstyle.rs` module.
- Split the marker-detection unit tests out of `src/core/hash/tests.rs` into a new `src/core/hash/tests/marker_detection.rs` module.
- Split the Kotlin bracket-wildcard assertion tests out of `src/e2e/codegen/kotlin/tests.rs` into a new `src/e2e/codegen/kotlin/tests/wildcard.rs` module.
- Split the e2e-generator-defer regression tests out of `src/bin_cli/all_commands_tests.rs` into a new `src/bin_cli/all_commands_e2e_defer_tests.rs` module.
- Split `alef all`'s pre-flight/helper functions out of `src/bin_cli/all_commands.rs` into a new `src/bin_cli/all_commands_run_setup.rs` module.
- Split the `Commands::Generate` match-arm body out of `src/bin_cli/core_commands.rs` into a new `src/bin_cli/core_commands/generate.rs` module; retargeted `strict_formatting_tests.rs`'s source-scan test at the new file.
- Split the inline unit-test module out of `src/e2e/codegen/presentation.rs` into a new `src/e2e/codegen/presentation/tests.rs` module.
- Split the inline unit-test module out of `src/scaffold/languages/ffi.rs` into a new `src/scaffold/languages/ffi/tests.rs` module.
- All nine splits are pure code movement with no behavior change; `#[test]` fn counts and pass counts are unchanged before and after each split.
- Split the 11 over-cap e2e codegen files that grew past their file-size ratchet ceilings (rust/assertions.rs, go/assertions.rs, go/tests.rs, go/test_function.rs, csharp.rs, csharp/assertions.rs, elixir/assertions.rs, elixir/test_case.rs, java/assertions.rs, python/assertions.rs, typescript/assertions.rs), moving self-contained test modules and one pure data-prep helper into sibling files with no behavior change.
- Dropped the stale elixir/test_case.rs entry from tests/file_size_baseline.txt after the split took the file under the 1,000-line cap, per the ratchet's own stale-entry rule; no other baseline ceilings were changed.
- Extracted the declared-key check into a shared `refuse_undeclared_json_keys` helper in `src/e2e/codegen/typescript/test_file/builders/mod.rs` and routed both `ts_builder_expression_inner` and `node_value_expression` through it, so the two call sites cannot drift apart. Both accept a field keyed by its Rust name or its wire name (`#[serde(rename)]` / `rename_all`), and skip types with a `serde_flatten` field or with no matching `TypeDef` (external/opaque types).
- Renamed `OcrBackend`/`PostProcessor` fixture trait names to `SampleBackend`/`SampleTransformer` in `tests/backends_swift_trait_bridge_snapshot.rs`, replacing a real consumer's domain vocabulary with neutral fixture names (no behavior change).
- Split per-language exclusion predicates out of `e2e::snippets::mod` into a new `e2e::snippets::exclusions` module to stay under the repo's per-file line cap.
- Replaced the vague "never diagnosed" comment on the scaffolded `pyproject.toml`'s `[tool.pyrefly.sub-config]` suppression block with the evidence gathered above and an explicit note of which pyo3 backend surfaces (service_api decorators, trait_bridge visitors, streaming adapters, capsule types) remain unaudited for these codes.
- Corrected the `binding-audit-pattern` ai-rulez rule (alef repo, `.ai-rulez/rules/binding-audit-pattern.md`), which told auditors to check config/attribute surfaces that do not exist in alef's schema.
- Corrected `.ai-rulez/skills/binding-audit/SKILL.md` to remove fictional config/attribute surfaces (`#[alef::exclude]`, `#[alef::opaque]`, `[workspace.exclude_types]`, `[crates.opaque_types]`, `[workspace.crates."<name>"]`) and align it with the already-fixed `binding-audit-pattern` rule: real surfaces are `[crates.exclude]`, per-language `exclude_types`/`exclude_functions` on `[crates.<lang>]`, workspace-only `[workspace.opaque_types]` (a type remap, not an exclusion), and attribute-level `#[alef::skip]`/`#[doc(hidden)]` only.
- Documented that `binding_excluded` is honored independently by every downstream consumer and that `language_excludes` never consults it, so audit tooling can misclassify a skipped item as a gap (alef-task #329).
- Split `src/snippets/gaps.rs` unit tests into `src/snippets/gaps/tests.rs`, dropping the file under the repository's 1,000-line cap and removing its file-size ratchet baseline entry.
- The fix is generic and IR-derived, not a hard-coded field list: `FieldResolver::accessor` now dispatches `"wasm"` through the same `render_typescript_with_optionals` renderer `"node"` uses (parameterized only by `TypescriptMapAccess`, the one real lowering difference — NAPI `HashMap` as object index vs. wasm-bindgen `Map.get`). Optionality itself comes from `FieldDef.optional` in the IR (`FieldResolver::ir_field_sets` / `with_ir_result_fields`), merged with any configured `fields_optional`, not from field-name matching. Confirmed against the tree-sitter-language-pack consumer: `ProcessResult.data: Option<DataNode>` in `crates/ts-pack-core/src/intel/types.rs`, with NO `fields_optional` entry for `data` anywhere in its `alef.toml` — the exact IR-only shape the fix must cover, and does.
- Added regression coverage pinning this: `src/e2e/codegen/presentation/wasm_optional_leaf_field_tests.rs` (reproduces the `ProcessResult { data: Option<DataNode> }` shape at the `resolve()` level: bare field unguarded, nested `data.kind`/`data.children` chained with `?.`, node/wasm agreement, and a negative control that a required field is not over-chained) and `src/snippets/validators/wasm_optional_chain_tsc_tests.rs` (compiles the actual accessor shapes with real `tsc` under `strict`: the guarded shape passes, the exact unguarded shape alef 0.67.5 published fails with TS18048/TS2532, and a required field passes without `?.`). A render-only assertion cannot see TS18048; these tsc-backed tests can.

### Fixed

- `Alef e2e snippets-migrate` now honors `[crates.e2e.snippets].curated_snippets`. The command called the curated-unaware `migration::compare_root`, so every hand-authored file reported as `no_generated_equivalent` regardless of configuration and a consumer could not tell a declared, intentional curated snippet from a genuine coverage gap. The comparison and its report rendering moved to `bin_cli::snippet_migration`, which routes through `compare_root_curated`.
- The `snippets-migrate` text report gives a declared curated file its own `curated` label instead of `no_generated_equivalent`, matching the `curated` flag the `--json` report already carried per entry.
- `Migration::compare_root` keyed existing files against `existing_root` and generated files against the configured `output`, so when `output` is a subdirectory of the migrated tree (`alef e2e snippets-migrate docs/snippets` against `output = "docs/snippets/generated"`) the two key spaces were disjoint by construction and every file alef itself had generated reported as `no_generated_equivalent`. One consumer saw 7796 false positives against 3 genuinely hand-authored files. The generated keys now carry the nested prefix so both sides key off `existing_root`; parallel roots are unchanged.
- A `[[crates.jni.target_dep_overrides]]` entry naming a core-crate aggregate feature (the `android-target` shape) now satisfies the `#[cfg(feature = "…")]` gates of every member that aggregate enables. Feature satisfaction matched configured names literally, with only `full` hard-coded as a universal umbrella, so a shim gated on a member of the configured aggregate was emitted behind `#[cfg(not(any(<target>)))]` and silently disappeared from the cross-compiled Android artifact while every desktop target kept it.
- The Kotlin Android build contract never produced the per-ABI native libraries its own release guard demands. `alef build --release` ran `gradle assembleRelease`, which fails `validateJniLibsForRelease` because `src/main/jniLibs/<abi>/lib<crate>_jni.so` was scaffolded as an empty directory and nothing ever filled it — the only native build in the contract, `buildHostJni`, produces a host-architecture library that can never satisfy an Android ABI directory. The generated `build.gradle.kts` now emits a `buildAndroidJniLibs` task that cross-compiles the JNI crate with `cargo-ndk` straight into `src/main/jniLibs/`, wired ahead of both the release guard and AGP's jniLibs merge.
- **`[Crates.include]` entries that match nothing now fail extraction instead of emptying the binding**: `include` is an allowlist, so an entry naming no extracted item did not fail open — it dropped every type and enum from the surface while `alef build` still exited 0 and generated empty bindings. A typo (`include.types = ["Kpet"]`) or the qualified `crate::path::Type` spelling that `[crates.exclude].types` accepts both produced an empty type list silently. Unmatched `include.types` / `include.functions` entries now abort with an error naming the entry, the config key, and how many types/enums/functions the crate actually exposes. Entries naming a declared `[crates.opaque_types]` type or an `unsupported_public_items` diagnostic still resolve, since both are legitimate include targets.
- **`[Crates.include].types` accepts the same qualified paths as `[crates.exclude].types`**: include entries are now resolved through the shared type-identity matcher instead of being compared against the short name only.
- **One matcher decides whether a configured entry names a type**: `[crates.exclude].types` demanded an exact `rust_path`, while `[crates.exclude].fields` and `[[crates.source_crates]].roots` also accepted the two-segment `crate::Type` shorthand — so `exclude.types = ["c::Foo"]` was a silent no-op for `c::inner::Foo` while `exclude.fields = ["c::Foo.bar"]` matched it. All three now share one rule. Exact-path entries still disambiguate two same-named types.
- **Unmatched `[crates.exclude]` entries are reported**: `exclude.types`, `exclude.functions`, and `exclude.methods` entries that match nothing now warn, as `exclude.fields` already did. An exclusion is only observable through what it removes, so a typo'd entry previously excluded nothing and said nothing.
- `FieldResolver::accessor` and `FieldResolver::rust_unwrap_binding` each carried a private copy of the virtual-namespace strip decision, gated on `result_fields.contains(..)` where the shared `result_relative_path` asks the broader `is_valid_for_result(..)`. The copies could place the same fixture field somewhere the classifiers did not — the defect shape that emitted `string(result.ActionResults)` into a generated Go package. Both now call `result_relative_path`, so accessor emission, `is_array`, and the zig/brew/C serialized-path navigation share one definition of where a field's value lives.
- An accessor whose virtual prefix hides a field the IR reaches but a hand-maintained `result_fields` omits now strips that prefix, instead of emitting a member access against the virtual label.
- A `result_fields` entry the IR marks `binding_excluded` no longer strips its virtual namespace prefix in accessor emission. `with_ir_fields` already warns that such an entry is a config bug and no binding emits an accessor for the field, so neither spelling compiles; the accessor now agrees with `is_array` and the serialized-path generators rather than keeping a private answer.
- `alef adopt`'s create-once-seed warning no longer names the wrong command as the moment of loss. It said adopting a seed consents to alef "replacing its contents with a placeholder seed on the next generate", but `write_scaffold_files_report`'s `can_skip` (`!overwrite && !generated_header && exists && !is_alef_derived_output`) runs before the ownership guard and consults no ownership signal, so a plain `alef generate` skips an adopted seed exactly as it skips an unadopted one. The replacement lands on the next write that passes `overwrite: true` -- an `alef version` scaffold regen, or `alef all --clobber-create-once-seeds`. An operator who tested the warning by running `alef generate`, saw the file untouched, and concluded the warning was false would have been reading accurate output; the loss was simply still days away. The flag help, the `NOT ADOPTED -- create-once seeds` stdout block, the per-path `warn!` and the seeds-only `bail!` now all name the overwriting regen and say a plain generate skips these paths.
- Honor `#[cfg_attr(alef, alef(skip))]` on a function reached through a `#[cfg(...)]`-gated `pub use` re-export. `apply_cfg_to_item` cleared `binding_excluded` on every same-named function when applying a re-export's cfg gate, so a skipped function republished by a gated `pub use` was resurrected into the binding surface — while its siblings behind a plain (ungated) `pub use` kept their skip. When the resurrected function's signature could not be represented, the run aborted with a fatal `lossy_sanitized_surface` error naming an item the author had explicitly skipped, and the error's own suggested fix ("mark the item with `#[cfg_attr(alef, alef(skip))]`") was already applied. A cfg-gated re-export now contributes only the cfg gate; the declared skip wins, and the `not(X)` stub-pairing path clones only entries that were not excluded.
- Fixed: the Kotlin Android build contract never produced the per-ABI native libraries its own release guard demands. `alef build --release` ran `gradle assembleRelease`, which fails `validateJniLibsForRelease` because `src/main/jniLibs/<abi>/lib<crate>_jni.so` was scaffolded as an empty directory and nothing ever filled it — the only native build in the contract, `buildHostJni`, produces a host-architecture library that can never satisfy an Android ABI directory. The generated `build.gradle.kts` now emits a `buildAndroidJniLibs` task that cross-compiles the JNI crate with `cargo-ndk` straight into `src/main/jniLibs/`, wired ahead of both the release guard and AGP's jniLibs merge.
- Fixed: a `[[crates.jni.target_dep_overrides]]` entry naming a core-crate aggregate feature (the `android-target` shape) now satisfies the `#[cfg(feature = "…")]` gates of every member that aggregate enables. Feature satisfaction matched configured names literally, with only `full` hard-coded as a universal umbrella, so a shim gated on a member of the configured aggregate was emitted behind `#[cfg(not(any(<target>)))]` and silently disappeared from the cross-compiled Android artifact while every desktop target kept it.
- Fixed: C snippets no longer wrap a batch input in an `AlefHandle`. An `args` entry typed `json_object` describes the fixture value, not the parameter's C shape, and `c.rs`'s `element_type` backfill unwraps `Vec<T>` to `T` — so a `Vec<ItemInput>` batch parameter, which the FFI backend declares as `const char *`, had a handle built from its element type and passed where a pointer was expected (`incompatible integer to pointer conversion passing '{PREFIX}AlefHandle' to parameter of type 'const char *'`). Handle construction now consults the declared parameter through the same `handle_param_type_name` seam the omitted-optional sentinel uses: only a bare `Named`/`Optional<Named>` becomes a handle; `Vec<_>`, `Map<_, _>` and `Json` receive the serialized JSON string.
- Fixed: a `serde_json::Value` (`TypeRef::Json`) parameter mapped to a `json_object` arg no longer aborts C e2e generation with "no resolvable type". `named_type` answers `None` for it, leaving `element_type` unset, which tripped the typed-handle panic whenever no `options_type` fallback was configured; it now renders as the JSON string the C ABI declares.
- Fixed: the IR cache (`.alef/<crate>/ir.json`) was keyed on the Rust sources, the consumer crate's own version and the config, but never on alef itself, so upgrading alef did not invalidate it. A newer alef replayed an older alef's extracted `ApiSurface` verbatim and generated from it, and `alef verify` — which re-enters the same extraction — agreed. Because most `ApiSurface` fields are `#[serde(default)]`, a field an older extractor never wrote came back as its default rather than as an error, so the replayed surface could be wrong rather than merely old. The alef build identity is now part of the key.
- Fixed: a per-language cache hit only checked that each recorded output file still *existed*. A generated file edited by hand stayed a hit, the language was dropped from the generation set before anything read it, and `alef generate` reported `Generated 0 files` while leaving the edit in place; deleting `.alef/` made the same command restore the file. A hit now also requires every recorded output carrying an `alef:hash:` stamp to still agree with that stamp — the same comparison `alef verify` makes. Outputs with no stamp (`generated_header: false`, create-once seeds) keep the existence-only rule.
- Fixed `alef all` deleting a cache-hit language's still-valid binding output as a false orphan: the manifest-based route in `sweep_manifest_orphans` compared against a `keep` set (`current_gen_paths`) that was populated only from languages actually regenerated this run, so a language skipped via the per-language cache counted as having emitted nothing and its previously-generated files were swept. This produced an unbroken hit/miss/hit/miss cycle across repeated `alef all` runs, since the next run found a manifested output missing and regenerated, and the run after that deleted it again. Fixed by folding the already-correctly-seeded `binding_ownership` map into `current_gen_paths` before the sweep runs.
- Closed the existence-only weakness in `is_stage_cached` (e2e, test-apps, scaffold, readme, docs stages): it now also requires every manifested output carrying an `alef:hash:` stamp to agree with that stamp under the run's `inputs_hash`, mirroring the fix already shipped for `is_lang_cached`. A consumer who hand-edits a generated e2e test, README, scaffold file, or docs page no longer gets a silent cache-hit skip. `is_stage_cached` now takes an `inputs_hash: &str` parameter; all 7 production call sites in `src/bin_cli/` were updated to compute it from the same `sources_hash`/`alef_toml_bytes` used to stamp those outputs, hoisted ahead of the cache check where needed.
- The four `CHUNKS_RECIPE` synthetic assertion handlers (`chunks_have_content`, `chunks_have_embeddings`, `chunks_have_heading_context`, `first_chunk_starts_with_heading`) hardcoded `{result_var}.chunks` and intercepted ahead of any field validation, across all seven wired backends (Rust, Python, TypeScript, Go, Java, C#, Elixir). A crate where `chunks` is declared on a nested IR type (e.g. `Document`) but not on the call's own root type (e.g. `Envelope { results: Vec<Document> }`) generated a `result.chunks` accessor that does not compile.
- the anchored, whole-path oracle (`FieldResolver::result_field_oracle_knows`, already fixed for the derived-snippet path in the prior release) is now wired into the seven backends' per-call `FieldResolver` construction via `with_ir_result_fields`, and the four chunk synthetic handlers consult it before rendering: the assertion is now skipped with a `not available on result type` comment when the call's own declared root type positively does not declare `chunks`. An unresolved root or a name the oracle has no anchor for keeps the pre-existing permissive behaviour.
- TypeScript and Go had no `functions`/`CallIr` threaded to the call site that builds this resolver; both now receive it (a new `functions` field on `GoTestFunctionContext`/`GoTestFileContext`, a new trailing parameter on `render_test_file`/`render_test_case`), so the WASM backend (which shares TypeScript's `render_test_file`) also gains the anchor for free.
- `alef e2e snippets-migrate` rebases each `existing_root`-relative entry path onto the project root before matching a curated glob, so one declaration means the same thing to the migration comparison and to the coverage resolver.
- `alef e2e snippets-migrate` no longer reports alef's own `.alef-snippet-coverage.json` as `no_generated_equivalent`; the coverage ledger is bookkeeping, not a snippet the project should hand-author.
- `excluded_default_features` now also drops the name from the core dependency's explicit `features = [...]` line (not just the wrapper's own `default = [...]` array), reusing the shared `core_dep_features_excluding` helper used by swift/ruby/node/elixir/php/ffi (alef-tasks #331)
- Handle `Optional<Vec<Named>>` on both sides of the trait bridge -- the inbound (`extern "Swift"`) return path now decodes it per-element (`Option<Vec<String>>`), matching the already-tested param convention; the outbound (`extern "Rust"` Box-class) path now collapses it to one JSON blob (`String?`), matching the proven `Optional<Vec<T>>` DTO-getter limitation, since the two boundaries have different swift-bridge support for Option-wrapped containers (alef-tasks #333)
- Downgrade a docs fixture's `display: true` to the debug formatter when the resolved Rust path is a struct/enum type from the crate's own IR, instead of emitting `println!("{}", ...)` against a type alef has no record of implementing `Display` (extraction discards `impl Display` via `STD_TRAITS`); warns naming the fixture and path so the mismatch is visible instead of a silent non-compiling snippet.
- Stop telling readers to hand-add the provenance marker in the frozen-file report; point at `alef adopt <path> --write` instead, matching `alef generate`'s own write-guard guidance.
- Fixed: the generated Rust e2e crate declared `tokio-stream` only when a fixture needed a mock server, but the streaming collect recipe writes `tokio_stream::StreamExt` into every streaming test body. A streaming fixture with no mock server emitted a test file naming a crate the manifest did not depend on, failing the suite with `error[E0433]: failed to resolve: use of unresolved module or unlinked crate tokio_stream`. The dependency is now derived from the emitted test bodies themselves, so it cannot disagree with the recipe that put the path there. The mock-server codegen never named `tokio_stream` at all, so mock-server-only crates no longer carry the unused dependency.
- Fixed: the same disagreement for `serde_json` in the generated Rust e2e crate. `needs_serde_json` was derived from the call's argument types (`json_object` / `handle`), which is blind to what an assertion emits: a `contains` over a collection field serializes each element through `serde_json::to_value`. A fixture with such an assertion, no JSON argument and no mock server emitted `serde_json::` without the dependency.
- Conftest.py now chdirs into `test_documents_dir` even when a fixture set also needs the mock server; the mock-server and file-fixture branches were an if/else, so a fixture needing both silently lost the chdir and file_path/bytes args resolved against pytest's invocation cwd instead of the configured test-documents directory.
- The `os` import is now derived directly from the rendered test-file body (`body.contains("os.")`) instead of from a fixture-level heuristic ANDed with the body. The heuristic never checked a fixture's declared `env.api_key_var`, so a fixture with an api key var but no mock/client-factory/mock_url/bytes-path arg emitted `os.Getenv(...)` without importing `"os"`.
- A typed `json_object` argument's DTO constructor (`ExtractInput.new(...)`) is now namespaced under the crate's module (`DemoCrawler::ExtractInput.new(...)`), matching how the adapter `mock_url` request type is already qualified. The unqualified form resolved against top-level Ruby scope, not the generated binding's namespace.
- A generated test now awaits a call whenever the core IR declares its target function/method `async fn`, not only when `alef.toml`'s `[call] async` (or a per-language override) says so. `CallConfig.r#async` is a plain `bool` defaulting to `false`, so a fixture's config never distinguished "not async" from "never told me" -- a Rust function that became `async fn` after the fixture was authored left the config stale and the generated call unawaited. Added `IrSignature.is_async` (`src/e2e/codegen/call_ir.rs`) and `ResolvedE2eCallRecipe::ir_is_async` (`src/e2e/codegen/recipe.rs`), and wired TypeScript/WASM's `_functions` IR parameter (previously unused) through to `render_test_case`. An explicit per-language `async` override still wins verbatim over a disagreeing IR.
- Fixed `[crates.e2e.env]` never reaching the local `alef test --e2e` run command's spawned process (only the pdfium library-path vars were injected); now each language's `e2e` command is prefixed with a plain `export K='V';` for every configured `[crates.e2e.env]` entry, mirroring what `test-apps run` (registry mode) already did. This unblocks runtimes (e.g. Elixir's `mix test`, whose Rustler NIF loads during Mix's compile phase, before `test_helper.exs` runs) where setting the var from generated in-tree fixture code happens too late.
- Fixed the napi/Node backend generating a struct field's `#[napi(js_name = ...)]` from the field's `#[serde(rename = ...)]` wire name instead of from casing policy, which made the compiled `.node` artifact expose a different property name (e.g. `max_chars`) than the one alef's own generated `.d.ts` declared for the same field (e.g. `maxCharacters`) — the two generators disagreed about the same IR input. `js_name` and the JSON wire rename are now derived independently, matching every other backend's separation of public identifier casing from `serde_rename`/`serde_rename_all`.
- `language_excludes`'s `Language::Jni` and `Language::KotlinAndroid` arms now fold `[crates.jni].exclude_functions`, so a function excluded only at the JNI level no longer shows as a false-positive gap in the KotlinAndroid docs page and snippet coverage ledger.
- `alef snippets check`'s `deny_unclassified` runner setting is now gated on the unified `strict` value (`--strict` flag OR `[crates.docs.snippets].strict`), not the raw `--strict` flag alone. Previously `[crates.docs.snippets].strict = true` left unclassified-side-effect snippets free to reach real execution at `ValidationLevel::Run` instead of being pre-emptively skipped.
- `alef snippets gaps` no longer fails unconditionally on any finding. It now applies the same split `check` already used: missing include targets, missing required language variants, undocumented skips, and unknown fence languages always fail; an unreferenced-snippet-only finding fails only under `--strict`. Added `GapReport::has_structural_gaps`/`GapReport::is_failure` as the single shared source of that rule for both commands.
- **`[crates.include]` entries that match nothing now fail extraction instead of emptying the binding**: `include` is an allowlist, so an entry naming no extracted item did not fail open — it dropped every type and enum from the surface while `alef build` still exited 0 and generated empty bindings. A typo (`include.types = ["Kpet"]`) or the qualified `crate::path::Type` spelling that `[crates.exclude].types` accepts both produced an empty type list silently. Unmatched `include.types` / `include.functions` entries now abort with an error naming the entry, the config key, and how many types/enums/functions the crate actually exposes. Entries naming a declared `[crates.opaque_types]` type or an `unsupported_public_items` diagnostic still resolve, since both are legitimate include targets.
- **`[crates.include].types` accepts the same qualified paths as `[crates.exclude].types`**: include entries are now resolved through the shared type-identity matcher instead of being compared against the short name only.
- **One matcher decides whether a configured entry names a type**: `[crates.exclude].types` demanded an exact `rust_path`, while `[crates.exclude].fields` and `[[crates.source_crates]].roots` also accepted the two-segment `crate::Type` shorthand — so `exclude.types = ["c::Foo"]` was a silent no-op for `c::inner::Foo` while `exclude.fields = ["c::Foo.bar"]` matched it. All three now share one rule. Exact-path entries still disambiguate two same-named types.
- **Unmatched `[crates.exclude]` entries are reported**: `exclude.types`, `exclude.functions`, and `exclude.methods` entries that match nothing now warn, as `exclude.fields` already did. An exclusion is only observable through what it removes, so a typo'd entry previously excluded nothing and said nothing.
- Fixed: a configured core-crate *aggregate* feature (the `android-target` / `wasm-target` shape, where the core crate declares `bundle-target = ["gated", ...]`) is now resolved through the core crate's own `[features]` table before it is used as an enabled-feature set. `cfg_feature_satisfied` matches `feature = "X"` leaves literally and treats only `full` as a universal umbrella, so every other aggregate name satisfied no gate at all: the Go, Java, C#, Kotlin, Zig, WASM and R backends silently dropped every `#[cfg(feature = "<member>")]` item from their surface even though cargo compiles it. The JNI shim gate was fixed earlier; this closes the same defect at every remaining site.
- Fixed: the generated WASM crate's `[features] default = [...]` list is derived from the expanded configured feature set, so a configured aggregate now turns the binding crate's own passthrough rows on. Matching the literal aggregate against the cfg-referenced names produced `default = []` — every passthrough declared and none enabled — which compiled the gated items out of the wasm artifact even after codegen kept them.
- Fixed: language reference docs (and the README function list, which shares the same derivation) expand a configured aggregate the same way the backends do. `effective_docs_features` matched the aggregate literally, so a page documented strictly less API than the binding beside it.
- Fixed: the JNI shim's default (non-overridden) target branch now counts the core crate's own `default = [...]` features. `scaffold::languages::jni` emits that branch's core dependency with no `default-features = false`, so those features are always active there; deriving the branch's enabled set from the configured `features` list alone dropped every shim behind a default-enabled gate.
- Fixed: a `[[crates.jni.target_dep_overrides]]` entry with `default_features = true` now counts the core crate's declared default features for that target. Reading only the override's literal `features` list left the branch looking empty, so the shim was emitted behind `#[cfg(not(any(<target>)))]` — present on every other target and absent from the one the override exists to describe.
- Expand a binding language's configured feature list through the core crate's `[features]` table in `warn_on_ffi_feature_drift` before differencing it against the FFI cdylib's effective set, matching the aggregate expansion `backends::go`/`java`/`csharp`/`kotlin`/`zig`/`wasm` now apply to their own `enabled_features` before `with_cfg_filtered_deep`. Previously the warning left the binding side unexpanded, so a fully FFI-covered aggregate produced a false "coverage gap" for every one of its members, and a real gap (an aggregate member the FFI side never reaches) went unreported because the literal, unexpanded set never matched the member's gate name.
- Fold scaffold-only manifest writes (`packages/java/pom.xml`, `crates/<name>-ffi/cmake/*.cmake`, `packages/python/pyproject.toml`, and any other language's scaffold-managed manifest) into `alef generate`'s `changed_languages` set, so a config-only edit (e.g. `package_metadata.license`) that rewrites a manifest with no corresponding bindings/service-api/public-api/stubs write no longer ships that manifest unformatted. Root cause: `reconcile_managed_scaffold_manifests`'s write report fed `any_written` but never `changed_languages`, so on a regen where a language's bindings were cache-hit, that language's scaffold-only write fell outside `format_scope` and `poly_paths` never named its directory. `alef all`'s full-tree convergence pass was never affected, since it reformats every byte under the repo root regardless of which phase wrote it -- which is why `alef generate` and `alef all` disagreed. Not Java-specific: reproduced identically for FFI's cmake config and Python's pyproject.toml on the same fixture; the fix is generic (`languages_owning_changed_paths` in `src/cli/pipeline/format.rs`), not a Java special-case.
- `alef snippets gaps`/`check`'s language-parity check now cross-references the e2e coverage ledger's expected set, so a function dropped for one language via `exclude_functions` (or any surface `function_excluded_for_language` folds in) no longer reports a false-positive missing-language-variant finding; a language the ledger genuinely expected but never generated still fails.
- `required_languages` (config key and `alef snippets gaps --required-languages`) now accepts a session target name (`kotlin_android`, `kotlin-android`, `node`, `wasm`) as well as a fence tag, and names both accepted vocabularies when a value resolves to neither instead of silently dropping it from the comparison.
- `alef generate` stamped scaffold, binding, public-API, stub, and service-API output with `alef:hash:` immediately after each write phase -- five `finalize_hashes` checkpoints, all ahead of the command's only format pass -- so `poly` (which refuses to format an already-stamped file) skipped every one of those files, shipping them in whatever shape the generator emitted. The five per-phase checkpoints are removed; `current_gen_paths` is now stamped exactly once, after formatting.
- `alef generate` now strips a stamp an earlier (including pre-fix) run left on a file before its format pass runs (`unstamp_before_formatting`), so a tree that was previously stamped-and-never-formatted heals on the next `alef generate` instead of staying non-canonical forever.
- `alef generate` now also formats when nothing changed this run but the on-disk tree is still non-canonical (`generated_tree_needs_formatting`), instead of gating purely on `any_written`/`changed_languages`; when that fallback fires, the format pass now covers every language this invocation resolved rather than silently no-op-ing on an empty `changed_languages` set.
- Fixed: a snippet session's `before` hook is now run once per package instead of once per configured session target. `kotlin` and `kotlin_android` both resolve to `Language::Kotlin`, and `typescript`/`node`/`wasm` all resolve to `Language::TypeScript`, so several targets routinely describe one physical package and each carried its own copy of that package's hook. Every copy was executed, sequentially, before a single snippet could validate — and when the hook outran `timeout_secs`, the run paid that whole timeout once per target. Within an activation group a hook whose command and environment match one already attempted now replays that attempt's outcome; failures replay too, preserving the timeout classification every affected target is reported with.
- Fixed: `run_command` no longer outlives the timeout it was given. The budget covered only the wait for the direct child; once that child exited, output collection waited for end of stream on pipes every descendant had inherited, so any process outliving the command — a Gradle daemon, an MSBuild node, an unwaited background job — held the call open indefinitely. A one-second budget was measured taking twenty seconds and still returning success. Output readers now buffer as bytes arrive and the drain gives up at a fixed grace, reporting everything the command actually wrote and tearing down the process group of anything still holding the pipes.
- Fixed: `SIGINT`, `SIGTERM` and `SIGHUP` are now forwarded to every snippet subprocess group before alef exits. Snippet children are spawned into their own process group so a timeout can kill the whole tree, which also removed them from the terminal's foreground group: Ctrl-C reached alef and nothing else, so alef exited 130 while the entire hook tree — shell, build wrapper and build daemon — survived and reparented to PID 1, where a stale daemon goes on to poison the next run. A signal already ignored on entry stays ignored.
- **e2e: classify `is_array` by the path the accessor actually addresses.** `FieldResolver::is_array` was a bare `fields_array` set lookup against the raw fixture spelling, while `accessor` — and `result_relative_path`, the answer the zig, brew and C generators share — strip a virtual namespace label first. A field spelled `interaction.action_results` therefore rendered as the slice `result.ActionResults` and classified as not-an-array, so Go's `contains`/`contains_all`/`not_contains`/`contains_any` renderers emitted `string(result.ActionResults)` instead of `jsonString(...)`; converting a `[]T` to `string` is not legal Go, so the generated package failed to build. `is_array` now routes its fallback through `result_relative_path` (which also applies alias resolution) rather than growing a second hand-rolled namespace strip beside `is_optional`'s, keeping one definition of where a fixture field's value lives.
- Fixed `package_dir` for `java` and `kotlin` (JVM, non-Android): a configured `[crates.output].<lang>` moved the generated source directory, but `package_dir` ignored config entirely and always answered the hard-coded literal `packages/java` / `packages/kotlin`, so `mvn -f {dir}/pom.xml` and `cd {dir} && gradle` targeted a directory the generator never wrote sources into whenever the configured tree moved outside `packages/java` / `packages/kotlin`. `package_dir` now derives the Maven/Gradle project root from the resolved output path the same way each backend's own generator disambiguates root-vs-source-dir shape.
- Fixed `jni_output_path` (Kotlin JNI-mode emitter, used by `kotlin_android`): it joined a filename directly onto `output_for("kotlin_android")`, which names the Gradle project root, not a source directory, so JNI-mode `.kt` files were written straight into the project root instead of under `src/main/kotlin/<pkg>/`. Now delegates to the `kotlin_android` backend's own `kotlin_source_dir`, which already resolves that ambiguity for the backend's own file placement.
- Fix `alef build`/`alef generate` running the umbrella `gradle build` (and `gradle build -Prelease`) for `kotlin_android` when no `[workspace.build_commands.kotlin_android]` overlay is declared, instead of the intended `gradle assembleDebug`/`gradle assembleRelease`. `build_command_for`'s `"gradle"` arm matched on the shared `bc.tool` string, which cannot distinguish `Kotlin` from `KotlinAndroid`; it now asks a new shared `build_defaults::gradle_build_task(Language, bool)` helper, the same one `default_build_config` uses, so both derivations agree (xberg-io/alef#259). An explicit `[workspace.build_commands.kotlin_android]` overlay is unaffected and continues to win.
- Fixed: the KotlinAndroid backend now expands aggregate feature names before cfg-filtering the API surface. `[crates.kotlin_android].features = ["<aggregate>"]` was matched literally against every `#[cfg(feature = "<member>")]` gate, so every item the aggregate enables was silently dropped from the Android binding even though cargo compiles it — API present on desktop, absent from the Android artifact, with no diagnostic. The list now goes through `codegen::cfg::expand_configured_features`, the same expansion the JNI shim target gate already uses.
- Fixed: `ResolvedCrateConfig::package_dir` now honors a configured `[crates.output].kotlin_android`. It previously hardcoded `packages/kotlin-android`, so a consumer who configured that key had every `cd {output_dir} && gradle` build, lint, test, and clean command target a directory the backend never wrote to. It now delegates to the KotlinAndroid backend's own `ProjectLayout`, which already distinguishes a configured Gradle project root from a configured Kotlin source directory, and reads the backend's `DEFAULT_AAR_ROOT` constant for the unconfigured case instead of repeating the literal.
- Fixed: `alef generate` wrote the unconfigured Kotlin/Android Gradle project to `packages/kotlin_android` while `alef build`, `lint`, `test`, `clean`, `setup`, the scaffolded `.gitattributes` and `sync_versions` all targeted `packages/kotlin-android`. The output template's `packages/{lang}` default was spelled from the config key; it now resolves to the backend's own `DEFAULT_AAR_ROOT`, so both halves name the same directory. Only projects with no `[crates.output].kotlin_android` entry are affected.
- Fixed: the `build.gradle.kts` snapshot for the Kotlin/Android backend did not record the `buildAndroidJniLibs` cross-compilation task, leaving `snapshot_basic` red.
- Escape map-key literals in `optional_renderers.rs` for rust, java, kotlin, kotlin_android, csharp, zig, dart, php, r, and c so a key containing `"` or `\` can no longer break out of its target-language string literal and emit code that fails to parse (#297).
- Filter a `json_object` argument's object literal against the bound type's declared fields before emitting it, so the typed-`const` snippet path (excess-property-checked by `tsc`) and the `as`-cast e2e test path (not checked) can no longer disagree about the same fixture; an undeclared key now panics generation naming the fixture, type, and key instead of silently compiling in one path and failing TS2353 in the other (#322).
- Fixed a quoted string map key in an e2e field path (`labels["theme"]`) being quoted twice. `parse_path` kept the surrounding quotes on the key and every renderer added its own, so Swift/Go/Java/Kotlin/Ruby/PHP/C#/R/C/Dart/Zig/Rust emitted the unparseable `labels[""theme""]` and TypeScript/wasm emitted `labels["\"theme\""]` — syntactically valid TypeScript that silently looked up a key no map holds. Quotes are now stripped once in `parse_path` (the only place that parses brackets); the renderers stay the sole owners of quoting. Single-quoted keys (`labels['theme']`) are handled the same way, and a quoted digit key (`labels["0"]`) now indexes identically to `labels[0]`.
- Fixed map keys being interpolated into generated string literals without escaping in the shared field-access renderers. A key containing `"` or `\` closed its literal early and emitted code that did not parse; the shared renderers now emit keys through a single escaping helper for TypeScript, Node, wasm, Go, Ruby, Elixir, Python, Gleam and Swift.
- Fixed: `alef e2e snippets-migrate` now honors `[crates.e2e.snippets].curated_snippets`. The command called the curated-unaware `migration::compare_root`, so every hand-authored file reported as `no_generated_equivalent` regardless of configuration and a consumer could not tell a declared, intentional curated snippet from a genuine coverage gap. The comparison and its report rendering moved to `bin_cli::snippet_migration`, which routes through `compare_root_curated`.
- Fixed: the `snippets-migrate` text report gives a declared curated file its own `curated` label instead of `no_generated_equivalent`, matching the `curated` flag the `--json` report already carried per entry.
- Fixed: `migration::compare_root` keyed existing files against `existing_root` and generated files against the configured `output`, so when `output` is a subdirectory of the migrated tree (`alef e2e snippets-migrate docs/snippets` against `output = "docs/snippets/generated"`) the two key spaces were disjoint by construction and every file alef itself had generated reported as `no_generated_equivalent`. One consumer saw 7796 false positives against 3 genuinely hand-authored files. The generated keys now carry the nested prefix so both sides key off `existing_root`; parallel roots are unchanged.
- Fixed a gap in the TypeScript e2e generator's undeclared-field filter: `ts_builder_expression_inner`'s declared-key guard only covered its own object literal and its recursive calls, not the SEPARATE `node_value_expression` path used to build nested struct-field object literals (an already-typed field's inner object value). A fixture with an undeclared key nested one level deeper than the top-level `json_object` argument silently reached both the docs snippet and the generated vitest test, reproducing the same snippet-vs-e2e asymmetry (excess-property-checked `const` binding vs unchecked `as` cast) the top-level filter (#322) was meant to close.
- Fixed: a documentation snippet no longer derives an accessor for a field the result type does not declare at any depth. The anchored availability oracle judged only a field path's first segment, so any path whose root was a real field was waved through whatever it named afterwards — one consumer shipped 28 non-compiling snippets reading a name that exists only as an `alef.toml` key (`Property 'documentStructure' does not exist on type 'ExtractedDocument'`). The oracle now walks the whole path through the IR struct graph, and abstains (deriving the accessor as before) whenever the walk leaves the graph into a map value, a `serde_json::Value`, a primitive, or a type outside the extracted surface.
- Fixed: C documentation snippets for trait-bridge registry operations now call the symbol the FFI backend exports. The derived ABI identity (`{prefix}_clear_{trait_snake}` / `{prefix}_unregister_{trait_snake}`) was applied only when no function name was configured at all, so a well-formed `[crates.e2e.calls.*] function` shadowed it — snippets called the plural `clear_fn` config text against a header that declares the singular trait-derived symbol, and lost the trailing `out_error` out-param with it. Only an explicit `[crates.e2e.calls.*.overrides.c] function` now outranks the derivation.
- Fixed: C# trait-bridge test stubs no longer carry an explicit `private` modifier. A docs snippet emits the stub at file scope after top-level statements, where `private` is CS1527; omitting the modifier is legal both there (defaults to `internal`) and nested in the e2e test class (defaults to `private`), matching the spelling `build_csharp_visitor` already uses.
- Fixed: Dart trait-bridge snippets now import `dart:typed_data` for every typed-list class the Dart mapper can emit, not just `Uint8List`. A stub whose methods take or return `Vec<f64>`/`Vec<i64>` spelled `Float64List`/`Int64List` with the library never imported. Both the stub emitter and the snippet preamble now ask `backends::dart::type_map` which names come from that library.
- Fixed: `alef --version` no longer reports `tree: DIRTY` for every binary installed with `cargo install --git`. Cargo drops a `.cargo-ok` completion marker into each checkout it creates, and the build stamp classified the working tree with `git status --porcelain`, which counts untracked files. Every git-installed binary therefore printed the "not reproducible from commit" warning, and a warning that fires on every install is one nobody reads — which is how genuinely dirty output ends up attributed to a commit it cannot be reproduced from.
- Stop reporting deliberately `#[alef::skip]`/`#[doc(hidden)]`-excluded functions and methods as missing snippet coverage — the snippet driver now checks the IR's `binding_excluded` flag before adding a fixture/language cell to `expected`, matching the carve-out the API-reference docs generator already applies for Rust's own page (Language::Rust is never excluded).
- A struct returned only through `Option<T>`/`Vec<T>`/a `Map` value (never returned bare) was wrongly classified as an input-only type, so `options.py` emitted it as a public dataclass while the generated function actually returned the native pyclass; `is_return_type` is now set from any function return type that references the struct at any depth, not only a bare `Named` return.
- An `AsyncMethod` adapter whose declared return type is a public `options` dataclass now converts the engine's native return value with the existing `_from_native_<snake>` converter before handing it back, instead of returning the native pyclass under a dataclass-typed annotation.
- An adapter param typed as a public `options` dataclass (on both `AsyncMethod` and the general `Streaming` param path) is now converted to the native pyclass with a `_to_rust_<snake>` converter before the engine call, instead of forwarding the dataclass instance directly.
- Fixed the Python `api.py` streaming wrapper yielding a type it does not declare. A streaming adapter whose item type is emitted as a public `options` dataclass annotated its return as `AsyncIterator[options.Item]` but re-yielded the native `_internal_bindings` pyclass, which pyrefly rejects with `invalid-yield`. The wrapper body now applies the `options._from_native_<snake>` converter alef already generates for that type, and imports it alongside the item type. Items with a single native identity (return types, opaque types) keep yielding the item unchanged.
- Fixed generated Rust documentation snippets for streaming fixtures failing to compile with `error[E0433]: cannot find module or crate 'tokio_stream'`. The streaming recipe drains a stream through `tokio_stream::StreamExt`, but nothing added the matching `crate:tokio-stream` requirement, so the snippet check project's `[dependencies]` never declared it. The three body-derived Rust crate requirements (`serde_json`, `tokio`, `tokio-stream`) now come from one table, and `tokio-stream` is pinned alongside them in the Rust snippet validator.
- Fixed generated Rust documentation snippets for streaming fixtures referencing a `result` binding their body never creates (`error[E0425]`). A streaming call binds `stream` and drains it into `chunks`; the snippet's tail was still rendered against the non-streaming result variable, both for field accessors and for the `match` an error fixture renders. A streaming snippet now presents the collection it actually binds, and an error-expecting streaming call is unwrapped before its stream is drained.
- Fixed: a docs snippet no longer drops a declared result field whose name collides with a legacy streaming pseudo-field name (`chunks`, `chunks.length`, `stream_content`, `tool_calls`, `finish_reason`, ...). The snippet show-derivation rejected those names unconditionally, while every e2e assertion renderer gates the same name list on `resolve_is_streaming` — so a non-streaming call whose result type genuinely declares such a field kept its e2e assertions but silently lost its snippet accessor. Both generators now ask `resolve_is_streaming` once and agree.
- `alef snippets check` no longer skips its gap pass silently. With neither `docs_dirs` nor `required_languages` configured under `[crates.docs.snippets]` the pass is still skipped, but the unset keys are now warned about by name, and under `strict` the skipped pass fails the run instead of reporting no failure.
- `alef all` now formats every file it emits. `poly` refuses to touch a file whose leading lines carry an `alef:hash:` line — under `--fix` and under `--check` alike — and `alef all` stamped each write phase's output immediately after writing it, ten `finalize_hashes` checkpoints all ahead of its single format pass. Every one of those stamps made the format pass a no-op for the files it covered, so binding, stub, public-API and scaffold output shipped in whatever shape the generator emitted. Measured on a neutral eight-language fixture: 21 of the 93 emitted files were files `poly fmt` would have rewritten — C# sources, `packages/java/pom.xml`, `packages/python/pyproject.toml` and `api.py`, Ruby, PHP, the node `.d.ts`, and the FFI cmake config.
- `alef all` also repairs output an earlier alef stamped without formatting. Such a file's generated body has not changed, so no writer rewrites it, so it keeps its stamp and stays invisible to every formatter forever; reordering alone reaches only the files that happen to change in the same run (measured: 6 of 21). The format pass now strips the `alef:hash:` line from the paths it is about to re-stamp, and a read-only `poly fmt --check --fix-generated` probe makes a run whose writers changed nothing still notice a non-canonical tree.
- Fixed `swift_shim_return_marshal` (Swift `FunctionParam` trait-box FFI shim) wrapping an enum-typed trait method return directly in `RustString(...)`, which requires a `String` argument and does not compile against an enum value; the shim now JSON-encodes the enum via `JSONEncoder` before wrapping it, decided by consulting `ApiSurface::enums` rather than the `TypeRef::Named` discriminant. Struct-typed (JSON) `Named` returns are unchanged.
- Fixed the Swift trait-bridge default method stub (`gen_single_trait_bridge_file`'s `default_body`) emitting `return "{}"` for a `has_default_impl` method with a non-excluded enum-typed return, which does not type-check against the enum's own declared Swift return type; it now constructs a real case of the enum (via a new `unit_enum_default_case` IR lookup) when one is available, falling back to the prior placeholder body otherwise.
- **swift**: derive the Rust bridge crate's literal Cargo feature-list widening from cfg alternatives (`#[cfg(any(feature = "a", feature = "b"))]`) discovered in the parsed API surface instead of a hard-coded pair of feature names, so any source crate that gates a capability behind two sibling features benefits, not just one whose features happened to be named a particular way.
- Fixed: `packages/swift/Sources/RustBridgeC/RustBridgeC.h` no longer changes shape between otherwise identical runs. Both writers of the umbrella header (the Swift scaffold and the `MaterializeSwiftBridge` post-build step) now compare a freshly assembled header against the committed one as C rather than as text, and keep the committed bytes when the declarations are unchanged. Previously the raw swift-bridge concatenation overwrote the formatted committed header on every run; because `poly fmt` skips any file carrying an `alef:hash:` line, whether the shipped header ended up formatted or raw depended on whether the format pass happened to see it before it was stamped — the same inputs produced two different files, hundreds of lines apart, with `alef` exiting 0 either way.
- Fixed: a partial swift-bridge build output can no longer produce a plausible-looking but uncompilable `RustBridgeC.h`. The assembled header is now checked for completeness — every `RustStr` / `__private__*` type its declarations reference must also be defined in it — instead of being trusted because both input files existed. A partial assembly keeps the committed populated header and warns; with nothing populated to fall back on, `alef` now fails with a message naming the undefined types and the build directory the inputs came from, rather than writing the degraded header and exiting 0.
- Fixed the Swift inbound trait bridge so `Vec<Named>` return values and params convert per-element instead of falling through as a raw `Vec<String>`/`Vec<Named>` type mismatch (alef-tasks #308).
- Fixed `inbound_bridge_type`'s `Map` arm to declare a single JSON `String` blob for the extern "Swift" boundary instead of a typed `HashMap<K, V>` that swift-bridge cannot parse (alef-tasks #309).
- Fixed the outbound Swift trait-bridge protocol (`swift_type_name`) to declare `Map` as a single JSON `String` blob instead of `[K: V]`, which previously double-encoded `Map<_, Named>` values (alef-tasks #309).
- Fixed the outbound box shim's return marshal catch-all to wrap the bridge call in `RustString(...)` whenever the shim's FFI return type is `RustString`, fixing a compile break for `Map` returns (and any other catch-all shape) that the Map fix above exposed.
- Fixed Swift documentation snippets subscripting, indexing, or iterating a field swift-bridge JSON-bridges to a single `RustString`. The snippet generator emitted `labels()["theme"]` for the very field the e2e file generated beside it from the same IR declared unspellable, so the snippet could not compile. A `show` operation whose path steps past a JSON-bridged leaf is now clamped to that leaf — the case the e2e derivation explicitly blesses, so the reader still sees the field — and an `iterate` over such a leaf is dropped.
- Fixed (swift): `FunctionParam` trait-bridge protocols and `Swift{Trait}Box` shims are emitted into `Sources/RustBridge/`, but `excluded_named_type_bridge_policy` exempted `has_default_impl` methods from the JSON-string boundary policy. A defaulted method's `Named` types therefore kept their real Swift names in the emitted signature, which cannot resolve in `RustBridge` — the package's target graph runs `<Module> -> RustBridge`, so public DTOs live downstream. Every `Named` type a bridged trait mentions now crosses as a JSON `String`, defaulted or not, matching the contract `gen_rust_crate::plugin_inbound` already documented and restoring the invariant intended by "exclude all Named types in trait bridges". This is the root cause of #258; it affected parameter position as well as return position.
- Fixed (swift): reverted the 0.67.5 change that JSON-encoded an enum-typed trait-method return before wrapping it in `RustString`. `JSONEncoder().encode(...)` required the DTO to be nameable and `Encodable` in `RustBridge`, where it is neither, so it traded one compile error for another; with the boundary policy corrected the bridge call already yields a JSON `String` and re-encoding would double-encode the payload.
- `resolve()` on an `alef.toml` with zero `[[crates]]` entries now returns `ResolveError::NoCratesConfigured` instead of `Ok(vec![])`, so `alef go-tag`, `alef validate versions --exit-code`, and `alef publish validate` can no longer silently process zero crates and exit 0.
- `alef check-registry --registry github-release` now warns when it verified only that the release exists (no `--asset-prefix` or `--required-assets` given), so a CI variable that expanded to nothing is no longer indistinguishable from "all N artifacts are attached"; `--registry zig`/`--registry swift` are unaffected since they intentionally check existence only.
- `alef e2e validate` now applies the same `[e2e].languages` fallback (to the crate's scaffolded languages) that `alef e2e generate --snippets-migrate` and `alef test-apps run` already applied, so an unset `[e2e].languages` no longer silently disables the "0 test functions" and "unsupported language" checks. The success message also now distinguishes "no fixtures found" from "N fixtures validated".
- `alef lint` now fails when `poly` is not on PATH instead of warning and reporting a clean run; `poly` is the entire implementation of `alef lint`, so there was no partial coverage to report.
- `alef release-metadata --targets` now rejects a CSV that trims to non-empty but splits into zero real target tokens (e.g. `,,,` or a lone `,`), which used to resolve identically to the deliberate `--targets none` (`release_any: false`, exit 0, no diagnostic).
- Fixed Swift trait-box shims for a `Vec<Named>` in return or parameter position. A bridged `Named` crosses as a JSON `String`, so the bridge protocol declares `[String]` and `inbound_bridge_type` declares `Vec<String>`, but `swift_shim_return_ffi_type` fell through to `RustString` and `swift_shim_return_marshal` returned the `[String]` unchanged — `cannot convert return expression of type '[String]' to return type 'RustString'`. On the parameter side the decode called `.toString()` on a `RustVec<RustString>`, which has no such member. Both now use the element-wise `RustVec` conversion `Vec<String>` already used.
- Fixed a Swift trait-box shim returning `Option<Named>`, which handed the bridge's `String?` back where the shim declares `RustString`. `nil` is now sent as the JSON literal `null`, which is what the Rust wrapper's `serde_json::from_str::<Option<T>>` expects.
- Fixed non-throwing `()`-returning Swift trait-box shims never calling the conformer. The marshaller emitted only `return ()` and discarded the bridge call, so every such method was a silently compiling no-op.
- `alef verify` no longer reports create-once scaffold seeds as frozen generated files. `FrozenFile` means "alef would write this path and the write guard refuses it forever", and for a create-once seed the antecedent is false: alef emits the path only when it is absent, so on an existing file there is no write to refuse and nothing is lost by the missing marker. Verify nonetheless listed them under a "Frozen create-once seeds detected" heading whose only remedy was `alef adopt <path> --write --clobber-create-once-seeds` -- a flag alef's own output labels DANGEROUS -- for files this project's documentation calls user-owned after scaffold (`generated_header: false`). Measured in a consumer repo: `alef adopt --converged-only` adopted 0 of 102 reported paths, 72 of them refused by alef itself as seeds, including 13 LICENSE files, `e2e/java/mvnw`, `kotlin-android/gradlew`, `build.zig.zon` and several `.gitkeep`s. Recording ownership instead was considered and rejected: it proves nothing further (alef still never rewrites the body, and the stamp covers generation inputs rather than the seed's hand-grown contents) while handing the write guard the licence `--clobber-create-once-seeds` exists to gate. The count is not dropped -- it is stated in the new coverage report on every run, including a clean one, whereas the old heading was printed only when some other check had already failed. Extends the 0.67.3 stale-seed fix rather than duplicating it: that one taught the stamping pass to ask the question verify asks (`stampable_output_paths`); this one removes the finding verify had no answer for.
- **A stamped `.clang-format` was written and then never read back by anything.** The ownership walk's scan set is documented to be a superset of everything the emit table can stamp, and it had drifted: `.clang-format` is scaffolded `generated_header: true` for every FFI target and is stamped on write (it is YAML, so `#` line comments apply), but a dotfile with a single leading dot reports `Path::extension() == None` and the name was never added to `VERIFY_SCAN_FILENAMES`. The walk filters on name and extension before reading any content, so the file was not merely unverified but unverifiABLE. The scan predicate now asks the emit side's own `is_markable_path` directly, so anything alef stamps on write is opened on read and the two sides cannot drift apart again by hand. Widening the scan set only causes more files to be read; a file is still reported only when it carries a marker, so this adds no false positives.
- Fixed: a documentation snippet now passes every argument the target binding declares required. The snippet generator read a fixture's `optional` argument flag as the arity of both JavaScript bindings, but only NAPI widens a parameter whose type derives `Default` to `settings?:` in its `.d.ts`; wasm-bindgen emits the parameter from the Rust signature alone. A wasm snippet therefore ended its argument list where only node may, which `tsc` rejects with `TS2554: Expected 2 arguments, but got 1`. Arity is now decided per target through `ParamOptionalityRule`, whose node arm calls the NAPI backend's own `param_is_optional` so the snippet and the declaration it compiles against cannot drift.
- Fixed: a generated loop binding no longer takes the name of the collection it iterates. A fixture may author `iterate` with the same name the call's result is bound to, which Rust, Python and Go accept but TypeScript rejects (`for (const result of result.results)` is `TS2448`/`TS7022`, not a shadow) and Java rejects as a redeclaration. The name is now decided once, before any accessor is rendered, and only when it actually collides.
- Fixed: a WASM documentation snippet that names a type `[crates.wasm] exclude_types` keeps out of the binding is refused instead of published. The snippet's imports come from the crate IR, which is not the package's export list, so the snippet imported a symbol that does not exist. This is the type-side twin of the existing refusal for a function the WASM target does not export, and it records a coverage gap the same way.
- Fixed: `wasm` docs snippets now optional-chain an access through an `Option<T>` field, matching the `node` snippet for the same fixture. `FieldResolver::accessor` dispatched `typescript`/`node` to the optionality-aware TypeScript renderer and let `wasm` fall through to a second renderer that knew nothing about optionality, so one fixture produced `result.document?.nodes` for node and the `TS18048` `result.document.nodes` for wasm — every wasm structure snippet on an optional field failed to type-check.
- Fixed: `configured_swift_features`'s cfg-alternative widening (alef-task #306) fed a companion feature straight into the core dependency's `features = [...]` line without checking `[crates.swift].excluded_default_features`, bypassing the exclusion entirely and reactivating an opt-in native/pkg-config-only feature that was never in the core crate's own default set. Confirmed as a net-new regression (not already-latent Cargo unification) via a synthetic fixture workspace measured with `cargo tree -e features`. `excluded_default_features` now blocks widening a companion into that line, not just into the wrapper's own `default = [...]` array.
- Fix a real flake in `src/bin_cli/core_commands/post_build_format_order_tests.rs` (`all_formats_scaffold_output_before_stamping_it`, `all_reformats_a_scaffold_file_left_stamped_and_uncanonical_by_an_earlier_run`), which failed under parallel `cargo test --lib` load with `Blocking waiting for file lock on package cache` and passed in isolation/on re-run. Root cause: these tests' `run_all` helper drives real `alef all`, whose full-regen `converge_full_regen` residuals (`cargo fmt --all`, `cargo sort -n -w`) spawn genuinely real `cargo` subprocesses whenever the fixture has a root `Cargo.toml` -- unlike `ALEF_SKIP_COMMANDS`/`SkipCommandsGuard`, which only gates `PostBuildStep::RunCommand`, not these residual passes. `crate::test_support::CWD_LOCK` did not close this: four tests in `src/cli/pipeline/format/tests.rs` (`run_workspace_cargo_sort_sorts_every_member_regardless_of_language`, `run_cargo_fmt_formats_workspace_rust_files_when_available`, `converge_full_regen_formatting_leaves_workspace_sorted_and_poly_fmt_check_clean`, `format_generated_full_regen_routes_through_convergence_loop`) call the real-cargo-spawning functions directly with an explicit `base` argument and never touch the process cwd, so they never take `CWD_LOCK` and can run concurrently with `alef all`'s own cargo invocations on cargo's shared machine-wide package-cache lock. Added a new shared `REAL_CARGO_LOCK` / `RealCargoGuard` to `src/test_support.rs`, mirroring the existing `CWD_LOCK`/`SKIP_COMMANDS_LOCK` pattern, and wired it into all five affected tests. No assertions were weakened; both flaky tests still assert the same stamp-before-format defect they were written to guard.
- Hardened a regression test's `poly fmt --check .` assertion (`post_build_format_order_tests.rs`) with `--fix-generated`: without the flag the assertion was vacuous for the post-build-owned header it targets, since poly skips any hash-stamped file under a plain `--check` regardless of its body.
- Fix `frb_version_check.rs`'s test module failing to compile on Windows (`std::os::unix::fs::PermissionsExt`, `Permissions::from_mode`) by gating the four unix-only items (`fake_codegen_binary` and the two tests that use it, plus the now-unix-only imports) behind `#[cfg(unix)]`, while keeping the four platform-neutral tests running unconditionally on every OS.
- Rebuild `ESCAPING_LANGUAGES` in `map_key_quoting_tests.rs` to drive per-language assertions directly (was a decorative length check only) and extend it to all 19 e2e target languages.
- Corrected a false claim in `post_build_format_order_tests.rs`, which asserted in prose that `alef all` "does not have this defect". It did, for every phase. The `alef all` scaffold phase now has executable coverage instead: one test that a scaffold-phase file the formatter would change ships canonical, and one that a file left stamped-and-uncanonical by an earlier run is repaired. Both open with an anti-vacuity control that fails loudly if the generator's own output ever becomes canonical.
- Fixed the RED swift e2e integration test `count_min_on_optional_vec_of_named_uses_native_optional_count`, which shipped red through 0.67.3 and 0.67.4.
- The test's premise was stale, not the codegen: it asserted an opaque parent's `Option<Vec<Named(struct)>>` field bridges natively as `Optional<RustVec<T>>`, but `field_needs_json_bridge` in `src/backends/swift/gen_rust_crate/type_bridge.rs` has no `is_opaque` dependence — it returns true unconditionally for any optional `Vec<_>` field, and both `wrappers::getters::emit_getters` and `extern_block::emit_extern_block_for_type` check it before `parent_first_class` is ever consulted.
- Confirmed by rendering the exact fixture: the field emits `result.elements().toString().count`, never `elements()?.count ?? 0`, regardless of the parent's opacity.
- Split the single test into two paired tests in `tests/e2e_swift_residual_e2e_codegen_bugs.rs` — opaque parent and first-class parent — both asserting the JSON-bridged `.toString().count` shape, so a fix that corrects one parent shape while silently regressing the other is caught by whichever arm it breaks.
- No production code changed; `src/e2e/codegen/swift/values.rs` and `src/backends/swift/**` are untouched.
- Restored the `Map<_, Named>` probe in the two-target SwiftPM trait-box compile gate (`trait_box_swiftpm_compile.rs`), which had been dropped.

### Removed

- Removed `bad-return` and `bad-argument-type` from the default `[[tool.pyrefly.sub-config]]` suppression list scaffolded for `**/api.py` — both were masking the boundary-mismatch bugs above. `bad-argument-count`, `not-iterable`, and `missing-attribute` remain suppressed; they were not diagnosed as part of this fix. Existing consumers keep their already-scaffolded `pyproject.toml` (scaffold output is user-owned after first write) until they regenerate or manually adopt the new default.

## [0.67.5] - 2026-08-24

### Added

- Added `[crates.e2e.snippets].curated_snippets`: glob patterns (relative to `output`) declaring hand-authored snippet files as curated on purpose rather than alef-generated. Resolved into `SnippetGenerationReport::curated_paths` and into `migration::MigrationEntry::curated`, so both the generation report and `alef e2e snippets-migrate` can distinguish a declared, intentional absence of a generated equivalent from a genuine coverage gap.
- A `curated_snippets` pattern that matches zero files, or that matches a path alef itself generates, now fails the run instead of being silently accepted.
- Implemented `render_snippet_body` for the brew (shell) e2e code generator: documentation snippets for CLI-based bindings now render a single `binary subcommand "<url>" --flags` line, built from the same call-config resolution the executable brew e2e suite already uses.

### Fixed

- `docs.snippets` validation now fails fast, before any toolchain runs, when a language needs a compiled artifact (`compile`/`typecheck`/`run`) but has no configured session that could plausibly have produced one yet -- no session at all, an ambiguous session, or a session with an empty `before` list. Warns always; under `strict`, bails immediately instead of spending an hour validating snippets that were doomed from the start (GH #256).
- `alef snippets check --lang <language>` (and any other filtered `run_validation` call) now prepares only the configured sessions its filtered snippet set actually needs, instead of running every configured `before` build hook regardless of the filter -- a single-language diagnostic no longer pays for every other language's build. Sessions sharing a working directory with a needed one are still prepared together, so the scratch sweep never treats a cohabiting session's live build cache as abandoned.
- `resolve()` on an `alef.toml` with zero `[[crates]]` entries now returns `ResolveError::NoCratesConfigured` instead of `Ok(vec![])`, so `alef go-tag`, `alef validate versions --exit-code`, and `alef publish validate` can no longer silently process zero crates and exit 0.
- `alef check-registry --registry github-release` now warns when it verified only that the release exists (no `--asset-prefix` or `--required-assets` given), so a CI variable that expanded to nothing is no longer indistinguishable from "all N artifacts are attached"; `--registry zig`/`--registry swift` are unaffected since they intentionally check existence only.
- `alef e2e validate` now applies the same `[e2e].languages` fallback (to the crate's scaffolded languages) that `alef e2e generate --snippets-migrate` and `alef test-apps run` already applied, so an unset `[e2e].languages` no longer silently disables the "0 test functions" and "unsupported language" checks. The success message also now distinguishes "no fixtures found" from "N fixtures validated".
- `alef lint` now fails when `poly` is not on PATH instead of warning and reporting a clean run; `poly` is the entire implementation of `alef lint`, so there was no partial coverage to report.
- `alef release-metadata --targets` now rejects a CSV that trims to non-empty but splits into zero real target tokens (e.g. `,,,` or a lone `,`), which used to resolve identically to the deliberate `--targets none` (`release_any: false`, exit 0, no diagnostic).

- Fixed the FFI feature-drift warning comparing each binding's configured feature set against `[crates.ffi]`'s configured set instead of the effective default set (`[crates.ffi]`'s configured features unioned with every feature discovered from emitted `#[cfg(feature = "X")]` gates) that `scaffold_ffi` actually writes into the generated FFI crate's `Cargo.toml`. Two configured lists could match while the effective set was a strict superset, and the warning stayed silent through that drift.
- Added `codegen::cfg::effective_ffi_default_features` as the single derivation of the FFI crate's effective default feature set, used by both `scaffold_ffi` and `warn_on_ffi_feature_drift` so the two can no longer disagree.
- `warn_on_ffi_feature_drift` now distinguishes unsafe host-only features (configured for a binding but absent from the FFI crate's effective default set, which can produce glue referencing a symbol the shipped library was never built with) from safe parity gaps (in the effective default set but undeclared by a binding, which the binding simply omits).
- Fixed `FfiTargetDepOverride.default_features` (per-target `[crates.jni] target_dep_overrides`) being ignored by the JNI Cargo.toml scaffolder: a per-target `default_features = false` never reached the generated `[target.'cfg(...)'.dependencies]` block, unlike the equivalent FFI crate scaffolding.

- Fixed snippet session identity so multiple configured targets that resolve to the same `Language` (e.g. `kotlin` + `kotlin_android`, or `typescript`/`node`/`wasm`) no longer collide when they validate the same physical package/working directory. `resolve_session_claim` now only reports `SessionClaim::Ambiguous` when same-language candidates validate genuinely different working directories; candidates sharing one directory collapse to a single deterministic `SessionClaim::Claimed` instead (issue #255).
- Added `SessionIdentity` trait (`src/snippets/runner/session_resolution.rs`) implemented for `ValidationSession` and `SessionSpec`, giving session-claim resolution access to a session's working directory alongside its language.
- Added regression coverage asserting session count collapses to one for `kotlin` + `kotlin_android` over one directory and for `typescript` + `node` + `wasm` over one package, plus a control case proving genuinely different directories still resolve as ambiguous.

- Swift snippet validation now resolves a session's SwiftPM module directories once per run.
  Every snippet previously launched its own `swift build --show-bin-path`; 32 concurrent warm
  lookups measured 1.33 seconds wall and 9.22 seconds CPU, versus 0.33 seconds wall and 0.26
  seconds CPU for a single lookup, before any `swiftc` validation work began. The cache is
  keyed on the resolved lookup inputs (package root plus environment) rather than on session
  identity, and holds no global state.

- `alef validate versions --exit-code` now asks `checks_pass` for its verdict instead of
  re-deriving it. The local copy disagreed in both directions: it exited 0 for a crate whose
  check set was EMPTY — the vacuous pass `checks_pass` explicitly refuses — and it exited 1
  on a `blocked_on_publish` row, which `checks_pass` deliberately tolerates because such a
  row is a lockfile entry pinning the crate at the very version being released and cannot
  resolve until that version is published. Failing it made the gate unsatisfiable by
  construction for any repo with a registry-depending test app. `--json` already reported
  `checks_pass`, so a single invocation could print `"ok": true` and still exit 1. The
  `blocked_on_publish` doc comment, which asserted the opposite and contradicted both
  `checks_pass` and its tests, is corrected.

- Fixed `alef build`/`alef generate` running the umbrella `gradle build` (and `gradle build -Prelease`) for `kotlin_android` when no `[workspace.build_commands.kotlin_android]` overlay is declared, instead of the intended `gradle assembleDebug`/`gradle assembleRelease`. `build_command_for`'s `"gradle"` arm matched on the shared `bc.tool` string, which cannot distinguish `Kotlin` from `KotlinAndroid`; it now asks a new shared `build_defaults::gradle_build_task(Language, bool)` helper, the same one `default_build_config` uses, so both derivations agree (GH #259). An explicit `[workspace.build_commands.kotlin_android]` overlay is unaffected and continues to win.
- Fixed `frb_version_check.rs`'s test module failing to compile on Windows (`std::os::unix::fs::PermissionsExt`, `Permissions::from_mode`) by gating the four unix-only items behind `#[cfg(unix)]`, while keeping the four platform-neutral tests running unconditionally on every OS.
- Fixed `swift_shim_return_marshal` (the Swift trait-box FFI shim) wrapping an enum-typed trait method return directly in `RustString(...)`, which requires a `String` argument and does not compile against an enum value; the shim now JSON-encodes the enum via `JSONEncoder` before wrapping it, decided by consulting `ApiSurface::enums` rather than the `TypeRef::Named` discriminant. Struct-typed (JSON) `Named` returns are unchanged.
- Fixed the Swift trait-bridge default method stub emitting `return "{}"` for a `has_default_impl` method with a non-excluded enum-typed return, which does not type-check against the enum's own declared Swift return type; it now constructs a real case of the enum when the IR has a fieldless variant, falling back to the prior placeholder body otherwise.
- Fixed the packaging template environment (`src/publish/package/template_env.rs`) never calling `strip_keep_markers`, the only built-in render path that did not, so a `~keep` marker left in a packaging template would have shipped verbatim into a consumer's package tree.
- Corrected the swift e2e integration test `count_min_on_optional_vec_of_named_uses_native_optional_count`, which shipped red through 0.67.3 and 0.67.4. Its premise was stale rather than the codegen: `field_needs_json_bridge` has no dependence on the parent's opacity and returns true for any optional `Vec<_>` field, so the JSON-bridged `.toString().count` shape is correct for both parent kinds. Split into paired opaque-parent and first-class-parent tests so a fix that corrects one shape while regressing the other is caught by whichever arm it breaks.

## [0.67.4] - 2026-08-24

### Fixed

- a snippet session's `before` hook is now run once per package instead of once per configured session target. `kotlin` and `kotlin_android` both resolve to `Language::Kotlin`, and `typescript`/`node`/`wasm` all resolve to `Language::TypeScript`, so several targets routinely describe one physical package and each carried its own copy of that package's hook. Every copy was executed, sequentially, before a single snippet could validate — and when the hook outran `timeout_secs`, the run paid that whole timeout once per target. Within an activation group a hook whose command and environment match one already attempted now replays that attempt's outcome; failures replay too, preserving the timeout classification every affected target is reported with.

- `run_command` no longer outlives the timeout it was given. The budget covered only the wait for the direct child; once that child exited, output collection waited for end of stream on pipes every descendant had inherited, so any process outliving the command — a Gradle daemon, an MSBuild node, an unwaited background job — held the call open indefinitely. A one-second budget was measured taking twenty seconds and still returning success. Output readers now buffer as bytes arrive and the drain gives up at a fixed grace, reporting everything the command actually wrote and tearing down the process group of anything still holding the pipes.

- `SIGINT`, `SIGTERM` and `SIGHUP` are now forwarded to every snippet subprocess group before alef exits. Snippet children are spawned into their own process group so a timeout can kill the whole tree, which also removed them from the terminal's foreground group: Ctrl-C reached alef and nothing else, so alef exited 130 while the entire hook tree — shell, build wrapper and build daemon — survived and reparented to PID 1, where a stale daemon goes on to poison the next run. A signal already ignored on entry stays ignored.

- `alef verify` refuses `--compile`, `--lint` and `--lang` instead of discarding them. All
  three are visible, documented flags (`--compile` reads "Also run compilation check") that
  the command destructured away, so `alef verify --compile` exited 0 having compiled
  nothing — indistinguishable from a passing compile check. They now fail with a message
  naming `alef build --lang` and `alef lint --lang`, which do implement that work.
  `--exit-code` is unaffected: it is a hidden, documented no-op. Nothing in the polyrepo
  passes the refused flags today.

- `alef --version` no longer reports `tree: DIRTY` for every binary installed with `cargo install --git`. Cargo drops a `.cargo-ok` completion marker into each checkout it creates, and the build stamp classified the working tree with `git status --porcelain`, which counts untracked files. Every git-installed binary therefore printed the "not reproducible from commit" warning, and a warning that fires on every install is one nobody reads — which is how genuinely dirty output ends up attributed to a commit it cannot be reproduced from.

- `alef snippets gaps` now prints a gap-coverage report on every run — snippet roots and files discovered, documentation roots and pages actually opened, references found versus supplied by configuration, and required languages against snippet groups compared — so a "No gaps found." result can no longer read as a wider claim than the check made. A consumer that omitted `required_languages`, `docs_dirs` and `include_base_paths` from its `alef.toml` previously read a clean gap report for a run in which the language-parity check never executed and not one documentation page was opened.

- `alef snippets gaps` now names every unset input (`docs_dirs`/`--docs`, `required_languages`/`-L`, `include_base_paths`/`--include-base-path`) together with the check class its absence disables.

- `alef snippets gaps` gained `--strict`, which fails the run when an unset input left a check class with nothing to compare, so a CI job whose purpose is gap detection cannot go green by being unconfigured. An unset `include_base_paths` is reported but deliberately not strict-fatal: it makes include targets over-report rather than manufacture a false clean.

- `alef snippets check` no longer skips its gap pass silently. With neither `docs_dirs` nor `required_languages` configured under `[crates.docs.snippets]` the pass is still skipped, but the unset keys are now warned about by name, and under `strict` the skipped pass fails the run instead of reporting no failure.

- Split `src/snippets/gaps.rs` unit tests into `src/snippets/gaps/tests.rs`, dropping the file under the repository's 1,000-line cap and removing its file-size ratchet baseline entry.

- **Java visitor bridge now derives the context struct layout from the IR.**
  `VisitorBridge.java` hardcoded a six-field context — `tagName`, `depth`, `indexInParent`,
  `parentTag`, `isInline` — with fixed offsets, a fixed `MemoryLayout`, and a fixed six-argument
  `decodeContext` return. Generated Java only compiled when the configured `context_type` happened
  to be exactly `(enum|i32, ptr, i64, i64, ptr, i32)`; every other shape failed `javac` with
  `constructor <Context> in record <Context> cannot be applied to given types`. The layout,
  the field offsets, the Panama value layout per field and the constructor arguments are now
  derived per context type, so any field count, order, and scalar width compiles.

- **The visitor context C ABI is derived once, in `codegen::visitor_context_abi`.**
  The FFI backend's `context_c_type` / `context_field_specs` decided the `#[repr(C)]` shape and
  which fields have no C representation; the Java bridge re-stated that shape by hand and the two
  drifted. Both backends now read the same derivation — field order, scalar widths, `#[repr(C)]`
  padding, struct size, and the skip decision.

- **Context fields the C struct cannot carry are decoded as Java's own zero value.**
  The FFI backend drops fields with no C representation (floats, collections, nested structs, any
  optional that is not `Option<String>`). The record component still exists, so the Java bridge
  passes `null` for reference components and the primitive zero for value components rather than
  fabricating a value or refusing to emit the bridge at all — the options record that holds the
  callback references the visitor interface whether or not the bridge exists.

- **A payload-carrying context enum is no longer decoded from its discriminant.** The Java binding
  emits tagged and untagged unions as sealed interfaces with no `values()`, so an ordinal cannot
  reconstruct a variant; such a component now takes the absent value instead of emitting Java that
  does not compile.

- `FieldResolver::accessor` and `FieldResolver::rust_unwrap_binding` each carried a private copy of
  the virtual-namespace strip decision, gated on `result_fields.contains(..)` where the shared
  `result_relative_path` asks the broader `is_valid_for_result(..)`. The copies could place the same
  fixture field somewhere the classifiers did not — the defect shape that emitted
  `string(result.ActionResults)` into a generated Go package. Both now call `result_relative_path`,
  so accessor emission, `is_array`, and the zig/brew/C serialized-path navigation share one
  definition of where a field's value lives.

- An accessor whose virtual prefix hides a field the IR reaches but a hand-maintained `result_fields`
  omits now strips that prefix, instead of emitting a member access against the virtual label.

- A `result_fields` entry the IR marks `binding_excluded` no longer strips its virtual namespace
  prefix in accessor emission. `with_ir_fields` already warns that such an entry is a config bug and
  no binding emits an accessor for the field, so neither spelling compiles; the accessor now agrees
  with `is_array` and the serialized-path generators rather than keeping a private answer.

Investigated whether `alef adopt`'s `--clobber-create-once-seeds` over-gates an
**unmarkable** create-once seed. It does not: the gate is correctly protective. Only the
*timing* stated in the warning was wrong. Bullets below, for `### Fixed`.

```markdown

- `alef adopt`'s create-once-seed warning no longer names the wrong command as the moment of
  loss. It said adopting a seed consents to alef "replacing its contents with a placeholder
  seed on the next generate", but `write_scaffold_files_report`'s `can_skip`
  (`!overwrite && !generated_header && exists && !is_alef_derived_output`) runs before the
  ownership guard and consults no ownership signal, so a plain `alef generate` skips an
  adopted seed exactly as it skips an unadopted one. The replacement lands on the next write
  that passes `overwrite: true` -- an `alef version` scaffold regen, or
  `alef all --clobber-create-once-seeds`. An operator who tested the warning by running
  `alef generate`, saw the file untouched, and concluded the warning was false would have been
  reading accurate output; the loss was simply still days away. The flag help, the
  `NOT ADOPTED -- create-once seeds` stdout block, the per-path `warn!` and the seeds-only
  `bail!` now all name the overwriting regen and say a plain generate skips these paths.

- The gating itself is unchanged, and is now pinned by tests rather than argued from doc
  comments. For an unmarkable seed (`LICENSE`, `mvnw`, `gradlew`, `.gitkeep` -- paths
  `marker_comment_style` answers `None` for), `alef adopt --write --clobber-create-once-seeds`
  writes no byte of the file: `stamp_for` yields `None`, so the entire adoption is one entry in
  the committed `.alef-ownership.toml`. That entry is precisely what
  `write_scaffold_files_report` accepts as proof of ownership for an unmarkable path
  (`owned = has_marker || (!is_markable && is_owned_by_ownership_record(..))`), so the
  adoption is what clears the guard for the next overwriting write. Five tests in
  `cli::commands::adopt::tests::create_once_seeds` measure the bytes on both sides of the
  adoption and both sides of the write, including a control proving the identical
  `overwrite: true` write refuses when the adoption did not happen.
```

Note for the integrator: the 0.66 entry ("adopted through the committed record without its
contents being touched. It remains a create-once seed, so `--clobber-create-once-seeds` is
still required") is accurate as written and needs no correction — both halves of it are true,
and it never claimed the flag was unnecessary.

- poly fmt no longer demotes every heading in a Markdown file that contains a stray second
  level-1 heading. The generated `poly.toml` now disables rumdl's `MD025` for `fmt` only: the
  rule still lints, so a stray `#` heading is reported, but its autofix demotes the offending
  H1 *and every heading after it*, which reparents an entire CHANGELOG under `## [Unreleased]`
  and makes heading-scoped release-note extraction emit the wrong section. Observed on this
  repo's own CHANGELOG: 338 lines rewritten, all 122 version sections pushed to H3.

- Java: a visitor upcall whose host callback throws now returns the result enum's default
  discriminant instead of `VISIT_RESULT_ERROR`, a constant the bridge never emitted. Every
  generated `VisitorBridge.java` previously failed to compile with
  `cannot find symbol: variable VISIT_RESULT_ERROR`.

- C e2e generator: the plain-function and engine-factory accessor emitters each carried a
  hand-inlined copy of the virtual-namespace strip instead of calling
  `FieldResolver::result_relative_path`, whose own documentation records that no further
  copy should exist. Both now call the shared helper, so a fixture field grouped under a
  virtual label (`interaction.total_count` addressing `total_count`) cannot be classified
  one way by the resolver and another by the emitter. Behaviour is unchanged; the
  divergence risk is not. Deleting the strip outright previously kept all 298 C codegen
  tests green, so the path had no coverage at all — both branches are now pinned by
  regression tests that fail when the strip is removed.

### Changed

- Java: the convert-with-visitor `operationFailure` slot is typed as the crate exception rather
  than `Throwable`. Every value assigned to it already is that exception and the `Throwable`
  clause of the catch chain rethrows the slot itself, so the `Throwable` typing compiled only by
  virtue of the enclosing outer `catch (Throwable)`.

- a snippet run that publishes the unconfigured `https://example.com` fallback now
  warns once, naming the affected fixtures and the config key that fixes it, and records
  them on `SnippetGenerationReport::placeholder_sample_url_fixtures`. Generated output is
  unchanged when `sample_base_url` is unset. An unusable `sample_base_url` (empty,
  whitespace-bearing, or scheme-less) fails generation instead of silently falling back.

- the build-time working-tree classifier now asks `git diff --quiet HEAD` — tracked paths only, index and working tree both, so a staged addition or a deletion still counts as dirty. Untracked files no longer count: reaching the compiler requires a `mod`/`include!` chain rooted at a tracked `src/lib.rs`, so untracked source that actually affects the build drags a tracked modification along with it. A denylist would have covered `.cargo-ok` and then waited for the next tool's marker file.

- a repository with no commit yet now stamps `unknown` instead of `clean`. There is no `HEAD` to call the tree clean relative to, and `clean` reads as a provenanced build.

### Added

- Java: `tests/backends_java_visitor_compile_test.rs` compiles the generated options-field visitor
  path with a real `javac` — the convert-with-visitor method together with the generated helpers
  it calls, and the generated `VisitorBridge` — so type errors in the emitted exception flow and
  upcall handlers are caught instead of passing substring assertions.

- `alef verify` no longer reports create-once scaffold seeds as frozen generated files.
  `FrozenFile` means "alef would write this path and the write guard refuses it forever", and
  for a create-once seed the antecedent is false: alef emits the path only when it is absent,
  so on an existing file there is no write to refuse and nothing is lost by the missing marker.
  Verify nonetheless listed them under a "Frozen create-once seeds detected" heading whose only
  remedy was `alef adopt <path> --write --clobber-create-once-seeds` -- a flag alef's own output
  labels DANGEROUS -- for files this project's documentation calls user-owned after scaffold
  (`generated_header: false`). Measured in a consumer repo: `alef adopt --converged-only`
  adopted 0 of 102 reported paths, 72 of them refused by alef itself as seeds, including 13
  LICENSE files, `e2e/java/mvnw`, `kotlin-android/gradlew`, `build.zig.zon` and several
  `.gitkeep`s. Recording ownership instead was considered and rejected: it proves nothing
  further (alef still never rewrites the body, and the stamp covers generation inputs rather
  than the seed's hand-grown contents) while handing the write guard the licence
  `--clobber-create-once-seeds` exists to gate. The count is not dropped -- it is stated in the
  new coverage report on every run, including a clean one, whereas the old heading was printed
  only when some other check had already failed. Extends the 0.67.3 stale-seed fix rather than
  duplicating it: that one taught the stamping pass to ask the question verify asks
  (`stampable_output_paths`); this one removes the finding verify had no answer for.

- **`alef verify` now reports its own coverage on every run.** Every finding verify produces is
  a negative claim, so a green result was indistinguishable from a run that examined nothing --
  and consumer CI reads it under job names like "Alef-generated bindings freshness" as a
  whole-tree freshness gate. It is a far narrower claim: only files carrying an alef marker on
  disk are held to a hash; markerless generated output (`.json`, `.jar`, lockfiles) is checked
  for PATH PRESENCE only, so a present-but-wrong file passes; and anything outside the ownership
  walk's scan set is never opened at all. Each run now prints the managed surface split into
  content-verified / present-but-not-content-verified / absent, the files opened versus never
  examined, unmarked create-once seeds, and marked files the surface does not claim. Follows the
  `alef snippets audit` precedent of naming the check class a run skipped instead of printing a
  bare clean result.

- **A stamped `.clang-format` was written and then never read back by anything.** The ownership
  walk's scan set is documented to be a superset of everything the emit table can stamp, and it
  had drifted: `.clang-format` is scaffolded `generated_header: true` for every FFI target and
  is stamped on write (it is YAML, so `#` line comments apply), but a dotfile with a single
  leading dot reports `Path::extension() == None` and the name was never added to
  `VERIFY_SCAN_FILENAMES`. The walk filters on name and extension before reading any content, so
  the file was not merely unverified but unverifiABLE. The scan predicate now asks the emit
  side's own `is_markable_path` directly, so anything alef stamps on write is opened on read and
  the two sides cannot drift apart again by hand. Widening the scan set only causes more files
  to be read; a file is still reported only when it carries a marker, so this adds no false
  positives.

- **e2e: classify `is_array` by the path the accessor actually addresses.** `FieldResolver::is_array` was a bare `fields_array` set lookup against the raw fixture spelling, while `accessor` — and `result_relative_path`, the answer the zig, brew and C generators share — strip a virtual namespace label first. A field spelled `interaction.action_results` therefore rendered as the slice `result.ActionResults` and classified as not-an-array, so Go's `contains`/`contains_all`/`not_contains`/`contains_any` renderers emitted `string(result.ActionResults)` instead of `jsonString(...)`; converting a `[]T` to `string` is not legal Go, so the generated package failed to build. `is_array` now routes its fallback through `result_relative_path` (which also applies alias resolution) rather than growing a second hand-rolled namespace strip beside `is_optional`'s, keeping one definition of where a fixture field's value lives.

- Added `[crates.e2e.snippets].sample_base_url`: the public base URL generated
  documentation snippets bind for a fixture's `mock_url` / `mock_url_list` arguments. It is
  documentation-only — the executable e2e suite keeps binding the per-fixture mock server —
  so a project can publish snippets a reader can actually run without changing what its
  tests talk to. Relative fixture paths (`"/pdf/report.pdf"`) resolve against the mock
  server for tests and against the configured host for docs, from the same fixture, with no
  per-fixture edit. An explicit `$mock_url` placeholder resolves against it too.

- `src/bin_cli/tree_state.rs`, the classifier, compiled by `build.rs` via `#[path]` and by the crate normally, so `cargo test --lib` exercises the shipped code instead of a second copy of it. Its tests assert both directions — a checkout dirtied only by untracked files reports clean, and a tracked modification, deletion, or staged addition still reports dirty.

### Removed

- Java: 31 unrendered templates. Twenty-eight were registered in `TEMPLATES` but named at no
  render call site; three more had no `include_str!` of their own and passed the
  every-template-is-registered check only because a sibling file has byte-identical content.

## [0.67.3] - 2026-08-24

### Fixed

- **e2e/swift**: a getter's bridged shape is now read from the binding backend instead of
  re-derived. `build_swift_first_class_map` tracked `Vec<Vec<_>>`/`Map<_>` plus two hand-enumerated
  `Option<Vec<Named(..)>>` cases, so every other optional `Vec` was called countable —
  `Option<Vec<String>>` among them, which really emits `fn og_locale_alternates(&self) -> String`,
  making the generator emit `?.count` against a `RustString`. It now calls
  `field_needs_json_bridge`, the same predicate `wrappers::getters::emit_getters` uses to pick a
  getter's return type, so the two generators can no longer disagree about one field.
- **e2e/swift**: two assertion bugs with one cause — the renderer was asked to describe a leaf it
  does not model. The JSON-bridge guard was keyed on the trailing accessor's spelling (a
  `length`/`count`/`size` suffix), so it refused a count on a bridged leaf while emitting an
  indexed accessor against that same leaf — the generator wrote the correct "JSON-bridges it to
  RustString" skip and a broken assertion on adjacent lines. Keying on whether the path steps past
  a bridged leaf at all collapses the suffix, index and wildcard cases into one. Separately,
  `field_expr.contains("?.")` proves an ANCESTOR was optional and never the leaf, yet took
  precedence over the leaf's own optionality, emitting `article()?.publishedTime().toString()`
  where `publishedTime()` returns `Optional<RustString>`. The leaf's optionality now comes from the
  type cursor.
- **e2e**: `namespace_stripped_path` no longer drops a real struct segment the `result_fields`
  config omits. Any leading segment absent from that hand-maintained list was treated as a virtual
  namespace prefix and removed, so a consumer who listed a nested leaf without also listing its
  parent had the parent silently stripped and the accessor built on the wrong receiver
  (`result.favicons()` against a result type with no such field). The IR is now asked instead: the
  enum and collection maps already anchor the call's declared result type, so a first segment that
  type declares as a struct field is a real nested step whatever the config omits. Absent IR still
  answers `false`, leaving the config-only behaviour intact.

- **e2e/zig**: JSON-mode assertions no longer navigate a virtual namespace prefix as a real JSON
  key. A fixture field like `batch.completed_count` emitted
  `result.object.get("batch").?.object.get("completed_count").?`, force-unwrapping a key absent
  from every real payload and aborting the generated zig test. The conditional namespace
  stripping — previously duplicated in the brew and C e2e generators — is now
  `FieldResolver::result_relative_path`, shared by all three. A genuinely nested path
  (`metrics.total_lines`) still keeps its full chain.
- **docs**: rustdoc fence attributes are no longer copied verbatim into generated markdown. A doc
  comment fence of ` ```rust,no_run ` produced a page whose fence language was the literal
  `rust,no_run` — a markdown info string's language is its first whitespace-delimited token —
  which `alef snippets audit --docs` correctly rejected as an unknown fence language. Recognised
  rustdoc attributes (`no_run`, `ignore`, `should_panic`, `compile_fail`, `test_harness`,
  `standalone_crate`, `edition####`, `ignore-<target>`, `E####`) are dropped; unrecognised comma
  tokens move into the fence's meta slot so the language token stays intact. Consumers could not
  fix this at the source: dropping `no_run` makes the doctest actually execute.
- **cli**: `alef snippets audit` now names its coverage when no `--docs` root is given. A
  snippets-only invocation printed a bare `Audit clean: no issues found.` while the
  documentation-page checks (fence languages, include targets) never ran, so a CI job that
  omitted `--docs` read green for a check class it had skipped.

- Wire `src/codegen/config_gen/tests/generators.rs` into the module tree
  (`src/codegen/config_gen/tests.rs` was missing `mod generators;`), so its 18 config-generator
  unit tests actually compile and run. Fixed 14 stale `FieldDef`/`TypeDef` struct literals
  predating the `version` and `has_private_fields` IR fields, and one test function missing its
  own `#[test]` attribute -- all silently dead until now (#211).
- Fix a stale assertion in the Rustler kwargs-constructor test, which asserted the *pre-fix*
  buggy output (`unwrap_or_default()`, silently producing `""` for a `String` field with a real
  default) rather than the already-correct `unwrap_or("default".to_string())`. The generator was
  right; only the expectation was wrong.
- Remove three dead, never-compiled test files under `src/codegen/generators/trait_bridge/tests/`
  (`spec.rs`, `type_formatting.rs`, `helpers.rs`). All 41 of their `#[test]` bodies are byte-identical
  to ones in the wired `spec_and_formatting.rs`; `helpers.rs` carried no tests at all.
- `alef generate`/`alef build` now fail loudly, before invoking `flutter_rust_bridge_codegen`,
  when the `flutter_rust_bridge_codegen` binary on `PATH` reports a version that disagrees with
  the project's declared `[crates.dart] frb_version` pin. Previously the locally installed
  codegen binary's version was baked into generated Dart/Rust bridge output with no check at
  all, so two developers (or a developer and CI) with different `flutter_rust_bridge_codegen`
  installs produced different committed bytes from identical input (#204).

- **snippets:** a snippet that does not compile is no longer reported as `unavailable`. Every
  `is_dependency_error` implementation that could not distinguish "the binding package was never
  built" from "the generated code is wrong" now accepts only diagnostics that can mean nothing
  else: Rust `E0432`/`E0433`/`E0463`/`E0583` (no longer `E0425`, `E0308`, `E0599`, `E0609`,
  `E0061`, or the `could not compile` summary rustc prints on every failed build), Java `package
  ... does not exist` (no longer bare `cannot find symbol`), C# `CS0246`/`CS0234` (no longer
  `CS0103`/`CS5001`), Go `cannot find package`/`no required module` (no longer bare `undefined:`),
  Swift `no such module` (no longer `cannot find ... in scope`). Rust, Java and C# additionally
  require every diagnostic in the output to be a dependency diagnostic, matching the TypeScript
  validator. Reclassification took real failures out of the failure tally entirely — 283 Rust and
  51 Java snippets in two consumer repos were counted `unavailable`, so nothing went red.
- **e2e:** a docs snippet no longer emits an accessor for an assertion field that is not a member
  of the call's result. The operations derived from a fixture's own assertions (added in 0.66.x)
  are now filtered through the oracles the assertion renderers already consult, so an error-path
  fixture, a `result_is_simple`/`result_is_bytes` call, a streaming pseudo-field
  (`stream.has_page_event`), an assertion grouping prefix (`rate_limit.`) and a field the
  availability oracle rejects all fall back to showing the whole result instead of emitting
  `result.error()`, `result.Audio`, `result.CostTracked` or `result.stream.hasPageEvent`.
- **e2e/rust:** a snippet presenting derived fields now binds the result it references and
  unwraps a `Result`-returning call first. `Fixture::has_docs_presentation` — the one predicate
  the call emitter consults — could not see assertion-derived operations, so the emitter wrote
  `let _ = convert(...)` while the snippet printed `result.content` (`E0425`).
- **e2e/csharp:** indexing an optional collection now emits the same null-forgiving operator as
  reading its `.Count`, so a single snippet no longer contains both
  `result.Metadata.Headings!.Count` and `result.Metadata.Headings[0].Level` (`CS8602`).

- **e2e/brew:** the generated `run_tests.sh` harness reported `PASS` when any assertion but the
  last one failed. `run_test` invoked each test function as the condition of an `if`, which
  disables `errexit` for the entire call, so a failing assertion's `return 1` no longer aborted
  the function and the function's exit status was just its last command's. Assertion helpers now
  record failures in a per-test counter that `run_test` consults alongside the exit status, and
  the harness core is emitted from a Minijinja template. Treat every historical brew pass as
  unverified. (#227)
- **e2e/brew:** namespace-prefixed fixture fields produced jq paths that never matched the CLI
  payload. Brew built its path from `FieldResolver::resolve`, which only applies aliases, so a
  field like `batch.completed_count` — where `batch` is a virtual grouping label rather than a
  JSON object — became `.batch.completed_count`, `null` against every real payload. Brew now
  applies the same namespace stripping the C backend uses; genuinely nested paths whose first
  segment is a declared result field are unchanged. (#228)
- **Vendoring no longer strips a crate's inherited lint configuration.** `alef publish prepare`
  (both `VendorMode::CoreOnly` and `VendorMode::Full`, the latter being R/CRAN's default) copied
  the core crate out of its workspace and deleted its `[lints]\nworkspace = true` without
  inlining anything, so the vendored crate compiled under a *different* lint configuration than
  the sources it was copied from. The `[workspace.lints.rust]` `unexpected_cfgs` check-cfg
  allowlist went with it, which is what declares the crate's own `#[cfg(...)]` gates as expected
  cfg names — every gate in the vendored copy then became an `unexpected_cfgs` diagnostic. That
  is silent in a default build and a hard error under the `RUSTFLAGS="-D warnings"` CI sets, so
  the breakage was invisible to every local run and only ever surfaced in CI. Vendoring now
  materializes the whole `[workspace.lints]` sub-tree into the vendored manifest verbatim; a
  crate that spells out its own `[lints]` instead of inheriting is left untouched, and a
  workspace that declares no lints still just has the inheritance marker removed.

- **e2e/zig**: `equals` string assertions no longer wrap `std.mem.trim` around the actual value while emitting the fixture's expected literal verbatim. Any expectation ending in a newline was unsatisfiable by construction. Both sides are now compared exactly, matching every other e2e backend. Applies to the JSON-struct assertion template and to the `metadata.format` discriminated-union path, which carried its own copy of the trim.
- **e2e/kotlin**: a fully-qualified `[e2e.call.overrides.<lang>] class` is no longer double-qualified. The binding-class import split the name into an import path while the trait-bridge import prefixed the binding package onto it unconditionally, so generated test files carried both the correct import and an unresolvable `<pkg>.<pkg>.<Class>`, failing the Kotlin compile with `Unresolved reference`. Package qualification is now centralized in `naming::qualified_type_path`, and the import block is collected in one de-duplicating `ImportBlock` rendered from a template.
- `alef verify` no longer reports create-once scaffold seeds stale forever. The stamping pass
  decided which files to re-stamp from the in-memory `GeneratedFile`, while `alef verify` decides
  from the marker on disk. A seed an earlier alef wrote with a header (`packages/go/go.mod`,
  `packages/zig/build.zig.zon`, the Swift `RustBridge` placeholder) failed the in-memory predicate
  and was never re-stamped, so the first input change after that pinned it stale permanently:
  regenerating wrote nothing, `alef adopt` refused it as already alef-owned, and the file was
  content-correct throughout. The stamp scope is now computed by `stampable_output_paths`, which
  asks the same question verify asks — does the file on disk carry an alef marker — so the two
  sides can no longer disagree. Only the hash line is rewritten; the seed's body is untouched.

- PHP e2e fixtures no longer drop a deliberate empty string on a `String`/`Option<String>` config
  field. The handle-arg and `options_via = "json"` call sites ran fixture input through a
  type-blind filter that removed every `""`, making PHP the only one of 21 backends that never
  forwarded the value the fixture was testing (a fixture writing `bm25_query = ""` exercised the
  default-config path instead). Both sites now use the type-aware filter, which drops `""` only on
  an enum-typed field where it names no variant.
- Swift documentation snippets cast an optional expression to `Any` before printing it.
  `print`/`debugPrint` take `Any`, and Swift raises `expression implicitly coerced from 'T?' to
  'Any'` — an error under the `-warnings-as-errors` the snippet validator compiles with. Every
  prefix of the shown path is consulted, so an optional link (`result.markdown()?.content()`) is
  covered alongside an optional leaf (`result.finalUrl()`); total expressions stay uncast.
- Swift bindings now emit and parse serde's adjacent wire form for enums declared
  `#[serde(tag = "...", content = "...")]`. The trait-bridge result encoder hardcoded serde's
  external default, sending the bare string `"Variant"` where Rust expected
  `{"tag":"variant","content":payload}` and rejecting every callback with
  `invalid type: string "...", expected adjacently tagged enum ...`; the generated `Codable`
  conformance had the same gap in the decoding direction, and trait-bridge result enums got no
  custom `Codable` at all.

- **e2e/snippets**: docs-snippet field facts are now resolved against the call's own declared
  result type instead of by bare field name across the whole crate IR. `presentation::resolve`
  and `apply_derived_shows` take the `functions` registry, resolve the call's return type via
  `resolve_declared_result_type`, and anchor a new `IrResultFieldMap` at it — the same shape
  `IrEnumMap`/`IrCollectionMap` already use. A call whose return type does not resolve keeps
  every previous answer unchanged.

- **e2e/node**: a snippet reaching through a field the Node binding declares optional now emits
  `?.`. Optionality was decided by a unanimity vote over every declaration of the name in the
  crate, and it never saw the NAPI backend's own widening — a type implementing `Default` has
  every field emitted as `Option<T>`, so `metadata: PageMetadata` reaches TypeScript as
  `readonly metadata?: PageMetadata`. Generated snippets emitted `result.metadata.title` and
  failed `tsc` with `TS18048`. The e2e resolver now asks the binding backend's own
  `napi_field_is_optional` predicate, so the two cannot drift.

- **e2e/snippets**: an inferred accessor is no longer derived for a field the call's result type
  does not declare. `FieldResolver::result_field_oracle_knows` accepted any name declared on any
  IR type, so a non-error fixture asserting on a field that exists on an unrelated struct emitted
  a non-compiling member access. `is_valid_for_result` deliberately still default-allows an
  unrecognised name — a hand-authored assertion knows the type the oracle may not — and both
  directions of that asymmetry are covered by tests.

- **java**: An adjacently tagged enum (`#[serde(tag, content)]`) now puts its payload under serde's
  content key. The Jackson serializer flattened the payload beside the tag — serde's *internal*
  shape, which Rust rejects — and only did so when the payload happened to be a JSON object, so a
  newtype variant's scalar payload was dropped outright and crossed the FFI boundary as
  `{"tag":"variant"}` with the data gone. Both codecs now classify through
  `codegen::serde_enum_repr`, and an adjacently tagged enum always uses the hand-written codecs
  because `@JsonTypeInfo` can only express the internal shape. The deserializer moves from raw
  `push_str` to `sealed_union_deserializer.jinja`.
- **csharp**: Same fix for the sealed-union `JsonConverter`, which was parameterised by the tag
  alone and gated its payload write on `ValueKind == Object`, dropping a scalar payload. It now
  takes a `SerdeEnumRepr` and serialises the payload whole under the content key, reading it back
  from there.
- **kotlin / kotlin-android**: Same fix for the shared Jackson codecs. The serializer additionally
  cast the payload tree to `ObjectNode`, so a newtype variant with a `String` payload threw
  `ClassCastException` at runtime rather than reaching the wire. Emission moves from raw `push_str`
  to `tagged_serializer.jinja` / `tagged_deserializer.jinja`; internally tagged output is
  byte-identical.
- **pyo3**: The generated per-variant getter returned the whole serialized document, which is the
  tag envelope rather than the payload under adjacent tagging, and the `.pyi` TypedDicts declared
  the payload's fields flat beside the tag. Both now read serde's content key; an adjacent struct
  variant's payload gets its own TypedDict.

Note: `src/codegen/serde_enum_wire_cross_backend_tests.rs` records all five hand-writing backends
as `AdjacentSupport::Correct`; only Swift and Go were correct before.

### Changed

- **e2e**: `render_snippet_body_with_functions` is now implemented by the `r`, `zig`, `node`,
  `kotlin`, `php`, `ruby`, `elixir`, `python` and `rust` e2e generators, which previously had no
  access to the free-function registry when rendering a docs snippet.


### Added

- Golden vectors pinning the `alef:hash:` recipe (`compute_inputs_hash` / `compute_file_hash`) to
  `CODEGEN_FORMAT_VERSION`, the recorded revision of that recipe. Changing the framing now fails a
  test instead of silently invalidating every stamp in every consumer repo.
- `codegen::serde_enum_repr` — the single classifier for serde's four enum representations
  (external, internal, adjacent, untagged), derived from `serde_tag` / `serde_content` /
  `serde_untagged` in the IR. Backends must classify through it instead of re-deriving the wire
  form.
- A cross-backend guard (`codegen::serde_enum_wire_cross_backend_tests`) that measures each
  backend's generated JSON for an adjacently tagged fixture enum against what `serde_json`
  actually writes for an equivalent Rust enum, and records which backends hand-write that JSON so
  a new one cannot drift in unexamined.

- `tests/test_src_module_reachability_gate.rs`: fails if any `.rs` file under `src/` containing a
  `#[test]` function is unreachable from a crate root. It re-derives the real module graph from
  `src/lib.rs`/`src/main.rs`, following `mod name;`, inline `mod name { .. }` bodies, `#[path]`
  redirects and `include!` splicing the way `rustc` resolves them -- the durable guard against a
  test file silently never compiling (#211).

### Removed

- `template_versions::cargo::FLUTTER_RUST_BRIDGE_CODEGEN`: the constant carried a renovate marker
  but was read nowhere in the tree. The `flutter_rust_bridge_codegen` version gate ships through
  `[crates.dart] frb_version` (resolved by `backends::dart::naming::dart_frb_version`, defaulting
  to the sibling `FLUTTER_RUST_BRIDGE`), so the second constant was a renovate-bumpable duplicate
  of the same version with nothing keeping the two in sync (#218).

### Notes

- The Swift generator now fails at generation time, rather than emitting JSON Rust cannot accept,
  for two shapes it does not support: a newtype variant of an internally tagged enum (which serde
  itself cannot serialize) and a multi-field tuple variant of an adjacently tagged enum (whose
  content serde writes as a JSON array).


## [0.67.2] - 2026-08-23

### Fixed

- Java: a non-optional `Vec`/`Map` field carrying `#[serde(default, skip_serializing_if = "...")]`
  no longer emits `@Nullable` on the generated record component. The builder already defaulted such
  fields to `List.of()`/`Map.of()`, but the record component was independently marked `@Nullable`
  because `has_serde_default` alone drove that decision -- so a payload omitting the key (which
  `skip_serializing_if` guarantees for an empty collection) passed `null` into the record's
  canonical constructor, throwing `NullPointerException` on `.isEmpty()` downstream even though the
  underlying Rust `Vec<T>`/`HashMap<K, V>` is never null. The record now emits a compact-constructor
  line normalizing `null` to the same empty-collection literal the builder uses, and both
  generators now read that literal from one shared function
  (`serde_default_collection_literal`) rather than each deriving it. Changes generated Java output
  for any consumer with such a field.

- **Dart generation now uses flutter_rust_bridge 2.13 and bypasses its redundant dependency
  preflight.** Alef emits the bridge dependencies itself, while FRB's check rejected valid Dart
  prereleases such as `freezed 4.0.0-dev.3` before generation could complete.
- **Swift tagged-enum parameters are deserialized before the source call.** A data-carrying enum
  crosses swift-bridge as a JSON string; a referenced parameter was emitted as `&param.0`, treating
  the bridge `String` as an opaque wrapper and failing to compile with E0609.
- **The generated FFI crate builds by manifest path rather than package ID.** `cargo build -p
  <crate>-ffi` assumes the emitted crate is a member of the invoking workspace; a standalone
  generated manifest is not, and cargo rejected the package spec outright.
- **The `alef all` format gate and the publish-asset guard are hermetic across platforms.** The
  format gate installs its own stub formatter on `PATH` instead of depending on `poly` being
  present, and the publish-asset guard's Unix-only shell helpers are `cfg`-gated so the suite
  compiles on Windows.
- Dart FRB: `frb_generated.rs` no longer diverges between `alef build` and `alef generate` on
  identical input. `alef build`'s `CarryFrbCfgGates` post-build step wrote
  `flutter_rust_bridge_codegen`'s raw, unformatted output straight to disk, while `alef generate`
  additionally ran a separate `poly fmt` pass over the same file afterward -- two alef commands
  regenerating unchanged input then disagreed on the committed bytes (e.g. `use` import grouping
  order), producing spurious diffs on every regeneration. `CarryFrbCfgGates` now normalizes the
  file through the same `normalize_content` pass the guarded generator path hashes against, so
  both commands converge on one canonical form. (#179)
- `alef verify` now detects Dart FRB `frb_generated.rs` drift. The file is written by an external
  tool and rewritten in place by `CarryFrbCfgGates`, so it never carries alef's own embedded hash
  marker and was structurally invisible to `alef verify`'s per-file staleness check -- it could
  silently fall behind (stale `#[cfg(...)]` gates, or non-canonical formatting) with zero signal.
  `alef verify` now recomputes the same canonical form `CarryFrbCfgGates` would write and reports
  a difference as drift. (#179)
- **e2e/java**: stop inlining large fixture values as a single Java string literal. The JVM caps
  a `CONSTANT_Utf8` constant-pool entry (and `javac` a string literal) at 65535 bytes, a limit no
  amount of escaping can raise; a fixture body long enough to threaten it made the generated Java
  doc snippet, e2e test method, or HTTP mock body fail to compile. `java_string_literal` (new,
  `src/e2e/codegen/java/values.rs`) renders short values exactly as before and splits longer ones
  into `+`-concatenated literal chunks, each safely under the cap. Wired through
  `json_to_java_typed`, `emit_java_object_array`, `java_builder_expression`, the doc-snippet
  `json_object` setup (`snippet.rs` + `snippet_json_object_setup.jinja`), the e2e test method's
  `from_json` builder path (`test_method.rs`), the HTTP mock request body (`http.rs`), the
  `equals` assertion literal (`assertions.rs`), and the `handle`/IR-typed-struct JSON embeds in
  `args.rs`. Task #180.
- **e2e/kotlin**: apply the identical fix to the Kotlin backend. Kotlin compiles to the same JVM
  bytecode as Java and shares the exact 65535-byte `CONSTANT_Utf8` cap, so it had the same live
  defect. `kotlin_string_literal` (new, `src/e2e/codegen/kotlin/values.rs`) mirrors
  `java_string_literal`. Wired through `json_to_kotlin`, both `snippet_json_object_setup.jinja`
  call sites (the `handle`-config and `json_object` paths in `args.rs`), the streaming-request
  `from_json` builder path shared by `snippet.rs` and `test_method.rs`, the HTTP mock request body
  (`http.rs`), the `equals` assertion literal, and the array-element `json_object` embed.
- `alef build` no longer silently discards `PostBuildOutcome::skipped_missing_tools`: both
  post-build call sites in `build_with_environment` now route through
  `record_post_build_outcome`, which warns per language and adds a "post-build tool(s) skipped
  (not on PATH)" count to the backend build summary, matching the signal `alef generate`/`alef
  all` already gave via `run_resolved_post_builds`. A missing post-build tool remains non-fatal
  (falling back to committed generated output is intentional), but is no longer indistinguishable
  from a clean run.
- `alef test-apps run --lang <target>` now fails with a clear error when the requested target(s)
  matched no crate's configured `[e2e].languages`, instead of silently exiting 0 with no test
  apps run. Mirrors `ensure_requested_suites_will_run`'s semantics for `alef test`. A run with no
  `--lang` filter and no `[e2e].languages` configured anywhere is unaffected (still a legitimate
  non-fatal no-op).

- **e2e/java**: an `equals` assertion carrying a literal `null` against a non-optional collection
  field no longer renders `assertEquals(null, result.field())` -- a comparison the generated
  binding can never satisfy, because its Jackson builder defaults an absent, serde-defaulted
  collection to `List.of()`. `with_ir_collection_map` was wired into the csharp, kotlin, swift and
  rust e2e generators but never java, so java's assertion side had no IR-backed view of which
  result fields are collections. Task #200.
- **e2e**: a docs-tagged fixture with neither `docs.shows` nor `docs.presentation` no longer emits
  a snippet that bottoms out at a bare `print(result)`. Field access is derived from the fixture's
  own assertions, which already anchor on the same field paths the assertion resolver renders
  against. Python and Rust additionally resolved presentation after clearing assertions; both are
  hoisted above the clear. Task #199.

- **`alef scaffold` now allowlists bare `cfg(alef)` in `[workspace.lints.rust]`, not just
  `feature = "alef-meta"`.** `#[cfg_attr(alef, alef(skip))]` is alef's documented and far more
  common exclusion marker, but `cfg(alef)` is never a real declared cfg, so rustc's
  `unexpected_cfgs` fired on every use and any lane compiling with `-D warnings` denied it.

- A user `[e2e.format]` override's `{dir}` placeholder now expands to a path a POSIX shell can
  `cd` into on Windows. `canonicalize` returns the extended-length form `\\?\C:\...`, and `sh`
  reads every `\` as an escape, so the `cd` in the conventional `(cd {dir} && ...)` override
  failed before the formatter ever ran. The shell then exited 1, which is not the
  command-not-found status 127, so an absent formatter was misclassified as "the formatter ran
  and rejected the code" and killed the run instead of being recorded as a deferred
  environment gap. `run_in_dir`'s built-in residual steps already avoided this by never going
  through a shell; the override path, which must go through one, now normalises the path.
- The generated FFI crate's `build.rs` nested its stale-backup cleanup inside `if
  had_destination`, which clippy rejects as `collapsible_if` under `-D warnings`. Because the
  file carries `generated_header: true`, no consumer edit survived regeneration, so a consumer
  had to suppress the lint in its own CI — and any lint pass alef did not know about (poly runs
  its own whole-project clippy) hit it anyway. Flattened with an early return rather than a
  let-chain, so the emitted crate's edition does not matter.
- Emit `checksum-Elixir.*.exs` in `mix format`'s canonical wrapped form so regeneration no longer
  produces pure-reformat diffs. Each map entry was written on a single line; `mix format` (the sole
  formatter for generated `.ex`/`.exs`) then moved every over-width digest onto its own continuation
  line and dropped the trailing comma, so the file was dirty after every `alef build`. The emission
  now wraps per entry exactly where the formatter would — honouring `line_length` from the package's
  `.formatter.exs`, falling back to Elixir's default of 98 — and renders through a Minijinja template
  instead of `push_str(&format!(...))`.
- Tests that shell out to `git` no longer inherit the ambient global git configuration. Fixture
  repositories are now built through a single hermetic `test_support::git_command` helper that
  neutralizes `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` and pins identity, signing, excludes and the
  default branch name. Previously a developer with `commit.gpgsign = true` set globally signed
  every fixture commit, making the suite depend on a working gpg-agent, and the same tests would
  behave differently on CI, which has no signing key.
- `go_tag`'s fixture builder asserted only that `git` could be spawned, not that it succeeded, so a
  failed commit or annotated tag left the tests asserting against an empty repository instead of
  the fixture they name. Each step now checks the child's exit status.
- Four `#[test]` functions had empty bodies and had passed unconditionally since they were written;
  they now assert the PHP namespace-qualification, PHP streaming-field disambiguation and
  kotlin_android `file_path` behaviours against real codegen output. Fifteen further integration
  assertions could not fail — ten had a dead disjunction arm, five matched needles too weak to
  detect a regression. Tightening the PHP `options_type` import assertion exposed a wrong
  expectation the dead arm had masked (`use SampleCrate\…` where codegen emits `use Mylib\…`).
- Refreshed Dart snapshots left stale by the flutter_rust_bridge 2.13 bump and the relocation of
  `carry_frb_cfg_gates()` into the successful-regeneration arm. Both changes are deliberate and
  separately tested; only the committed snapshots lagged, and `cargo test --lib` does not run them.

### Changed

- Split four over-cap source files at their concept boundaries so the file-size ratchet passes:
  dart `build.rs`/`flutter_rust_bridge.yaml` emission out of `gen_rust_crate/cargo.rs`, java
  bracket-wildcard assertion rendering out of `e2e/codegen/java/assertions.rs`, crate-attribute
  formatting out of `codegen/shared.rs`, and fixture field resolution out of `e2e/codegen/mod.rs`.
  Pure restructuring; existing public paths are preserved by re-export.

### Added

- `tests/test_vacuity_gate.rs`: a mechanical guard over `tests/**/*.rs` rejecting three shapes of
  check that cannot fail — a `#[test]` with an empty body, a `x.contains(a) || x.contains(b)`
  disjunction whose needles subsume each other, and a narrowed `cargo test` invocation in
  `ci.yml`'s `test` job (which would retire all ~253 integration binaries while every signal
  stayed green).

## [0.67.1] - 2026-08-23

### Fixed

- **Generated FFI build scripts no longer rewrite tracked headers during ordinary Cargo builds.**
  Header export is now explicit via `ALEF_EXPORT_GENERATED_HEADERS=1`; cbindgen output is buffered,
  validated as UTF-8, and published atomically to the canonical and Go destinations with rollback
  on failure. This prevents failed workspace lint or build commands from truncating generated files.

## [0.67.0] - 2026-08-23

### Fixed

- **Dart bridge-coverage no longer reports a present function as missing when `dartfmt` wraps its
  return type.** `missing_bridge_functions` looked for a facade function's camelCase name in the
  generated bridge with a literal `" name("` substring check, which only holds while the return
  type and the name share a line. A long return type wrapped onto its own line put a newline
  before the name, the match failed, and the post-build check aborted the whole run — taking the
  stubs, public-API, e2e, test-apps, README, and docs stages with it. The sibling
  `filter_excluded_functions` used the same idiom and had the same latent bug (a line-wrapped
  excluded function was silently never stripped). Both now share one token-boundary matcher that
  is insensitive to whatever whitespace `dartfmt` emits.
- **C e2e renders a deliberately empty `mock_url_list` literally instead of falling back to mock
  scaffolding.** The engine-factory pattern collapsed the preserved URL list to its first element,
  which made "declared empty on purpose" indistinguishable from "unset" and emitted
  `getenv("MOCK_SERVER_URL")` scaffolding for a fixture that had explicitly asked for no URLs.
  C's engine-factory pattern was the only site affected — every other backend passes the whole
  list through to an empty collection literal.
- **A `# Note` section is no longer emitted twice in generated docs.** Two parsers read the same
  rustdoc and disagreed about who owned the section: `convert_doc_headings_to_bold` turned it into
  a `**Note:**` label, while `parse_rustdoc_sections` — which does not recognise `note` — folded
  the raw heading into whichever section preceded it, usually `# Example`. Both paths now consult
  one shared list of bold-labelled section names, so they cannot drift apart again.
- **`alef check-registry --registry github-release` authenticates.** It sent no `Authorization`
  header, so it used the 60 req/hour anonymous limit shared across the runner IP pool and got
  HTTP 403s. It now sends `GITHUB_TOKEN` (falling back to `GH_TOKEN`); an empty value falls
  through rather than forcing an anonymous request.
- **`alef all` reports every stage failure instead of aborting on the first.** A pre-flight
  snippet-coverage failure aborted the run before the per-crate loop started (nothing was written
  at all), and one crate's post-build failure returned immediately — so stubs, public-API, e2e,
  test-apps, README, and docs never ran for that crate or any later one. Failures are now
  accumulated: the run still fails, but it names every failure rather than the first. A single
  failure still returns its original error unchanged, so existing `.context()` chains are intact.
- **Strict snippet validation no longer fails on a missing build artifact.** `unresolved_dependency`
  is set only when a real toolchain ran and reported a missing import or link target — and neither
  `alef all` nor `alef docs` builds per-language artifacts in the same invocation, so it was
  failing on a precondition it structurally cannot satisfy. A toolchain genuinely absent from
  `PATH` still fails strict mode, and every unavailable result is still reported via `tracing::warn!`
  with its per-snippet attribution.
- **kotlin_android/JNI: a function excluded only from the JNI shim no longer drops the destructor
  for the type it returns.** `[crates.jni].exclude_functions` tells the JNI backend to skip
  generating one function's own native shim; it does not tell Kotlin to stop calling that function.
  The destructor emitter was narrowing reachability by that JNI-only list, so Kotlin declared and
  called a `nativeFree<Type>` the JNI crate never implemented — an unresolved symbol that stopped
  the AAR building. The JNI shim generator now computes reachability with the same
  `kotlin_visible_functions`/`handle_only_type_names` predicate the Kotlin emitters use, so the
  declaration, its call site, and its native implementation cannot name different sets.
- **A failed registry pre-check no longer turns a successful publish run red.** The three
  `check-*` steps in `publish.yaml` are advisory — every downstream job already runs under
  `always()` and treats a missing output as "not published yet" — so they now carry
  `continue-on-error: true`. alef v0.66.0 published correctly to crates.io while its run reported
  failure for exactly this reason.

### Added

- Table-order coverage for the Dart and Swift bridge-crate `Cargo.toml` emitters, which were the
  only two of eleven emission sites with no test driving their real output through the
  `cargo_sort_order` checker. Both were already canonical; the tests keep them that way.

## [0.66.0] - 2026-08-23

### Added

- **`alef generate --strict`**, meaning exactly what `alef all --strict` already documented: a
  configured formatter whose executable is not installed fails the run. A missing formatter stays
  non-fatal by default — `poly`, `rustfmt`, `cargo-sort` and `mix` are host toolchains a fresh
  clone may legitimately lack.
- **e2e fixture validation for discarded URL literals.** A fixture that declares a
  scheme-carrying URL (`https://…`, `gopher://…`) for a `mock_url` / `mock_url_list` argument
  without setting `preserve_input_urls` is now a validation **error**. That literal was silently
  discarded at codegen time — every backend fell through to the mock server address — so the
  fixture proved something other than what its author wrote. `alef e2e validate` exits non-zero
  on it. A mock-server-relative path (`/seed1`) and the `$mock_url` placeholder are unaffected.
- **File-size ratchet enforcing the 1,000-line `file-modularization` cap.** The rule was
  aspirational — nothing measured it, and 124 files under `src/` and `tests/` had drifted past
  the cap. `tests/file_size_ratchet.rs` freezes today's sizes against the committed ceilings in
  `tests/file_size_baseline.txt`: an over-cap file may shrink freely but fails the build the
  moment it exceeds its recorded ceiling, a file not in the baseline may never cross 1,000 lines,
  and a baseline entry whose file has dropped under the cap must be deleted so the ratchet
  tightens instead of licensing a regrowth. `task lint:file-size` runs it alone;
  `task lint:file-size:tighten` rewrites the baseline after a split.

### Changed (BREAKING)

These change the code alef GENERATES. Regenerate with `alef all` and review the diff before
releasing a consumer package.

- **Generated doc snippets omit the `level:` front-matter key instead of rendering `level: null`.**
  These files are Astro content entries, and Astro's collection schema types `level` as an
  optional STRING: zod distinguishes "absent" from "present and null", so an absent key validates
  while an explicit YAML null does not (`level: Expected type "string", received "object"`). A
  single bad entry aborts the whole `astro build` — 810 generated snippets in one consumer all
  carried `level: null` and the docs build died on the first one alphabetically. Alef's own parser
  deserialises both spellings to `SnippetMetadata::level == None`, so the validation contract is
  unchanged. Regenerate docs snippets.
- **Generated bindings no longer depend on IR arrival order.** Every backend concatenated
  `ApiSurface`'s `types`/`enums`/`functions`/`errors` into a single generated file in raw `Vec`
  order, so a change in extraction or pipeline-orchestration order could make two `alef` runs over
  an unchanged tree emit the same blocks in a different order. All 17 backends now order the
  surface once at their `Backend` entry points, before any emission or function deduplication —
  including before `with_deduped_functions`, whose `any(...)` cfg union and canonical-entry pick
  were themselves input-order sensitive. Expect a one-time reordering diff on regeneration.

### Fixed

#### Generated code correctness

- **swift**: the SwiftPM package root (`Sources/RustBridge{,C}` placement) is no longer derived by
  probing the filesystem for an existing `Sources/` directory. The output prefix is now a pure
  function of the resolved output dir and whether `[crates.output] swift` is configured, so
  generated paths no longer depend on ambient directory state. The empty-path fallback was
  silently reachable any time no ancestor happened to have an on-disk `Sources/` directory yet —
  e.g. a project's very first `alef build`.
- **swift**: `--manifest-path packages/swift/rust/Cargo.toml` was hardcoded in
  `build_config()`, and `gen_rust_crate::emit` independently hardcoded the same literal as the
  crate's actual write location. An explicit `[crates.output] swift` override moved every other
  swift artifact but left both of these behind, producing
  `error: manifest path packages/swift/rust/Cargo.toml does not exist`. Both now route through the
  same `swift_package_root` helper, so where the crate is written and where `cargo build` looks
  cannot disagree.
- **php**: `#[serde(default = "crate::serde_defaults::…")]` no longer references a function the
  generated crate never defines. The attribute emitter and the `serde_defaults` module emitter
  decided independently, from two hand-mirrored type tables; a non-optional scalar field whose
  `#[serde(default = "Type::function")]` sits on a primitive got the attribute but no function,
  and the generated crate failed to compile with `E0425`. Both emitters now go through one
  predicate, and such a field is answered with the fully-qualified core function the extractor
  resolved, cast to the PHP-facing width — so the mirror keeps the core default instead of
  deserializing to `0`.
- **magnus (ruby)**: `default_timeout`'s `#[serde(default = "...")]` reference is now paired with
  its definition. The free-function emitter and the per-field attribute emitter decided
  independently whether a `request_timeout`/`timeout` field needed the fallback — one matched
  `FieldDef::ty` directly and ignored `field.optional`, the other matched the type-mapped Rust
  string against the literal `"u64"`, which an `Option<u64>` field never produces. An
  `Option<u64>` field with no non-optional counterpart generated
  `fn default_timeout() -> u64 { 30000 }` with nothing referencing it. Both sides now go through
  one predicate.
- **java**: the `#[serde(default)]` Vec/Map builder-field eager `List.of()`/`Map.of()` default is
  now scoped to fields that also carry `skip_serializing_if` — the actual signal that makes the
  wire key go missing. The unscoped version silently regressed the nullable-without-eager-default
  contract for every other bare `#[serde(default)]` collection field.
- **dart**: the generated bridge crate's `build.rs` re-applied flutter_rust_bridge's missing
  `#[cfg(...)]` gates only from the success arm of the `flutter_rust_bridge_codegen` spawn, itself
  behind the opt-in `ALEF_FRB_REGENERATE_ON_BUILD` early return. Every build that needs the repair
  — a plain build of the *committed* bridge on a machine without FRB installed, i.e. CI — returned
  before reaching it (`E0425: cannot find function ... in the crate root`, on both Android ABIs).
  The repair now runs unconditionally at the top of `main()`, derives the gates from `lib.rs` on
  every build, needs no external tool, and is idempotent.
- **wasm / pyo3 / extendr**: a struct field whose typed default is a bare zero-argument path call
  was emitted as `unwrap_or_else(|| path::to::fn())`, tripping `clippy::redundant_closure` under
  `-D warnings` and failing the generated crate's own lint gate. The path is now passed directly
  to `unwrap_or_else`. Any other shape — arguments, a trailing `.into()`, or field access — keeps
  its closure, since only a bare call is a valid function-item substitute.
- **rustler (elixir) / extendr (r)**: both backends now sort IR items at every `Backend` entry
  point, so `lib.rs`, `*.ex`, `extendr-wrappers.R` and `NAMESPACE` no longer depend on the arrival
  order of `ApiSurface.functions`/`errors`. Types and enums were already sorted; a determinism
  test that only reversed those never exercised the still-broken axis.
- **e2e/c**: the engine-factory pattern's URL construction only ever consulted a scalar
  `input.url` field, so batch/list fixtures fell through to the raw `MOCK_SERVER_URL` `getenv`
  scaffolding even when `preserve_input_urls` was set — which the mock-harness leak guard rejects
  in a published documentation snippet. Now routed through the same shared
  `resolve_urls_field`/`preserved_url_list` helpers every other backend already uses.
- **php/e2e**: enum-variant assertion availability is decided from the binding's own enum
  lowering rather than a hard-coded consumer field path. The old predicate was false for the shape
  it named, was classified as a fatal `AuthoringGap` that no consumer could close, and hard-coded
  one consumer crate's type path. It is now a non-fatal `LanguageLimitation`, and the new
  `enum_variant_access` walks the same IR type graph the accessor renderer walks.
- **e2e snippets**: a call declaring `skip_languages` is now honoured by the documentation-snippet
  generator, not only by the executable e2e suite. A call declaring `skip_languages = ["c"]` was
  correctly excluded from the executable C suite but still reached the snippet generator's built-in
  C recipe, which rendered mock-harness scaffolding and tripped `reject_mock_harness_scaffolding`.
  Both sides now resolve through one `call_skip_reason` seam rather than deriving the check twice.
  This stays deliberately narrower than a fixture-level `skip` directive, which opts a fixture out
  of the executable harness only and must not suppress a documentation snippet.
- **php/e2e**: property access is no longer emitted for data-enum-typed struct fields. The e2e
  getter map was built from a local copy of the binding's scalar predicate fed with ALL enum
  names, so a tagged or untagged data enum field was rendered as a property when the binding
  exposes it only as a getter. The copy is gone.

#### Generation pipeline

- `alef generate` now runs post-build (Swift's `MaterializeSwiftBridge`, Dart's
  `flutter_rust_bridge_codegen`) **before** its formatting pass, matching `alef all`. Previously a
  post-build step's writes were stamped with `alef:hash:` before ever being run through
  `poly fmt`, so the shipped file was permanently non-canonical — poly's hash-stamped-file skip
  cannot tell "canonical and stamped" apart from "never formatted and stamped" — and a later
  `alef verify` or standalone `poly fmt --fix .` could disagree with it.
- A partial regen now formats every directory it stamps, in two respects that were separately
  broken. `format_generated` could skip a language that had a post-build step but nothing else
  changed this run (`poly_langs` was derived from an intersection with the written-files list, not
  from the caller's `only_languages` set). And for `python`, `ffi` and `php` — every language whose
  default output template is `<crate-root>/src` — `poly_paths` named only the `src` subdirectory,
  one level below the crate root holding the binding crate's own `Cargo.toml`, so that manifest
  shipped non-canonical straight out of generation.
- `--strict` now guards the pass that formats `packages/<lang>` — the shipped bindings — and not
  only the e2e formatter. Missing-tool skips were swallowed in a `warn!` the caller never saw, so
  `alef all --strict`'s own promise held for the e2e tree while the more important surface went
  unguarded.
- The stage and per-language generation caches are invalidated when the alef version changes.
  `compute_stage_hash` and `compute_lang_hash` carried no compile-time alef identity — the only
  input tracking the alef build was a best-effort `current_exe()` mtime+size probe that silently
  degrades to an empty string on failure. Two different alef releases could produce identical
  cache keys, so `alef generate` reported `up to date (skipping)` for every target and kept the
  previous release's generated code until someone passed `--clean`. This does **not** change the
  embedded `alef:hash:` value, which is a separate mechanism.

#### Verification and reporting

- `alef verify` no longer reports permanently-stale generated files after a command that runs a
  whole-tree formatter pass. `alef stubs`, `alef init` and `sync-versions`' regeneration each
  formatted the entire repository and then stamped only the narrow set of paths they had
  generated, so every other alef-marked file the formatter rewrote kept a hash derived from its
  pre-format bytes and was reported stale forever, with no regeneration able to clear it.
- `alef verify -vv` now dumps the managed path set the orphan report is diffed against, so an
  orphan finding can be checked rather than guessed at.

#### Documentation output

- A public function reachable under two `cfg` gates was documented twice, byte-identical, under
  one `### Functions` heading. All 26 backend call sites OR-merge those groups; the docs generator
  did not. It now calls `with_deduped_functions()` alongside its existing cfg filter.
- Rustdoc headings alef does not rewrite to a bold label could escape above the section they were
  spliced under, in the worst case as a bare `#` H1 inside a page rooted at `##` — tripping rumdl
  MD001/MD025 and turning `poly lint` red in consumer repos. Two independent causes: the language
  pages anchored the doc's *first* heading at the target level, and the shared pages shifted by a
  fixed number of levels, which is only correct for a doc opening at `#`. Headings are now
  re-levelled by mapping the doc's distinct levels onto consecutive levels from the target, which
  is order-independent and gap-free. No section name was added to an allowlist.
- `alef sync-versions --help` no longer names `alef generate` as a way to regenerate `test_apps/`.
  `alef generate` calls `sync_versions` with `no_regen = true`, so it never reaches
  `regenerate_test_apps_after_sync`; only `alef all`'s test-apps stage and `alef test-apps
  generate` write that tree. `alef generate --help` states the exclusion, and `alef all --help`
  now names the test-apps stage it had been omitting.

### Changed

- **php**: the `serde_defaults` module is emitted through a Minijinja template instead of being
  assembled with `format!`.
- Consumer project names replaced with neutral fixture names across 27 source files, per
  `project-agnostic-codegen`. The project-mention gate now reports every failing chunk instead of
  returning on the first one.
- Internal module splits to hold files under the 1,000-line cap, with no behaviour change:
  cache-key identity moved to `src/cli/cache_identity.rs`, `preserve_input_urls` validation to
  `src/e2e/validate/url_preservation.rs`, frozen-file logic to `src/bin_cli/helpers/frozen.rs`,
  and trailing test modules out of eight further files into siblings.
- `backends::order_invariance_tests` now holds the suite's shared `CWD_LOCK`. Several backends
  resolve `version_from` (default `Cargo.toml`) with a relative-path read against the process
  working directory, so a sibling test chdir'ing mid-run could make the forward and reversed
  generation calls read two different manifests — a spurious diff with nothing to do with IR
  order. Test-only; no production code changed, and no retry bracket was reintroduced.
- Dependencies: `jsonschema` to v0.50.1, `freezed` to v4.

## [0.65.0] - 2026-08-22

### Changed (BREAKING)

These change the code alef GENERATES. Regenerate with `alef all` (which already covers the
test-apps stage) and review the diff before releasing a consumer package.

- **Swift / Elixir / Dart / Kotlin / Java `is_empty` on a collection.** This defect shape has now
  been fixed in six backends, each of which was wrong differently, and each of which silently
  emitted an assertion that could never pass:
  - **Swift**: `is_empty`/`not_empty` on a non-optional array reached through an optional PARENT
    (`data.children` where `data: Option<Data>` but `children: Vec<T>` is not) emitted a `Bool?`
    into `XCTAssertTrue`, which does not typecheck at all -- so the Swift e2e gate never ran.
    Both arms now coalesce (`?? true` / `== false`) when the accessor is optional.
  - **Elixir**: `is_empty` had NO collection branch, only `nil`/`""`. An empty `Vec<T>` (Elixir
    `[]`) satisfied neither, so the assertion was simply false. It now mirrors `not_empty`'s
    membership form, `in [nil, "", [], %{}]`.
  - **Dart**: collection fields were classified from config alone, degrading to a
    `.toString()`-based check that compares against Dart's `List.toString()` (`'[]'`) and can
    never pass. Classification now comes from the IR. Separately, a freezed union field's
    `toString()` is case-sensitive and never matched the expected variant name; the union branch
    now compares `.runtimeType.toString()`.
  - **Kotlin**: `isEmpty()` was called without a null check on an optional collection.
  - **Java**: a `serde(default)` collection field now defaults to `List.of()`.
- **C e2e / doc snippets**: the "none" sentinel for an omitted optional argument is now selected
  from the core IR's declared parameter type rather than the fixture's `arg_type` label (which
  defaults to `"string"` when never set). A handle-typed optional parameter previously rendered
  `NULL` against an `AlefHandle` (`unsigned long long`) parameter -- an incompatible
  pointer-to-integer conversion -- instead of `0`. The decision now lives in one seam,
  `resolve_optional_sentinel`, consulted by all three C surfaces that previously disagreed. A
  `const char *` optional argument still renders `NULL`; this is type-directed, not a blanket swap.
- **TypeScript / node (napi-rs)**: an internally-tagged data enum's payload is now nested under
  its synthesized per-variant field instead of being flattened alongside the discriminant.
  napi's own `index.d.ts` declares such an enum as a discriminated union (`{ role: 'user'; user:
  UserMessage }`), so the flattened literal type-checked against no member of that union and
  every affected snippet failed `tsc` with TS2353. The array-argument path -- the site of most
  failures, since chat-message lists are array arguments -- previously bypassed the enum-aware
  builder entirely.
- **rustler / extendr**: `alef generate` could emit a different byte sequence for the same
  unchanged IR depending on the incoming order of `ApiSurface.types`/`.enums`/`.functions`/
  `.errors`. Roughly a dozen emission loops in both backends iterated the raw `Vec` fields
  directly; all now sort by name, so output depends only on IR content, never Vec order. Note a
  generate-twice-in-one-process test does NOT catch this -- the guard that does is
  order-invariance (generate twice, once with the collections reversed).
- **Python / pyo3**: generated snippets imported a base exception class name that does not
  exist, and streaming stub methods were emitted as `async def` rather than plain `def`.
- **C# e2e**: a streaming call in a snippet is now iterated with `await foreach` instead of being
  awaited directly.
- **Dart e2e**: error-path snippets now print a bound result, so the binding is actually used.
- **e2e snippets**: a `mock_url`/`mock_url_list` argument whose fixture DECLARED a relative path
  (`"batch_urls": ["/seed1"]`) still leaked `MOCK_SERVER_URL` into its published documentation
  snippet and was rejected by the mock-harness leak guard. Declared bare paths are now rewritten
  under the same illustrative `https://example.com` base as undeclared ones. A declared value
  carrying an explicit scheme (an SSRF fixture's loopback literal) is still left untouched.
  Python's `mock_url_list` handler also emitted its `MOCK_SERVER_URL` setup line before checking
  `preserve_input_urls`, unlike every sibling backend; it now checks first.

### Fixed

- `alef verify`'s frozen-file report and `alef adopt`'s refusal no longer contradict each other.
  Every frozen path used to be reported with one remedy ("run `alef adopt <path>`") regardless of
  whether it was a create-once seed -- which `alef adopt --write` refuses by design. The printed
  remedy therefore failed for exactly the paths it was pointed at: 85 of 85 frozen paths refused
  in one consumer repo, 99 of 99 in another. `FrozenFile` now carries a `create_once` field
  computed by calling `is_create_once_seed` directly rather than re-deriving the classification,
  so report and remedy cannot drift apart again. Create-once seeds print under their own heading
  naming `--clobber-create-once-seeds`.
- `alef verify`'s EXIT CODE is now gated on adoptable frozen files only. A repo whose only frozen
  paths are create-once seeds could not reach exit 0 by any available action.
- `alef adopt` no longer manufactures a phantom "content differs" diff for a self-marking,
  non-Markdown backend (custom Swift/Kotlin/Dart/Gleam/Zig headers) whose body is unchanged.
  Adoption stamped the generic default header instead of the backend's own marker, so `classify`
  compared two header spellings and called the file Drifted though every body line was identical
  -- training an operator to consent to a diff with nothing real to review.
- `alef test-apps run` now warns when a target's generated output is stale, checked against the
  same embedded `alef:hash:` stamp `finalize_hashes` writes (no second staleness signal
  invented). Previously a stale test app failed with no indication the "regression" was actually
  stale generated sources.
- Snippet validation cache entries are now keyed by the alef version that computed them, and
  session claims resolve by language. A cache written by an older alef could otherwise replay a
  stale classification.
- A fixture that declares a scheme-carrying `mock_url` literal without setting
  `preserve_input_urls` now logs a warning naming the fixture and field. That literal was already
  being silently discarded in favour of the mock-server address, with no signal that the author's
  declared value had no effect.

### Changed

- `src/bin_cli/helpers.rs`'s frozen-file logic moved to `src/bin_cli/helpers/frozen.rs` with its
  own test module, splitting the file back under the repository's line-count cap.

## [0.64.0] - 2026-08-22

### Fixed

- Generated Go is gofmt-clean in two more places. The e2e visitor-struct emitter wrote
  `type X struct{` without the space gofmt requires and left no blank line after the closing
  brace (gofmt separates consecutive top-level declarations), and the Go visitor preamble ran the
  `import (...)` block straight into the next declaration with no blank line. Both surfaced as
  `gofmt -l` failures in consumers with no local cause, since the files are generated.

### Changed (BREAKING)

These change the code alef GENERATES. Regenerate (`alef all` plus `alef test-apps generate`) and
review the diff before releasing a consumer package.

- **Ruby / RBS**: a wire name that is not a valid bare Ruby symbol is now emitted quoted
  (`:"fine-tune"`, `:"og:image"`) instead of the syntactically invalid bare form. Identifier-safe
  names stay bare, so most `sig/*.rbs` files are unchanged. The wire value itself is untouched --
  only its literal encoding. Previously-generated RBS containing a hyphen or colon in a variant
  name did not parse at all.
- **C# / Java**: `Duration` fields no longer force the serde-derive `{"secs","nanos"}` object
  shape when `serde_with` declares a scalar, and sealed-union `Display` now emits the real wire
  value rather than a lowercased variant name. A multi-word or explicitly renamed variant that
  previously stringified as `"elementbased"` now correctly yields `"element_based"`.
- **C# e2e**: generated trait-bridge stubs now derive their types and member names from the same
  seam the binding uses, so a stub that previously failed to compile (`ulong??`, `IXmlBackend`,
  `GetUuid`) now matches its interface.
- **Elixir**: rustler and the e2e generator now agree on `rename_all` wire tags via
  `wire_variant_value`; an absent `rename_all` no longer snake_cases the variant.
- **Snippets**: a `docs.snippets.docs_dirs` entry pointing at a path that does not exist now
  fails the docs stage's audit instead of passing silently.
- **Snippets**: a `tsc` type error (TS2322, TS2304, ...) is now reported as a real failure with the
  compiler's own text, instead of being relabelled `Unavailable` with "run `alef build` first".
  Runs that appeared incomplete may now report genuine failures.

### Added

- `alef verify` now reports create-once (`generated_header: false`) scaffold files whose
  on-disk content predates a fix to the template that produced them. These files (a zig
  `build.zig`, a kotlin test seed, a params struct) are written once and are user-owned
  thereafter, so a template fix landing upstream could previously never be surfaced to a
  consumer who had already scaffolded the file -- `alef verify` had no opinion on the
  condition at all. Detection is deliberately conservative: a file is only reported when its
  on-disk content differs from what the current template would produce AND its entire git
  history is exactly one commit (nothing has touched it since whatever commit introduced it),
  so a legitimate hand-edit is never mistaken for template drift except in the narrow case of
  an edit folded into that same single commit (amend/squash) -- see
  `src/cli/pipeline/generate/scaffold_drift.rs`'s module doc for the full false-positive/
  false-negative analysis. Purely informational: it never fails `alef verify`, is never folded
  into `ensure_success`, and alef never rewrites the file -- the remedy is a human reviewing
  the current template and deciding whether to hand-port the fix.
- `alef docs --skip-snippet-validation` and `alef all --skip-snippet-validation` skip the
  compile/type-check/run step of `docs.snippets.validation_level` validation. Snippet
  discovery, the reference-page audit, and gap detection still run; only the step that spawns
  a compiler, interpreter, or type-checker per snippet is skipped. Exposes the existing
  `docs::generate_docs_stage_without_snippet_compile_validation` path (already used
  internally by `alef adopt` and `alef verify`) through the CLI, so regenerating docs is
  cheap again when the toolchains or built artifacts snippet validation depends on are not
  present (e.g. a clean checkout before `alef build`).
- Generated documentation snippets for an error fixture that names the error variant it provokes
  (`{"type": "error", "value": "Authentication"}`) now catch that variant's generated exception
  class before the flat catch-all, instead of emitting a single generic `except Error` that prints
  `type(error).__name__`. The decision lives in one shared seam,
  `e2e::codegen::snippet_error_branch`, which reuses `declared_error_variant`'s existing
  per-backend verdict on whether a binding can tell one error variant from another — so a snippet's
  typed catch and the e2e suite's type assertion can never claim different things about the same
  variant. Backends whose bindings expose only a flat error type keep the generic branch unchanged.
  Python is wired today (pyo3 generates a distinct exception class per variant unconditionally);
  Go, Java, and Zig have their per-variant names registered in the same table and gate on
  `#[alef(error_code = N)]`, awaiting emitter adoption.

### Changed

- The `extension == "jar"` test and its base64 decode, previously inlined in
  `generate::write` and `generate::scaffold` and simply absent from `generate::diff`, are now one
  shared `generate::binary` module (`is_base64_binary_output` / `decode_base64_binary`).
- `alef adopt`'s "NOT ADOPTED -- not text" report now covers only matches alef genuinely cannot
  read: non-UTF-8 bytes on a path alef emits as text, or a binary path whose generated content
  did not decode.
- e2e/kotlin_android: `render_build_gradle_kotlin_android` now takes a `KotlinAndroidBuildGradleInputs` params struct instead of 8 positional arguments, removing the `#[expect(clippy::too_many_arguments)]` added under release pressure.
- A directory that exists but is empty remains a legitimate silent zero everywhere; only a
  directory that is missing outright is reported. The policy now lives in one place,
  `snippets::discovery::missing_configured_directories`, which `discover_snippets`,
  `snippets::audit::audit`, the `snippets audit`/`gaps` commands, and `alef verify` all consult.
- `snippets::audit::AuditIssueKind` gains a `MissingDirectory` variant (additive).
- Behaviour change worth calling out for consumers: a `docs.snippets.docs_dirs` entry pointing at
  a path that does not exist now fails the docs stage's snippet audit, where it previously passed
  silently. This matches how `docs.snippets.dirs` / `inline_dirs` already behaved
  (`docs::build_snippet_context` has always refused a missing root).

### Fixed

- rustler: the Elixir `@type` alias for a flat data enum's variant and the `wire_value/1` map
  clause it generates could name the discriminator key differently -- the `@type` alias
  hard-coded a literal `type:` key while `wire_value/1` used `serde_tag` (or a fallback). Both
  are now derived from one `flat_data_enum_discriminator` helper so they cannot disagree.
- rustler/extendr: replaced the `"format_type"` fallback discriminator name (a domain-specific
  word, not a generic default) with `"type"`, matching the fallback the wasm backend already
  uses for the identical concept and matching serde's own `tag = "type"` convention. All call
  sites that name a flat data enum's discriminator field (the Rust struct field, the `From<core>`
  and `From<binding>` impls, and the Elixir `@type`/`wire_value/1` pair) now read the fallback
  from a single shared function per backend instead of hard-coding it independently.
- rustler: removed an unused, unreachable `elixir_data_enum_type.jinja` template that carried
  the same `type:`-hard-coding bug but was never rendered by any call site.
- `alef sync-versions` no longer stamps `alef:hash:` on the files it rewrites before writing
  the `[crates.e2e.registry.packages.*].version` bump to `alef.toml`. Because that field is a
  real generation input (it feeds registry-mode `test_apps/` content), the previous ordering
  stamped every rewritten file against a pre-bump `inputs_hash`, so those files verified stale
  immediately after the very run that wrote them. `sync_registry_package_versions` now runs
  before the `finalize_hashes` pass over the rewritten file set.
- **e2e**: fixtures declaring an `error` assertion's message/type-name check as a SECOND
  `"error"`-type assertion (a bare `{"type": "error"}` followed by
  `{"type": "error", "value": "..."}`) silently lost the message check across every backend —
  `declared_error_value()` selected only the fixture's positionally-first `"error"` assertion,
  found it bare, and returned `None`. Affected go, csharp, java, zig, dart, ruby, c, php, swift,
  gleam, elixir, r, typescript/node, brew, and python (which duplicated the same buggy lookup
  locally instead of calling the shared helper). The dropped assertion was then mislabeled by
  `error_path_assertions::render()` as an unrenderable `error.<field>` access
  (`... has no accessor for error field <none> in this backend`), hiding that a real message
  value existed. `declared_error_value()` now scans every `"error"` assertion for the first one
  carrying a value regardless of position; python is routed through the same shared function
  instead of its own copy; a bare or single-valued `"error"` assertion is never marked
  unrenderable. A genuinely unsupportable SECOND declared value (no observed fixture uses this
  shape) is now named honestly via a new `AdditionalDeclaredErrorValueNotChecked` skip instead of
  reusing the "no accessor for error field" wording.
- Ruby/Magnus bindings no longer emit invalid RBS/Ruby symbol literals for enum wire values that
  contain characters a bare Ruby symbol cannot carry (e.g. a serde-renamed variant like
  `fine-tune`). `sig/types.rbs` previously wrote `:fine-tune` verbatim
  (`src/backends/magnus/gen_stubs.rs`), which `rbs` rejects with `Syntax error: unexpected
  token='-'` and which consumers cannot fix themselves since the file is alef-generated. A single
  helper, `crate::backends::magnus::ruby_symbol_literal`, now renders every generated Ruby symbol
  literal derived from a wire value, quoting it (`:"fine-tune"`) only when the value is not a bare
  Ruby identifier — a bare-safe value like `KeyValue` still renders unquoted. The wire value itself
  is never altered (no hyphen-to-underscore sanitizing); only how the literal is written changes.
  Routed through the helper: the RBS `type value = ...` union in `gen_stubs.rs`, and the
  `#[serde(tag = "...")]` discriminator symbol emitted into the tagged-enum `from_hash` dispatcher
  in `gen_bindings/tagged_enums.rs`. The two other symbol sites in `tagged_enums.rs`
  (`:attr_name`/`:_0` Data.define attribute names, and the matching `hash[:field]` lookup) are
  built from the Rust struct field name, not from a serde-renamed wire value, so they cannot carry
  non-identifier characters and were left as bare symbols.
- The C# e2e trait-bridge test stub (`e2e::codegen::csharp::stubs`) no longer maps parameter and
  return types through a hand-rolled, duplicate mapper. It now routes every type through
  `backends::csharp::trait_bridge::csharp_type_visible_pub` — the same seam the production
  `I{TraitName}` interface is generated from — and derives the interface/method names through
  `codegen::naming::csharp_type_name` / `to_csharp_name` instead of a bare
  `heck::ToUpperCamelCase`. The duplicate mapper had drifted on `Json` (`object` vs the
  interface's `string`), `Duration` (`ulong?` vs `ulong`), `Option<Duration>` (`ulong??` — invalid
  C# syntax — vs `ulong?`), and any type or method name that folds under a C# initialism (e.g.
  `UuidPair` → `UUIDPair`, `get_uuid` → `GetUUID`), all of which produced stubs that failed to
  compile (CS0535/CS0246) against the real interface. `csharp_type_for_stub` and
  `csharp_type_for_stub_visible` are deleted; the C# e2e visitor generator
  (`e2e::codegen::csharp::visitor`) was routed through the same seam for consistency.
- The C# and Java e2e sealed-union `Display` helpers (`e2e::codegen::csharp::values::render_sealed_display`,
  `e2e::codegen::java::project::render_sealed_display`) no longer compute their per-variant wire
  string with a hardcoded `.to_lowercase()` of `serde_rename.unwrap_or(variant_name)`. That
  ignored `serde_rename_all` entirely (`ElementBased` under `snake_case` displayed as
  `"elementbased"` instead of `"element_based"`; under `kebab-case` it stayed `"elementbased"`
  instead of `"element-based"`) and lowercased an explicit `#[serde(rename = "...")]` that the
  binding preserves verbatim (`#[serde(rename = "Image")]` displayed as `"image"` instead of
  `"Image"`). Both now call `codegen::naming::wire_variant_value`, the same seam the production
  C# `json_name` (`backends::csharp::gen_bindings::enums`) and Java discriminator
  (`backends::java::gen_bindings::types::serializers`) compute from.
- Added cross-generator guard tests
  (`e2e::codegen::csharp::trait_bridge_stub_interface_seam_tests`,
  `e2e::codegen::csharp::sealed_display_wire_name_tests`,
  `e2e::codegen::java::sealed_display_wire_name_tests`) that render both the production and e2e
  paths independently and assert they agree, so a future hand-rolled mapper reintroduced in
  either generator fails the build instead of silently drifting again.
- The Elixir e2e generator no longer derives enum `rename_all` wire tags with a local,
  hand-rolled `apply_rename_all` helper that disagreed with the canonical
  `crate::codegen::naming::wire_variant_value` on two strategies: an absent (or unrecognized)
  `rename_all` left the variant name unchanged in the canonical helper but lowercased it to
  snake_case here, and `"UPPERCASE"` uppercased the raw name canonically but routed through
  `to_shouty_snake_case` (inserting underscores) here. The rustler binding itself already
  computes wire tags via `wire_variant_value`
  (`src/backends/rustler/gen_bindings/public_api_args.rs`,
  `src/backends/rustler/gen_bindings/types.rs`), so for the common no-`rename_all` case the two
  generators disagreed on the same IR: `match_unit_enum_atom` (`src/e2e/codegen/elixir/args.rs`)
  would fail to recognize a fixture's wire-tag string as matching any unit-enum variant and fall
  back to emitting a binary literal where the NIF's `NifUnitEnum` decoder expects an atom. Both
  call sites (`match_unit_enum_atom` and `emit_tagged_enum_array`'s tag matching) now call
  `wire_variant_value` directly so the e2e generator and the rustler binding cannot drift apart
  again.
- `alef diff` no longer reports every base64-encoded binary output (`gradle-wrapper.jar`) as
  pending on every run. `diff_files` read the on-disk bytes with `read_to_string(...)
  .unwrap_or_default()`, which yields an empty string for a jar and can therefore never equal the
  base64 `GeneratedFile::content` it was compared against — so the entry was permanent, unrelated
  to the file's actual bytes, and `alef diff --exit-code` could not return 0 in any repo with a
  kotlin-android target. Binary outputs are now compared as decoded bytes.
- `alef adopt` can now take ownership of a binary generated output instead of refusing it as
  "not text". alef's writers already guard binaries with `is_scaffold_owned_path` and record them
  with `record_scaffold_owned_path`, so the ownership rail existed — but no command could put a
  pre-existing binary into `.alef-ownership.toml`, leaving such a file refused by the write guard
  permanently. A binary match is now classified on its decoded bytes (converged when it already
  equals what alef would write, drifted otherwise), reviewed as size plus blake3 digest per side
  in place of the line diff bytes cannot have, and adopted through the committed record without
  its contents being touched. It remains a create-once seed, so `--clobber-create-once-seeds` is
  still required — `gradle-wrapper.jar` satisfies neither the reserved-namespace nor the
  sole-reader criterion of `cli::cache::is_alef_derived_output`.
- `adopt::managed_outputs` no longer runs the text normalizer over base64 binary content. The
  appended trailing newline made the payload undecodable, so the bytes alef would actually write
  were unrepresentable in the adopt candidate set.
- `alef e2e generate` no longer aborts documentation-snippet generation for fixtures whose `mock_url` / `mock_url_list` call argument has no declared `input` value. Previously such a fixture's snippet body fell back to `MOCK_SERVER_URL` / `MOCK_SERVER_<ID>` harness wiring and was rejected outright by the mock-harness leak guard, and the rejection aborted the whole batch — a structural failure for URL-centric consumers, where nearly every fixture is shaped this way. `Fixture::docs_call_fixture()`'s snippet rendering path now injects an illustrative `https://example.com` literal for any undeclared `mock_url`/`mock_url_list` argument before rendering, so no fixture edits are required; the executable e2e suite is unaffected since the injection runs only on the docs-transformed clone, never on the fixture the executable generator renders from.
- `hooks/check_backend_naming_helpers.py` only scanned `src/backends/` and matched banned
  helper names as an exact whole function name, so a backend-local casing/serde duplicate
  could evade it two ways: living outside `src/backends/` entirely (e.g.
  `src/codegen/generators/enums.rs::apply_rename_all`), or living inside the enforced path
  under a language-prefixed name (`src/backends/java/gen_bindings/helpers.rs::java_apply_rename_all`).
  The hook now scans all of `src/` and matches a banned name with an optional prefix
  (`\w*_?(apply_rename_all|wire_variant_value|...)`), plus a small `ALLOWLIST` of
  `(path, function name)` pairs — each with an inline reason — for the canonical definitions
  in `src/codegen/naming.rs` itself (`wire_variant_value`, `pascal_to_snake`) and for
  `java_apply_rename_all`, which stays a thin wrapper because
  `src/backends/java/gen_bindings/types/enums.rs` still calls it directly.
- Consolidated the confirmed-equivalent duplicate wire-value computations onto
  `crate::codegen::naming::wire_variant_value`/`apply_serde_rename_all`:
  `src/backends/java/gen_bindings/types/serializers.rs` (three call sites recomputing a
  variant's discriminator via `.serde_rename.clone().unwrap_or_else(...)` +
  `java_apply_rename_all`) now call `wire_variant_value` directly, and
  `java_apply_rename_all` (`src/backends/java/gen_bindings/helpers.rs`) now delegates its body
  to `naming::apply_serde_rename_all` instead of reimplementing the `rename_all` match arms.
  `src/backends/csharp/gen_bindings/enums.rs` was already routed through `wire_variant_value`
  end to end; no change was needed there. Widening the hook's scope surfaced additional
  casing-helper name collisions in `src/adapters/`, `src/codegen/error_gen/`,
  `src/codegen/generators/`, `src/cli/pipeline/helpers.rs`, `src/docs/`, `src/readme/`,
  `src/core/config/resolved/`, `src/backends/kotlin/`, `src/backends/wasm/`, and
  `src/e2e/codegen/` — all outside this change's file lane, left flagged for follow-up rather
  than allowlisted away.
- `alef snippets` typescript validation no longer mislabels genuine `tsc` type errors as a
  missing dependency. `TypeScriptValidator::is_dependency_error` previously matched 27 `TS`
  diagnostic codes -- including TS2322 (not assignable), TS2345 (argument mismatch), TS2304
  (cannot find name, ambiguous between a missing import and a real typo), TS7006 (implicit
  `any`), and a dozen other ordinary type/syntax errors -- as evidence the toolchain "ran but
  reported a missing dependency or build artifact", captioning the result with
  "run `alef build` first" and reclassifying it from `Fail` to the incomplete `Unavailable`
  status (`--strict` fails a run on zero `Unavailable`). A real content failure -- e.g. TS2322,
  TS2304 -- was laundered into the environment bucket, sending the reader to rebuild toolchains
  for a defect no rebuild could fix. `is_dependency_error` now matches only the 7 codes `tsc`
  emits when it could not *locate* a module, namespace, or declaration file (TS2307, TS2305,
  TS2306, TS7016, TS2792, TS2503, TS2580) -- the shape that actually means "this run's
  environment lacks a dependency or build artifact". Every other `tsc` failure stays `Fail`
  with the compiler's own message verbatim, so an unrecognized or ordinary failure is always
  shown, never re-captioned by guess.
- e2e/zig: guard `build.zig`'s `RunStep.setCwd` on the `test_documents` directory's existence via a new `zig/guarded_set_cwd.zig.jinja` template, closing the same unguarded-fork crash already fixed for Gradle (kotlin/kotlin_android) and Maven Surefire (java) in 0.63.0.
- e2e/kotlin: plain-Kotlin's `build.gradle.kts` now resolves the fixture working directory from `E2eConfig::test_documents_dir` (via `test_documents_relative_from`) instead of a hard-coded `"test_documents"` literal, matching the kotlin_android fix.
- Fixed a flaky `run_post_build_aborts_before_patching_a_stale_bridge_when_frb_is_skipped` test
  (`src/cli/pipeline/commands/build/frb_bridge_coverage.rs`) that raced under high thread count:
  it guarded its `ALEF_SKIP_COMMANDS` mutation with a test-local lock that other test modules
  setting the same process-global env var did not share. The test no longer touches the real
  environment at all — it points the `RunCommand` step at a command name that is never
  installed, which `run_run_command` already reports as `Ok(false)` (skipped) deterministically.
- `a_failing_language_does_not_abort_the_remaining_post_builds` (`src/bin_cli/helpers/post_build.rs`)
  set the process-global `ALEF_SKIP_COMMANDS` under a file-local lock distinct from
  `build.rs`'s `run_command_tests::env_lock()`, so it could race with that module's own
  `ALEF_SKIP_COMMANDS` mutations under real parallel test execution. Split
  `run_required_post_builds` into registry resolution (`run_required_post_builds`) and an
  aggregation loop over an explicit resolved `(language, BuildConfig)` list
  (`run_resolved_post_builds`), and pointed the test's Dart `RunCommand` step at a command
  name that is never installed on any host instead of forcing a skip via the env var -- the
  same fix shape as `f968767b6`. This also removes the test's dependence on whether
  `flutter_rust_bridge_codegen` happens to be on the host `PATH`, which previously made the
  "2 of 2" assertion depend on ambient toolchain state.
- `patch_root_package_manifest_replaces_release_placeholders` (`src/publish/package/swift.rs`)
  set the process-global `ALEF_SWIFT_CHECKSUM` with no lock at all. `patch_root_package_manifest`
  now takes the checksum as an explicit `Option<&str>` parameter instead of reading
  `ALEF_SWIFT_CHECKSUM`/`SWIFT_ARTIFACT_CHECKSUM` itself; `package_swift` resolves the env vars
  once and passes the result in. Added
  `patch_root_package_manifest_errors_when_checksum_placeholder_has_no_checksum` to cover the
  previously-untested missing-checksum error path, now expressible without touching `std::env`.
- C# and Java bindings no longer force the serde-derive `{"secs":u64,"nanos":u32}` object shape
  onto a `Duration` struct field that carries `#[serde(with = "...")]` (the common `duration_ms`
  convention, which writes a bare millisecond integer). Both backends previously applied their
  `DurationMillisJsonConverter` / `DurationMillisSerializer`+`DurationMillisDeserializer`
  unconditionally to every `Duration` field, which made the field round-trip as a JSON object
  instead of a scalar and broke Rust-side deserialization with `invalid type: map, expected u64`
  whenever the core used a hand-written millisecond codec. The decision now lives in a single
  predicate, `crate::codegen::naming::field_uses_duration_map_wire`, that Go (which already
  special-cased this), C#, and Java all consult, so the two wire forms cannot drift apart again
  per-backend.
- `discover_snippets` no longer silently skips a configured snippet directory that does not exist
  on disk; it now returns an error naming the missing path. Previously `if !dir.exists() { continue; }`
  made a snippets root that was repointed but never populated indistinguishable from one that was
  fully validated — `alef snippets list` reported "0 snippets" and exited 0, and `alef snippets check`
  passed silently whenever the missing root was only in `docs.snippets.inline_dirs` (a `dirs` entry
  happened to fail already, incidentally, via an unrelated coverage-ledger walk). A directory that
  exists but is genuinely empty is unaffected and still reports zero snippets without erroring.
- `alef snippets audit` no longer reports "Audit clean: no issues found" for a `--docs` root that
  does not exist. A missing documentation root was walked as an empty one, so an audit could pass
  having opened not one file; it is now reported as a `MissingDirectory` audit error naming the
  path. The same check covers `--snippets` roots, and it reaches `alef snippets check` and the
  docs stage's snippet audit (`docs::validate_snippets`), which share `snippets::audit::audit`.
- `alef snippets audit` and `alef snippets gaps` now name the real cause when a configured
  directory does not exist. Both already failed on a missing `--snippets` root, but only because
  the coverage-ledger walk tripped over it first, reporting "reading generated snippet coverage:
  ... IO error" for what is simply a path that is not there. A missing `--docs` root made `gaps`
  fail for a different wrong reason: with no documentation tree to walk, every snippet read as
  unreferenced.
- `alef verify` now reports a `docs.snippets.dirs` / `inline_dirs` entry that does not resolve to a
  directory on disk. `verify` checked generated-file hashes and generated-snippet coverage-ledger
  freshness, neither of which asks whether the configured roots exist, so a root renamed or deleted
  after the last generation passed as "All bindings and versions are up to date". The condition was
  in fact already being detected during `verify` — its managed-surface pass reaches
  `docs::build_snippet_context`, which refuses it — and then discarded, because a docs-stage error
  there is deliberately downgraded to a debug log. `--report-only` prints the finding without
  failing, as it does for every other verify finding.
- `alef all` no longer announces a formatter whose executable is missing as a step
  "deferred until the pinned version is published". A configured e2e `format` hook
  pointing at an absent binary (e.g. a `vendor/bin/php-cs-fixer` a fresh checkout does
  not have) is now reported under its own heading, naming the real cause and stating
  that the output was left unformatted. Both the `alef all` and the standalone-stage
  reporters now share one implementation, so the two can no longer classify the same
  deferral list differently.


## [0.63.1] - 2026-08-22

### Fixed

- `packages/go/cmd/setup/main.go` is gofmt-clean again. The template emits `versionIdent` inside a
  gofmt-aligned `const (...)` block, but the post-generation version rewriter
  (`sync_go_cmd_setup_version_ident`) replaced the whole assignment with a hard-coded single space,
  so every consumer regenerated at 0.63.0 shipped a `cmd/setup/main.go` that failed `gofmt -l` —
  a lint failure with no local cause, since the file is generated. The rewriter now captures and
  replays the alignment padding: the template owns the column, the rewriter owns the quoted value.
- Rust e2e codegen: the "Option field unwrap bindings" loop is now guarded by `!is_streaming`,
  matching the neighbouring array-binding loop. A streaming fixture asserting a string-typed
  optional field (e.g. `finish_reason`) emitted `let _finish_reason = result.finish_reason...`
  even though a streaming test never binds `result` — only `stream`/`chunks` — producing
  `error[E0425]: cannot find value 'result' in this scope`.
- Go e2e codegen: the "optional locals" loop is guarded the same way, for the same reason; it
  built `<local> := result.<field>` unconditionally for streaming fixtures, failing generated
  tests with `undefined: result`.
- Go e2e generation derives the `strings` import from the rendered test body instead of a
  fixture-level assertion-kind heuristic that never covered the declared-error-value path
  (`strings.Contains(err.Error(), ...)`), producing files that failed with `undefined: strings`.
- Elixir e2e codegen: the `tool_calls` streaming accessor no longer crashes on a content-only
  delta chunk. `Map.get(_, :tool_calls, [])`'s default only substitutes for an *absent* key, so a
  present-but-`nil` field returned `nil` into `Enum.flat_map`'s callback. Normalised with `|| []`.
- NAPI string enums compute each variant's runtime wire value with the same `convert_case`
  algorithm `napi-derive-backend` uses instead of `heck`, which diverges from it for variant names
  with a letter-to-digit boundary (`Bm25` — `heck` gives `"bm25"`, napi-rs gives `"bm_25"`). The
  mismatch surfaced as a generated `ts_type` literal TypeScript accepted but Rust rejected at
  runtime.
- NAPI plain string-enum `.d.ts` declarations derive their values from the canonical
  `string_enum_js_values` helper rather than re-deriving serde's wire name, which disagreed with
  napi's runtime value for the same letter-to-digit case.

## [0.63.0] - 2026-08-22

### Changed (BREAKING)

- **Generated Ruby, Dart and Elixir bindings now expose the serde *wire* value for enum
  variants, matching what the Go, Java, C#, Node and Python backends have always emitted.**
  Previously these three backends surfaced a host-idiomatic spelling with no way to reach the
  wire value, so a caller comparing a returned enum against a real JSON payload — or an e2e
  fixture declaring a language-agnostic literal such as `"KeyValue"` — silently never matched.

  Migration:
  - **Ruby**: a unit-variant enum now converts to the real wire symbol (`:KeyValue` where the
    Rust enum declares no `rename_all`), not an unconditionally snake_cased one (`:key_value`).
    `TryConvert` still accepts the legacy always-snake_case spelling, the verbatim Rust name and
    the wire value, so existing input code keeps working; code that *compares* a returned symbol
    against a snake_case literal must be updated. RBS/Sorbet stubs regenerate to match.
  - **Dart**: enum members keep their idiomatic lowerCamelCase names — nothing to change there.
    A new `wireValue` getter returns the wire spelling. Compare against `x.wireValue`, not
    `x.name`.
  - **Elixir**: atoms keep their existing spelling. A new `wire_value/1` returns the wire
    spelling. Replace `to_string(x)` with `Mod.wire_value(x)` — `to_string/1` was never correct
    here and raised `Protocol.UndefinedError` outright on data-carrying enums.

  Host-language identifiers, wire names, internal generated Rust names and ABI symbols remain
  four separate name surfaces; this change only makes the wire surface reachable where it
  previously was not.

### Fixed

- Magnus (Ruby): internally-tagged enum dispatch (`gen_tagged_enum_ruby_classes`) no longer
  fabricates a `snake_case` `serde_rename_all` default when the Rust enum declares none. Serde's
  real default is the verbatim variant name; the fallback made the generated `case`/`when`
  discriminator match the wrong wire value for any internally-tagged enum without an explicit
  `rename_all`.
- The Dart e2e `is_empty` assertion applied `isEmpty` to any field shape, throwing
  `NoSuchMethodError` on a struct- or scalar-shaped field. Its sibling `not_empty` arm already
  branched on `is_collection`; `is_empty` had no such branch at all. Both now share the decision.
- `scaffold(ruby)`: the generated `.gemspec` filtered `spec.files` with
  `.reject { |f| f.match?(%r{...}) }`, which the generated `.rubocop.yml` (`NewCops: enable`)
  flags via `Style/SelectByRegexp`. Both files carry `generated_header: true`, so `alef build`
  clobbered any consumer edit — a deadlock with no consumer-side escape, and
  `Style/SelectByRegexp` is `SafeAutoCorrect: false` so `rubocop -a` provably could not fix it.
  Switched to `grep_v` and added `*.gemspec` and `Rakefile` to `AllCops.Exclude` so a future cop
  cannot reopen the same deadlock on a file the consumer cannot hand-edit.
- Trust `goldziher/tap` before `brew install`ing `poly` in the `generated-output-gate` CI job —
  Homebrew 6.x refuses formulae from an untrusted third-party tap.
- Report whether a post-build `RunCommand` step (e.g. `flutter_rust_bridge_codegen`) actually ran
  or was silently skipped because its tool was absent from `PATH`, instead of treating both as
  success. `run_post_build` now returns a `PostBuildOutcome` carrying `skipped_missing_tools`, and
  `alef generate`/`alef all` warns per language when a step was skipped rather than run.
- Dart snippet validator's dependency-error classifier compared `dart analyze --format=machine`
  output byte-exact against lowercase literals, but the machine format upper-cases the CODE field
  (`URI_DOES_NOT_EXIST`), so missing-dependency diagnostics were misclassified as genuine snippet
  failures instead of `Unavailable`. Comparison is now case-insensitive.
- Snippet validator subprocess environment sanitization dropped `HOME` on Unix hosts, causing
  cargo, gradle, `dart pub`, `gem`, `mix` and npm to fail or fall back to surprising cache paths.
- `alef diff` now reports orphaned generated files alongside content changes, reusing
  `verify_orphans::find_orphaned_generated_files` — previously only `alef verify` surfaced them.
- `alef verify` now surfaces a refused write for an unmarkable, `generated_header: false`
  generated file with no committed `.alef-ownership.toml` record (e.g. the PHP backend's
  `config.m4`) — previously such a refusal was invisible to verification.
- `content_has_alef_marker` no longer treats an explicitly negated header ("This file is not
  generated by alef") as a real ownership marker. This was a potential data-loss path: alef's
  write guard decides whether it may overwrite a file from this marker. Covers a negation cue
  (`not`, `never`, `isn't`, `wasn't`, `aren't`, `weren't`, `doesn't`, `didn't`) immediately
  preceding the matched phrase, case-insensitively; non-adjacent negation and typographic
  apostrophes are documented, accepted gaps.
- `alef verify` no longer reports a generated file as plain "missing" when a `.gitignore` rule
  guarantees it can never be committed — running `alef generate` writes the file and the ignore
  rule discards it again, so the old report named a remedy that can never succeed. Such paths now
  appear under their own heading naming the real fix: narrow the ignore rule, then commit.
- The e2e Gradle and Maven projects set a test working directory without checking it exists.
  kotlin_android's `workingDir` is now guarded on `.isDirectory` and driven by the configured
  `test_documents` path instead of a hard-coded literal; Java's `<workingDirectory>` moved into a
  profile activated by Maven's file-existence `<activation><file><exists>` check. A cross-backend
  test drives all four backends through their public `E2eCodegen::generate()` entry points.
  Unguarded, Gradle test workers fail to fork against a consumer whose `test_documents/` has no
  tracked files in a fresh checkout, surfacing as a misleading "Gradle Test Executor N ... not in
  started or detached state" with no assertion text.
- The `kotlin_android` e2e generator no longer hard-codes the literal `test_documents` directory
  name in the generated `build.gradle.kts`, resolving it from `E2eConfig::test_documents_dir`
  instead — a consumer with a custom fixture directory previously got a path that did not exist.
- Java e2e doc snippets now name their `catch` clause using
  `backends::java::naming::exception_class_name`, matching the checked-exception class the Java
  backend actually declares. The snippet generator previously suffixed the public facade class
  (which has its `Rs` marker stripped) with `Exception`, naming a class that does not exist for
  any crate whose name does not already end in `Rs` — e.g. crate `tree-sitter-language-pack`
  generated a snippet catching `TreeSitterLanguagePackException` when the real class is
  `TreeSitterLanguagePackRsException`, breaking `javac` on every generated Java snippet that
  expects an error.
- Fixed a load-dependent `lib` test flake. `get_host_target_survives_a_deleted_ambient_cwd`
  deleted the directory backing the live process cwd while holding the shared `CWD_LOCK`, which
  corrupts `std::env::current_dir()` for *every* thread in the test binary, not just its own —
  `CWD_LOCK` serializes only against other lock-holders and cannot protect the crate's many
  unguarded `current_dir()` readers. The reproduction is now a deterministic assertion on the
  built `Command`'s `current_dir`, so the corrupting window is gone rather than merely rarer.

## [0.62.12] - 2026-08-22

### Fixed

- **The Java e2e assertion generator (`src/e2e/codegen/java/assertions.rs`) emitted a
  non-compiling `.getValue()` accessor on data-carrying IR enum fields (e.g. a
  `#[serde(untagged)]` union), a regression from 0.62.11's `field_is_enum` broadening
  (cd866bfdc).** `getValue()` only exists on the plain Java `enum` the Java binding backend
  emits for a non-data-carrying enum; a data-carrying enum still classifies as "enum" in the
  IR, but the Java binding backend renders it as a tagged/untagged-union wrapper class with no
  such method. `field_is_enum` now also checks
  `backends::java::gen_bindings::emits_get_value` — the exact predicate the Java binding
  backend itself uses to choose between a plain enum and a wrapper class
  (`src/backends/java/gen_bindings/types/enums.rs`) — via a new IR-derived
  `FieldResolver::java_enum_emits_get_value` lookup, so the two can never disagree again.
  Regression coverage: `src/e2e/codegen/java/assertion_union_enum_field_classification_tests.rs`.

- **A `bool` (or any other scalar-primitive) e2e result field with no explicit `fields_c_types`
  entry lowered its `equals` assertion into `strcmp` against a value the FFI actually returns as
  `int32_t`, segfaulting the generated C test.** `fields_c_types` is an operator-declared map in
  `alef.toml`; a field the operator never listed there fell through
  `emit_nested_accessor`'s/`render_test_function`'s leaf lookup to the `char*` default, so the
  local was declared `char* field = accessor(...)` for a value that is really a scalar, and
  `render_assertion`'s `equals` arm compared it with `strcmp(field, 1)` — passing an `int`
  literal where `strcmp` requires a `const char*`. Reproduced by crawlberg's generated
  `test_browser.c`: `browser_used: bool` (`int32_t cberg_crawl_result_browser_used(...)` in the
  header) asserted `== true` crashed with `make: *** [Makefile:97] Error 139`.

  Added `src/e2e/codegen/c/primitive_field_inference.rs`
  (`primitive_fields_c_types_from_ir`), mirroring the enum-field IR inference
  (`enum_field_inference.rs`) already unioned into `effective_fields_c_types`: every field whose
  declared Rust type (after peeling `Option`) is a scalar primitive gets an inferred
  `fields_c_types` entry — the real ABI/header spelling (`bool` -> `int32_t`, `u32` ->
  `uint32_t`, ...) via the same Rust-type -> C-header mapping the trait-bridge test-backend
  stubs already use (`trait_bridge_snippet::c_type`, now `pub(super)`). An explicit operator
  `fields_c_types` entry still wins.

  Fixing this also surfaced a second, narrower defect in the same `equals` arm: once a `bool`
  field's inferred type spells `int32_t` rather than the literal string `"bool"`, an *optional*
  bool field's `equals` assertion degraded into the "0 means unset" numeric-optional widening
  (`assert(field == 0 || field == 1)`) — vacuously true for either boolean value. `is_numeric`
  now also excludes any assertion whose fixture value is a JSON boolean, regardless of which
  type-string spelling produced `field_is_primitive`.

  Audited every other e2e backend for the same class of bug (config-only field-type
  classification causing a numeric/boolean field to route through a string-comparison path): C
  is uniquely affected because it alone is stringly-typed at the FFI ABI boundary (everything
  crosses as `char*` unless overridden); every other backend (Go, Java, Kotlin, C#, Swift, PHP,
  Ruby, Python, Dart, Rust, TypeScript, Zig, Elixir, Gleam) derives the "primitive vs string"
  decision from the fixture's own JSON value type against a statically/IR-typed accessor, not
  from a hand-maintained per-field type override map.

  Regression coverage: `src/e2e/codegen/c/primitive_field_inference.rs`'s `tests` module —
  asserts on the C TEXT `render_test_file` actually emits (not a hand-written mirror of the
  intended semantics), including the optional-bool-widening regression.

- **A collection-typed e2e result field with no per-element path declared anywhere in the
  fixture suite (nothing ever indexes into it, e.g. a recursive `List<DataNode> Children`) had
  no config signal telling `FieldResolver::is_array`/`is_collection_root` it was a collection at
  all, so C#/Kotlin/Swift/Rust each emitted broken generated code for it.** `is_array`/
  `is_collection_root` are both derived purely from `fields_array`/`fields_optional` — sets
  populated from element-traversal paths like `choices[0].message`, not from the bare collection
  accessor itself — so a field nothing ever indexes into has no such entry. Four distinct
  manifestations of the same missing classification:
  - **C#**: `is_empty` fell through to `Assert.True(string.IsNullOrEmpty(Children.ToString()))`
    — `List<T>.ToString()` returns the type name, so the assertion could never pass.
  - **Kotlin**: `not_empty` on an `Option<List<T>>` field degraded to a bare
    `{field} != null` check, which also passes for an empty-but-non-null collection.
  - **Swift**: `not_empty` on an `Optional<[T]>` field degraded the same way (`{field} != nil`).
  - **Rust**: `contains` emitted the scalar `{field}.contains(&expected)` shape, which requires
    the collection's own element type and does not compile against `Vec<DataNode>`.

  Added an IR-derived collection classification, `IrCollectionMap`
  (`src/e2e/field_access/ir_collection.rs`), mirroring `IrEnumMap`/`ir_enum.rs` exactly: keyed
  by `(owner_type, field_name)`, anchored at the call's declared Rust return type, answering
  whether a field's declared type (after peeling `Option`) is `Vec<T>`. `FieldResolver::
  is_collection_root` now consults it as a fallback after the hand-maintained config, via the
  new `FieldResolver::ir_collection_fields`/`with_ir_collection_map` (mirroring
  `ir_enum_fields`/`with_ir_enum_map`'s precedence: an explicit config entry still wins). Wired
  into all four affected backends' per-call `FieldResolver` construction
  (`src/e2e/codegen/csharp.rs`, `kotlin/test_method.rs`, `swift/test_method.rs`,
  `rust/test_file/test_function.rs`) alongside their existing `with_ir_enum_map` calls.

  Audited every other e2e backend consulting `is_array`/`is_collection_root` for a type-
  sensitive rendering decision (Go, Java, PHP, Ruby, Python, Dart, Gleam, Elixir, TypeScript,
  Zig): none share this shape — each derives the field's collection-ness from a per-language
  binding signature or a different IR-backed check already anchored per call, not from
  `is_array`/`is_collection_root` alone. `segment_name`, previously duplicated in `ir_enum.rs`,
  was extracted into `parse.rs` and shared by both IR path-walkers.

  Regression coverage, one file per affected backend, each driving the real per-fixture render
  entry point with zero `fields_array`/`fields_optional` config, mirroring
  `enum_field_classification_tests.rs`: `csharp/collection_field_classification_tests.rs`,
  `kotlin/collection_field_classification_tests.rs`,
  `swift/collection_field_classification_tests.rs` (all three exercise the real
  `render_test_method`/`render_engine_factory_test_function`-style entry point), and
  `rust/collection_field_classification_tests.rs` (drives `render_assertion` directly with an
  IR-anchored resolver, matching `assertion_containment_tests.rs`'s existing style for that
  backend). A second, narrower unit test in `csharp/collection_is_empty_tests.rs` covers
  `is_collection_root`'s IR fallback directly.

## [0.62.11] - 2026-08-22

### Fixed

- **A `bool` (or any other scalar-primitive) e2e result field with no explicit `fields_c_types`
  entry lowered its `equals` assertion into `strcmp` against a value the FFI actually returns as
  `int32_t`, segfaulting the generated C test.** `fields_c_types` is an operator-declared map in
  `alef.toml`; a field the operator never listed there fell through
  `emit_nested_accessor`'s/`render_test_function`'s leaf lookup to the `char*` default, so the
  local was declared `char* field = accessor(...)` for a value that is really a scalar, and
  `render_assertion`'s `equals` arm compared it with `strcmp(field, 1)` — passing an `int`
  literal where `strcmp` requires a `const char*`. Reproduced by crawlberg's generated
  `test_browser.c`: `browser_used: bool` (`int32_t cberg_crawl_result_browser_used(...)` in the
  header) asserted `== true` crashed with `make: *** [Makefile:97] Error 139`.

  Added `src/e2e/codegen/c/primitive_field_inference.rs`
  (`primitive_fields_c_types_from_ir`), mirroring the enum-field IR inference
  (`enum_field_inference.rs`) already unioned into `effective_fields_c_types`: every field whose
  declared Rust type (after peeling `Option`) is a scalar primitive gets an inferred
  `fields_c_types` entry — the real ABI/header spelling (`bool` -> `int32_t`, `u32` ->
  `uint32_t`, ...) via the same Rust-type -> C-header mapping the trait-bridge test-backend
  stubs already use (`trait_bridge_snippet::c_type`, now `pub(super)`). An explicit operator
  `fields_c_types` entry still wins.

  Fixing this also surfaced a second, narrower defect in the same `equals` arm: once a `bool`
  field's inferred type spells `int32_t` rather than the literal string `"bool"`, an *optional*
  bool field's `equals` assertion degraded into the "0 means unset" numeric-optional widening
  (`assert(field == 0 || field == 1)`) — vacuously true for either boolean value. `is_numeric`
  now also excludes any assertion whose fixture value is a JSON boolean, regardless of which
  type-string spelling produced `field_is_primitive`.

  Audited every other e2e backend for the same class of bug (config-only field-type
  classification causing a numeric/boolean field to route through a string-comparison path): C
  is uniquely affected because it alone is stringly-typed at the FFI ABI boundary (everything
  crosses as `char*` unless overridden); every other backend (Go, Java, Kotlin, C#, Swift, PHP,
  Ruby, Python, Dart, Rust, TypeScript, Zig, Elixir, Gleam) derives the "primitive vs string"
  decision from the fixture's own JSON value type against a statically/IR-typed accessor, not
  from a hand-maintained per-field type override map.

  Regression coverage: `src/e2e/codegen/c/primitive_field_inference.rs`'s `tests` module —
  asserts on the C TEXT `render_test_file` actually emits (not a hand-written mirror of the
  intended semantics), including the optional-bool-widening regression.
- **A collection-typed e2e result field with no per-element path declared anywhere in the
  fixture suite (nothing ever indexes into it, e.g. a recursive `List<DataNode> Children`) had
  no config signal telling `FieldResolver::is_array`/`is_collection_root` it was a collection at
  all, so C#/Kotlin/Swift/Rust each emitted broken generated code for it.** `is_array`/
  `is_collection_root` are both derived purely from `fields_array`/`fields_optional` — sets
  populated from element-traversal paths like `choices[0].message`, not from the bare collection
  accessor itself — so a field nothing ever indexes into has no such entry. Four distinct
  manifestations of the same missing classification:
  - **C#**: `is_empty` fell through to `Assert.True(string.IsNullOrEmpty(Children.ToString()))`
    — `List<T>.ToString()` returns the type name, so the assertion could never pass.
  - **Kotlin**: `not_empty` on an `Option<List<T>>` field degraded to a bare
    `{field} != null` check, which also passes for an empty-but-non-null collection.
  - **Swift**: `not_empty` on an `Optional<[T]>` field degraded the same way (`{field} != nil`).
  - **Rust**: `contains` emitted the scalar `{field}.contains(&expected)` shape, which requires
    the collection's own element type and does not compile against `Vec<DataNode>`.

  Added an IR-derived collection classification, `IrCollectionMap`
  (`src/e2e/field_access/ir_collection.rs`), mirroring `IrEnumMap`/`ir_enum.rs` exactly: keyed
  by `(owner_type, field_name)`, anchored at the call's declared Rust return type, answering
  whether a field's declared type (after peeling `Option`) is `Vec<T>`. `FieldResolver::
  is_collection_root` now consults it as a fallback after the hand-maintained config, via the
  new `FieldResolver::ir_collection_fields`/`with_ir_collection_map` (mirroring
  `ir_enum_fields`/`with_ir_enum_map`'s precedence: an explicit config entry still wins). Wired
  into all four affected backends' per-call `FieldResolver` construction
  (`src/e2e/codegen/csharp.rs`, `kotlin/test_method.rs`, `swift/test_method.rs`,
  `rust/test_file/test_function.rs`) alongside their existing `with_ir_enum_map` calls.

  Audited every other e2e backend consulting `is_array`/`is_collection_root` for a type-
  sensitive rendering decision (Go, Java, PHP, Ruby, Python, Dart, Gleam, Elixir, TypeScript,
  Zig): none share this shape — each derives the field's collection-ness from a per-language
  binding signature or a different IR-backed check already anchored per call, not from
  `is_array`/`is_collection_root` alone. `segment_name`, previously duplicated in `ir_enum.rs`,
  was extracted into `parse.rs` and shared by both IR path-walkers.

  Regression coverage, one file per affected backend, each driving the real per-fixture render
  entry point with zero `fields_array`/`fields_optional` config, mirroring
  `enum_field_classification_tests.rs`: `csharp/collection_field_classification_tests.rs`,
  `kotlin/collection_field_classification_tests.rs`,
  `swift/collection_field_classification_tests.rs` (all three exercise the real
  `render_test_method`/`render_engine_factory_test_function`-style entry point), and
  `rust/collection_field_classification_tests.rs` (drives `render_assertion` directly with an
  IR-anchored resolver, matching `assertion_containment_tests.rs`'s existing style for that
  backend). A second, narrower unit test in `csharp/collection_is_empty_tests.rs` covers
  `is_collection_root`'s IR fallback directly.
- **The PHP php_ext e2e smoke-app generator probed a global `<extension>_<function>()` symbol
  for crate-level free functions, but the PHP ext backend never emits one.** The php-ext
  backend (`src/backends/php/gen_bindings/rust_bindings.rs`) always places free functions as
  static methods on a namespaced facade class (`{namespace}\{Extension}Api::{method}`) —
  ext-php-rs's `#[php_impl]` registration derive walks every method in a fixed `impl` block and
  unconditionally references it by Rust identifier, so a free function can never be a standalone
  `#[php_function]` global there. The php_ext smoke-app generator
  (`src/e2e/codegen/php_ext.rs`) instead assumed the C-ABI naming convention
  (`{prefix}_{function}`), so it emitted a `function_exists()`/direct-call smoke test against a
  symbol the generated extension could never provide. `resolve_smoke_call` now resolves the
  facade class and static-method name the same way the backend derives them (via the new shared
  `backends::php::naming::php_ext_api_class_name` helper, also adopted by the backend itself so
  the two call sites cannot drift again), and `main.php` probes with
  `class_exists()`/`method_exists()` and calls through the class.

  Regression coverage: `tests/backends_php_ext_facade_symbol_subset.rs`. It diffs the facade
  method set the php-ext backend actually emits against the callable the smoke-app generator
  actually calls, rather than pinning one symbol name.
- **An already-scaffolded consumer's `poly.toml` kept re-merging the `alef-snippets`
  pre-commit hook forever, after alef itself stopped emitting it in 0.61.1.**
  `merge_managed_toml_core`'s prune pass removes only array *values* it tracked itself; it has
  no counterpart that retracts a whole *table* alef stops emitting, so the union pass kept
  re-adding a table already present on disk. Every commit shelled out to an `alef` binary the
  consumer's lint job never installs, failing `poly lint`/pre-commit with `alef-snippets: 1:
  alef: not found`. Added `migrate_poly_toml_drop_snippet_hook`
  (`src/scaffold/languages/poly_migrations.rs`), registered in
  `write_scaffold_files_report` next to the other pre-existing-file repairs: self-guarding on
  an exact match of the table's own `run` command and `workspace = true`, so it never touches
  a consumer's own differently-configured `alef-snippets` entry. Regression coverage:
  `src/scaffold/tests/poly_migrations.rs`.

- **The Ruby scaffold's `spec/<name>_spec.rb` seed failed `Style/WordArray` on any DTO with
  two or more `String` fields.** `ruby_construct_example` asserted a multi-field DTO's values
  with a bracketed array literal (e.g. `["alef-scaffold", "alef-scaffold"]`); the repeated
  seed literal is a hyphenated word, which the scaffolded `.rubocop.yml`'s default
  `Style/WordArray` (`WordRegex` permits one hyphen, `MinSize: 2`) still flags. An all-String
  field list now emits `%w[...]` instead, leaving mixed-type field lists (never all-String
  literals) on the bracket form the cop does not flag.
- **The C e2e generator lowered a `field[].key` fixture path (e.g. `structure[].kind`) to a
  scalar `alef_json_get_string(array_json, "key")` call against the ARRAY's own JSON text,
  making every "contains"-shaped assertion built from it unsatisfiable by construction — the
  array never has a `"key"` property, no matter what its elements contain.**
  `emit_nested_accessor`'s json-extraction leaf (`src/e2e/codegen/c/assertions.rs`) did not
  distinguish a true wildcard (`field[]`, "every element") from an explicit numeric index
  (`field[N]`, one concrete element) once both had entered the same json-extraction code path,
  so both fell through to the same single-scalar accessor. Every other e2e backend already
  quantifies over the array for this fixture shape (`field_resolver.wildcard_split` +
  `.iter().any(..)` / `Enum.any?` / `any(...)`, depending on language) — C was the only backend
  missing it. Fixed by tracking whether `json_extract_mode` was entered through an empty `[]`
  key, and deferring a wildcard leaf to a new per-element quantifier
  (`src/e2e/codegen/c/collection_wildcard.rs`, rendered through the new
  `templates/c/wildcard_collection_assertion.jinja`) instead of emitting the scalar accessor.
  Supports `contains`, `contains_all`, `contains_any`, `not_contains` and `equals`; any other
  assertion type against a wildcard field now renders an honest skip comment instead of a
  silently-wrong assertion. As a forced consequence of unifying the three call sites
  (`call_patterns.rs`, `test_function.rs` x2) onto one classification function, also fixed a
  latent divergence where one of the three filed every nested opaque-handle leaf under
  `primitive_locals` instead of `opaque_handle_locals`.

  Surfaced by tree-sitter-language-pack's generated `e2e/c/test_process.c`, whose
  `structure[].kind` / `Module`/`Class`/`Function` "contains" assertions could never pass.

  A second, distinct defect on the same tslp gate: `is_empty`/`not_empty` against a `char*`
  leaf holding serialized JSON collection text (e.g. a `Vec<T>` field) compared with
  `strlen(field_expr) == 0`, which reads an empty collection's own serialization (`"[]"`/`"{}"`)
  as non-empty. Fixed via a new `templates/c/scalar_or_collection_empty.jinja` that accepts
  either empty form. Reproduced by `data_extraction_json_empty_object` /
  `data_extraction_properties_empty`'s `is_empty` on `data.children` (`test_data_extraction.c`).

  Audited every other e2e backend for the same `field[].key` mis-lowering: Python, Rust,
  TypeScript, Ruby, Java, C#, Go, Dart, Zig, Swift, Kotlin, PHP and R already quantify over the
  array correctly; Gleam and (via delegation) WASM/Kotlin Android deliberately refuse the path
  with an honest skip comment rather than mis-emit. Elixir also already quantifies correctly
  (`Enum.any?`) — its "many `Assertion with ==` failed" C FFI-gate-adjacent failures on the same
  tslp run are a different, not-yet-investigated defect.

  Regression coverage: `src/e2e/codegen/c/wildcard_collection_regression_tests.rs` (five tests,
  asserting on generated C TEXT — not a hand-written mirror of the intended semantics, which
  the original bug would not have caught) and
  `src/e2e/codegen/c/collection_empty_assertion_tests.rs` (two tests). Both new sibling
  modules, not additions to `assertions.rs`/`test_function.rs` (already over the 1,000-line
  file cap).
- **A release-asset upload whose glob matched zero files reported success, so a release could be
  published carrying no CLI binaries with no red X anywhere.** `upload-release-assets` in
  `.github/workflows/publish.yaml` downloaded the `cli-*` artifacts into `dist/cli` and handed
  `artifacts: "dist/cli/*"` straight to the release action. `actions/download-artifact` has no
  `if-no-files-found` input, so a pattern matching nothing leaves `dist/cli` empty and merely
  warns; the upload action then publishes a release with zero assets and exits 0, and `finalize`
  reports the job result as `success`. This is the same vacuous-green shape as the empty CLI
  target matrix fixed in 72c7b055a, one job downstream. A `Verify CLI release assets are present`
  step now expands the glob under `nullglob` and exits 1 naming `dist/cli/*` when it matched
  nothing, before the upload step runs.

  Regression coverage: `src/publish/release_asset_guard_tests.rs` parses the real
  `.github/workflows/publish.yaml` and asserts that *every* glob handed to an upload action is
  guarded — either by `if-no-files-found: error` on the step or by a same-job `run:` step that
  names the glob and `exit 1`s. It fails on a new unguarded upload step, not just on this one,
  and refuses to pass vacuously if the scan matches no upload steps at all. The sibling
  homebrew-bottle upload already carried `if-no-files-found: error` and passes unchanged.

- **A typo in `[[crates.services.registrations.variants]].languages` or
  `[[crates.trait_bridges]].exclude_languages` silently no-oped instead of failing.** Both
  fields are `BTreeMap`/`Vec` of raw language-name strings, keyed or valued against the same
  canonical names as `languages`, but neither was checked against `is_known_language` the way
  the sibling `skip_languages` fields on adapters and services already are — an unknown name
  like `variants.languages.knotln` or `exclude_languages = ["wasm32"]` just described a
  language override or exclusion that never matched anything, with no error and no warning.
  `NewAlefConfig::resolve_one` now validates both against `is_known_language` at config-resolve
  time, mirroring the existing `skip_languages` check byte-for-byte in message shape, so a typo
  is a hard `InvalidConfig` error naming the crate, the owner, and the bad name instead of dead
  config.

  Regression coverage: `resolve_rejects_unknown_language_in_registration_variant`,
  `resolve_accepts_valid_language_in_registration_variant`,
  `resolve_rejects_unknown_language_in_trait_bridge_exclude_languages`, and
  `resolve_accepts_valid_trait_bridge_exclude_languages` in
  `src/core/config/new_config/tests.rs`.

- **`commands::test::get_host_target` spawned `rustc --version --verbose` without pinning a
  working directory, so it could intermittently fail with "Could not locate working directory"
  when it raced another test's `CwdGuard`.** `cargo test` runs every `#[test]` as a thread in one
  process, so a spawn that never calls `.current_dir(..)` inherits whatever the process-wide cwd
  happens to be, including a tempdir another test entered and has since deleted — the same race
  class fixed for twelve compile-harness spawns in 22baa34ac, missed for this one because it is
  production code (`src/cli/pipeline/commands/test.rs`) rather than a test file, and because this
  crate's own e2e-phase unit tests (`before_hook_runs_before_e2e_command` and friends) call it
  unconditionally whenever `e2e = true`, regardless of target language. Pinned to
  `std::env::temp_dir()`, which needs no case-by-case tempdir since the call reads nothing
  relative to its cwd.

  A sweep of `Command::new` in test code beyond the twelve already-pinned sites found nine more
  unpinned spawns with the identical shape (`javac -version`/`mvn --version`/`dotnet
  --version`/`dart --version`/`pyproject-fmt --version` availability probes, plus `gofmt`, `zig
  ast-check`, and two `cargo`/`--manifest-path` invocations). Rather than a thirteenth-through-
  twenty-first manual `.current_dir` fix, added `test_support::spawn_from_stable_dir`, a
  `Command::new` wrapper pre-pinned to the system temp directory, and migrated the version-probe
  call sites to it; the sites with a natural test-owned tempdir already in scope were pinned to
  that tempdir directly, matching the existing 22baa34ac pattern.

  Regression coverage: `get_host_target_survives_a_deleted_ambient_cwd` in
  `src/cli/pipeline/commands/test.rs` deterministically reproduces the race (enter a tempdir as
  cwd, delete it, call `get_host_target`) rather than relying on true thread interleaving.

- **`is_known_language` hand-maintained a fifth copy of the canonical `Language` name list,**
  alongside `Language::ALL`, `Display`, and two identical "valid names are: ..." error strings in
  `new_config.rs` — a variant added to the enum without also updating this list would be
  silently rejected by `skip_languages` validation with an error message that also omitted it
  from the "valid names" it printed. `is_known_language` now derives its answer from
  `Language::ALL` and `Display` instead of a second hand-typed list, and the two `new_config.rs`
  error messages build their "valid names" text from the new `Language::all_names_joined()`
  instead of a literal copy of the same names. (`src/core/config/extras.rs`,
  `src/core/config/new_config.rs`)

- **The C# backend declared `[DllImport]` entry points for symbols the C FFI backend never
  exports, whenever a scalar-crossing enum reached a parameter position.** A fieldless `Copy`
  enum crosses the C ABI as `int32_t`, not as an `AlefHandle`, so the FFI backend gives it
  `from_i32`/`from_str` and deliberately emits no `{prefix}_{enum}_from_json`. The
  `gen_native_methods` parameter loop (`src/backends/csharp/gen_bindings/functions.rs`) gated
  its `FromJson` emission only on "is this an opaque struct", and emitted the paired `Free`
  outside that guard entirely — so any enum reaching a parameter position got `{E}FromJson`
  and `{E}Free` P/Invokes bound to non-existent symbols. Both are dead declarations until
  something calls them, at which point they throw `EntryPointNotFoundException`; they parse
  cleanly, so every parse-based gate passed. The loop now skips scalar-crossing named types
  outright, using `backends::ffi::type_map::scalar_c_abi_named_types` — the module whose own
  documentation already designates it the single source for this decision and names the C#
  `[DllImport]` emitters as required consumers. A scalar type that is also *returned* still
  gets its `Free` from the return loop, which mirrors the FFI's returned-enum condition.

  Surfaced by html-to-markdown, where the 0.62.9 regen re-introduced a
  `htm_node_type_from_json` P/Invoke (`NodeType` is a fieldless `Copy` enum) and broke the
  repo's export-vs-caller CI gate. That same symbol had been allowlisted and retired once
  before, making this a re-regression.

  Regression coverage: `tests/backends_csharp_ffi_symbol_subset.rs`. Beyond pinning the two
  specific symbols, it asserts generically that the C# `EntryPoint` set is a subset of the
  FFI crate's `extern "C" fn` set — the cross-backend check that was missing. Nothing in the
  repo previously diffed the two symbol sets;
  `backends_csharp_native_method_declaration_coverage` checks only the opposite direction
  (every call site has a declaration), so a declared-but-uncalled extern passed it trivially.
- **`alef all` / `alef generate` swallowed the one diagnostic that explains a Dart FRB post-build
  failure caused by a frozen manifest, leaving operators chasing the wrong fix.** A Dart FRB
  facade (`packages/dart/rust/src/lib.rs`) self-marks and so always regenerates freely, but its
  sibling manifest (`packages/dart/rust/Cargo.toml`) is markable and `generated_header: true`;
  in a consumer tree where that `Cargo.toml` predates alef ever stamping `.toml` output, it
  carries no `alef:hash:` marker, so the ownership guard refuses every write to it forever —
  including the forwarding `[features]` entry `collect_cfg_features` adds the moment a facade
  function gains a new `#[cfg(feature = "...")]` gate. The facade gains the gated function; the
  manifest that would activate its feature for a real `cargo-expand` does not; `flutter_rust_bridge_codegen`
  correctly omits the function; `VerifyFrbBridgeCoverage` (alef #135) correctly fails the build.
  `pipeline::report_refused_writes` — the only call site that turns that refusal into an
  actionable "run `alef adopt <path>`" message — was only ever invoked at the very end of
  `handle`'s `Commands::All` / `Commands::Generate` arms, unreachable once `complete_generated_artifacts`'s
  bare `?` had already propagated the post-build error. Reordered both call sites to surface the
  refusal report before returning the post-build error. `VerifyFrbBridgeCoverage`'s own detection
  is unchanged and still fails the build. Regression:
  `all_surfaces_the_refusal_report_before_a_post_build_coverage_failure`
  (`src/bin_cli/all_commands_refusal_tests.rs`, a new sibling test module —
  `all_commands_tests.rs` is already near the 1,000-line file cap).
- **Java `equals` assertions on a real Java enum field the consumer's `alef.toml` never listed
  under `fields_enum` rendered a raw `assertEquals("KeyValue", result.data().kind())` instead of
  `result.data().kind().getValue()`, an assertion that can never pass — `assertEquals`'s
  `.equals()` never matches a `String` against an enum constant regardless of the actual value.**
  `java/assertions.rs`'s `field_is_enum` classified a field only from the hand-maintained
  `enum_fields` config; unlike every other backend with the same enum/equals codegen
  (csharp, kotlin, dart, gleam, swift, python, ruby, rust, zig), it never consulted
  `FieldResolver::is_enum`, the IR-derived classification anchored at the call's declared Rust
  return type. `java/test_method.rs` compounded this: its `FieldResolver` was built without ever
  calling `.with_enum_fields`/`.with_ir_enum_map`, so the IR data was never even wired in. A
  recursive struct's own enum field reached only through its parent's field path (e.g.
  `DataNodeKind` via `data.kind` on a self-referential `Option<Box<DataNode>>`) is exactly the
  shape a hand-maintained config is least likely to list. Wired both in, mirroring
  csharp/kotlin/swift's `resolve_declared_result_type` anchor. Regression coverage:
  `java/assertion_enum_field_classification_tests.rs`.
- **`alef verify --exit-code` could never pass on a tree that had just been cleanly
  regenerated: its disk walk descended into gitignored dependency-fetch caches and
  build-output directories, claiming content it neither generated nor manages.**
  `collect_alef_hashes` (`src/bin_cli/helpers.rs`) only pruned a small, hand-maintained list of
  build/cache directory names (`target/`, `node_modules/`, ...), which cannot know about a
  consumer's own package-manager fetch cache or a build tool's own output directory. Observed in
  html-to-markdown: a zig package manager's local dependency cache
  (`test_apps/zig/zig-pkg/<fetched-package>/src/*.zig`) carried old `alef:hash:` headers from
  whichever alef version generated the upstream package release at fetch time, and
  `wasm-pack build` copies the crate's own alef-marked `README.md` into
  `crates/<crate>-wasm/pkg/<target>/README.md` as part of packaging — both directories are
  gitignored, untracked, and not part of this run's generation input or output, yet `alef
  verify` opened them anyway and reported the fetched/copied files stale and orphaned on every
  run, regardless of how many times `alef all` had just regenerated the tree. Added
  `verify_gitignore::gitignored_dirs` (`src/bin_cli/verify_gitignore.rs`), which asks git
  directly (`git ls-files --others --ignored --exclude-standard --directory`) which directories
  are ignored and prunes them from the walk — generalizing the hand-maintained skip list to
  "whatever this repo itself says is ignored" instead of adding one more magic directory name
  per incident. Falls back to an empty set outside a git work tree or without `git` on `$PATH`,
  so the walk's existing hand-maintained baseline is unaffected there. Regression coverage:
  `gitignored_dirs_reports_a_nested_dependency_cache_directory` and its siblings
  (`src/bin_cli/verify_gitignore/tests.rs`), and
  `verify_passes_with_zero_findings_despite_a_gitignored_dependency_cache_directory`
  (`src/bin_cli/core_commands/tests.rs`), which drives a real `alef all` + `alef verify` through
  the CLI dispatch path against a fixture carrying a gitignored, alef-marked-but-foreign file.

- **The C FFI backend never exported `{prefix}_{enum}_from_json` for a data-carrying enum
  reached only through a method parameter, the mirror image of the C# bug fixed above.**
  `enum_pointer_param` in `src/backends/ffi/gen_bindings/lib_rs.rs`, which decides which enums
  need a `_from_json`/`_free` companion pair, was built solely from `api.functions[].params`.
  Its sibling `enum_pointer_return` already walked free-function returns, method returns, and
  struct fields, but the param-side set never walked `TypeDef::methods[].params`. An enum
  passed only as a method parameter (never a free-function parameter) got no `_from_json`
  export, even though the C# backend already declares the DllImport for any `has_serde`
  parameter-position enum regardless of whether a free function or a method carries it — a
  live `EntryPointNotFoundException`. `enum_pointer_param` now walks method parameters too,
  guarded by the same `scalar_c_abi_named_types` exclusion so fieldless `Copy` enums stay
  correctly omitted (confirmed by extending
  `tests/backends_csharp_ffi_symbol_subset.rs` with a data-carrying, method-only-parameter
  fixture alongside the existing scalar one).

  Regression coverage: `src/backends/ffi/gen_bindings/tests/method_param_enums.rs` (new file,
  kept separate from `regressions.rs` to avoid growing it past the file-modularization line
  cap) and a fourth test in `tests/backends_csharp_ffi_symbol_subset.rs` asserting the FFI
  export and the C# P/Invoke agree for a method-only-parameter data-carrying enum.
- **The Dart FRB bridge crate's `Cargo.toml` could permanently miss a `#[cfg(feature = "X")]`
  gate its own `lib.rs` already forwards, producing `unexpected cfg condition value` once
  `flutter_rust_bridge_codegen` re-emitted that gate on the wire wrapper (alef #154).**
  `codegen::cfg::merge_missing_cfg_features` -- the additive repair that backfills a missing
  cfg-forwarded feature into an already-scaffolded, guard-refused binding manifest without a
  full overwrite -- was wired up for Ruby (Magnus) and Elixir (Rustler) only
  (`scaffold::repair::managed_manifests`); Dart's `packages/dart/rust/Cargo.toml` is exactly as
  exposed to the same staleness (also `generated_header: true`, also derived from
  `collect_cfg_features` on the same `ApiSurface` as `lib.rs`) but was never added when this
  repair was written. Reproduced against liter-llm's actual shape: `lib.rs` already emits
  `#[cfg(feature = "tokenizer")] pub fn count_tokens(...)` /
  `#[cfg(feature = "tower")] pub fn record_cost_usd(...)`, forwarded there from
  `count_tokens`/`count_request_tokens`/`record_cost_usd`'s real gates in the core crate, but the
  committed Dart Cargo.toml never picked up `tokenizer`/`tower` because nothing ever repaired it
  after it was first scaffolded. `managed_manifests` now also covers
  `packages/dart/rust/Cargo.toml` (`backends::dart::gen_rust_crate::dart_native_manifest_path`).
  Dart's forwarding rows key off the Cargo dependency-table key its own generator uses
  (`[crates.dart] core_crate_override`, or the crate name with `-` replaced by `_` --
  `backends::dart::gen_rust_crate::dart_core_dep_key`), which differs from Ruby/Elixir's raw,
  unmodified crate name, so `repair_missing_cfg_binding_features` now threads a per-language
  dependency key through `managed_manifests` instead of assuming Ruby/Elixir's convention
  everywhere. Regression coverage: `repair_adds_missing_features_to_dart_manifest`
  (`src/scaffold/tests/repair.rs`), asserting the forwarded rows use the underscored dependency
  key, not the raw crate name.

- **`sync-versions` (without `--regen`, the default — regenerating code is opt-in) bumped
  `[package] version` in the Rust e2e harness manifest (`<e2e.output>/rust/Cargo.toml`,
  `<e2e.registry.output>/rust/Cargo.toml`) but never touched the manifest's own
  self-referential dependency pin, leaving it stuck on the pre-bump version (alef #152).**
  `sync_rust_test_app_version`/`sync_rust_harness_cargo_toml`
  (`src/cli/pipeline/version_workspace.rs`) called `write_version_to_cargo_toml` (patches
  `[package] version` only) but never called `patch_workspace_dep_versions` (patches the
  dependency pin) — unlike the sibling `packages/ruby/ext/*/native/Cargo.toml` block in
  `version.rs`, which already pairs both calls for exactly this reason. This was initially
  reported as a `package = "..."` rename discriminator (crawlberg 1.3.2's plain-form
  `crawlberg = { version = "1.3.1", ... }` stayed stale while sibling releases' renamed-form
  pins, e.g. liter-llm's `liter_llm = { package = "liter-llm", version = "...", ... }`, looked
  correctly bumped in their committed trees) — but reproducing against crawlberg's actual
  alef.toml and source tree showed the rename spelling was never the deciding factor: neither
  spelling was ever patched by `sync-versions` itself; the sibling releases' pins were correct
  only because a later, separate full regen (`alef generate`/`alef all`, which does run
  `render_cargo_toml`) happened to land before crawlberg's did. The downstream consequence:
  the stale requirement leaves `cargo update --offline -w` unable to resolve against the crate's
  newly-bumped (not-yet-published) version, and `blocked_on_publish` detection — which keys off
  the *expected* version matching the requirement — doesn't recognize the mismatch as "blocked
  on this release," so the release gate fails with a confusing message instead of the intended
  graceful skip. `sync_rust_harness_cargo_toml` now calls `patch_workspace_dep_versions` with
  the crate's own name for every Rust e2e harness manifest it owns, alongside the existing
  `[package] version` write — `patch_workspace_dep_versions`'s membership check already covers
  both the plain form and the `package = "..."` renamed form, so one call fixes both spellings.
  Regression coverage: `sync_versions_bumps_registry_dep_pin_with_package_rename` and
  `sync_versions_bumps_registry_dep_pin_without_package_rename`
  (`src/cli/pipeline/version_tests/registry_dep_pin.rs`), both run with `no_regen: true` to
  match the real, default `sync-versions` invocation that reproduced the bug.
- **The `[workspace] alef_version` pin exclusion from `compute_inputs_hash` (0.62.2) had no
  coverage of the real write path, only of the hash function in isolation.** Investigating a
  ~8,300-file `alef:hash:`-only diff reported against a consumer repo traced it to three prior
  commits that edited `alef.toml` (`languages`, e2e registry package versions) without ever
  running `alef generate` afterward — the observed diff was that backlog converging in one
  regeneration, not a live regression: `compute_inputs_hash` was verified byte-for-byte stable
  across the reported before/after `alef.toml` pair once the unrelated drift is excluded, and a
  version-only bump reproduces zero embedded-hash changes. Added
  `tests/cli_generate_version_pin_hash_stability.rs`, which runs the real `alef` binary through
  two full `generate` invocations differing only in the pin and asserts the entire output tree is
  byte-identical, including every `alef:hash:` line — closing the gap left by
  `inputs_hash_alef_version_pin_table`, which only ever exercised `compute_inputs_hash` directly
  and could not have caught a bug anywhere else in `write_files_report` or `finalize_hashes`.
  Confirmed this new test fails (8 files rewritten) against the pre-0.62.2 hash formula and
  passes against the current one.

### Changed

- **Centralized the `not_error`-must-not-assert-presence-beside-a-sibling-assertion decision
  (alef #165) into shared `src/e2e/` logic instead of letting each backend rediscover WHETHER
  to guard.** The rule had been independently discovered and fixed seven times under seven
  different flag names — `has_other_assertions` (typescript, csharp, elixir),
  `bare_result_is_option` (swift, kotlin, r), `result_is_option && bare_field` (java) — with
  every new backend starting unguarded by default and Kotlin's own doc comment at one point
  falsely claiming the case was already handled. Added
  `not_error_presence::may_assert_presence(fixture, result_is_option) -> bool`
  (`src/e2e/codegen/not_error_presence.rs`), the one place that now decides whether a
  `not_error` assertion may render an explicit presence check: unsafe when the call's result is
  `Option<T>` (a legitimate `None` on success) or when a sibling assertion already gives the
  fixture non-vacuous coverage. Converted csharp, typescript, elixir, java, and kotlin to call
  it once per fixture and thread the single resulting boolean into `render_assertion` (backends
  keep control of *how* to render the check; the shared function decides only *whether*).
  Closes a real, previously-unfixed gap in csharp/typescript/elixir: a fixture whose *sole*
  assertion was `not_error` on an `Option<T>`-returning call still asserted presence
  unconditionally there (their `has_other_assertions` guards only fired beside a sibling
  assertion), which fails whenever the call's success path legitimately returns `None`. Also
  fixed a second latent bug this exposed in Elixir: `apply_vacuous_assertion_fallback`
  (`test_case.rs`) reinjected the identical unsafe `refute is_nil(...)` whenever the assertions
  body ended up empty, silently undoing the new guard for a `not_error`-only fixture on a bare
  `Option<T>` result — now gated on the same `result_is_option` fact. Added
  `not_error_presence::tests` (4 cases) plus per-backend regression coverage
  (`csharp/not_error_presence_guard_tests.rs`, `typescript/not_error_sibling_assertion_tests.rs`,
  `elixir/not_error_sibling_assertion_tests.rs`,
  `elixir/not_error_bare_option_underscoring_tests.rs`, `java/not_error_bare_option_tests.rs`,
  `kotlin/not_error.rs`) driving the real generators — not hand-written mirrors of them — with
  flags produced by the real shared function.

## [0.62.10] - 2026-08-21

### Fixed

- **Snippet validation passed `-I` to `zig build`, which rejects it, failing every zig snippet
  routed through a real zig package (one with `build.zig` and `build.zig.zon` next to the
  binding module).** `ZigValidator::validate_in_session` (`src/snippets/validators/zig.rs`)
  reconstructs the command as `zig build --summary none --build-file ...` when
  `zig_package_root` finds a real package, but the include-path application that follows
  (`apply_include_paths`, which emits `-I`) ran unconditionally for both that path and the
  `zig build-exe` path — and `-I` is a `build-exe`-only flag, so `zig build` fails outright with
  `unrecognized argument: '-I'` before compiling a line. The build-system path needs no
  `-I` at all: the snippet imports only the binding module, and the package's own `build.zig`
  already declares its include directories, reaching the compilation through the dependency.
  Guarded both `apply_include_paths` calls behind a `uses_build_system` flag set only on the
  `zig build-exe` path. This was a field regression from the `build-exe` include-path fix
  (0.61.0's `ca830e3fc`); its own 25 tests all construct a bare manifest with no
  `build.zig.zon`, so none of them ever drove `zig_package_root` to `Some` and exercised the
  `zig build --build-file` branch. Regression coverage:
  `session_include_paths_are_not_forwarded_to_the_build_system_command`
  (`src/snippets/validators/zig/session_command_tests.rs`), a new sibling test module (`zig.rs`
  is already at the 1,000-line file cap).

## [0.62.9] - 2026-08-21

### Fixed

- **A renamed workspace-member path dependency (dependency-table key different from the crate's
  published `package = "..."` name) was silently left as a `path = "..."` dependency when
  `alef publish` rewrote a shipped binding manifest to registry version-pins, producing a
  manifest that cannot resolve off the workspace it was cut from.**
  `rewrite_dep_table` (`src/publish/vendor.rs`) determined workspace-member path deps by matching
  the raw dependency-table key against `WorkspaceMembers::names`, so an aliased entry like
  `mylib_core = { path = "../mylib-core", version = "1.7.0", package = "mylib-core" }` was never
  recognized as a member dependency and its `path` was never stripped — while
  `registry_dependencies_on_local_crates` (`src/cli/commands/version_manifests.rs`, feeding
  `alef validate versions`) already resolved the same shape correctly through the entry's
  `package = "..."` field. `dependency_crate_name` (`src/publish/workspace.rs`) is now the single
  place that resolves a dependency-table entry's real crate name (its `package` alias, or the key
  when there is none), and both `rewrite_dep_table` and the `alef publish prepare` defense-in-depth
  check `assert_no_member_path_deps` (`src/publish/mod.rs`) call it, so the two can no longer
  disagree about which entries are workspace members. Regression coverage:
  `rewrite_path_deps_resolves_package_alias_for_membership`
  (`src/publish/vendor/tests.rs`).
- **The FFI backend's crate-level clippy allow list carried `dropping_references`, a dead
  entry.** Every `drop(...)` call the FFI templates emit drops an owned value (`free_bytes.jinja`,
  `free_string.jinja`, `handle_registry.rs.jinja`'s `self.take::<T>(..)` and `guard`, and
  `orchestration.rs`'s `std::mem::drop(obj)` for a method named `drop`, always bound owned via
  `null_check_self_owned.jinja`), never a reference, so the lint had nothing left to allow.
  Confirmed dead with a real `cargo clippy --all-targets -- -D warnings` run over the gate
  fixture with the entry removed. Removed from `src/backends/ffi/gen_bindings/lib_rs.rs` and
  pinned in `clippy_allowlist.rs`'s dead-entry test. Added
  `tests/generated_output_downstream_gate/ffi_allowlist_gate.rs`, a real-clippy-backed check for
  the direction the existing allow-list tests never covered: an allow entry with no matching
  generated pattern now fails the gate instead of sitting there silently.
- **The CLI release matrix step could resolve to zero targets and still report success,
  building no CLI binaries.** `.github/workflows/publish.yaml`'s "Resolve CLI target matrix"
  step read `.github/cli-targets.json` and fed it straight into `build-cli`'s
  `strategy.matrix.include` with no check that the list was non-empty; GitHub Actions runs a
  zero-entry matrix as zero job legs and reports the job as a vacuous success rather than a
  failure — the same "SKIPPED reads as PASSED" shape that produced a false-premise bug report
  in html-to-markdown. The step now hard-fails when the target list is empty, before ever
  writing `matrix=`/`assets=` to `$GITHUB_OUTPUT`.
- **The TypeScript e2e generator emitted `.resolves` on synchronous void calls, throwing
  `TypeError: You must provide a Promise to expect() when using .resolves, not 'undefined'`
  (alef #B5).** A `returns_void` call whose only assertion is `not_error` wraps `call_expr`
  in an expectation instead of a bare statement, but `test_case.rs`'s `void_not_error`
  computation never consulted `call_is_async`, so `typescript/test_function.jinja` emitted
  `await expect(call_expr).resolves.not.toThrow()` even for synchronous NAPI bindings
  (`cleanCache()`, `configure()`, `init()`, `prefetch()`) that resolve no Promise at all.
  `call_is_async` (`src/e2e/codegen/typescript/test_file/test_case.rs:71-75`) is now threaded
  into the template context; `test_function.jinja`'s `void_not_error` branch selects
  `.resolves.not.toThrow()` for async calls and `expect(() => call_expr).not.toThrow()` for
  sync calls. Regression coverage:
  `src/e2e/codegen/typescript/test_file/void_not_error_call_tests.rs` (new, table-driven
  sync/async pair) and the corrected sync-shape assertion in
  `test_case.rs::void_not_error_tests::void_not_error_wraps_the_call_without_asserting_tobedefined`.
- **Generated Zig e2e assertions for a wire-optional JSON key did not compile at all, failing
  the entire Zig gate in consumer repos.** 0.62.7 taught `json_get`
  (`src/e2e/codegen/zig/assertions.rs`) to guard a `#[serde(skip_serializing_if = "...")]` key
  with `orelse .null` instead of force-unwrapping it with `.?`, so a missing key renders the same
  as a present JSON `null` instead of panicking. That guard compiles fine as the right-hand side
  of a `const` whose type Zig infers from the optional's own payload, but the bare `.null` enum
  literal has no result-type context the moment the guarded expression is chained straight into
  more field access with no intervening declaration — exactly what happens when the wire-optional
  key sits in the *middle* of a field path (`a.b.c` where `b` is wire-optional) rather than at the
  leaf. `zig` rejected it with "incompatible types: '*const json.dynamic.Value' and '*const
  @EnumLiteral()'", and this shipped uncaught because snapshot tests only assert on emitted text,
  never that it compiles. `json_get` now emits the fully-qualified `std.json.Value{ .null = {} }`
  instead of the bare literal, which removes the ambiguity regardless of where the expression is
  used next. Regression coverage:
  `orelse_null_compile_tests::nested_wire_optional_key_assertion_compiles_under_real_zig`
  (`src/e2e/codegen/zig/orelse_null_compile_tests.rs`) renders the exact nested-key shape and
  compiles it through the same `zig build-exe` toolchain `ZigValidator` uses for doc snippets,
  so a regression here fails a real compile rather than a text comparison.
- **The Rust e2e generator's `contains`/`contains_all`/`contains_any`/`not_contains` assertions on
  a collection field only matched an item's `"name"` key with exact equality, so a fixture like
  `{"type":"contains","field":"structure","value":"Function"}` against items shaped
  `{"kind":"Function","name":"main",...}` failed with `expected collection item name: Function` —
  the match text lived under `kind`, which the predicate never inspected (alef defect B1). The
  other five e2e backends (Python, Node/TypeScript, Ruby, Java, C#) already implement the intended
  semantics: a substring search over several item keys (or the whole serialized item). Rust's
  `containment_predicate` (`src/e2e/codegen/rust/assertions.rs`) now searches `kind`, `name`,
  `source`, `alias`, `text`, and `signature` via substring, falling back to the item's whole JSON
  text, matching the Python/Node/Ruby key list and the Java/C# whole-item fallback. Regression
  coverage: `src/e2e/codegen/rust/assertion_containment_tests.rs`.
- **C# `not_error` asserted presence beside a sibling `is_empty` on a bare `Option<T>` result,
  a contradictory pair that could never pass (alef #165, C# arm).** `csharp/assertions.rs`'s
  `not_error` arm unconditionally emitted `Assert.NotNull(result)`, even when a fixture
  legitimately paired it with `is_empty` on a call whose success path returns nothing (`None`
  -> C# `null`). Mirrors the guard already shipped for typescript/wasm: `render_assertion` now
  takes a `has_other_assertions` flag (`fixture.assertions.len() > 1`, threaded from
  `csharp.rs`) and skips the presence fallback whenever a sibling assertion already gives the
  test real coverage. Regression coverage:
  `csharp/not_error_presence_guard_tests.rs`.
- **C# `is_empty` on a `List<T>` field fell back to a `ToString()`-based emptiness check that
  could never pass.** `field_needs_json_serialize` (`csharp/assertions.rs`) only checked
  `field_resolver.is_array(f)`, missing a collection field whose entries are tracked only via
  their element paths in `fields_array` (e.g. `children[0]` for a recursive `List<DataNode>
  Children`, never a bare `children` entry). Without `is_collection_root`, `csharp/assertion
  .jinja`'s `is_empty` branch fell through to `Assert.True(string.IsNullOrEmpty(field
  .ToString()))`: `List<T>.ToString()` returns the type name, a non-empty string, so the
  assertion could never pass. Now checks `is_array`/`is_collection_root` against both the raw
  and resolved field path, matching kotlin/swift's identical `field_is_collection` guard.
  Regression coverage: `csharp/collection_is_empty_tests.rs`.
- **Java `not_error` also asserted presence beside a sibling `is_empty` on a bare `Option<T>`
  result, the same alef #165 shape audited across every e2e backend while fixing the C# arm
  above.** `assertions.rs`'s `not_error` arm fell through the `result_is_option && bare_field`
  block that already special-cases `is_empty`/`not_empty` for this shape, so it hit the general
  arm's unconditional `assertNotNull(result, ...)`; the block now treats `not_error` as inert
  there too. Regression coverage: `java/not_error_bare_option_tests.rs`.
- **Elixir `not_error` had the identical unguarded-presence defect.** `assertions.rs`'s
  `not_error` arm had no guard of any kind against its own `is_empty` arm's `assert is_nil
  (field_expr) or ...` a few lines above; it now takes the same `has_other_assertions` flag as
  C#/typescript, threaded from `test_case.rs`. Regression coverage:
  `elixir/not_error_sibling_assertion_tests.rs`.
- **Kotlin's `not_error` arm (`kotlin/not_error.rs`) was documented as already immune to this
  same defect class but never actually was.** A comment in the swift fix's doc claimed "Zig and
  Kotlin already treat `not_error` as inert in this shape", but `render_not_error` took no
  `bare_result_is_option`-equivalent parameter and always emitted `assertNotNull(result, ...)`,
  contradicting a sibling `is_empty`/`not_empty` arm's `assertNull`/`assertNotNull` in
  `assertions.rs` (which already computes `bare_result_is_option` for its own use). Threaded the
  same predicate into `render_not_error`, mirroring swift's `bare_result_is_option` guard.
  Regression coverage: `kotlin/not_error.rs`'s `bare_optional_result_emits_no_not_null_assertion`.
- **The downstream generated-output gate never exercised either code path fixed in the
  `clippy::unnecessary_cast` regression (alef commit `c82f8f117`), so a regression of that
  bug would not have been caught by the gate that exists specifically to catch this class of
  bug.** `c82f8f117` added thorough unit coverage for `primitive_cast` (JNI param
  unmarshalling), `emit_return_marshal_with_indent` (JNI return marshalling), and
  `capsule_into_raw_expr` (FFI capsule return), but its own CHANGELOG entry flagged that the
  gate's shared fixture (`tests/generated_output_downstream_gate/fixture.rs`) still had no
  `f64`-typed field or parameter and no `[crates.ffi.capsule_types]` configuration, so neither
  fixed path was ever compiled and linted by a live `cargo clippy -- -D warnings` run over
  real generated output. The fixture now includes `round_trip_cost(cost_usd: f64) -> f64`
  (exercises the JNI f64 param/return cast) and a `Language`/`RawLanguage` capsule pair
  configured via `[crates.ffi.capsule_types.Language]` (exercises the FFI capsule return
  cast). Verified by temporarily reverting `c82f8f117`'s source changes against the new
  fixture: `emitted_tree_passes_clippy` failed with
  `casting to the same type is unnecessary (f64 -> f64)` at
  `core_crate::round_trip_cost(cost_usd as f64)`, and passed again once the fix was restored.
- **`sync-versions` bumped every `Cargo.toml` it owned but never refreshed the sibling
  `Cargo.lock`, so `alef validate versions` — which discovers lockfiles through a separately
  derived, broader enumeration — found the stale pin and failed the release gate (alef #148).**
  Three releases (tree-sitter-language-pack 1.15.3, liter-llm 1.17.2, crawlberg 1.3.1) were
  tagged and pushed with a stale lockfile in directories like `e2e/rust`, `test_apps/rust`,
  `fuzz`, and a Ruby native-extension crate, failed validation, and never reached crates.io.
  `sync_versions` now relocks every `Cargo.lock` immediately after bumping the manifests it
  pins, via a new `cargo update --offline -w` step
  (`src/cli/pipeline/version_lockfiles.rs::relock_cargo_lockfiles`) that skips locks
  `blocked_on_publish` (a registry dependency pinned at the version being released, which cannot
  resolve until that release is live). Both the write side and `alef validate versions` now call
  the same discovery function,
  `src/cli/commands/version_manifests.rs::discover_cargo_locks`, so the write set and the
  validate set can no longer diverge into checking a different set of lockfiles. Regression
  coverage: `sync_versions_relocks_a_nested_lockfile_so_validate_versions_then_passes` and
  `sync_versions_does_not_touch_a_lockfile_blocked_on_the_pending_release`.
- **Snippet session preparation timeouts were reported as opaque validation errors instead of an
  ordering problem (alef #142).** A `docs.snippets.sessions` `before` hook builds a language's
  artifacts (`cargo build --release -p <crate>-jni`, `pnpm run build:all`, ...) before any of its
  snippets can validate; when that hook itself outlived `timeout_secs` — readily hit on a loaded
  machine, or right after `alef all --clean` removed the artifacts it exists to rebuild —
  `activate_session` (`src/snippets/session/mod.rs`) collapsed the resulting `Error::Timeout` into
  the same generic `Error::Other` every other preparation failure produces, and both
  `runner::validate_one`'s fail-fast path and `batch::group_batchable_snippets`'s parallel path
  independently stamped every snippet in that session `SnippetStatus::Error` from the bare
  stringified message — indistinguishable from a genuinely broken snippet or a misconfigured
  session. `before`-hook timeouts now propagate as `Error::Timeout` intact, and
  `SessionPreparationError::ordering` reclassifies them through the same `unresolved_dependency` /
  `SnippetStatus::Unavailable` bucket a validator's own dependency-shaped `Fail` already uses, with
  a message that names the ordering problem instead of reading as a bare timeout. Every other
  preparation failure (missing manifest, missing directory, a `before` hook that ran to completion
  and failed on its own terms) keeps `SnippetStatus::Error`. New
  `src/snippets/runner/session_prep.rs` unifies the two previously-independent classification call
  sites. Regression coverage: `src/snippets/session/preparation_error_tests.rs`,
  `src/snippets/runner/session_preparation_classification_tests.rs`.
- **`packages/dart/rust/build.rs` raced alef's own FRB regeneration and corrupted committed
  Dart bindings (alef #140).** alef's post-build `RunCommand` step and the generated `build.rs`
  both invoke `flutter_rust_bridge_codegen generate` — the former is the canonical path
  (`alef generate`/`alef build`, with alef's full post-processing pipeline), the latter fired
  unconditionally on every consumer `cargo build`/`cargo test`/`cargo clippy`. `build.rs`'s
  embedded post-processing only ever replicated 3 of alef's ~8 `PostProcessFile` steps
  (`fix_handler_executor_calls`, `carry_frb_cfg_gates`, `patch_published_loader`), silently
  omitting the native-library-loader package-import rewrite/`dart:core` aliasing and
  injected-text-method steps bundled into `PostProcessor::FrbDartSealedVariants`/
  `FrbDartInjectTextMethods` — so a plain `cargo build` after `alef generate` flipped
  already-correct committed output back to a different, partially processed form. `build.rs`
  now only invokes `flutter_rust_bridge_codegen` behind an explicit opt-in
  (`ALEF_FRB_REGENERATE_ON_BUILD=1`) for local Flutter-only iteration; by default it leaves the
  committed, alef-processed bridge untouched
  (`src/backends/dart/templates/rust_build_rs.rs.jinja`).

- **alef's own bare `flutter_rust_bridge_codegen` invocation could silently degrade to the
  same broken output as `build.rs` (alef #140, second cause).** `flutter_rust_bridge_codegen`
  treats the presence of `CARGO_MANIFEST_DIR` in its environment as proof it is nested inside
  an already-running `cargo` process (Cargo sets that variable only for processes it spawns
  itself) and, to avoid deadlocking on that process's jobserver, silently skips its
  `cargo-expand` macro/cfg-expansion pass, falling back to a raw `syn` parse that emits a
  binding for every `pub fn` regardless of whether its `#[cfg(feature = ...)]` gate is active
  for the crate — this is what let a consumer's committed bridge keep missing feature-gated
  functions across repeated `alef generate` runs. Confirmed by bisecting a real build script's
  captured environment down to this one variable: with it present, a bare, non-nested
  invocation reproduces the degraded output; without it, the same invocation runs a real
  `cargo-expand` and correctly includes/excludes feature-gated functions. alef's post-build
  `RunCommand` step now strips `CARGO_MANIFEST_DIR` from the environment it spawns
  `flutter_rust_bridge_codegen` in, so alef's own invocation always takes the full, cfg-aware
  path regardless of how alef itself was launched (`src/cli/pipeline/commands/build/frb_cache.rs`).
- **A non-empty `Vec<String>` default (`vec!["noscript"]`) failed to compile in every
  Rust-emitting binding backend (alef #156).** `config_scalar_default`
  (`src/codegen/config_gen/shared.rs`) rendered a `DefaultValue::StringLiteral` list element the
  same way for every language — a bare `"noscript"` — which is a `&'static str` and does not
  coerce to the `Vec<String>` the source field actually has, producing `E0308: mismatched types`.
  Every caller that renders real Rust source (`default_value_for_field_in_type(field, "rust",
  typ)`) hit this identically: Magnus/Ruby, Rustler/Elixir, NAPI/Node, and PHP. It killed every
  `Build Ruby gem` and `Build Elixir NIF` leg of a real release. `config_scalar_default` now emits
  `"noscript".to_string()` for `"rust"` (every other language's list literal already accepts a
  bare element and is unchanged). `Option<String>` and `Vec<Vec<String>>` were checked and are
  not affected: `Option<String>` literal defaults already went through a different, already-correct
  branch, and nested list defaults already fall back to the safe empty collection rather than a
  partial literal. `HashMap<String, String>` has no non-empty literal default path at all yet
  (only `Empty`/`Unresolved`), so there was nothing to fix there.
- **`sync-versions` could leave `packages/go/cmd/setup/main.go`'s `versionIdent` const
  stale after a release, referencing a `RequireNativeSetup_<ident>` symbol that no longer
  existed in `native_setup.go` and failing the Go build (alef #159, html-to-markdown

  #463).** `cmd/setup/main.go` carries two version-derived consts: `moduleVersion` (a plain
  semver string) and `versionIdent` (the sanitized Go identifier `renderShim` bakes into
  its `RequireNativeSetup_<ident>` runtime reference). `sync_versions` (`src/cli/pipeline/
  version.rs`) patched `moduleVersion` on every run but never touched `versionIdent`, while
  `native_setup.go`'s `RequireNativeSetup_<ident>` sentinel was independently re-derived
  from the version via its own `to_go_version_ident` call — two computations of the "same"
  value that silently diverged across sync-versions-only releases. `versionIdent` and the
  sentinel identifier are now both derived from a single `to_go_version_ident` call made
  once per `sync_versions` run and threaded into both files via the new
  `sync_go_cmd_setup_version_ident` (paired with the updated `sync_go_native_setup_sentinel`
  signature), so a version bump can no longer move one file's identifier without moving the
  other's. Regression coverage: `sync_versions_updates_go_module_version_in_cmd_setup`
  (`src/cli/pipeline/version_tests/e2e_manifests.rs`), which cross-checks the identifiers
  extracted from both files against each other and against an independently computed
  `to_go_version_ident`, not just two hardcoded literals.
- **A fieldless-only enum return broke the C# build with CS1503 (`ulong` to `nint`), same family
  as the JNI/FFI cast bugs fixed in 0.62.7 (alef #155).** `liter-llm` v1.17.3's `RefreshCatalog`
  free function returns `RefreshOutcome`, a fieldless-only enum (`Disabled`, `FromCache`,
  `Fetched` — no variant carries data). The FFI crate's `gen_owned_value_to_c`
  (`backends::ffi::gen_bindings::helpers`) has no enum-ness branch, and no
  fieldless-vs-data-carrying branch either, for owned return conversion: *every* enum return boxes
  as `AlefHandle` via `insert_handle`, exactly like a struct, and its FFI header unconditionally
  exports `{prefix}_{enum}_to_json`/`{prefix}_{enum}_free`. Two C# backend predicates disagreed
  with that reality by filtering enums to "has at least one data-carrying variant":
  `marshalling::enum_names_with_data_variants` (which decides which returns `errors.rs`'s
  `emit_return_marshalling_indented` routes through the `{Pascal}ToJson`/`{Pascal}Free` round
  trip) and `functions::ffi_handle_type_names` (which decides which `{Pascal}ToJson`/`{Pascal}Free`
  P/Invoke declarations `gen_native_methods` emits at all). For a fieldless-only enum return, both
  predicates excluded it, so the generated wrapper passed the `ulong`-declared `nativeResult`
  handle straight to `Marshal.PtrToStringUTF8`/`NativeMethods.FreeString` (both `nint`-typed) —
  the CS1503 defect that broke `Build C# NuGet package` and skipped `Publish NuGet` entirely for
  that release. Both predicates now include every enum, fieldless or data-carrying alike.
  (`src/backends/csharp/gen_bindings/marshalling.rs`, `src/backends/csharp/gen_bindings/functions.rs`)
- **The JNI manifest declared a capsule-backing crate (e.g. `tree-sitter`) as a direct dependency
  it never actually used, so `cargo machete` correctly stripped it and fought every subsequent
  `alef generate` (alef #145).** `scaffold_jni` added the capsule type's `package` (from
  `[crates.ffi.capsule_types]`, intersected with `[crates.kotlin_android.capsule_types]`) to
  `[dependencies]` on the premise that the JNI shim emitted an
  `value.into_raw() as *const {into_raw_type}` cast referencing it. That cast was already removed
  from `method_capsule_return.rs.jinja` as a same-type cast tripping `clippy::unnecessary_cast`
  (see `capsule_returns_transfer_the_pointer_without_a_redundant_cast`); the JNI shim now
  transfers a capsule return through `.into_raw()` type inference alone and never spells the
  capsule crate's path, so the dependency the manifest declared was genuinely unused. `scaffold_jni`
  (`src/scaffold/languages/jni.rs`) no longer adds a capsule package dependency to the JNI
  manifest. Regression coverage: `scaffold_jni_never_declares_a_capsule_package_dependency`.
- **`tests/e2e_equals_assertion_exact_no_trim.rs` audited the wrong side of the html-to-markdown
  Rust e2e trailing-newline reports (alef #162).** 12+ generated Rust `equals` assertions were
  failing in CI (`test_conversion_autolink_https_url` and others) because the library's actual
  output carries a trailing `\n` that the fixture's `expected` literal lacks. Comparing actual CI
  runs for the *same* commit across languages (Python, Ruby) showed the identical mismatch failing
  there too — `equals` assertions are rendered as an exact, unnormalized literal comparison in
  every backend, Rust included (`render_equals_assertion`,
  `src/e2e/codegen/rust/assertion_helpers.rs`), so no backend "passes for the wrong reason" here.
  The root cause is stale `expected` values in the consumer repo's fixture JSON, not an alef
  codegen divergence; no alef backend needed a behavior change. `rust` was, however, missing from
  the cross-language `equals`-assertion-exactness regression table in
  `tests/e2e_equals_assertion_exact_no_trim.rs` (14 of the other 15 consumed backends were
  covered) — added so a future one-sided-trim regression in the Rust path is caught here instead
  of only by its own per-backend unit tests.
- **`not_error` combined with `is_empty` emitted a presence assertion for a fixture whose
  contract is absence (alef #165).** `render_assertion`'s `not_error` arm (shared by the
  `node`/`typescript` and `wasm` e2e generators) unconditionally emitted
  `expect(result).toBeDefined();` as a stand-in for "the call succeeded", even when a sibling
  `is_empty` assertion declared the same call's success path can legitimately return nothing —
  e.g. detecting a language from empty content, which maps Rust's `Option::None` to JS. WASM
  (`wasm-bindgen`) maps `None` to `undefined`, so the fallback contradicted the fixture and failed
  every run; NAPI's `None` -> `null` mapping only passed by accident, since `null !== undefined`
  for `toBeDefined()`. `not_error` now yields to any sibling assertion instead of asserting
  presence, so both backends derive the same, correct behavior from one code path. Regression
  coverage: `not_error_paired_with_is_empty_does_not_assert_presence`
  (`src/e2e/codegen/typescript/assertions.rs`).

## [0.62.8] - 2026-08-21

### Fixed

- **Dart FRB post-build could patch a stale bridge and silently drop newly added functions
  (alef #135).** `PostBuildStep::RunCommand`'s `flutter_rust_bridge_codegen` invocation treats a
  missing tool or `ALEF_SKIP_COMMANDS` as a non-fatal skip (`run_run_command`,
  `src/cli/pipeline/commands/build.rs:995`), and every `PostProcessFile` step that follows it in
  Dart's post-build sequence (`src/backends/dart/gen_bindings/mod.rs`) patched whatever bridge
  `lib.dart` was already on disk regardless of whether frb actually regenerated it that run. When
  the FRB facade gained new `pub fn`s while `flutter_rust_bridge_codegen` was unavailable, the
  bridge stayed stale for those functions while still receiving alef's other patches (e.g.
  extension injection) — an internally inconsistent, silently "successful" build. A new
  `PostBuildStep::VerifyFrbBridgeCoverage` step now runs immediately after the `RunCommand` step
  and before any `PostProcessFile` rewrite: it compares every free function declared in the
  facade against the bridge and fails the build loudly, naming the missing functions, instead of
  letting later steps patch a bridge that disagrees with the facade. New
  `src/backends/dart/frb_rewrite/bridge_coverage.rs` (`missing_bridge_functions`) and
  `src/cli/pipeline/commands/build/frb_bridge_coverage.rs` (`verify`).

- **A void `not_error` fixture over a synchronous call emitted `await` inside a non-async arrow
  function**, which is a TS/JS syntax error rather than a weak test. `render_test_case` froze
  `async_kw` from `test_is_async` — which accounted for `call_is_async`, byte-file reads and trait
  bridges — roughly two hundred lines before `void_not_error` was computed, and never folded the
  latter back in. `typescript/test_function.jinja` then emitted
  `await expect(..).resolves.not.toThrow()` into that non-async callback. Because `alef all`
  formats every language in one phase, `oxfmt` rejecting the generated file aborted formatting for
  *all* languages, leaving the whole tree unformatted and unstamped; the failure presented as a
  repo-wide formatting error rather than as one bad test. `void_not_error` is now computed before
  `async_kw` and folded into `test_is_async`. Affects the `wasm` backend too, which shares the
  template. Regression coverage: `sync_void_not_error_marks_the_test_callback_async`.

## [0.62.7] - 2026-08-21

### Added

- **The IR now tracks `#[serde(skip_serializing_if = "...")]` as a fact distinct from
  `Option<T>`-optionality.** A new `FieldDef::serde_skip_serializing_if` flag (threaded through
  `src/extract/extractor/helpers/attributes.rs::extract_serde_skip_serializing_if` and
  `src/extract/extractor/helpers/fields.rs`) records that a field's JSON key may be entirely
  absent from the wire format — not `null`, absent — even when the underlying Rust field is a
  required, non-`Option` type (e.g. `Vec<T>` with `skip_serializing_if = "Vec::is_empty"`). This
  fixes a Zig e2e crash: `FieldResolver::ir_wire_optional_fields` and the new
  `FieldResolver::is_wire_optional_key` predicate let the Zig e2e generator's JSON-tree accessor
  chain (`src/e2e/codegen/zig/assertions.rs`) guard a `.object.get(key)` lookup with
  `orelse .null` instead of `.?`, so a fixture like tslp's `data_extraction_json_empty_object`
  (asserting `is_empty` on a `Vec<DataNode>` field serde omitted because it was empty) no longer
  panics with "attempt to use null value". Other JSON-consuming backends (C FFI, and every other
  language binding) were audited and found not exposed: they access the real typed value through
  the binding rather than re-parsing a generic JSON blob, so a `Vec<T>` field is always present
  (possibly empty) regardless of `skip_serializing_if`. `optional_fields` (the existing
  `Option<T>`-driven set) is left untouched, since conflating the two would make e.g. the Rust
  e2e backend emit `.as_ref().unwrap()` against a plain, always-present `Vec<T>`.

### Fixed

- **The JNI and FFI backends no longer emit `as <T>` casts on expressions that are already
  type `T`, which tripped `clippy::unnecessary_cast` under `-D warnings` in any consumer that
  lints generated code.** Two independent sightings shared one mechanism: a cast target was
  chosen without checking whether the source expression already had that type.
  - **JNI** (`src/backends/jni/gen_shims/type_helpers.rs`): `primitive_cast` used a hand-picked
    per-`PrimitiveType` table of cast targets that assumed every primitive needs converting to
    its own JNI wire type, which is false whenever the wire type already IS that Rust type (e.g.
    `F64` against `jni::sys::jdouble`, itself a type alias for `f64`) — so a call like
    `record_cost_usd(..., cost_usd as f64)` cast an already-`f64` value. The same defect existed
    independently on the return path (`emit_return_marshal_with_indent` in
    `src/backends/jni/gen_shims/marshalling.rs`, `src/backends/jni/templates/return_primitive.rs.jinja`),
    for the same reason. Both directions now consult a single `jni_wire_repr` table pairing each
    primitive's JNI wire type with the Rust type that wire type is an alias for, and only emit a
    cast when the two differ. `I8`, `I16`, `I32`, `I64`, `F32`, and `F64` (whose wire
    representations are aliases of their own Rust type) no longer get a redundant cast in either
    direction; `U8`/`U16`/`U32`/`U64`/`Usize`/`Isize` (whose wire types are signed/widened) still
    do.
  - **FFI** (`src/backends/ffi/gen_bindings/capsule.rs`): `capsule_into_raw_expr` unconditionally
    appended `as *const {into_raw_type}` to a capsule-typed function's `value.into_raw()` call,
    even though `into_raw_type` is documented as the pointee type `value.into_raw()` already
    returns and the exported function's return type is declared as exactly the same
    `*const {into_raw_type}` — so the cast's source and destination were the same type by
    construction. Confirmed against tree-sitter's own
    `Language::into_raw(self) -> *const ffi::TSLanguage`. The cast is no longer emitted;
    `capsule_into_raw_expr` now just calls `value.into_raw()`.
  - `tests/generated_output_downstream_gate.rs`'s self-check previously skipped to whichever
    emitted Rust file allowed `clippy::unnecessary_cast` at crate level, which happened to
    always be the FFI crate's `lib.rs` — so the self-check "passed" without ever running clippy
    against a lintable file. That gap was already closed on `main` (the crate-level allow was
    dropped and the self-check now hard-fails instead of silently picking an allow-listed file).
    However, the gate's shared fixture (`tests/generated_output_downstream_gate/fixture.rs`)
    still has no `f64`-typed field or parameter and no `[crates.ffi.capsule_types]`
    configuration, so neither of these two code paths is ever exercised by a live gate run — a
    regression of either bug would still not be caught today. Flagged here rather than fixed:
    the fixture drives every clippy-lane language and changing it needs validation against the
    full (heavy, `#[ignore]`d, multi-toolchain) gate suite this session could not run.

- **`sync-versions` now bumps the `[package] version` field itself in every alef-owned Cargo.toml
  it touches, not just dependency pins.** Two manifests kept their pre-bump version after
  `alef sync-versions --bump`: the Ruby native (Magnus) crate manifest at
  `packages/ruby/ext/<gem>_rb/native/Cargo.toml` only ever ran `patch_workspace_dep_versions`
  (which walks dependency tables and never touches the file's own `[package].version`), and the
  e2e Rust harness manifest at `<e2e.output>/rust/Cargo.toml` (default `"e2e"`) was skipped
  entirely because `sync_rust_test_app_version` only checked `e2e.registry.output` (default
  `"test_apps"`) — a different directory used only by registry-mode e2e. Both manifests now get
  a direct `write_version_to_cargo_toml` call against their own `[package]`/`[workspace.package]`
  version key, which — being scoped to that key via `toml_edit` — cannot touch a dependency pin
  whose version string coincidentally matches the old project version. Separately, the catch-all
  rewrite guard's "skipping a stampable file that carries no alef marker" warning is now logged
  at `debug` instead of `warn` when the refused file has no semver-shaped substring at all (e.g.
  a workspace member using `version.workspace = true`) — refusing it changed nothing on disk
  either way, so it was never the alarming case the warning is for.

- **The publish workflow's GitHub-release guard now checks the release's *assets*, not the
  release object.** `check-github-release` passed `asset-prefix: alef-`, which the homebrew
  bottles (`alef-<version>.<bottle_tag>.bottle.tar.gz`) also match — so the v0.62.6 re-run, on a
  release that carried two bottles and no CLI archive, read "already published" and skipped both
  `build-cli` and `upload-release-assets`, leaving `cargo binstall alef` and the direct-download
  path broken on the published version. The guard now demands the exact archive set, and both
  that set and `build-cli`'s matrix are derived from a single new source of truth,
  `.github/cli-targets.json`, so the demanded assets cannot drift from the built ones.
  `tests/publish_workflow_cli_asset_guard.rs` extracts and executes the workflow's own steps to
  hold that invariant.

- **WASM `Cargo.toml` honors `[wasm].core_crate_override` again when computing the core
  dependency's `path = "..."`.** Routing that path through `core_crate_dep_path` (shared with
  every other backend) fixed the general `crates/`-vs-root-flat depth bug, but the shared helper
  derives the path from `sources` via `core_crate_root()` — which is the wrong crate entirely
  once an override names a different, unrelated sibling crate `sources` never describes. In a
  root-flat project this silently emitted `path = "../.."` (the umbrella crate) instead of
  `path = "../<override>"`, so `alef build` produced a wasm manifest depending on the wrong Rust
  crate — a config that compiles clean until the override crate's exclusive types are referenced
  and cargo can't resolve them, or a stale duplicate compiles instead. New
  `ResolvedCrateConfig::core_crate_dep_path_for_language` resolves the override to a `crates/`
  sibling of the binding crate before falling back to `core_crate_dep_path`; `wasm`'s
  `gen_cargo_toml` now calls it. Dart and Swift, which also honor `core_crate_override`, compute
  their manifest paths independently and were not affected. JNI has no `core_crate_override`
  concept, so its use of the shared (override-blind) helper was already correct.

- **`alef all` now formats every language it actually wrote this run, not only the ones that
  registered a bindings/service-API/stub change.** The whole-tree `poly fmt` convergence pass in
  `src/bin_cli/all_commands.rs` was gated on `changed_languages`, a set populated only from the
  bindings, service-API, and stub write phases. Scaffold, public API, e2e/test-apps, README, and
  docs output could all be freshly written to disk without the gate ever noticing, and a
  post-build step (e.g. Dart's `flutter_rust_bridge_codegen`, which reruns unconditionally every
  pass and writes straight to disk with no write-report at all) was invisible to every signal
  built from write reports. A run that only changed one of those phases — observed as README
  regeneration alone in a real project — left its output permanently unformatted, since the same
  narrow gate applied on every subsequent run too. The gate is now driven by `any_output_changed`,
  set from every write phase's `changed_count()` plus a new `languages_have_post_build_steps`
  check (`src/bin_cli/helpers/post_build.rs`) that treats a configured post-build step as "output
  may have changed" by construction. `format_generated`'s own `None` (full-regen) branch had the
  identical shape one level down — it early-returned when the `bindings`/`stubs` file list handed
  to it was empty, independent of whether the caller had already decided formatting was needed
  from the fuller write set — so that early return now only applies to the partial-regen
  (`Some(_)`) branch, where the language list is actually load-bearing.

- **`alef fmt` no longer disagrees with `alef all` about the canonical formatting of the same
  file.** `pipeline::fmt` (`src/cli/pipeline/commands/lint.rs`) ran a bespoke single `poly fmt
  --fix` pass plus the old per-language `cargo sort` residual list — a second, independently
  maintained formatting implementation that never invoked `mix format` at all (leaving every
  `alef fmt`-only run's Elixir output completely unformatted) and never looped `poly fmt --fix`
  to a fixed point, unlike `alef all`'s `converge_full_regen_formatting`. That single-pass gap is
  what let `alef fmt` rewrite a consumer's `packages/dart/.../frb_generated.dart` to an incorrect
  intermediate form (relative imports, two dropped `dart:core` imports) that a following `alef
  all` then reverted. `fmt` and `fmt_post_generate` now delegate to the exact same
  `converge_full_regen_formatting` function `alef all` uses, so the two commands cannot produce
  different output for the same tree. The now-unused single-pass `poly_fmt` and
  `run_cargo_sort_residuals` helpers (and their `cargo_sort_residuals` selector) were removed.

- **Generated Java no longer carries a dead `import java.util.List;`.** `ffi_imports.jinja`'s
  import gate decides whether to emit the import by substring-matching `body.contains("List<")`,
  which fires identically on a genuine bare `List<...>` and on an already-qualified
  `java.util.List<...>`. The visitor-cleanup failure aggregator spelled its own declaration
  fully qualified, so a main class whose only function is visitor-bridged tripped the gate on
  text that never uses the import — and checkstyle's `UnusedImports` flagged it, failing 283
  snippet checks in a consumer. The three affected templates now declare `cleanupFailures` as a
  bare `List<Throwable>`, matching the convention `opaque_handle_header.jinja` already used, so
  the gate and the body agree.

- **A generated Kotlin test file no longer imports `assertNotNull` when nothing spells it.**
  The import was written unconditionally, but only `not_error`'s non-streaming branch emits that
  identifier — the streaming branch asserts on the drained `chunks` list via `assertTrue`. Files
  with no `not_error` fixture, or only streaming ones, got an import they never used.

- **A docs-stage failure no longer blames the ownership guard for unrelated refusals.** The error
  context fired whenever *any* write anywhere in the run was refused, so a refused scaffold or
  README write attached an "ownership guard" excuse to a validation failure it had nothing to do
  with — wrong attribution that sent investigators chasing the guard for a plain checkstyle
  defect. It now fires only on refusals inside the crate's `docs.snippets` roots, matching the
  `Ok` arm.

- **A `not_error` assertion on a call that returns void no longer renders a test body that
  asserts nothing.** This is the void half of the vacuous-`not_error` defect whose non-void half
  was closed in `dc8f5ff75`: with no result value to assert on, nine backends either rendered an
  empty body, a comment, or — worse — an assertion that could never pass. A void `not_error` now
  asserts what it actually means, that the call does not fail, using each framework's own idiom:
  `XCTAssertNoThrow` / do-catch-`XCTFail` (Swift, sync/async), `assertDoesNotThrow` (Java, JUnit
  5), `Record.Exception` / `Record.ExceptionAsync` + `Assert.Null` (C#, since xUnit has no
  `Assert.DoesNotThrow`), `expectLater(…, completes)` (Dart), `expect(…).resolves.not.toThrow()`
  (TypeScript, and WASM through the same renderer), and `expect_no_error(…)` (R).

  Three backends were emitting an assertion that *fails on every successful void call*, because
  each binding maps Rust `()` to that language's null: `assertNotNull($result)` in PHP, `assert
  result is not None` in Python, and `refute is_nil(result)` in Elixir. Those are replaced by a
  check the success path can pass — PHP asserts the call did not throw, Python emits the bare
  (already non-vacuous, since an uncaught exception fails a pytest test) call statement, and
  Elixir relies on the `{:ok, result} = call(…)` match it already emits, underscore-prefixing the
  now-unused binding so `mix compile --warnings-as-errors` stays green.

  Go, Gleam, Rust, Zig, and Ruby were audited and left unchanged: each already emits a real,
  visible check at the call site for void calls — `if err != nil { t.Fatalf(…) }`, `should.be_ok()`
  plus `let assert Ok(…)`, `.expect("call failed")`, Zig's `try`, and `expect { … }.not_to
  raise_error` respectively — so a wrapper there would be redundant, not missing.

- **R's non-void `not_error` no longer emits `expect_true(TRUE)`**, an expectation that can
  never fail. The obvious fix, `expect_true(!is.null(result))`, is itself unsafe for a
  `result_is_simple` extendr scalar return or a bare `result_is_option` (`Option<T>`) result:
  `Result<Option<T>, E>::Ok(None)` is a successful call whose result is legitimately R
  `NULL`/`NA`, and asserting non-null there would fail correct behaviour (the same trap Swift's
  `bare_result_is_option` documents). For those two shapes the real, failable check now moves to
  the call site instead: `r/test_case.rs` wraps the fallible call itself in testthat's
  `expect_no_error(...)`, verified to both propagate the call's return value on success and to
  fail the test when the call raises. Every other result shape gets a real
  `expect_true(!is.null(result))` check. New `r/not_error_assertion.rs` holds the shape decision
  and its regression coverage, split out of the already-oversized `r/assertions.rs`.

- **`MaterializeSwiftBridge`'s ownership manifest no longer claims swift-bridge files it never
  wrote.** `PostBuildStep::owned_paths` predicted the full `SwiftBridgeCore.swift` /
  `{binding_crate_name}.swift` / `RustBridgeC.h` trio unconditionally, but the step's actual
  write (`emit_swift_bridge_files`) only produces that trio once it finds a real swift-bridge
  build output directory (or a header already carrying its marker from an earlier real build) —
  before that, it writes the placeholder header alone. On a project's first successful
  generation, before any real `cargo build` output exists, the ownership manifest ended up
  naming two paths that were never written, which `alef verify` and the orphan sweep could
  never find on disk. `owned_paths` now filters its prediction to paths that actually exist,
  which every caller only inspects after the post-build step already ran, so a real trio
  already on disk from an earlier run still stays protected from the orphan sweep.

- **Doc-comment rationale in three e2e modules named real downstream consumer projects
  (`html-to-markdown`, `liter-llm`, `crawlberg`) in violation of alef's project-agnostic-codegen
  rule.** `src/e2e/format_tests.rs`, `src/e2e/codegen/kotlin/not_error.rs`, and
  `src/e2e/field_access/resolver/construct.rs` genericized the offending mentions to
  "a downstream consumer" while preserving the rationale each comment records.

### Changed

- The `Publish` workflow no longer gates on a green `CI` run for the released commit. The gate
  polled `ci.yml` for up to 60 minutes and refused to publish on any non-success conclusion,
  which meant a single unrelated lint or test failure on the release commit blocked the release
  outright with no way through short of re-cutting the tag. Publication is now guarded only by
  the per-registry existence checks. `CI` still runs on `main` and on pull requests; it simply
  no longer holds the release hostage.

## [0.62.6] - 2026-08-20

### Added

- **The WASM backend gives `#[serde(untagged)]` data enums a real structural TypeScript type**
  instead of `any`. A field of that type still round-trips through `JsValue` via
  `serde_wasm_bindgen` exactly as before (unchanged Rust-side bridging), but its getter/setter
  now returns/accepts a wasm-bindgen extern type carrying a hand-written `typescript_type`
  (e.g. `content: string | ContentPart[]` instead of `content: any`). The new
  `src/backends/wasm/gen_bindings/ts_union.rs` module recursively maps each variant's payload —
  primitives, `String`, `Vec`, `Option`, string-keyed `Map`, newtype and struct variants, named
  structs (emitted as a TS `interface`), and named fieldless enums (emitted as a string-literal
  union, since the enum's own wasm-bindgen `enum` uses an incompatible numeric ABI
  representation) — with a per-variant fallback to `any` for opaque/excluded/unresolvable
  payloads only, never the whole union. Every untagged enum in a crate shares one combined
  `typescript_custom_section` so a struct or fieldless enum reachable from more than one union is
  declared exactly once.

### Changed

- **`src/e2e/mod.rs` and `src/bin_cli/helpers.rs` shed their newest additions into
  `e2e::inert_report` and `bin_cli::helpers::post_build`.** Both files were at or over this
  repo's 1,000-line cap; the split is behavior-preserving (no logic changed, only moved) and
  keeps the warning-naming and post-build-isolation fixes above from growing an
  already-over-limit file further.

- **The emitted FFI crate's crate-level `#![allow(...)]` list is narrower.** Every allow was
  audited by removing it, regenerating an emitted tree, and running
  `cargo clippy --all-targets -- -D warnings` over it (see `tests/generated_output_downstream_gate.rs`
  for the harness). Three entries never fired and are gone: `missing_docs` (allow-by-default under
  rustc; `-D warnings` never escalates it, so the crate-level allow was a no-op), and
  `clippy::too_many_arguments` (already covered by a per-item `#[allow]` the emitter attaches at
  every free function, len companion, method wrapper, constructor, and field accessor that can
  exceed the threshold). `clippy::useless_conversion` is gone from the crate-level list too, but not
  because it was dead — `bytes_result_match.jinja`'s `Vec::<u8>::from(..)` conversion (kept
  polymorphic on purpose so the same code works for `Vec<u8>` and `bytes::Bytes` return types) is a
  real no-op specifically for the `Vec<u8>` case, so its four call sites now each carry their own
  `#[allow(clippy::useless_conversion)]` instead of hiding behind a crate-wide one. The remaining
  entries (`dead_code`, `unused_imports`, `unused_mut`, `noop_method_call`,
  `unsafe_op_in_unsafe_fn`, `unsafe_attr_outside_unsafe`, and the rest of the `clippy::` list) either
  fired against a real emitted tree during the audit or could not be proven dead, so they stay.

- **`clippy::unnecessary_cast` is also gone from the FFI crate's crate-level allow list.** The
  prior audit above could not reach the one template that emits an `as i32` cast on a field whose
  Rust type is a genuine enum (`ffi_visitor_context_enum_init.jinja`, used only when a trait
  bridge's visitor context struct has an enum-typed field) because doing so needs a configured
  trait bridge, which the audit's fixture did not have. Reached this time via a real trait-bridge
  fixture: `context_c_type` (`gen_visitor/context.rs`) only routes a field to this template when
  it resolves to a real IR enum, never when the field is already `i32` (that case emits through
  the cast-free passthrough template instead), so the cast's source is never nominally the same
  type as its `i32` target and `clippy::unnecessary_cast` — which fires only on same-type casts —
  can never flag it. Verified with a real `cargo clippy --all-targets -- -D warnings` run, both
  over the non-visitor gate fixture and, for the enum-context site specifically, over a real
  `alef generate` run with a trait bridge configured.

### Fixed

- **The generated Java `pom.xml` now excludes test, scratch, and build directories from the
  compiler plugin.** `sourceDirectory` is `project.basedir`, so without excludes the compiler
  also walks `src/test/java/**` (hand-written test sources), `.alef/**` (disposable doc-snippet
  validation scratch), and `target/**` (its own prior output). Unexcluded, `.alef/`
  snippet-scratch `Example.java` files from different sessions collide as duplicate top-level
  classes and break `mvn compile`. The same template already excluded `**/.alef/**` for
  checkstyle; the compiler block never got the equivalent, so consumers carrying the fix by hand
  could not adopt the file without regeneration deleting it.

- **`alef verify`'s orphan check now accounts for post-build-owned paths.** `alef verify` never
  runs post-build steps (`complete_generated_artifacts` is `Commands::Generate`/`Commands::All`
  -only), so a path a configured `PostBuildStep` writes unguarded (`PostBuildStep::owned_paths`)
  could never appear in the in-memory managed surface `find_missing_and_frozen_generated_files`
  builds from `collect_managed_surface`. Any such step that plants an alef-marked file at a path
  the in-band generator does not itself produce would have been reported as an orphan on every
  single `alef verify` run. No shipped backend does this today (Swift's `MaterializeSwiftBridge`
  is the only non-empty `owned_paths` today, and the trio it writes never carries alef's marker),
  so the gap was latent rather than live, but `find_missing_and_frozen_generated_files` now folds
  `verify_orphans::post_build_owned_paths`'s union into `managed_paths` regardless, the same way
  `Commands::Generate`'s own orphan sweep already does before its disk-scan diff.

- **Swift `alef generate` no longer fails to compile on an enum with data-carrying variants.**
  `gen_rust_crate::enums::emit_enum_wrapper` unconditionally emitted a
  `__alef_{enum}_from_swift_string` reconstruction helper for every enum, with a bare
  `EnumName::Variant` match arm per variant -- valid for a unit variant, but E0308/E0533 against
  a tuple or struct variant, since a wire string carries only a variant's discriminant and never
  its field data. No call site ever invoked this helper for such an enum in the first place
  (`unit_enum_names`-gated call sites route tagged-enum parameters through
  `serde_json::from_str` instead), so the helper was both broken and unused for this shape. The
  fix stops emitting it entirely when any variant carries fields, rather than patching the arms
  to compile and panic at runtime. `tests/generated_output_downstream_gate.rs`'s fixture now
  includes a data-carrying enum (tuple and struct variants) so this shape has default coverage.

- **Swift e2e `not_error` assertions no longer contradict a paired `is_empty`/`is_true` check on
  a bare `Optional<T>` result.** `render_assertion`'s `"not_error"` arm always emitted
  `XCTAssertNotNil(result)`, even when the fixture's result is itself `Optional<T>` and `nil` is
  a valid non-error outcome (e.g. `detectLanguageFromContent("")` returning `nil`). A fixture
  combining `not_error` with `is_empty` on the bare result therefore generated both
  `XCTAssertNotNil(result)` and `XCTAssertNil(result)` back to back -- an assertion pair that can
  never pass. Zig and Kotlin already treated `not_error` as inert for this shape; Swift's arm is
  now split into `src/e2e/codegen/swift/not_error_assertion.rs` (new module, keeping
  `swift/assertions.rs` from growing past the file-size cap) with the same
  `bare_result_is_option` guard.

- **kotlin_android's local-mode e2e `build.gradle.kts` now declares a `testImplementation`
  for every `[crates.kotlin_android.capsule_types]` host package.** Local mode compiles
  `packages/kotlin-android`'s wrapper sources directly into the e2e project's own `test`
  sourceSet, and those sources import the capsule's `host_type` (e.g.
  `io.github.treesitter.ktreesitter.Language`) whenever a function or method returns it --
  but `render_build_gradle_kotlin_android` had no capsule awareness at all, so every
  local-mode e2e build failed to compile with `Unresolved reference 'github'` (or whatever
  the host package's top segment is) before a single test ran.

- **kotlin_android's and plain kotlin's local-mode e2e `build.gradle.kts` now pin
  jackson-annotations to its own version scheme instead of `JACKSON_E2E`'s.**
  jackson-annotations stopped publishing patch-version releases after 2.19.x (`2.20`,
  `2.21`, `2.22`, ... with no third component), unlike jackson-databind /
  jackson-datatype-jdk8 / jackson-module-kotlin, which still publish `major.minor.patch`.
  Reusing `JACKSON_E2E`'s value for jackson-annotations resolved a coordinate (e.g.
  `2.22.2`) that was never published on Maven Central, so Gradle failed to resolve the test
  classpath before Kotlin compilation ever started. `scaffold::languages::kotlin` (the real
  package `build.gradle.kts` generator) already used the dedicated `JACKSON_ANNOTATIONS`
  constant; these e2e generators had their own copy of the dependency list and never picked
  it up.

- **The C# and kotlin_android e2e generators' enum-field `equals`/`contains` assertions now
  compare against the enum's real serialized wire value instead of guessing a snake_case
  transform.** Both derived the expected/actual pair from a naming-policy heuristic that
  assumed every enum's wire value is the lowercased, underscore-separated form of its host
  constant name (C#: `JsonNamingPolicy.SnakeCaseLower` on both the literal and the runtime
  value; kotlin_android: `.name.lowercase()`) -- correct only for enums whose Rust source
  carries `#[serde(rename_all = "snake_case")]`. An enum with no `rename_all`, whose unit
  variants serialize verbatim (e.g. `DataNodeKind`, `KeyValue`/`Element`/`Sequence`), was
  compared as `"keyvalue"` against `"key_value"` (C#) or `"KeyValue"` against `"keyvalue"`
  (kotlin_android) and failed every such assertion at runtime. C# now serializes the actual
  value through `System.Text.Json.JsonSerializer.Serialize`, which picks up the enum's own
  always-generated `[JsonConverter(typeof(<Enum>JsonConverter))]` and reproduces the exact
  wire string for any rename policy. kotlin_android now calls the enum's always-generated
  `fun toWire(): String`, built from the same `wire_variant_value` mapping the
  `@JsonProperty` annotations commit to.

- **The Ruby `cargo sort` residual formatting step now targets the directory the scaffold
  actually creates.** It derived the extension directory from the configured (or
  crate-name-derived) `gem_name`, but `scaffold::languages::ruby` always names it
  `ext/{core_crate_dir}_rb/native`, independent of any `gem_name` override -- those two
  namespaces can legitimately diverge (a gem renamed for RubyGems without renaming the crate).
  A configured `gem_name` silently pointed the residual step at a directory that was never
  created, failing with "No file found at" against a real consumer. Both now go through a new
  `ResolvedCrateConfig::ruby_native_ext_name`.

- **The Swift backend no longer emits a wire-string-to-enum conversion that cannot compile for
  a data-carrying variant.** `emit_enum_wrapper` unconditionally generated a reverse
  (`__alef_<enum>_from_swift_string`) conversion for every enum, including one arm per variant
  regardless of fields -- `"heading" => real::Enum::Heading,` does not type-check when
  `Heading` is a struct or tuple variant (E0533/E0308: "expected value, found struct/tuple
  variant"). This broke `alef generate` outright for any crate with a data-carrying enum
  crossing the Swift boundary; it reproduced independently in three separate consumer repos.
  Every call site that invokes the emitted function already gates on the same all-fieldless
  check (`unit_enum_names`), so the function is now only emitted when every variant is
  fieldless -- removing dead, non-compiling code rather than any code a caller could reach.

- **`alef generate` no longer lets one language's post-build failure hide every later
  language's post-build result.** `run_required_post_builds` propagated the first failure with
  `?` immediately, aborting the loop before any later-listed language's post-build (e.g.
  Kotlin/Android's Gradle build, Dart's `flutter_rust_bridge_codegen`) ran at all -- even
  though each is an independent step with no dependency on the others having succeeded. This
  mirrors the `e2e::run_generators` fix for the identical shape (see its doc comment: a
  consumer's C backend `bail!` silently starved every later e2e backend for two days). Every
  language's post-build is now attempted regardless of an earlier one's failure, with every
  failure named once every backend that could run has.

- **`alef e2e generate`'s inert-example summary now names which language and fixture were
  refused, not just a count.** The end-of-run line ("N generated example(s) across M
  language(s) had no runnable expectation... X that rendered nothing at all") gave an operator
  nothing to act on without re-running at `-vv` and grepping generated files for the same
  reason text the ledger already carried. Each refusal is now logged individually
  (`[language] alef rendered no runnable expectation for fixture ...`) before the aggregate
  line, via the new `e2e::report_inert_examples`.

- **`alef e2e generate`'s deferred-formatter warning no longer blames a missing toolchain on an
  unpublished version.** `warn_deferred` hard-coded the prefix "deferred until the pinned
  version is published" for every deferred step, but that is only true of the registry-mode
  dependency-resolution case (`UNPUBLISHED_VERSION_REASON`); a step deferred because its
  executable is simply absent (`MISSING_TOOLCHAIN_REASON` -- the shape a CI job without
  `mix`/`go` on `PATH` hits on every run) got the same false claim, pointing the operator at
  "wait for a release" instead of "install the toolchain". The prefix is now reason-agnostic
  and lets each entry's own `reason` field say why.

- **Kotlin's `not_error` assertion renders a real, visible check instead of a vacuous body.**
  Every sibling backend (java, csharp, typescript, swift, python, elixir) already carried this
  fix; Kotlin still rendered nothing on the theory that the call succeeding without throwing
  already proved it, so a fixture whose only assertion was `not_error` compiled and passed
  while asserting nothing at runtime. Surfaced by the `alef e2e generate` inert-example warning
  naming 9 such fixtures in a real consumer (`liter-llm`) once that warning started naming
  refusals instead of only counting them. Non-streaming fixtures now render
  `assertNotNull(<result>, "expected non-null result")`; streaming fixtures assert on the
  drained `chunks` list instead, matching every other streaming assertion in this backend.
  `assertNotNull` rather than `assertTrue(x != null, ...)` because Kotlin flags an explicit
  `!= null` comparison against a statically non-nullable type as "condition is always true".
  The rendering moved to a new `kotlin/not_error.rs` module rather than growing
  `kotlin/assertions.rs`, which is already over this repo's 1,000-line file cap.

- **A `result_fields` entry that contradicts the IR's `binding_excluded` set now warns at most
  once per worker thread per run, not once per resolver build.** `FieldResolver::warn_on_result_fields_contradicting_ir`
  fires from `with_ir_fields`, which every backend's assertion codegen calls once per (fixture,
  language, reachable/excluded pass) — a single bad config entry produced the identical WARN
  line thousands of times in one run (2600+ occurrences of one field in a single `crawlberg
  adopt`), burying the one finding worth reading. A thread-local dedup set now suppresses
  repeats of the same field name on the thread that already warned about it — verified against
  `crawlberg`: `-j1` now emits exactly one warning for the field, the default parallel job count
  emits at most one per thread that touched it (two, on that run) — down from 2614.

- **The generated-output gate's clippy self-check now sabotages a file where the lint can
  actually fire.** It appended a redundant pointer cast to the alphabetically first emitted
  source, which is the FFI crate's `lib.rs` -- and that file allows `clippy::unnecessary_cast`
  at crate level, so the sabotage compiled clean and the lane reported success while proving
  nothing about whether it examines the emitted Rust at all.

- **Scaffolded Node platform-loader JS, wasm `package.json`, Java checkstyle/versions-rules
  XML, and the Ruby RSpec seed now emit already `poly fmt`-clean.** Each was hand-formatted at
  a width or tag/matcher style poly's bundled engines (oxc for JS/JSON, a generic XML
  reindenter, rubocop-style RSpec parenthesization) normalise differently, so the very first
  `alef scaffold` left `poly fmt --check` red on files a consumer would never touch again
  (`crates/<crate>-node/index.js`, `crates/<crate>-wasm/package.json`,
  `packages/java/checkstyle.xml`, `checkstyle-suppressions.xml`, `versions-rules.xml`,
  `packages/ruby/spec/<crate>_spec.rb`). Each fix matches poly's actual output, measured by
  running `poly fmt --fix` over the emitted file rather than guessed.
- **The swift-bridge placeholder `RustBridgeC.h` header (emitted before the binding crate's
  first `cargo build`, in both `bridge_artifacts::emit_swift_bridge_files` and
  `scaffold::languages::swift::render_rust_bridge_c_header`) now emits already `poly
  fmt`-clean.** The struct typedefs were hand-formatted on single lines; poly's clang-format
  catalog tool (enabled for every FFI-bearing consumer) expands them onto one member per line
  and sorts the `#include`s. Unlike cbindgen's own header (see the poly_fmt gate exclusion
  below), this placeholder is alef's own literal string -- alef fully controls its formatting
  and has no reason not to match poly.
- **The generated-output gate's poly_fmt lane no longer fails on cbindgen's own C header, and
  its wide-TOML-array sabotage test no longer sabotages a file poly never checks.** cbindgen
  writes `crates/*-ffi/include/*.h` (and build.rs's `packages/go/include/` copy) directly at
  `cargo build` time -- never through alef's writer -- and the gate materializes it as a side
  effect of the swift lane's post-build `cargo build` pulling in the FFI crate as a dependency,
  before the follow-up `poly fmt --fix` a real consumer would run after building. The lane now
  passes `--exclude '**/include/*.h'`, declared and justified in the new
  `tests/generated_output_downstream_gate/poly_fmt_exclusions.rs` module (split out to keep the
  gate file under the repo's 1,000-line cap), with its own
  `every_poly_fmt_lane_exclusion_is_justified` test mirroring the clippy lane's discipline.
  Separately, `Sabotage::WideTomlArrayIndent` picked the alphabetically-first emitted TOML file,
  which is `Cargo.toml` at the tree root -- itself excluded from poly's format pass in favor of
  `cargo sort` -- so the sabotage landed somewhere `poly fmt --check` never looks and the lane's
  own anti-vacuity proof passed while examining nothing. It now skips `Cargo.toml`.
- **The JNI shim templates emit `e.to_string()` instead of `format!("{e}")`.** The generated
  crates are checked with `cargo clippy -- -D warnings`, where that spelling is
  `clippy::useless_format` and therefore a hard error, so the emitted JNI crate could not build
  at all. A new test scans every `.jinja` under `src/` for the pattern.

- **The JNI binding crate's `Cargo.toml` no longer claims workspace inheritance that may not
  exist.** It hard-coded `version.workspace = true` / `edition.workspace = true` /
  `license.workspace = true`; in a root-flat emitted tree there is no workspace root, so cargo
  rejected the manifest outright ("error inheriting `edition` from workspace root manifest") and
  every downstream command over the JNI crate failed before compiling anything. It now routes
  through `detect_workspace_inheritance_for_crate` and `cargo_package_header`, as the FFI,
  Python, PHP, and Ruby scaffolders already did. Its core dependency path is likewise derived
  from the emitted layout rather than a hard-coded `../{core_crate_dir}`.
- **The wasm binding crate's core dependency path is derived from the layout the manifest is
  written into**, instead of a hard-coded `../{core_crate_dir}` that only resolves when the core
  crate is a `crates/` sibling. For a root-flat core crate the manifest pointed at a
  `crates/<core>` the emitted tree does not contain. This is the same defect fixed for the FFI,
  Python, Node, and PHP scaffolders; the wasm backend builds its own manifest and was missed.

- **An absent Elixir or Go toolchain no longer kills the e2e format pass on Windows.**
  The two residual steps (`mix format`, `go mod tidy`) ran as `sh -c "(cd {dir} && ...)"`
  with the output directory interpolated raw. `canonicalize` yields Windows'
  extended-length form (`\\?\C:\...`), which POSIX `cd` rejects, so the shell exited 1
  from the failed `cd` before the tool was ever reached -- and 1 is not the
  command-not-found status the absent-toolchain check looks for, so a missing `mix` was
  reported as the formatter running and rejecting the generated code. Both steps now spawn
  the tool directly with the working directory set by the OS, which also fixes the same
  failure on any platform when the output path contains a space.

- **`alef test-apps generate`'s write path now has an idempotency regression test.** It writes
  through `write_scaffold_files_report` + `finalize_hashes`, not `write_files_report`, and only
  the latter was covered. The reported 153-file `alef:hash:`-only rewrite was traced to consumer
  bindings last stamped before `strip_alef_version_pin` (fixed in 0.62.2) and converges in one
  run; a second clean-tree run against a real consumer wrote zero files.

- **`alef generate` no longer refuses a different set of Swift files, or reports a different
  file count, on a second run over an unchanged tree.** Two bugs compounded. First,
  `emit_swift_bridge_files` read `target/`'s swift-bridge build output directly from the
  `alef generate` code path, but that directory is populated by this same command's own
  post-build step (`cargo build` + `MaterializeSwiftBridge`) — so whether it existed yet
  depended on run ordering, not on source input. A run before any build saw nothing and
  emitted a placeholder (or nothing); an otherwise-identical run after a build populated
  `target/` emitted `SwiftBridgeCore.swift` and `{crate}.swift` in full and fed them through
  the ownership-guarded writer, which refused both as foreign since swift-bridge's own
  header/import conventions rule out an alef marker on them — moving both the refusal set
  and the "Generated N files" count between two runs of identical source. `alef generate` now
  never consults `target/` (`consult_build_output: false`); only the `MaterializeSwiftBridge`
  post-build step, which runs after the build it triggers, does. Second, the "Generated N
  files" total itself counted every file the generator computed in memory, including files
  that were cache-skipped or refused and never actually written — now it sums actual writes,
  matching every per-phase "Generated N ... files" line already printed alongside it. Third,
  `PostBuildStep::MaterializeSwiftBridge` writes its three files unguarded, outside
  `pipeline::generate()`'s tracked output, so a run where the generator itself found nothing
  new to emit for those paths (a healthy, common case) dropped them from that run's
  generation-ownership record; the very next run's orphan sweep then read the absence as
  "alef no longer generates this" and deleted `RustBridgeC.h` from an otherwise unchanged
  tree. `PostBuildStep::owned_paths` now names every path a post-build step writes outside
  the guarded writer, and `alef generate` folds those into the same run's ownership record
  unconditionally, so the sweep never mistakes a build tool's own output for an orphan.
- **`alef scaffold` no longer silently skips recreating a deleted create-once seed file
  (e.g. a language's `<crate>_test.zig`/`.dart` sample suite).** Its stage cache
  (`cache::is_stage_cached`) is a hit only when the input hash matches *and* every path in
  the recorded manifest still exists on disk — but the manifest passed to `write_stage_hash`
  was filtered to marker-carrying files only, and a create-once seed is deliberately emitted
  unmarked (`generated_header: false`) so a hand-grown suite is never clobbered on a later
  run. Deleting one left the seed invisible to the disk-presence check: the stage hash was
  unchanged (source, config, and fixtures were untouched), so the cache read as a hit and
  `pipeline::scaffold`'s own create-if-absent logic never ran again to replace it. The
  manifest now records every path `pipeline::scaffold` returned, not only the marked subset —
  presence is a weaker claim than ownership, and it's exactly what a create-once file's
  absence should invalidate.

- **The generated WASM API reference no longer documents `#[serde(untagged)]` data enums as a
  fixed set of named, referenceable values.** `render_enum` rendered the same `| Value |
  Description |` table for every enum regardless of language or lowering, but the WASM backend
  never calls `enums::gen_enum` for a payload-carrying untagged enum -- there is no `Wasm{Enum}`
  class or member a JS/TS caller could reference by name, only the structural TypeScript union
  `ts_union.rs` emits. The docs page now calls a new `docs_ts_type_for_untagged_enum` (exposed
  from the WASM backend) to embed that SAME computed union text for WASM specifically, instead of
  restating a description of it that could drift; every other language's docs are unaffected.
- **A stale `[crates.ffi.capsule_types]` entry no longer leaves an unusable typedef in the
  generated C header.** `gen_cbindgen_toml`'s capsule forward-declaration block unconditionally
  forward-declared every configured capsule type's `c_return_type`, reading only the static
  config -- never whether any function in the current API surface actually returns that capsule
  type (the same decision `super::capsule::capsule_return_name` already makes for the real
  function codegen). A capsule type whose returning function was removed, renamed, or excluded
  still got a `typedef struct {C_TYPE} {C_TYPE};` in the header with zero generated functions
  using it -- a type a C consumer could neither construct nor pass anywhere. The forward
  declaration is now gated on the same shared usage check.
- **The TypeScript and R e2e visitor fixtures now emit the flat `{ custom: ... }` payload
  the napi and extendr backends actually look up, instead of a nested
  `{ type: "custom", output: ... }` envelope.** `visitor_method.jinja` (napi) reads
  `obj.get_named_property(variant.wire_name)` and `visitor_method.jinja` (extendr) reads
  `val.dollar(variant.wire_name)` — both flat lookups keyed by the variant's wire name. The
  TypeScript e2e template and the real R generator (`e2e::codegen::r::visitor`, not the dead
  `e2e/templates/r/visitor_method.jinja`) instead emitted a nested envelope that satisfies
  neither lookup, so every custom visitor callback result was silently dropped to the default
  action in generated TypeScript and R e2e suites.
- **A project that configures no `[crates.output]` for Python, Node, PHP, FFI, or wasm no
  longer scaffolds a manifest cargo cannot build.** The scaffolders write each of those
  languages' `Cargo.toml` at `crates/{crate}-<suffix>`, but `OutputTemplate::resolve`'s
  unconfigured single-crate default was `packages/{lang}` -- so `alef generate` wrote the
  matching `lib.rs` under `packages/ffi/src/` while the scaffolded manifest at
  `crates/{crate}-ffi/Cargo.toml` declared a library at `crates/{crate}-ffi/src/lib.rs`,
  and cargo failed with "can't find library `<crate>_ffi`, rename file to `src/lib.rs` or
  specify lib.path" on the very first sibling crate that depended on it. The default now
  resolves to `crates/{crate}-<suffix>/src` for these five languages, via one
  `default_binding_crate_root` formula shared by `OutputTemplate::resolve` and
  `package_dir`'s Node/Wasm no-override formula, so a scaffolded manifest and the sources
  `alef generate` writes for it can no longer name two different crate roots.

- **Java no longer drops every lifetime-parameterized type (e.g. `NodeContext<'a>`) from
  generation.** A lifetime parameter alone is not a reason a type can't cross the JNI boundary —
  the binding holds an opaque handle (or a plain record whose fields are already owned values),
  and the lifetime is erased at the C ABI, exactly like every other FFI-dependent backend
  (csharp, go, kotlin, kotlin_android). The blanket exclusion also silently broke the visitor
  trait-bridge pattern whenever its configured `context_type` had a lifetime parameter:
  `resolve_visitor_generation` could no longer find the type in the filtered surface, so
  `VisitorBridge.java` and its sibling files stopped being generated with no error surfaced.
  Java now only excludes a lifetime-bound type from a *service*'s constructor, configurators,
  registrations, and entrypoints — the one place the IR can't prove borrowed data outlives a
  long-running `run`/`finalize` call — leaving ordinary types, functions, and the visitor
  pattern unaffected.
- **The Ruby (Magnus) e2e visitor fixture now returns the lowercase wire values the generated
  binding actually matches on.** `e2e/templates/ruby/visitor_method.jinja` hardcoded
  `'Skip'`/`'Continue'`/`'PreserveHtml'`/`{ Custom: ... }`, while the Magnus backend's
  `gen_visitor_bridge` matches on the enum's lowercase wire name (`"skip"`/`"continue"`/
  `"preserve_html"`) and looks up the custom payload via `ruby.to_symbol("custom")`. Every
  skip/continue/preserve_html callback fell through to the catch-all as `Custom("Skip")`
  and every custom-hash lookup missed. `php/visitor_method.jinja` and the unused
  `wasm/visitor_method.jinja` and `r/visitor_method.jinja` templates had the same
  hardcoded-PascalCase defect and are fixed alongside it.

- **A project that configures no `[crates.output]` no longer has its wasm manifest written into
  the directory that holds every other language's package.** `OutputTemplate::resolve` defaults
  wasm to the crate-root-shaped `packages/wasm`, while the wasm, ffi, and php emitters recovered
  the crate root from that path with a bare `Path::parent` -- correct only for the `src`-suffixed
  shape every consumer repo spells out. On a stock scaffold the wasm `Cargo.toml` landed at
  `packages/Cargo.toml` (declaring a library whose `lib.rs` sat one level below it), ffi's
  `build.rs` and `cbindgen.toml` beside it, and php's stubs at `packages/stubs/`. Cargo walks
  upward looking for a workspace, so the stray manifest failed the *sibling* languages' builds:
  swift's post-build step was the one that went red. The crate root is now derived from the path's
  shape by `OutputLayout` -- parent of a `src`-suffixed path, the path itself otherwise, with
  sources under `<root>/src` -- so the manifest always contains the sources it declares, and every
  `src`-suffixed config resolves to exactly the paths it did before.

- **Three snippet-session tests and two config tests no longer examine nothing on Windows.**
  Their `before` hooks were POSIX shell (`! find ... | grep -q .`, `$(( ))`), and alef runs hooks
  through `cmd` on Windows, which split the `;`-sequenced line on spaces and handed `touch` a
  `-lt` flag; the mix dependency check was driven with an absolute Windows path through `sh`,
  where every backslash is an escape, so it answered "no deps" whatever was on disk and its
  first assertion passed for the wrong reason. The hooks are now `cmd`-metacharacter-free and
  handed to `sh` explicitly on Windows, the mix check runs against the relative package directory
  a real config carries, and the README snippet-root assertion builds its expected path with
  `Path::join` instead of a hard-coded `/`.

- **A session-scoped Zig snippet's `build.zig.zon` no longer names its dependency with a host
  path separator.** The generated `.path` value was rendered with `Path::to_string_lossy`, which
  on Windows emits `..\package`. Zig resolves `.path` dependencies POSIX-style on every platform,
  so that reached the manifest as one nonsensical component and the binding package could not be
  fetched. The value is now built by joining the path's components with `/`, which is what Zig
  accepts everywhere.

- **Gleam e2e assertions no longer compare an enum-typed field against a raw string.**
  `render_test_case` decided enum-ness solely from the hand-maintained `fields_enum` /
  `[e2e.call.overrides.gleam] enum_fields` config, so a consumer that never declared the entry
  got `r.kind |> should.equal("key_value")` for a genuine `DataNodeKind` field. Gleam emits that
  enum as a custom type and `should.equal` is homogeneous, so the generated module failed to
  compile. Gleam now uses the same IR-derived classification the rust and csharp e2e generators
  use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`), anchored at the call's declared
  Rust return type so a leaf name meaning different things on different types resolves per
  owner. The IR only ever adds: an explicit config entry still wins.

- **Kotlin snippet validation no longer reports a toolchain it cannot launch.**
  `KotlinValidator::is_available` answers with `which::which("kotlinc")`, which searches the whole
  of `PATHEXT`, while spawning went through `Command::new("kotlinc")`, which on Windows only ever
  appends `.exe`. Kotlin ships as `kotlinc.bat` there, so availability reported yes and every
  batch then failed with `spawn failed: program not found` -- the two halves disagreeing about the
  same tool. Toolchain spawns now go through `core::tool_command`, which hands `Command` the
  resolved path so std routes the batch shim through `cmd.exe`.

- **The node scaffold now gitignores the declarations `napi build` generates.** Pointing
  napi-rs's `--dts` at `index.native.d.ts` stopped it clobbering alef's own `index.d.ts`, but
  left the redirect target untracked, so every `npm run build` dropped a new file into the
  consumer's tree. That file is a pure build artifact — absent from `package.json` `files` so
  npm never receives it, named by no `types` or `exports` entry, and imported by nothing — so
  the scaffolded crate now ships a `.gitignore` for it, following the same per-language pattern
  the dart, gleam, kotlin, swift, wasm and zig scaffolders already use.

- **Snippet validation no longer strips the environment every Windows toolchain needs.** Each
  validator subprocess has its environment cleared and rebuilt from an allowlist, and that
  allowlist was Unix-shaped: it kept `SYSTEMROOT` and `WINDIR` but dropped `USERPROFILE`,
  `ProgramFiles(x86)` and the rest of the variables that identify a Windows machine. Every
  `dotnet build` then failed inside `NuGet.targets` with `Value cannot be null. (Parameter
  'path1')` -- NuGet resolves its global packages folder through `USERPROFILE` -- and every Rust
  snippet failed to link, because rustc locates the MSVC linker by running `vswhere.exe` under
  `ProgramFiles(x86)` and without it fell back to whatever `link.exe` came first on `PATH`, which
  on a box with Git for Windows is GNU coreutils' `link`. C# and Rust snippet validation were
  broken on every Windows host, not just CI.

- **The Windows test suite no longer fails on its own fixtures' path quoting.** Six tests built
  an `alef.toml` fixture by interpolating a `tempfile` directory into a TOML *basic* string. A
  Windows tempdir is `C:\Users\RUNNER~1\...`, where `\U` opens a unicode escape, so `toml`
  rejected the whole document ("too few unicode value digits") and the FFI custom-module and
  kotlin-android gradle walk-up tests died before reaching what they assert. The fixtures now use
  TOML literal strings, which take no escapes at all.

- **The Zig runtime-file-IO compile check now links libc, the way every generated `build.zig`
  does.** The check compiled its snippet with `zig build-exe -fno-emit-bin` and no `-lc`, while
  the emitted body allocates through `std.heap.c_allocator` and every generated Zig project sets
  `.link_libc = true`. Zig 0.16 rejects `c_allocator` outright without libc, so the check was
  compiling under settings no real generated project uses — and passed only on macOS, where
  libSystem is linked implicitly.

- **C e2e codegen no longer misrenders an undeclared enum-typed leaf field as `char*`.**
  `emit_nested_accessor`'s leaf arm defaulted to `char* {local} = {accessor}(...)` for any
  field not explicitly listed in `[crates.e2e.fields_c_types]`, which is only correct when the
  FFI accessor genuinely returns a C string. For a field whose Rust type is a real registered
  enum (e.g. `DataNode.kind: DataNodeKind`, as shipped in
  `tree-sitter-language-pack/e2e/c/test_data_extraction.c`), the accessor instead returns an
  opaque `AlefHandle` requiring a further `_to_string()` call — a mismatch gcc rejects as
  "incompatible integer to pointer conversion", failing the C FFI e2e build with no diagnostic
  from `alef e2e generate` itself. A new `enum_fields_c_types_from_ir` pass derives the missing
  `fields_c_types` entry directly from the IR struct definition before `render_test_file` builds
  its effective field-type map, so an enum-typed leaf renders correctly with zero operator
  configuration.
- **`kotlin_android` no longer aborts the whole `alef generate` process when
  `package_metadata.repository`/`.license` are unset.** `gen_build_gradle::emit` called
  `panic!` for each, which is unrecoverable and — unlike every sibling scaffolder (`java`,
  `kotlin`, `r`, `gleam`, which return a clean `anyhow` error from `alef scaffold`, a one-time
  step) — fires on *every* `alef generate`, since `build.gradle.kts` is rebuilt on every run.
  Repository and license only feed the published POM's optional URL/SCM/license sections, so
  generation now degrades gracefully — matching the "must not invent repository metadata"
  convention the C#, WASM and npm scaffolders already follow — by omitting those sections and
  logging a `tracing::warn!` naming the missing config, rather than crashing the process. The
  generated-output downstream gate's fixture also gained the `package_metadata` the `java` and
  `elixir` backends require to complete `generate` at all, which this panic had always masked.

- **`alef e2e generate`'s formatter tests no longer assert the behaviour the formatter stopped
  having.** Deferring a missing `poly`/`mix` executable instead of aborting (non-`--strict` mode)
  left three `e2e::format` tests still asserting `is_err()` in their toolchain-absent branch, so
  the suite failed on every machine without those tools installed — including all three CI
  runners — while passing locally. The tests now assert the real contract: generation returns
  `Ok`, the file is left untouched, and the absent step is recorded as a `DeferredFormatting`
  entry. Asserting the record rather than bare `Ok` keeps a silently-skipped run distinguishable
  from a fully-formatted one.

- **`napi build` no longer overwrites the `index.d.ts` alef generates.** napi-rs writes its own
  auto-derived type declarations to whatever the crate's `package.json` `"types"` field names,
  which is `index.d.ts` — the exact file alef's node backend hand-derives, complete with union
  types, doc comments and the `alef:hash:` provenance line. Every `napi build` therefore replaced
  alef's canonical declarations with napi-rs's own and stripped the provenance header `alef
  verify` relies on to detect staleness, leaving a `.d.ts` full of dangling `Js*` references and
  duplicate identifiers. All four invocation sites — the default `[build_commands.node]` entry,
  `alef build`'s own command construction, `alef publish`, and the scaffolded `npm run build` —
  now pin `--dts index.native.d.ts`.

- **`csharp` e2e no longer emits a raw string comparison against an enum-typed result field.**
  The generator decided enum-ness purely from hand-maintained `[e2e.call.overrides.csharp]
  enum_fields` config, so a consumer that never declared the entry got
  `Assert.Equal("KeyValue", result.Data!.Kind)` for a genuine enum field — a `CS1503` whose
  reported message names `IAsyncEnumerable<char>?` (xunit's closest-matching but unrelated
  overload) rather than anything resembling the real defect. C# now wires the same IR-derived
  classification the Rust e2e generator uses, anchored at the call's declared return type, so a
  field renders as enum-typed whenever the IR says so, config or not.

- **`alef e2e generate` no longer rejects an `overrides.<lang>.class` that names a class the
  backend really emits.** The validator built its candidate set by PascalCasing the crate name,
  which is not what any of the six class-consuming backends actually name their crate facade:
  `kotlin_android` and the `java` public facade strip a trailing `Rs`, `php` derives from
  `[crates.php] extension_name`, `ruby` emits both a crate-name native module and a
  `[crates.ruby] gem_name` wrapper module, and `dart` appends `Bridge` to `[crates.dart]
  lib_name`. A correct config therefore failed validation with `Severity::Error`, refusing all
  regeneration. Each language now calls the exact function its own codegen uses, so a rename on
  either side breaks a test instead of silently drifting, and a backend whose facade cannot be
  derived downgrades to a warning rather than claiming an override is wrong on the strength of a
  candidate set known to be incomplete.
- **`alef extract` no longer warns that a bare `#[serde(default)]` disagrees with a manual
  `impl Default` that names the field type's `#[default]` enum variant.** Both spell the field's
  type-zero; only the literal spelling was recognised, so every enum-typed field carrying both
  produced a spurious `defaults disagree` warning. An enum whose `#[default]` variant is not
  known at that point is treated as "cannot prove agreement" rather than "agrees", so the
  diagnostic still fires on a genuine mismatch.

- **The two committed records no longer ping-pong against `poly fmt`.** `.alef-ownership.toml`
  and `.alef-toml-merge-provenance.toml` wrote every array one element per line, but the format
  gate consumers commit through collapses an array whose inline form fits 120 columns. Since both
  records are rewritten wholesale on every `alef generate`, that was not a one-time cosmetic diff:
  alef re-expanded what `poly fmt` had collapsed, the gate collapsed it again, and the files
  changed in every commit forever with no way to settle it by hand. Both now collapse at exactly
  the boundary the formatter does (measured, not assumed: 120 columns inline is collapsed, 121 is
  left expanded), so a converged tree stays converged.

- **`Publish` now refuses to publish a tag whose `CI` run is not green.** `Publish` fires on
  `release: published`, which is entirely independent of `CI`, and nothing in the workflow looked
  at whether the tagged commit ever built — a red `main` could be released to crates.io and
  Homebrew unnoticed. A new `check-ci` job resolves the release commit, polls the `CI` workflow
  for that exact SHA for up to 60 minutes (so a release cut moments after the push waits rather
  than failing on a still-queued run), and fails on any non-`success` conclusion.
  `publish-crates` and `build-cli` now gate on it, which transitively gates the Homebrew and
  release-asset jobs. A `skip_ci_gate` `workflow_dispatch` input exists for emergencies.

- **`wasm` no longer emits a struct field and its accessors with different types when a union
  is in `untagged_union_text_types`.** A `#[serde(untagged)]` data enum listed in
  `untagged_union_text_types` was picked up by both opt-ins at once: the `type_overrides` entry
  pinned to `String` drove the constructor, getter, and setter, while the JsValue-bridged set
  drove the struct field and both conversions. The emitted struct declared `Option<JsValue>` and
  handed it to accessors typed `Option<String>`, so the binding crate failed to compile with
  `E0308`. The text opt-in is the more specific signal and now wins on every surface.

- **The swift backend no longer emits an unresolvable `From<String>` conversion for
  fieldless enum parameters, and no longer borrows owned `String` parameters it passes
  through.** A method or function taking a plain fieldless enum by value (e.g. `fn
  analyze(&self, input: String, mode: Mode)`) produced `<toolkit::Mode as
  ::std::convert::From<String>>::from(mode)` — a trait alef never emits an impl for, and
  cannot legally emit for a foreign type in the swift crate (an orphan impl, E0117) — plus
  `&input` where the core method takes `input` by value. Both were compile errors
  (E0277, E0308) in every swift binding for a crate exposing a fieldless enum as a
  parameter, an entirely ordinary shape. Fixed by generating a local free function per
  enum (mirroring the `ffi` backend's `i32`-discriminant `from_i32` helpers, which have
  the same shape without the orphan-impl problem) that matches the wire string back to
  the enum's variants, and by only borrowing `String` parameters the core signature
  actually takes by reference.
- **The generated FFI crate's `alef_ffi_error_code` helper no longer triggers an
  `unused_variables` warning in every consumer whose fallible functions return a plain
  `String` error.** Its `error: &dyn std::any::Any` parameter is only read inside the
  per-typed-error `downcast_ref` arms `gen_last_error` emits from `api.errors`; a crate
  with no registered error types got a body that never reads `error` at all. The
  parameter is now underscore-prefixed when there are no typed error variants to
  downcast against.

- **Java e2e's `json_object` arg builder now classifies enum-typed fields from the IR, not
  only from hand-maintained config.** `java_builder_expression` decided whether a field is
  enum-typed purely from the hand-maintained `enum_fields` config — a flat, type-unaware set
  of camelCase field names — so a consumer whose `alef.toml` never listed a field emitted
  `.withStyle("fenced")`: a `String` literal passed to a builder method whose parameter type is
  a Java enum, which does not compile. `java_builder_expression` now also consults the
  IR-derived classification (`FieldResolver::ir_enum_fields`, keyed by owner type), anchored at
  the exact struct the JSON object maps to — a builder expression has no "declared Rust return
  type" the way a result-field assertion does, so it anchors on the type name it is already
  building rather than on `resolve_declared_result_type`. An explicit config entry still wins.
- **Ruby e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained config.** `render_assertion` read enum-ness solely from the effective
  `fields_enum` config (plus a per-call `enum_fields` override), so a consumer that never
  declared either got `field_is_enum = false` for a genuine `DataNodeKind` enum field. Ruby
  already applies `.to_s` unconditionally for string-valued `equals` comparisons, so
  misclassification is masked for that common shape — but a bare-enum "simple" result
  (`result_is_simple = true`, no struct field to traverse) relied on `field_is_enum` alone for
  its `.to_s` coercion, and the classification gap meant it silently compared the Magnus
  `Symbol` against the fixture's wire `String`. Ruby now also consults the same IR-derived
  classification the rust/csharp/gleam/swift/dart e2e generators use, anchored at the call's
  declared Rust return type; a related gap where `result_is_simple` bypassed `field_is_enum`
  entirely (dropping the coercion for a correctly-classified bare-enum result) is fixed
  alongside it. An explicit config entry still wins.
- **Elixir e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained config.** `render_assertion` read enum-ness solely from the effective
  `fields_enum` config (plus a per-call `enum_fields` override), so a consumer that never
  declared either got a bare `assert result.kind == "key_value"` for a genuine `DataNodeKind`
  enum field. The NIF binding serializes that field as an atom (`:key_value`), and Elixir does
  not fail to compile on `:key_value == "key_value"` — it silently evaluates to `false`, so the
  test asserts the wrong thing instead of refusing to build. Elixir now also consults the same
  IR-derived classification the rust/csharp/gleam/swift/dart e2e generators use, anchored at
  the call's declared Rust return type. An explicit config entry still wins.
- **Python e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained config.** `render_assertion` read enum-ness solely from the effective
  `fields_enum` config (plus a per-call `assert_enum_fields` override and an accessor-shape
  heuristic), so a consumer that never declared `fields_enum` got a bare
  `assert result.kind == "key_value"` for a genuine `DataNodeKind` enum field. Python does not
  fail to compile on that — it silently compares the PyO3 enum object against a plain string,
  which is `False` for a real enum even when the wire value matches, so the test asserts the
  wrong thing instead of refusing to build. Python now also consults the same IR-derived
  classification the rust/csharp/gleam/swift/dart e2e generators use, anchored at the call's
  declared Rust return type. An explicit config entry still wins.
- **Kotlin e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained config.** `render_assertion` read enum-ness solely from the effective
  `fields_enum` config (itself merged with a per-call `type_enum_fields` auto-detect that
  needed a `result_type` override to anchor), so a consumer that never declared either got
  `assertEquals("key_value", result.kind())` for a genuine `DataNodeKind` enum field. The
  JVM binding wraps that field in a Java enum exposing `.getValue()`, so the comparison does
  not compile. Kotlin now also consults the same IR-derived classification the rust/csharp/
  gleam/swift/dart e2e generators use, anchored at the call's declared Rust return type. An
  explicit config entry still wins.

- **`alef verify` now detects orphaned generated files** — an alef-marked file still on disk
  that the current run's backends would no longer produce (a dropped emit, a removed language,
  or a config change), the exact failure mode that let Java's `NodeContext.java` /
  `HtmlVisitor.java` / `VisitorBridge.java` sit unnoticed across releases. Detection is
  report-only: alef never deletes a file it flags. The finding is folded into `verify`'s
  existing hard-fail exit code (downgraded to a report with `--report-only`, same as every
  other finding), and excludes both unmarked user-owned files and known create-once seeds
  (`rust-toolchain.toml`, the wasm-only `.cargo/config.toml`) that a scaffold stage only emits
  once, when absent.

- **`alef verify`'s in-memory regeneration pass no longer skips a language whose output happens
  to already be cache-fresh**, which previously dropped every one of that language's
  bindings-stage files from the surface `verify` compares against disk — the exact steady state
  right after `alef generate`/`alef all` writes the cache, i.e. the most common moment `alef
  verify` runs. Combined with the new orphan check above, this made a self-marking backend's own
  bindings file (`packages/python/lib.rs` and its pyo3-style equivalents) a guaranteed false
  orphan on a tree that had just been generated cleanly.
- **`jni` no longer hard-fails generation when `[crates.kotlin_android]` is unconfigured.**
  Every downstream accessor (`jni_kotlin_package`, `jni_excluded_functions`,
  `jni_excluded_types`, `jni_capsule_types`) already tolerated its absence, falling back to
  the same vendor-neutral placeholder package `kotlin`/`java` use when unconfigured — the
  `generate_bindings` guard was the only place still bailing on a config gap every sibling
  accessor already treated as a soft default, so enabling `jni` without also configuring
  `kotlin_android` produced a hard generate failure for a language the consumer did
  configure.
- **Swift e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained `fields_enum`/`[e2e.call.overrides.swift] enum_fields` config.** A consumer
  whose `alef.toml` never declared that entry got a bare `XCTAssertEqual(result.kind,
  "key_value")` for a first-class Codable struct's genuine `DataNodeKind` enum property — Swift
  compares an enum and a `String` directly there with no implicit conversion, so the generated
  test target failed to compile. Swift now wires the same IR-derived classification
  (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's declared Rust
  return type) the rust/csharp/gleam generators use; an explicit config entry still wins.
- **Dart e2e assertions now classify enum-typed result fields from the IR, not only from
  hand-maintained `fields_enum`/`[e2e.call.overrides.dart] enum_fields` config.** A consumer
  whose `alef.toml` never declared that entry got no `_alefE2eText` serde-wire conversion on a
  genuine `DataNodeKind` enum field — `expect(result.kind.toString(), equals('key_value'))`
  compares Dart's default enum `toString()` (its declaration name) against the fixture's serde
  wire value, so the assertion silently asserts the wrong string instead of failing to compile.
  Dart now wires the same IR-derived classification (`FieldResolver::ir_enum_fields` +
  `with_ir_enum_map`, anchored at the call's declared Rust return type) the rust/csharp/swift/
  gleam generators use; an explicit config entry still wins.
- **e2e/kotlin_android**: stop emitting a call to a function `[crates.kotlin_android].features`
  gated out of the binding. The kotlin_android binding generator already drops any
  `#[cfg(feature = "...")]`-gated function whose feature isn't in the binding's configured
  `features` list (`with_cfg_filtered_deep`), but the e2e test generator had no equivalent
  check and emitted a real call to the dropped symbol anyway, producing a Kotlin "Unresolved
  reference" compile failure (e.g. `manifestLanguages`, gated on the `download` feature, for a
  binding whose `features` omits it). The fixture now renders as a `@Disabled` entry in
  `ExcludedBindingsTest.kt` naming the unsatisfied cfg gate, the same way a visitor-excluded
  fixture already does.
- **The FFI, Python, Node, and PHP scaffolders no longer hard-code the core crate's Cargo
  dependency as `path = "../{core_crate_dir}"`.** That formula only resolves when the core
  crate is a workspace-shaped `crates/` sibling of the binding crate; for a root-flat core
  crate (`Cargo.toml` at the project root -- the shape alef itself has used since 0.18.0) it
  pointed at a `crates/<name>` directory that does not exist, and `cargo` failed to find the
  dependency on the very first build. `ResolvedCrateConfig::core_crate_dep_path` now derives
  the relative path from the binding crate's actual root to `core_crate_root` (a root-flat
  layout resolves to the project root itself; a workspace layout to its `crates/` sibling),
  so both shapes get the correct `..` depth instead of one hard-coded assumption.
- **`is_tool_available` no longer reports every formatter absent on Windows.** It shelled out
  to `Command::new("which")`, but `which` is not a Windows command -- the spawn itself fails
  there, and `unwrap_or(false)` silently reported every tool missing regardless of whether it
  was actually on `PATH`, skipping formatting for the whole run. It now resolves via the
  `which` crate's own cross-platform `PATH` walk instead of an external `which`/`where`
  binary.

- **Zig e2e's opaque-handle accessor path can now emit a real method call.**
  `render_zig_with_optionals` could only ever produce `.field` for a `method_calls` path,
  never `.field()`, so a fixture asserting on a field the generated Zig binding exposes as an
  FFI getter method on an opaque handle (`tree.language()`) rendered a path the struct does
  not declare. Mirrors the existing Rust accessor's `method_calls`/`result_fields`
  disambiguation: a path in `method_calls` and not in `result_fields` now gets `()`; a path
  classified as both keeps the pre-existing tagged-union-variant shape (plain dot access).
- **Zig e2e now auto-detects a JSON-struct return from the core IR.** `render_test_fn` only
  took the JSON-parsing assertion path when a call declared `result_is_json_struct` or a
  `client_factory`, but `zig_return_type` maps EVERY `Named` struct return with `has_serde`
  to `[]u8` unconditionally. A plain function returning such a struct, with neither config,
  emitted `result.<field>` against the byte slice the backend actually returns — a compile
  error on every field. The generator now also resolves the call's declared Rust return type
  through the IR and treats it as JSON whenever that type is one the Zig backend serializes,
  additively, without disturbing existing overrides/`client_factory` behavior.
- **The Zig e2e IR (`enums`, `functions`) was never wired into the generator at all** — both
  were bound `_enums`/`_functions` in `ZigE2eCodegen::generate` and discarded. Wiring
  `functions` in is what makes the JSON-struct auto-detection above possible; `enums` remains
  unused until the enum-field classification fix below.
- **Zig e2e's enum-field classification now consults the core IR, not only
  `fields_enum`/`[overrides.zig].enum_fields`.** With the two fixes above making the
  typed-struct assertion path reachable for real enum-typed fields, the previously dead
  `enum_fields` check would have emitted `testing.expectEqual("value", result.kind)` against
  a genuine Zig enum — a type Zig's `expectEqual` cannot compare against a string literal.
  Wires the same IR-derived classification (`FieldResolver::ir_enum_fields` +
  `with_ir_enum_map`, anchored at the call's declared Rust return type) the gleam e2e
  generator uses, and skips the `equals` assertion instead of emitting code that cannot
  compile. Config still wins; the IR only adds.
- **Zig backend: a method returning a real enum no longer emits an invalid `._handle` struct
  literal.** `opaque_handles::returns::method_unwrap_return_expr`'s bare-`Named` return arm
  treated every non-`struct_names` `Named` return as an opaque handle and produced
  `MyEnum{ ._handle = raw }` — not valid Zig for an `enum { ... }` declaration, which has no
  `._handle` field. A genuine enum return is now cast back from its raw discriminant with
  `@as(EnumName, @enumFromInt(raw))`, and skips the zero-sentinel null check the handle arm
  uses (a real enum variant can legitimately serialize to `0`).

## [0.62.5] - 2026-08-20

### Added

- **`[crates.test.<lang>].e2e_precondition`** lets a block scope the `e2e` tooling gate
  separately from the block's main `precondition`. A block with only `before` + `e2e` (no
  `command`) previously had to satisfy validation by writing a `precondition` for whatever
  `command`/`before` needed, and `alef test --e2e` then gated `e2e` on that same, often
  unrelated, check.

### Fixed

- **The scaffolded Maven `attach-javadocs` execution no longer fails for any consumer that has
  Java tests.** The pom sets `<sourcepath>${project.basedir}</sourcepath>` because alef emits a
  flat source layout with no `src/main/java/`, but that also pointed javadoc at `src/test/java/`,
  whose JUnit/AssertJ imports are test-scoped and absent from the javadoc classpath. Combined with
  the `failOnWarning` the same pom sets, `mvn package` died with hundreds of
  `package org.junit.jupiter.api does not exist` errors. maven-source-plugin already restricted
  itself to the publishable subtrees for the same underlying reason; javadoc was the one plugin
  left unrestricted, and it now carries the matching `<sourceFileIncludes>`.

- **`alef test --lang <X> --e2e` no longer skips the e2e suite on a precondition written for
  the block's `command`, not for `e2e`.** `e2e` is now gated by the new `e2e_precondition` when
  set; when unset, `e2e` runs ungated instead of inheriting the main `precondition` (which was
  authored for a different command and could name tooling `e2e` never uses, e.g. a linter). The
  main `precondition` still gates `command`/`coverage` exactly as before. `before` is unchanged
  and still runs ahead of `e2e`, since it commonly builds the native library the e2e suite loads.

- **E2e enum-field detection is now derived from the IR, not only from a hand-written
  `alef.toml` `fields_enum` list.** `E2eConfig::effective_fields_enum` returned purely
  author-declared sets, so a consumer that never enumerated its enum fields got `false` for
  every one of them, and the Rust generator emitted `<field>.to_string()` for an enum-typed
  field -- a compile error (`E0599: doesn't implement std::fmt::Display`) for any enum that
  only derives `Debug`. `FieldResolver::is_enum` now falls back to a new IR-derived
  classification (`FieldResolver::ir_enum_fields` / `with_ir_enum_map`, in the new
  `e2e::field_access::ir_enum` module) that walks a field path from the call's declared Rust
  result type -- resolved from the crate's own function/method signatures, not a
  per-language override -- through `Option`/`Vec`/`Box`-wrapped and array-traversed
  (`links[].link_type`, `choices[0].finish_reason`) paths to the exact struct that owns the
  leaf field, and checks whether its declared type is a real IR enum. The classification is
  keyed by `(owner type, field name)`, so a field name that means different things on
  different types (`kind: String` on one struct, `kind: SomeEnum` on another) is never
  conflated. An explicit `fields_enum` config entry still wins over the IR when both apply.

## [0.62.4] - 2026-08-20

### Fixed

- **The Rust e2e generator no longer emits `.to_string()` on enum-typed fields, which does
  not compile unless the enum happens to derive/implement `Display`.** An `equals` assertion
  on an enum field (e.g. `kind: DataNodeKind`) rendered
  `result.kind.to_string()`, but alef requires no such trait -- most bound enums only derive
  `Debug`. `render_equals_assertion` already had `field_is_enum` plumbed through
  `FieldResolver::is_enum` for containment assertions, but never consulted it for `equals`, so
  every enum-field equals assertion failed to compile (`error[E0599]: doesn't implement
  std::fmt::Display`). It, and the analogous wildcard array-traversal `contains`/`not_empty`
  predicates (`links[].link_type`), now stringify enum-typed leaves via `format!("{:?}", ...)`
  (Debug), matching what the existing containment predicate already does -- for a unit variant
  this renders exactly the variant name, matching the fixture's captured expected literal.

- **A backfilled cfg-forwarded feature is now also enabled by default, not just declared.**
  Declaring `<feature> = ["<core-crate>/<feature>"]` in the Ruby/Elixir native manifest's
  `[features]` table does not turn the feature on -- `#[cfg(feature = "X")]` stayed false, and
  the affected definitions kept silently compiling out, even after the previous repair pass added
  the forwarding row, because nothing added `X` to `default` and no build wrapper alef scaffolds
  passes `--features`. `merge_missing_cfg_features` now also appends any referenced feature
  missing from `default` (whether newly declared or already declared but never defaulted),
  mirroring what `scaffold_ruby_cargo`/`scaffold_elixir_cargo` already write on a fresh scaffold.
  `warn_on_undeclared_binding_cfg_features` now keys on `read_default_enabled_cargo_features`
  (reachable from `default`) instead of mere declaration, so a feature that is declared but not
  defaulted still warns instead of reading as fixed.

- **`alef scaffold` now actually adds a cfg-forwarded feature the compile-out warning names,
  instead of leaving the prescribed remedy a no-op.** The Ruby (Magnus) and Elixir (Rustler)
  native manifests are `generated_header: true`, so a full regen already includes every feature
  `collect_cfg_features` finds — but once the manifest exists on disk,
  `write_scaffold_files_report`'s ownership guard only overwrites it wholesale when it can prove
  alef authored the existing bytes, and a manifest predating the marker scheme (or one whose
  marker a hand-edit or formatter moved past the guard's scan window) is refused forever, so
  "re-run `alef scaffold`" never converged. `alef scaffold` and `alef generate` now also run a
  narrower, always-safe repair: it inserts only the missing `<feature> =
  ["<core-crate>/<feature>"]` row(s) into the manifest's `[features]` table (creating the table
  if absent) via `toml_edit`, which cannot reorder, reformat, or drop anything else already in
  the file, and never invents a row for a feature the core crate itself does not declare.

## [0.62.3] - 2026-08-19

### Fixed

- **The serde-default-disagreement warning no longer fires when both defaults are the same
  zero value spelled differently.** A bare `#[serde(default)]` always folds to
  `DefaultValue::Empty`, but a hand-written `impl Default` that spells the zero out explicitly
  (`count: 0`, `enabled: false`, `label: String::new()`, `handle: None`) folds to a literal
  instead. `warn_on_default_disagreement` compared the two spellings structurally and reported a
  divergence that did not exist; it now treats `Empty` and its type-zero literal counterparts as
  equal before deciding whether to warn.

- **The disk-scan orphan report no longer asserts a file "was not emitted".** What it can actually
  observe is that a path is absent from the run's recorded output, and the check immediately above
  it warns that some backends record nothing beyond their Rust crate path — so non-emission is one
  of four explanations, not a fact. The report now says what it knows.

- **e2e fixture diagnostics are logged at the severity they carry.** Both arms of the severity
  match emitted `warn!`, so a diagnostic that aborts the run two statements later was
  indistinguishable in the log from one that changes nothing; field-classification errors were
  likewise logged as warnings immediately before bailing. Errors now log at error level.

- **The "requires FFI" warning no longer fires on a deliberate single-language regen.** It tested
  the `--lang`-filtered language list for an FFI entry, so `alef generate --lang csharp` warned
  that FFI was missing even when the FFI crate was configured, generated and committed. The
  condition it describes is a property of the crate's configured languages, not of one
  invocation's scope, and is now checked against those.

- **`alef verify` no longer reports a file as frozen once alef has durable proof it owns it.**
  The write guards in `write_files_report`/`write_scaffold_files_report` treat a marker-less file
  as owned when either it carries the provenance marker or — for formats with no comment syntax
  at all — the committed `.alef-ownership.toml` record says so. `alef verify`'s frozen-file report
  only ever checked the marker, so a file `alef adopt` had just recorded, or one a
  delete-and-regenerate had just rewritten and recorded, kept being reported "frozen" forever, even
  though the write guard would happily accept it. Both write guards and the report now share one
  `is_owned_by_ownership_record` predicate instead of three independently drifting copies.

- **`.clang-format` can now carry a provenance marker.** It is YAML underneath (`#` line comments),
  scaffolded `generated_header: true` for every FFI target, but was missing from
  `marker_header_syntax`'s file-name table — an oversight, not a deliberate exclusion like
  `DESCRIPTION`'s (which stays off the table on purpose; see that entry's doc). A pre-existing,
  unmarked copy previously reported frozen with no remedy to paste in; it now gets the real `#`
  header, the same way `Makefile`/`go.mod`/`Rakefile` already do.

- **`[e2e.call(s).*.overrides.<lang>] result_type` is now validated against the core IR,
  mirroring the `class` validation added in 0.62.2.** `result_type` names the struct/enum
  type a call's result binds to, and — for the `c` generator specifically — the value is
  baked verbatim into accessor/free symbols. Nothing checked it against the IR before it
  reached the emitter, so a typo surfaced late: either as uncompilable generated C, or as a
  wall of per-call "call did not resolve to a core IR function..." warnings once the
  misconfigured call's return type couldn't be derived any other way, both naming
  `result_type` as the fix. Generation now fails fast at config-validation time instead,
  with a did-you-mean suggestion against the type/enum names the crate actually declares. A
  `result_type` set to a primitive/pointer C spelling (`char*`, `int32_t`, ...) — a
  different misuse, where `raw_c_result_type` or `result_is_bytes`/`result_is_simple` was
  the field that belonged there — is now reported as its own warning rather than being
  silently accepted or conflated with an unknown-type error.

### Changed

- **Demoted `tracing::warn!` sites that fire on correct, working configurations to `info!` or
  `debug!`.** A triage of the 229 `warn!` call sites in `alef` found a set that were not
  reporting problems: an `[e2e]` block being detected (advice, not a defect), the CLI running
  newer or older than a project's pinned `alef_version` (expected after every release until a
  consumer bumps the pin), suppressed validation diagnostics re-printed despite
  `suppress_validation_codes` (now `debug!`, since re-warning defeats the consumer's own
  setting), a `precondition` skip (the consumer's own declared skip switch), an optional command
  failing or missing (the command is declared optional), the eleven "repaired pre-existing
  `<file>`" self-heal announcements (all fire only after the repair already succeeded), and the
  Swift artifactbundle build/checksum steps on hosts without Xcode. These no longer drown the
  warnings that matter in `generate`/`adopt`/`verify`/`diff` output.

### Removed

- **Removed three provably-unreachable code paths.** `check_signature_breakage`'s "no
  consumer-file scan is wired up" warning could never fire: every backend except Zig defaults
  `public_function_signatures` to empty, so the baseline comparison always short-circuits before
  reaching it for those languages, and Zig always has a non-empty `scan_extensions_for` entry, so
  its changes always take the attributed-caller branch instead. `ValidationReport::warnings()` was
  always empty because every diagnostic pushed into a `ValidationReport` is built with
  `ValidationDiagnostic::error`; the pipeline's own warning diagnostics travel through a separate
  `language_diagnostics` vec. Removed the dead `warnings()` method and its two always-empty
  iteration sites. The C# backend's `callback_specs_from_trait` (and its private
  `snake_to_lower_camel` helper and `CallbackSpec`/`ExtraParam` types) was only ever called from
  its own `#[cfg(test)]` module; removed the function, its helper types, and the test that only
  exercised it.

## [0.62.2] - 2026-08-19

### Fixed

- **C# no longer reports files as unemitted that the same run emitted.** The visitor-support check
  tested only whether a path existed on disk, and ran before the type and enum emitters had pushed
  anything. In the branch where visitor callbacks are off — which includes a consumer having no
  `[ffi]` section at all, since an absent section is indistinguishable from an explicit `false` —
  the candidates are `{context_type}.cs` and `{result_type}.cs` taken from `[[trait_bridges]]`, and
  those emitters go on to write exactly those files. Every `generate`, `adopt`, `verify` and `diff`
  on such a repo therefore warned about files it had just written. The check now runs after
  emission and excludes anything the run is actually writing.

- **Snippet validation no longer serializes languages that have nothing to serialize.** The
  per-snippet fallback runs inside a rayon pool, but every snippet took its session's mutex and
  held it across the whole toolchain subprocess, making the pass strictly serial per session. The
  mutex was introduced alongside the change that moved TypeScript, C# and Java onto a shared
  fingerprint-keyed workspace with fixed filenames (`snippet.ts`, `Program.cs`, `<Class>.java`),
  where concurrent snippets really would overwrite each other's sources mid-compile — but it was
  applied to every language, including the majority that write only into a per-call scratch
  directory. Validators now declare their own need via `requires_session_exclusivity`, and only
  those four (plus Kotlin, whose Gradle init script shares the workspace) are serialized. On a
  consumer repo this was 521 zig snippets of ~6s each running for half an hour.

- **`Starting per-snippet validation` no longer announces work that never happens.** The count came
  from the batch pass leaving an entry unclaimed, which conflates "not batched" with "will be
  validated". `validate_one` short-circuits on a cache hit, a `skip` annotation, a side-effect
  rejection, a missing validator and an unavailable toolchain. Because snippet validation runs once
  per crate with `changed_only`, later passes are almost entirely cache hits, so fully-cached
  languages reported `snippet_count=521` while doing nothing at all — making a run in which four
  languages fell back read as thirteen. The event is now emitted from the point where a toolchain
  is actually invoked, and the summary reports `resolved_without_toolchain` alongside it.

- **Per-snippet `duration_ms` no longer includes time queued behind other snippets.** The elapsed
  timer started before the session lock was acquired, so recorded durations were dominated by wait
  time — zig snippets doing ~5.9s of real work were recorded at a 58s median, which disguised the
  serialization above as per-invocation cost.

- **Generated C snippets no longer call a `_from_json` constructor the FFI never exports.** For an
  argument like `Vec<String>` the element type resolves to the std type `String`, and the e2e C
  generator built a typed handle from it — emitting `<prefix>_string_from_json("[]")` and a
  matching `<prefix>_string_free(...)`. The FFI crate exports `_from_json` / `_free` only for types
  the crate itself defines (and, for enums, only when one is used as a pointer parameter), so
  nothing declares those symbols and every snippet taking such an argument failed to compile with
  "call to undeclared function". The C ABI takes the argument as a plain `const char *` JSON string
  anyway, so std-typed arguments now skip the handle and are spliced in as a literal. Crate-defined
  types, including enums, are unaffected.

- **Bumping the `[workspace] alef_version` pin in `alef.toml` no longer rehashes every generated
  file.** `compute_inputs_hash` folded the entire normalized `alef.toml` into the embedded
  `alef:hash:` line, including the `alef_version` pin — so the standard consumer upgrade workflow
  (bump the pin) invalidated every file's hash with zero emitted-content change. The pin only
  feeds a version-mismatch warning (`cli::version_pin::check_alef_toml_version`); nothing in
  codegen branches on it. `alef_version` is now stripped from the canonical TOML before hashing;
  every other `[workspace]`/`[[crates]]` key is still a real input.

- **`[crates.e2e.call(s).*.overrides.<lang>] class` is now validated against the classes the
  target backend actually emits.** For java, kotlin, kotlin_android, php, ruby, and dart — the
  languages whose e2e generators read this field — a typo or a stale rename used to be trusted
  blindly by the emitter, silently producing hundreds of e2e tests and snippets that call methods
  on a class that does not exist, surfacing only as a wall of compile errors in generated code far
  downstream. `alef e2e` (and `alef build`) now checks every `class` override against the crate's
  facade class, every struct/enum wrapper, and every active trait bridge for that language, and
  fails generation with the offending config key, the bad value, and the closest valid
  candidate(s) by edit distance. The check is skipped when the caller supplies no IR (some
  legitimate generation paths do), matching the same rule the field-classification validator uses.

### Removed

- **Dropped the wall-clock companion to the subprocess-backoff test.** It timed 20 trivial commands
  and asserted the amortised cost stayed below the fixed interval the backoff removed, but bare
  process-spawn overhead on a loaded machine exceeds that bound, so it failed on load rather than
  on regression at two successive thresholds. The sibling test asserts the poll schedule directly
  and covers the same property without depending on machine load.

## [0.62.1] - 2026-08-19

### Fixed

- **`alef validate versions --json` no longer fails a release on a check that only the release can
  satisfy.** A test app's lockfile pins the crate at the version being published and resolves it
  from the registry, so cargo cannot refresh that entry until the version is live — which cannot
  happen while this gate blocks the publish job. alef already recognised the situation (the check
  is tagged `UNPUBLISHED`, and the human summary reports it as "unresolvable until the pending
  release is published"), but the JSON `ok` field still counted it as a failure, and
  `xberg-io/actions/validate-versions` fails on `ok != true`. The gate was therefore unsatisfiable
  by construction for any repo with a registry-depending test app, and it blocked the crates.io leg
  of a real release. `blocked_on_publish` checks are now excluded from `ok` while still being
  reported; a genuine mismatch sitting beside one still fails, and an empty check set is still not
  a pass.

## [0.62.0] - 2026-08-19

### Fixed

- **e2e suites no longer contain error assertions that can never pass.** A fixture's declared
  `error` value is either a message substring or an error variant name, and every backend rendered
  the same message-or-type-name disjunction for both. That serves the first convention and is
  structurally unsatisfiable for the second: the message is lowercase `#[error(...)]` prose that
  never contains the PascalCase identifier, and the "type name" side is a generic exception class
  the binding never differentiates per variant. Measured across two consumer repos this was the
  single largest class of e2e failure alef generates — Go 47/162, Java 47/162, PHP 47/162, Dart
  47/127, C# 45/162, Ruby 45/47, C 44/162, Zig 45. `declared_error_variant::classify` is now the
  one place that decides substantiability: Go, Java and Zig can still assert a variant that
  carries an `error_code`, and the backends that cannot emit a registered skip instead. Nothing is
  matched fuzzily to force the type-name side to work, and unaudited backends keep their existing
  behaviour.

- **The subprocess-polling regression test no longer fails under load.** It asserted that 20
  trivial commands finish in under *half* the 1s the old unconditional 50ms-per-subprocess sleep
  cost. That sleep ran before the first `try_wait` regardless of command speed, so any amortised
  cost below 50ms/command already proves it is gone on any machine at any load; the extra halving
  proved nothing and instead measured process-spawn overhead, which legitimately reaches 25ms+ per
  command on a loaded machine and failed the suite at 509ms and 527ms against a 500ms bound.

- **R bindings no longer drop every feature-gated function outright.** `extendr_module!` rejects a
  `#[cfg(...)]` on its entries ("expected mod, fn or impl"), so R cannot gate a registration the
  way Magnus gates its `define_module_function` call. The workaround was to exclude any genuinely
  cfg-gated function from both the registration block and the R wrapper surface — unconditionally,
  whether or not the feature was actually enabled — so a crate with a cfg-gated function could
  never expose it through R even in the default build, silently. The predicate is now resolved
  before generation, exactly as the field policy beside it already did: an enabled function
  reaches R with its gate discharged, and a disabled one is removed outright so no
  `extendr_module!` entry or wrapper can name a symbol the crate never compiled.

- **Ruby bindings no longer fail to build when a function is feature-gated.** `prepend_cfg` put
  `#[cfg(feature = "X")]` on the generated `fn`, but the registration loop in `gen_module_init`
  never read `func.cfg` and emitted the `module.define_module_function(..., function!(...))` line
  unconditionally. With the feature off the definition compiles out while the registration still
  names it, so the binding crate fails with `E0425` — a broken build, not a missing Ruby method.
  `#[magnus::init]`'s body is a flat statement list, so the attribute on the registration
  statement is the only place the gate can go; the method loop directly above already did this
  via `method.cfg`.

- **`#[serde(untagged)]` data enums no longer lose their payload in WASM bindings.** `gen_enum`
  special-cased internally-tagged data enums only. An untagged one has no serde tag, so it fell
  through to the fieldless path and was emitted as a bare discriminant enum with every variant's
  payload replaced by `Default::default()`; the containing struct's setter then took that
  fieldless enum, so no JS caller could supply the string or array the variant actually carries.
  These now bridge to `JsValue` through `serde_wasm_bindgen` — the mechanism this backend already
  used for internally-tagged data-enum fields — so wasm-bindgen emits `any` for the property and
  the `.d.ts` and the runtime setter agree by construction. NAPI already gated on the same
  `EnumDef::serde_untagged` flag. Affects `EmbeddingInput`, `ModerationInput`, `StopSequence` and
  `ToolChoice` in `liter-llm`.

- **The NAPI `.d.ts` can no longer contradict itself about a type's name.** `dts_type` and its
  siblings each took a `prefix: &str` that all eleven-plus call sites had to remember to pass as
  `""`; passing the real prefix at any one of them emits `Array<JsMessage>` against a type
  declared as `Message`. NAPI-RS wraps `Foo` as `JsFoo` in Rust and maps it back via
  `#[napi(js_name = "Foo")]`, so the `.d.ts` — which describes the JS boundary, not the Rust
  struct — must use the identity name everywhere. `codegen::naming::node_type_name` is now the one
  place that decides this and the parameter is gone, so the declaration site and the reference
  site cannot drift.

- **Two Dart e2e regression tests no longer assert a naming convention alef deliberately
  dropped.** `b5808da3c` stopped emitting leading-underscore Dart locals — Dart privacy is
  library-scoped, so the prefix carried no meaning and only tripped
  `no_leading_underscores_for_local_identifiers`, failing 188 of 207 published snippets under the
  `dart analyze --fatal-infos` that alef itself runs. That commit updated
  `e2e_dart_client_factory.rs` but missed `e2e_generic_call_recipe.rs` and
  `e2e_unified_extract_input_args.rs`, which kept expecting `_settings`/`_input`/`_config`. The
  generator was right and the expectations were stale; both now assert the lint-clean names. The
  same test file also carried a real consumer project name through its fixture config and all
  three language assertions, which `project-agnostic-codegen` forbids — renamed to a neutral
  fixture identity.

- **Generated Go e2e files no longer carry an unused `strings` (or `os`) import.** Go rejects an
  unused import outright, so `tree-sitter-language-pack`, `liter-llm` (6 files) and `crawlberg`
  (3 files) all had e2e suites that could not compile. Both flags are fixture-level heuristics —
  "some assertion is of a kind that might want this package" — and deliberately a superset, since
  an assertion can be skipped, degraded to a stub, or rendered without ever naming the package.
  They were OR-ed with the rendered body rather than narrowed by it, so the heuristic alone forced
  the import; they now match how `needs_fmt` and `needs_pkg` on the adjacent lines already
  authorise themselves against the body they actually produced.

- **Whitespace-control tags no longer eat the indentation of generated e2e code.** The e2e
  template environment already sets `trim_blocks` and `lstrip_blocks`, so a plain `{% %}` tag
  strips exactly the one newline that separates it from its content. An explicit `-` on top of
  that strips *all* whitespace to the next non-whitespace character, which deletes the emitted
  statement's own indentation and, on a `{% for %}`/`{% endfor %}` pair, the newline between
  iterations. Two of the results were not cosmetic: `python/app_harness.py.jinja` glued an
  assignment onto the preceding comment line, so `_config` was never defined and the next
  statement raised `NameError`; and `r/test_case.jinja` concatenated setup lines into
  unparseable R (`x <- 1res <- foo(1, 2)`). `csharp/http_test_open.jinja` and
  `java/http_test_open.jinja` also turned out to be a second path that collapsed `[Fact]`/`@Test`
  onto the method signature, distinct from the `test_method.jinja` path fixed in 0.61.1. Redundant
  `-` modifiers are removed across the csharp, java, php, ruby, typescript, swift, go, r, python
  and elixir templates, with layout tests pinning the exact emitted indentation for the C# and
  Java assertion templates.

- **Generated e2e assertions now read a field's optionality from the IR instead of a
  hand-maintained config table.** `FieldResolver`'s `optional_fields` was populated only from
  the consumer's `[crates.e2e] fields_optional` list in `alef.toml`, never from `FieldDef.optional`
  — which extraction already sets correctly, and which every language backend already consults to
  wrap the field in that language's optional type. A consumer that declared no `fields_optional`
  therefore got assertions that dereference an `Option` directly: `assert!(result.data, "expected
  true")` against `pub data: Option<DataNode>` does not compile, and the equivalent in twelve other
  backends either fails to compile or is silently false at runtime. `FieldResolver::ir_field_sets`
  now also derives an optional-field set and `with_ir_fields` merges it into the config-declared
  one, so `fields_optional` remains an override for what the IR cannot see rather than the only
  source of truth. Derivation is deliberately unanimous — a bare field name counts as optional only
  when *every* type declaring it marks it `Option<T>` — because a false positive here emits code
  that does not compile, whereas a false negative merely reproduces the previous behaviour.
  Alongside it, `is_true`/`is_false` now mean "present"/"absent" for an optional field in rust, go,
  java, python, kotlin, kotlin_android, dart, swift, php, ruby, elixir, typescript and zig, matching
  the convention the Rust backend already used; csharp and c were already correct. The shared
  doc-snippet path (`e2e::codegen::presentation::resolve`) is wired to the same IR data, so a
  snippet showing an optional field renders the same unwrap an assertion on it would.

### Added

- **`alef build` warns when a binding crate never declared a feature its generated source
  references.** `scaffold` computes each binding crate's `[features]` table once and `alef build`
  never revisits it, so a cfg-gated symbol added to the core crate after the last scaffold run
  resolves against a feature the binding crate does not declare — false, unconditionally. For Ruby
  that now surfaces as a build error; for Elixir nothing surfaces at all, because `#[rustler::nif]`
  gates a definition and its registration atomically, so the NIF is simply absent while the
  generated facade still advertises it. `warn_on_undeclared_binding_cfg_features` reads the
  scaffolded `Cargo.toml` back off disk and names the missing features. A warning rather than a
  hard error or an automatic rewrite: `Cargo.toml` is scaffold-owned and written once by design.

## [0.61.1] - 2026-08-19

### Fixed

- **`cargo publish` runs again.** The publish workflow gated every downstream job on a
  `validate-versions` step that ran `alef validate versions` against alef itself. That check
  exists to confirm a *consumer's* package manifests agree with the crate version; alef is the
  generator — a single Rust crate with no target-language packages — so it had nothing to
  validate, and crate resolution rejected the config outright with "crate `alef` has no target
  languages". Because `publish-crates` required `needs.validate-versions.result == 'success'`,
  the failed gate skipped the actual publish while the release itself still looked created. That
  is why 0.61.0 never reached crates.io, and why the last version published there was 0.60.1.
  The job and the fictional `[[crates]]` block in `alef.toml` that had been added to satisfy it
  (b00e72d60) are both removed.

- **The xUnit attribute on a generated C# e2e test no longer collapses onto the method
  signature.** `test_method.jinja` picks between `[Fact]` and `[Fact(Skip = "...")]` in a
  conditional; written with whitespace-trimming delimiters, that block also ate the newline after
  the attribute, emitting `[Fact]    public void Test_X()`. The result still compiled, so nothing
  failed -- every generated C# e2e suite simply carried the mangled line. Regression from 0.61.0,
  now pinned by tests that assert the attribute occupies its own line in both branches.
- **A struct field defaulted only through `<FieldType>::default()` now resolves to a concrete
  value instead of forcing every generated language binding into an unconstructible `required`
  member.** The extractor folded `SomeEnum::default()` (and `Default::default()`) to
  `DefaultValue::Empty` regardless of the field's own type, which is correct for a primitive,
  string, or collection field but ambiguous for an enum-typed one -- `Empty` names "the type's own
  zero" without saying which variant that is. A new postprocess pass,
  `extract::extractor::postprocess::resolve_enum_field_defaults`, narrows `Empty` on an
  enum-typed field to `DefaultValue::EnumVariant` when the enum's default variant is known. To
  make it known in the hand-written case, `impl Default for SomeEnum { fn default() -> Self {
  Self::Variant } }` is now read directly and its variant marked `EnumVariant::is_default`;
  previously only `#[derive(Default)]`'s `#[default]` attribute set that flag, so every consumer
  of it (the Go, Rustler, Dart, WASM, Kotlin, Magnus and PHP backends, and the generated Rust
  mirror enum's `#[default]` marker) silently fell back to the first declared variant or to no
  default at all. Only a bare unit variant is narrowed: a tuple or struct variant needs
  `TupleVariant`/`StructVariant` and a payload this pass cannot read, and emitting a bare variant
  name for one would fabricate a value that does not compile. An enum whose default variant stays
  unknown is left `Empty`, preserving every backend's existing honest fallback.
- **C# no longer emits a `required` member for a struct-typed field whose nested record is itself
  fully default-constructible.** A field defaulted only by a container-level `impl Default` (no
  per-field `#[serde(default)]`, so the sole signal is `Empty`) now reuses the existing
  `record_is_default_constructible` walk rather than falling through to `required`. A nested
  record that carries a `required` member of its own still correctly keeps the outer field
  `required`. Together with the fix above this resolves a real regression: a record with several
  enum fields and a nested-struct field, each defaulted only via `T::default()`, emitted a
  `required` member for every one of them, making a bare `new Record()` -- exactly what alef's own
  snippet generator emits for a type with no constructor arguments -- fail to compile with
  `CS9035` on every generated snippet that touched the type.
- **Kotlin/Android snippet validation resolves a real Gradle classpath instead of guessing a
  directory layout.** `KotlinValidator::class_path` probed exactly three fixed directories
  (`build/classes/kotlin/main`, `build/classes/java/main`,
  `build/intermediates/javac/debug/classes`) plus `build/libs/*.jar`, falling back to the project
  root when none existed. AGP's actual compiled-output path is variant- and version-dependent (AGP
  9.x lands classes at `build/intermediates/built_in_kotlinc/debug/compileDebugKotlin/classes`,
  matching none of the three probes), and directory probing can never see a project's *dependency*
  classpath at all -- so every snippet touching a dependency-typed symbol (kotlinx-coroutines,
  Jackson DTOs, ...) failed with `unresolved reference`, and the fallback-to-root path made every
  other snippet fail too. A Gradle manifest (`build.gradle.kts` / `build.gradle`) with a `gradlew`
  wrapper is now resolved by asking Gradle itself, via a `--init-script` that matches every
  `compile*Kotlin` task by name and prints its destination directory and resolved classpath --
  no consumer build-file change required. The resolution is cached per manifest for the process
  lifetime, since batch validation calls it once per batch and a Gradle invocation costs whole
  seconds even warm. A Gradle invocation that fails still falls back to the original directory
  probing rather than failing the session outright.

- **Zig snippets stop rebinding a discarded value that is not a call.** The generator rewrites a
  statement-opening `_ = <call>(...)` into `const result = ...` so the snippet can show its result,
  but the rule matched any `_ =` discard. Every generated visitor callback opens by discarding its
  unused typed parameters (`_ = _ctx;`, `_ = _user_data;`, `_ = out_custom;`), and those lines
  precede any real call in a visitor body, so the first one became `const result = _ctx;` -- a bound
  value nothing reads, which Zig rejects outright as an unused local constant. A call discard is
  syntactically distinct from a bare-identifier discard, carrying a parenthesised argument list, and
  the rule now requires one.

- **Snippet generation honours a language's visitor exclusion, as e2e test generation already did.**
  A per-language `exclude_functions = ["visitor"]` drops the fixture engine's trait-bridge entry point,
  and `e2e::codegen::kotlin_android::project` already fell back to an excluded-bindings placeholder for
  it. The snippet generator applied no such rule, because `exclude_functions` normally names a real Rust
  function while a visitor fixture's *call* resolves to an ordinary one (`convert`) and the visitor
  itself attaches through an options field that has no IR function name of its own. The two generators
  therefore disagreed about the same config, and every visitor fixture was rendered as a real snippet
  against an API the binding never exposed -- 46 of them for one consumer, each importing a visitor
  interface, a node-context type and a result enum that are absent from the generated package. The token
  is now a single named constant both generators read, so they cannot drift on which fixtures an
  exclusion covers.

- **One binary file no longer ends an `alef adopt` run.** Candidate collection read every match with
  `read_to_string`, so a single non-text match aborted the whole target: `alef adopt 'packages/**'`
  on a repo with a `gradle-wrapper.jar` failed with `stream did not contain valid UTF-8` before one
  of the hundreds of adoptable text files under the same glob was stamped, and no narrower target was
  suggested. Binary matches are now collected separately and reported under a `NOT ADOPTED -- not text`
  heading. They are still never adopted: a drifted path is only ever adopted after its diff is printed,
  and a binary artifact has neither a diff to review nor a syntax that could hold a provenance marker.

- **C# `[DllImport]` parameters now derive their width from the same fact as the emitted C signature.**
  `marshalling::pinvoke_param_type` mapped every `TypeRef::Named` to `ulong`, but the C FFI backend
  narrows a `Named` parameter whose type is `Copy` to `i32` — cbindgen renders that `int32_t`. A `Copy`
  enum parameter was therefore declared eight bytes wide against a four-byte signed argument, and the
  wrapper's own `(int)` cast (`named_param_enum_required.jinja`) could not even be passed to it, so the
  generated package failed to compile with `CS1503: cannot convert from 'int' to 'ulong'`. Casting the
  argument to `ulong` would have silenced the compiler while cementing the ABI violation; instead the
  scalar/handle split is now constructed once, in `backends::ffi::type_map::scalar_c_abi_named_types`,
  and read by the C FFI backend, both P/Invoke emitters and the service-API emitter. The C# service-API
  path additionally stopped keying that decision off enum-ness, which disagreed with the C header for a
  non-`Copy` enum (boxed as a handle) and for a `Copy` struct.
- **C# `bool` parameters cross the C ABI as `int32`, not a one-byte managed `bool`.** The free-function
  and method P/Invoke emitters declared `[MarshalAs(UnmanagedType.U1)] bool`, which marshals one byte,
  while the C FFI crate declares `i32` and cbindgen emits `int32_t`. The callee reads four bytes, so the
  upper three were whatever the calling convention left there — reachable in practice for any argument
  passed on the stack rather than in a register. Every other boundary in the same backend already
  agreed on `int` (trait-bridge delegates, the service-API map, and the `bool` *return* mapping), so the
  P/Invoke declaration was the outlier; the wrapper now passes `(value ? 1 : 0)` to match.

- **`alef snippets check --lang` accepts the session names a user actually has.** The filter resolved
  its values as fence tags only, so every session target whose name differs from its fence tag —
  `kotlin_android`, `node`, `wasm`, `c_ffi` — was rejected as unrecognised. Those names are the only
  ones a consumer has for those sessions, because they are the keys of the `[workspace.docs.snippets.sessions]`
  table they were just reading, and the rejection meant the one language they most needed to narrow to
  could not be selected at all. Session targets and their `-`/`_` spellings now resolve alongside fence
  tags, aliases of one language collapse to a single entry, and the error names the values it could not
  resolve instead of listing the ones it could.

- **Zig packages no longer search an FFI include directory guessed from the crate name.** `scaffold_zig`
  started deriving the `-Dffi_include_path` default from `[crates.output] ffi`, but `packages/zig/build.zig`
  is a create-once seed, so every repo scaffolded before that kept searching `crates/<crate-name>-ffi/include`
  forever. Where that is not the real FFI crate directory the binding's `@cInclude` never resolves and
  `zig build` — along with every generated Zig documentation snippet — fails with `C import failed` /
  `'<header>.h' not found`. A migration now repairs the default in place, and only when it still matches
  the crate-name-derived shape, so a consumer who repointed the option keeps their value.
- **Generated `build.zig` resolves its FFI library and include defaults against its own build root.** Both
  are attached with `.{ .cwd_relative = ... }`, which zig resolves against the invoking process's working
  directory, so the raw relative defaults only found anything when zig ran from inside `packages/zig`:
  `zig build --build-file packages/zig/build.zig` from the repo root failed to open `../../target/release`,
  and consuming the package as a `.path` dependency — which is exactly how the Zig snippet validator builds
  it — failed to find the header. The Zig snippet validator reads the rebased binding back correctly, and
  now resolves manifest-declared include paths against the manifest's own directory rather than the
  session's working directory.
- **`alef adopt` no longer lets one unusable target cancel every remaining one.** `run` bails whenever a
  target resolves to nothing adoptable — no match, or (far more common on a repo-wide sweep) only
  create-once seeds — and that error propagated straight out of the per-target loop. A single `config.m4`
  early in a sorted list of 54 refused paths therefore ended the command before one file was stamped,
  reporting only that path. Each target is now reported independently and the run fails at the end iff
  any did.

### Added

- **`alef snippets check --lang <tag>`** validates only the named languages. Diagnosing one backend's
  snippets previously meant paying for all of them: a full consumer tree is thousands of snippets across
  sixteen toolchains. The audit and gap passes still see the whole corpus, because an unreferenced snippet
  or a missing language variant cannot be judged from a subset.

### Changed

- **A batched invocation's timeout scales with the number of snippets it covers.** `timeout_secs` is a
  per-invocation budget, and while only Rust batched a "batch" was a handful of snippets. Now that one
  `tsc` or `dotnet build` covers a language's several hundred, a flat grant would kill the group as a
  toolchain timeout long before the compiler finished — a failure mode batching itself introduced. The
  grant stays far below the serial path's total, since paying one startup instead of N is the whole point.
- **Snippet batch groups and session preparation now run concurrently.** Batch groups holding different
  session locks were dispatched from a plain sequential loop, so the whole pass cost the sum of every
  language rather than the slowest one; group results are now merged back at their snippet positions
  after concurrent dispatch. Session preparation was likewise serial — sixteen languages meant sixteen
  `pnpm build` / `mvn package` / `cargo build --release` hooks back to back before a single snippet was
  validated. Preparation now parallelizes across distinct working directories only, because two sessions
  sharing one must not run their `before` hooks at the same time, and the scratch purge still runs
  strictly between the resolve and activate phases (it needs the complete set of live fingerprints).
- **The Rust snippet batch reuses a persistent, fingerprint-keyed `CARGO_TARGET_DIR`.** It allocated a
  fresh scratch directory per run with no target directory set, so `cargo check` recompiled the path
  dependency and its entire transitive tree from cold on every single run.
- **`session_fingerprint` no longer hashes build output.** Its exclusion list covered six directories and
  missed `dist`, `bin`, `obj`, `_build`, `vendor`, `Pods`, `.gradle`, `.next`, `.dart_tool`, `.zig-cache`
  and `__pycache__`, so a repo with built artifacts hashed hundreds of megabytes per session per run.
  Hashing is now parallel over a path-sorted file list, which keeps the digest stable across runs — a
  fingerprint that varies invalidates the whole cache silently.
- **Subprocess waiting backs off from 1ms instead of polling on a fixed 50ms sleep**, removing ~25ms of
  pure sleep per subprocess. Timeout semantics are unchanged: the final sleep is clamped to the remaining
  budget.
- **The docs snippet pass reads the cache it writes.** It built its `RunnerConfig` from a default with
  `changed_only: false` while setting `cache_dir`, so every run wrote an entry per snippet and read none
  back — a guaranteed 100% miss.
- **C, Dart, Elixir, PHP, R, Ruby and Zig snippets validate in one invocation per language**, closing
  the batching sweep: `cc -fsyntax-only` and `dart analyze` take every file at once, and Elixir, PHP,
  R and Ruby each run one interpreter over a checker script that reports per file. Two toolchain
  findings drove the shape: `ruby -c a.rb b.rb` checks only the FIRST file and hands the rest to the
  script as `ARGV`, so a broken second file passed silently; and `zig ast-check` refuses a second
  path outright, so the Zig batch goes through `zig fmt --ast-check`. Levels that link an executable
  decline and fall back, since each `main` needs its own artifact.
- **Swift deliberately does NOT batch.** `swiftc` compiles one module per invocation and a module
  permits top-level code in exactly one file, so `swiftc -parse a.swift b.swift` fails the other
  snippets with "expressions are not allowed at the top level" before judging their own code. There is
  no way to scope a snippet's top-level statements into its own namespace, so batching would fail
  snippets for a reason the per-snippet path never had.
- **Java, Kotlin and C# snippets validate in one compiler invocation per language instead of one per
  snippet.** Java and Kotlin collapse a JVM startup per snippet into one; C# collapses a `dotnet build`
  per snippet into one. Measured on 20 snippets: `javac` 5.44s to 0.23s, `kotlinc` ~76s to 4.86s,
  `dotnet build` 15.7s to 1.36s against an already-warm project directory. Each snippet is given a unique
  synthetic package/namespace so two snippets declaring the same top-level name cannot fail each other —
  a batch is declined outright when two snippets declare the *same* explicit package, since that
  collision is real and only the compiler can tell it apart from the synthetic case. One consequence is
  deliberate: entry-point-ness is a project-level property in C#, so a batched project builds as a
  library and a snippet that used to fail `CS5001` (no static `Main`) now passes.
- **TypeScript, Python and Go snippets validate in one compiler invocation per language instead of one per
  snippet.** Only Rust batched before; every other language paid a full `tsc` / interpreter / `go build`
  startup per snippet, and all of a language's snippets were serialised behind its session lock, so the
  cost was fully additive. On a consumer tree with ~283 snippets per language this is 283 processes to 1
  at every non-`Run` level. `Run` still validates per snippet by design — each one's stdout, exit status
  and side effects belong to it alone. Batch diagnostics are attributed back per file, and a compiler
  failure that no file owns fails every snippet in the batch with the real output rather than passing them.
- **Java snippets call a class in the imported package by its simple name.** The snippet emits
  `import <package>.*;` and then spelled the package again at the call site, rendering
  `io.example.pkg.Facade.convert(...)` under an import that made `Facade` alone sufficient. Only the
  exact configured package is stripped; a nested or foreign class stays qualified, since no import
  covers it.
- **Python snippets omit a trailing `None` for an absent optional argument.** `convert(html, None)` now
  renders as `convert(html)`, matching the binding's own `options=None` signature. A placeholder in the
  MIDDLE of the argument list is still emitted — these calls are positional, so dropping one there would
  slide the following argument into the wrong slot.

- **`alef adopt` and `alef verify` render the managed surface in parallel.** Every stage reads the same
  IR and config and returns owned files, so rendering them concurrently is safe; absorption stays strictly
  sequential because `absorb_stage` is last-wins on a path collision and the fold order is what decides
  which stage owns a contested path. The two e2e stages alone emit several thousand files each on a full
  consumer tree, which was most of what made a single-path `alef adopt` cost half a minute.

- **An uppercase `.R` script now receives the generated header.** `.R` is the conventional extension for an R
  script and alef emits `install.R`, `run_tests.R` and every `packages/r/R/*.R` with `generated_header: true`, but
  the emit predicate matched a lowercase `"r"` only — so every one of them was written unstamped and then frozen
  by the write guard for want of a marker nothing had been emitting. Extension matching is now case-folded on the
  emit side only; the ownership predicate is deliberately unchanged, because reclassifying `.R` from unmarkable to
  markable would retroactively freeze every already-committed `.R` that proves ownership through the record.

- **A stripped `# Arguments` bullet no longer leaks its own continuation lines into the reference
  page.** The two arms that recognise a wrapped bullet tested the TRIMMED line for leading
  whitespace, which cannot match by construction, so the skip ended at the first wrapped line and
  published the bullet's tail — mid-sentence, with no heading above it — into every generated
  reference page.
- **Doc-comment sections nest under the item that owns them.** Function, error, enum and streaming
  pages shifted rustdoc headings by a fixed number of levels, which assumes the doc comment starts at
  `#`. A `# Observability` section under a `####` item surfaced as `###`, reading as a sibling of the
  page's `### Functions` section and taking a bogus table-of-contents entry with it. Each now demotes
  so the first heading starts one level below its own parent.
- **A C# snippet's visitor class is emitted at file scope without the e2e test class's nesting
  indent**, and the batch validator finds the statement/declaration boundary by brace depth rather
  than by column. Either half alone left the class inside the wrapper method, where C# does not allow
  one: 54 of one consumer's 283 C# snippets failed on `CS1513: } expected`.
- **Zig snippets no longer rebind the allocator teardown.** The rewrite that names the discarded call
  result ran per line with no guard, so it also matched the teardown every snippet emits:
  `defer _ = gpa.deinit();` became `defer const result = gpa.deinit();`, which no Zig grammar accepts.
  It failed 54 of one consumer's 283 Zig snippets on `expected block or expression`. The rewrite now
  requires the discard to open the statement, and applies once per body.
- **The generated `build.zig` resolves its FFI search paths against its own build root.** Both were
  attached with `.{ .cwd_relative = ... }`, which resolves against the invoking process's working
  directory — so the package built correctly only when invoked from its own directory, and failed as a
  `.path` dependency or from the repo root.
- **A stale `-Dffi_include_path` default is repaired in place.** `build.zig` is create-once, so a repo
  scaffolded before the default was derived from `[crates.output] ffi` kept a path guessed from the
  crate name (`crates/<crate>-ffi/include`) that alef could never correct. The migration fires only when
  the on-disk value still matches that guessed shape and differs from what this run generates; a
  consumer who repointed the option keeps their value.
- **A Go visitor fixture attaches its visitor to the options value the call already binds.** The
  generator unconditionally introduced a second `opts` object, which was wrong twice over: the call then
  carried both bindings — `Convert(html, &options, opts)`, a hard "too many arguments" from the Go
  compiler, because the substitution helper only recognised a literal trailing `nil` and appended in
  every other case — and the fresh empty object silently discarded whatever options the fixture had
  configured.
- **A snippet result that comes back `Unavailable` is now reported per language, with the validator's
  message.** Only `Fail` and `Error` were tallied, but `Unavailable` fails the run under `strict`, and the
  `unresolved_dependency` reclassification turns a real validator failure — diagnostic and all — into one.
  That is how 566 snippets across two languages reached the final summary as "283 unresolved dependency"
  apiece without one line anywhere saying *which* dependency, while the message sat unread on every
  result. A language whose every result came back unvalidated also no longer logs like a clean pass.
- **Go snippets pass an absent options object by address when the binding takes a pointer.** Six of the seven
  `json_object` branches in the Go snippet argument builder consult `options_ptr`; the native-DTO branch that
  handles a fixture supplying *no* options never did. On any crate whose options parameter is `Option<T>` — a
  `*T` in the emitted Go signature — every optionless fixture, which is most of them, produced
  `Convert(html, options)` against `func Convert(html string, options *ConversionOptions)` and failed to compile.
- **Generated snippet bodies no longer open a blank line before the closing fence.** Generators hand the renderer
  a body already ending in a newline and the template emits its own, so every generated code fence carried a
  trailing blank line.

- **A subscripted `fields_optional` / `fields_array` entry is a claim about the element, not the container.**
  `validate_field_classifications` stripped the `[...]` suffix and ruled on the field it was attached to, so
  `metadata.document.open_graph[title]` — a key lookup on a `HashMap<String, String>`, optional in every host
  binding — was reported as "contradicts the core IR" and failed the whole run. The map is precisely the right
  home for an optional key lookup and the wrong home for an optional bare field; one predicate cannot judge both.
  Subscripted entries now resolve one level through the container: `Optional` clears anything subscriptable,
  `Array` requires the element itself to be indexable, and a subscript against a scalar is still an error whose
  message says so.

### Removed

- **`poly.toml` no longer schedules snippet validation as a pre-commit hook.** Snippet validation compiles every
  snippet against built language artifacts — minutes of work needing a toolchain per target language — and
  `alef all`'s docs stage already runs it against the tree it just generated. Running it again from a git hook
  made a one-line docs edit pay for a full multi-language compile. Regenerate to drop the
  `[hooks.pre-commit.commands.alef-snippets]` table.

## [0.61.0] - 2026-08-18

### Added

- **`MethodDef` carries its `#[cfg]`**: it was the only IR node without one, and extraction saw the attribute and
  discarded it. Methods now inherit their impl block's gate (AND-combined) and survive `with_cfg_filtered_deep`, so
  each backend filters against its own feature set — one surface is extracted once and handed to every backend in
  parallel, so dropping at IR level is impossible. `cbindgen_feature_defines` moved in lockstep: it is a second,
  independent feature collector, and a feature missing from `[defines]` makes cbindgen emit the declaration
  **unguarded**, which is worse than not gating. **Behavioural risk**: a language whose `features_for_language`
  omits a feature the cdylib was built with now loses those methods — the divergence `warn_on_ffi_feature_drift`
  announces. Native `#[cfg]` emission for pyo3/napi/magnus/rustler/dart, swift method filtering, and gleam are
  deliberately deferred: each needs the method and its runtime registration gated together.
- **`TypeDef::serde_container_default`**: `FieldDef::default` is populated only from per-field attributes, so a
  struct carrying `#[serde(default)]` at the type level was indistinguishable from one with no defaults at all.
- **`DefaultValue::Unresolved`**: `Empty` meant two opposite things — "the default is exactly the type-zero" and
  "the extractor could not read it". `has_default` cannot separate them (it is set for a manual `impl Default`
  too), so the distinction lives in the value. The extractor now follows `Self::new(<literals>)` delegation to
  recover real values, and refuses at validation time when it cannot, with `suppress_validation_codes` as the
  release valve. This is the shape that shipped `DetDbThresh = 0.0f` into generated C# beneath a doc comment
  reading "default: 0.3". Still conflated: a field initializer that cannot be read *inside* an otherwise-readable
  struct literal, which remains `Empty`.
- **`validate versions` distinguishes unresolvable-until-publish drift from stale manifests.** A `Cargo.lock` whose
  own manifest depends on a workspace crate *from the registry* at exactly the version being released cannot be
  refreshed at all until that release is published — cargo cannot resolve `x = "1.15.1"` while the index tops out
  at `1.15.0` — so the lockfile stays pinned to the last published version. That row used to be indistinguishable
  from a chore somebody forgot. It is still a mismatch and still fails `--exit-code`, but it now prints as
  `[UNPUBLISHED]`, its summary line reads `unresolvable until <name>@<version> is published`, and the JSON payload
  carries a `blocked_on_publish` field per check. A dependency taken by `path` is refreshable today and stays
  plain drift.

### Changed

- **`exclude_functions` is now actually enforced** — and for some consumers this removes symbols at upgrade with no
  other warning. The key has been honoured inconsistently: a downstream repo declared four exclusions in July and
  three subsequent regenerations on 0.60.x emitted the functions anyway. This release closes that gap across Go
  (`func` declarations), Java/Panama (methods, both `MethodHandle` downcalls, and the symbol-lookup and
  not-found-message entries) and Ruby (`.rbs` signatures). If your `exclude_functions` was silently a no-op, the
  named functions disappear from the generated surface on the next regeneration. That is the configured behaviour,
  but it arrives as a breaking change to the emitted API. **Why it hid for a month**: where a C ABI exposes only
  the async variant of a function, the async pair is the *only* observable test of the exclusion, so entries naming
  a non-existent sync symbol are unfalsifiable.
- **Generated binding crates now always carry a `[lints.clippy]` deny block** (`dbg_macro`, `print_stderr`,
  `print_stdout`), instead of emitting one only when a consumer declared `[crates.cargo_lints.clippy]`. Binding
  manifests are `generated_header: true` and therefore rewritten in full on every run, so an opt-in block that a
  consumer had hand-added into the `DO NOT EDIT` manifest was deleted on each regeneration — silently removing the
  logging enforcement the block existed to apply. A consumer-configured value for any of these keys still wins, so
  a crate with a real reason to relax one can. No alef template emits `println!`/`eprintln!`/`dbg!` outside
  `build.rs`, whose `cargo:` directives are natively exempt.

### Fixed

- **emitter, so a literal that had fallen behind wrote itself back over the consumer's own bump on every run — the
  reported shape was a repo on `base64 = "0.23"` being handed `base64 = "0.22"` and hand-reverting it after each
  regeneration. Before a rendered manifest is returned, each requirement is now compared against the one the
  committed manifest declares for the same crate and the higher lower-bound wins. **This makes emitted manifests a
  function of disk state, not of config alone** — deliberately, and it converges in one pass. It also means a
  consumer pinned *ahead* of what a generated shim compiles against now gets a compile error at the version they
  chose rather than a silent downgrade. The floor declines to rule rather than guess: a requirement with no lower
  bound (`*`, `<2`), an entry with no `version` key (`foo.workspace = true`, path-only deps), an unparseable
  requirement, or a manifest that is not valid TOML all leave the emitted value untouched.

- **scaffold**: the `[lints.clippy]` rationale alef stamps into every generated binding manifest now carries a
  `~keep` marker. It did not, so poly's uncomment pass — which runs *between* regenerations and strips any
  comment without a marker — deleted it, and the deletion landed in a commit that read as unrelated formatting.
  Where alef overwrites a consumer's own `~keep`-marked rationale above that block (it does; these manifests have
  no comment-preserving merge, unlike `poly.toml`), what replaces it is now at least as durable as what it
  displaced. This is the mirror image of 0.61.0's fix for alef *leaking* `~keep` into generated output, and does
  not collide with it: `strip_internal_doc_markers` runs only inside `normalize_rustdoc`, on doc comments harvested
  from a consumer's Rust source, never on scaffold-emitted TOML. The literal also no longer carries trailing
  whitespace on every line, which made the in-memory `GeneratedFile` content disagree with the whitespace-normalised
  bytes on disk.
- **renovate**: the `customManager` regex matched `pub const [A-Z_]+`, which excludes every constant whose name
  contains a digit. `PYO3` and `PYO3_ASYNC_RUNTIMES` were therefore never bump-proposed and had no way to be — and
  a const is indistinguishable, from outside, from one nobody has needed to bump. `base64` and `jni` are hoisted
  out of `scaffold::languages::jni` into `template_versions.rs` in the same change, which only means anything
  now that the regex can see them. `phpunit/phpunit` and `guzzlehttp/guzzle` remain unreachable on purpose: their
  rationale comments sit between the marker and the const, and their compound `||` constraints span several majors
  deliberately, so an auto-bump would collapse exactly what they exist to express.
- **records**: indent `.alef-toml-merge-provenance.toml` arrays like `.alef-ownership.toml`. The two records sit
  side by side in a consumer's repo root and pass through the same `poly fmt --check` gate, but the array indent was
  derived twice: the ownership record hand-rendered two spaces while the provenance record inherited four from
  `toml::to_string_pretty`, whose pretty serializer hard-codes `"    "` per element. Every generated tree therefore
  carried one standing "would reformat" file that no consumer could repair — hand-formatting is overwritten by the
  next `alef generate`, which the record's own header says. The provenance record is now rendered like the
  ownership one, both read the indent from a single constant, and a test compares the two writers' actual output so
  the next divergence fails whichever side moves. Regenerating rewrites the whole record once, whitespace only.
- **validate**: `alef validate versions` now discovers manifests through git, not through a disk walk, and reports
  which mismatches cannot be fixed in-tree. A consumer at `1.15.1` got five mismatches of which two were real. Two
  rows described `packages/ruby/tmp/ruby/stage/**` — gem-build staging, `tracked=no, ignored=yes` — whose tracked
  originals were correct. The third was worse: a *tracked* `Cargo.lock` reading `1.15.1` was reported as a
  mismatch while every other `1.15.1` row printed `ok`, because the staged copy of that crate's `Cargo.toml`
  declares the same package name at the previous version, and `cargo_manifest_versions` keys its map by name alone
  — glob order puts the staged copy last, so it became the *expected* value for the live lockfile. This is the
  third instance of the same shape (`vendor/`, `deps/`, now build staging), and directory-name blocklists cannot
  close it: `tmp`/`dist`/`build`/`stage` are per-tool names, whereas "not committed" is the property that
  actually separates a consumer's manifests from a build tool's copies. Both the `Cargo.toml` scan and the
  `Cargo.lock` check (and the `.csproj` scan) are now filtered to git-tracked paths; when git cannot answer (no
  work tree, no `git` binary) the previous unfiltered walk still runs, with a warning. The name-based exclusions
  stay for that fallback, and because a `vendor/` tree carried for offline builds is legitimately tracked.
- **csharp**: report unemitted visitor support files instead of deleting them. `generate_bindings` ran two
  `fs::remove_file` loops from inside the stage `collect_managed_surface` documents as "a pure in-memory render;
  nothing here writes to disk". `alef verify`, `alef adopt` — including without `--write`, since the delete fired
  before `AdoptOptions` was even constructed — and `alef diff` all compose that stage, so three read-only commands
  unlinked files in the consumer's tree, and for `verify`/`adopt` only on a cache miss. A filename match was the
  entire test, and the deleted set included a class per configured bridge `context_type`/`result_type`: names taken
  from the consumer's own config. The disabled branch was weaker still — `config.ffi` is an `Option`, so never
  having written an `[ffi]` section read identically to having disabled the feature.
- **c**: refuse to name a result type rather than inventing one. The old fallback derived it from the call name, and
  that name feeds three things — the accessor prefix, the `_free` cleanup, and the `parent_is_ir_type` flag
  `ensure_leaf_field_exists` reads. Because an invented name matches no IR type, that check returned `Ok` before
  examining anything: the fabrication switched off the check that would have caught the fabrication. Resolution now
  yields `Resolved` / `Unverified` / `Unresolvable` and fails at the point of emission. Trait-bridge registry calls
  (`register_fn` / `unregister_fn` / `clear_fn`) are classified from their derived C identity, not from their
  legitimately empty base `function`, which previously derived a degenerate `{prefix}__free`.
- **c**: stop splicing the fixture's `input` JSON into a typed parameter. With no configured `args` the emitter
  passed the whole JSON as one C string literal regardless of the target's signature, producing calls like
  `configure("{…}")` against a function taking an integer handle, and passing an argument to a `(void)` export.
  A zero-parameter target now emits `()`; a typed parameter `args` does not fill fails with a diagnostic naming the
  fixture, the call, the parameter and the config knob; an unresolvable signature refuses. Found independently in
  two repos on the same day — once in emitted output, once in the emitter.
- **cache**: treat an unreadable output manifest as a miss rather than a hit. `outputs_exist` returned `true` on any
  read error, so a cache with a missing or corrupt manifest reported a hit and skipped regeneration — and
  `write_lang_hash` writes the hash and the manifest as two separate writes, so an interrupted run leaves exactly
  that state. The ownership record now separates absent from unreadable: querying still degrades to "alef owns
  nothing", which makes the guard refuse and is non-destructive, while rewriting fails loud, because that path
  replaces the file whole and would otherwise silently un-own every path it could not read.
- **cache**: key the e2e stage cache by the `--lang` selection. A `--lang`-scoped run's partial output satisfied a
  later unscoped run, which then skipped the languages it had never generated.
- **release**: stop the version gate and `go-tag` from deciding on evidence they never read. `validate-versions`
  checked a manifest's existence and then discarded a failed read, so an unparseable `package.json` was dropped
  from the set and the gate reported "All N manifests consistent" over a smaller N, exiting 0. Zero checks is now
  an error rather than a vacuous pass, on both the text and JSON surfaces. `go-tag` ignored `git ls-remote`'s exit
  status, and a failed remote read yields empty stdout — indistinguishable from "tag absent" — so a transient auth
  or network failure created the tag and pushed it with `--force-with-lease`, which for a tag ref has no
  remote-tracking ref to lease against and degrades toward a plain force.
- **snippets**: release the client in generated Go, C# and WASM examples. Go defers `client.Free()` after the nil
  guard so it runs while a panic unwinds; C# uses a `using` declaration, which survives a throw; WASM uses
  `try`/`finally`. WASM releases only the client on purpose — every DTO-taking method already calls
  `__destroy_into_raw` on its argument and `free()` has no null guard, so releasing the request would have been a
  null-pointer free in most snippets, strictly worse than the leak it targeted. Java, Kotlin and Dart are staged
  separately.
- **csharp**: keep every vtable slot the Rust struct declares. C# carried the same unguarded `exclude_types` prune
  that cost Java a slot, and builds a positional `IntPtr` vtable, so a pruned trait method shifted every later
  function pointer and the last read ran past the allocation. Latent, but not for the assumed reason —
  `effective_exclude_types` draws from four sources, and `#[alef(skip)]` or `doc(hidden)` trips it with no
  `[crates.csharp]` config at all. A second defect ran the other way: `num_vtable_fields` never filtered
  `ffi_skip_methods`, emitting N slots into a struct declaring N−1.
- **csharp**: stop assigning `null` into non-nullable properties. 17 sites emitted `= default!` — compiles, the `!`
  silences the nullable analysis, first read throws. `CrawlConfig.Content` and `.Browser` are that case.
- **bindings**: stop substituting a zero for a default that was never read. Swift's memberwise init ignored
  defaults and decoded `Some(30)` as `nil`; Kotlin turned a function-call default on a collection into an empty
  one; the shared constructor path collapsed enum-variant and empty defaults into one `unwrap_or_default()`, so a
  `#[default]` variant other than the declared one shipped to wasm, pyo3 and extendr alike. pyo3's `None` was
  never the bug — three copies of a predicate matched only the bare `#[serde(default)]` spelling.
- **go**: send real defaults for a container-level `serde(default)` struct. `is_named_enum` shared the blind spot,
  and there it is worse — a unit enum's Go zero is `""`, never a valid variant.
- **e2e**: resolve result types from the IR on the generated-test-file path. `E2eCodegen::generate` carried no
  `functions`, so every IR lookup there was dead code. The invented type name was the visible symptom; the real
  cost was that a fabricated result type reports as not-an-IR-type, which switched off leaf-field verification on
  the very path it was written for.
- **e2e (ruby)**: assert streaming completion instead of dropping it. `stream_complete` and `no_chunks_after_done`
  matched a `None` accessor the resolver never returns, so both arms were unreachable — and the compensating
  assertion is suppressed precisely when a fixture asserts that field.

### Two invariants

Nearly every defect fixed below is one of two failures. They are worth more than the individual fixes, because the
fixes are local and the failures are not.

- **Absence from output is not evidence of intent to remove.** A value missing from a run's output has at least
  three causes: it was dropped on purpose, it was out of scope for this run (`--lang java` never computes the other
  languages), or it was never reached because an earlier step failed. Only the first licenses a removal, a prune or
  a skip, and no site in this release distinguished them. The worked example is the orphan reclaim gate: all five of
  its clauses — marker present, git-tracked, owned root, absent from this run's keep set, non-degenerate root
  manifest — passed for a consumer's 408-line Java public API class and for a class a live test depends on. The
  backend had run, emitted 56 files, and simply not emitted those two. The gate was introduced and disarmed inside
  this same release; the manifest-recording fix also in this release is what would have widened it, by making
  under-recorded manifests stop reading as degenerate. A second live delete path had the identical shape: the
  snippet prune ran ahead of the completeness gate its whole safety argument rests on, so a snippet that merely
  failed to render was unlinked as an orphan. The tell is an empty-collection fallback — `unwrap_or_default()`,
  `unwrap_or(&[])`, `Err(_) => continue` — whose result then drives a destructive or suppressive action. It is not
  confined to the generator: the cache, the verify pass and the release tooling all read an input they could not
  read as an input that was empty, and then report success.
- **One fact derived in two places, never compared.** Two emitters, or an emitter and a validator, or a backend and
  the docs renderer, each independently compute the same name, arity, nullability, sentinel or macro spelling. Each
  half is individually well-formed, so no compiler, linter or test observes a contradiction — only the composed
  output is wrong. This release closes such pairs in the C# capsule sentinel, the Zig declared error set, the Dart
  snippet call shape, Kotlin identifier escaping, pyo3 keyword fields, the cgo feature macros and `alef all`'s
  manifest. **The largest instance is still open**: `src/docs/naming.rs` renders Kotlin, Dart, Swift and PHP enum
  variants `to_pascal_case`, while the backends emit `to_screaming_snake` (Kotlin, `NETWORK_IDLE`),
  `to_lower_camel_case` (Dart and Swift, `networkIdle`) and `to_uppercase` (PHP, `NETWORKIDLE`). Every variant of
  every enum in four languages is documented under a spelling the binding does not export, with no configuration
  needed to trigger it.
  The fix direction is that the docs renderer must read the emitted binding rather than recompute it.
- **The remedy, which this codebase already applies well where it applies it at all.** Derive once rather than
  restate — `codegen::c_consumer::export_type_prefix` is the model. Where duplication is unavoidable, pin it with a
  test that drives *both* derivations and compares the composed output: `assert_error_set_covers_body` scans the Zig
  body that was just emitted, and the cgo feature-macro test parses the real generated `cbindgen.toml` rather than a
  copy of it. Enumerate from a registry rather than a hand-written list, and add a control assertion so the test
  cannot pass vacuously. A prose comment asserting that two modules agree cannot fail, and several such comments in
  this tree are false today; an assertion about a fact two modules share must name both sides, or it is a hostage
  rather than a guard.

### Upgrading

Read this before regenerating. Consumers pin `alef_version = "0.61.0"` in `alef.toml` today and 0.61.0 has never
been tagged, so those pipelines have been silently falling through to a branch build. Tagging resolves that pin for
the first time, which means the first resolved run is not a small step from whatever branch build preceded it.

0.60.2 was never tagged either — its `chore(release): 0.60.2` commit landed and the tag never followed, so the last
release any consumer could resolve is **0.60.1**. Upgrading from 0.60.1 therefore delivers the 0.60.2 section below
as well as this one; read both.

- **`--clean` no longer implies overwrite.** Until this release `--clean` was threaded into the `overwrite` argument
  of the scaffold and docs stages, disabling the create-only branch that otherwise leaves an already-existing
  unmarked file alone — so under `--clean` the ownership guard was the only remaining protection on a hand-written
  scaffold file. The two concerns are now separate: `--clean` bypasses cached results and nothing else, and
  overwriting a pre-existing unmarked scaffold or docs file is an explicit opt-in of its own. **Migration**: if your
  pipeline passes `--clean` only to force a fresh run, nothing changes and you gain back a protection you did not
  know you had lost. If it depends on `--clean` replacing pre-existing unmarked files, pass
  `--clobber-create-once-seeds` alongside it, or — better, because it is durable and reviewable — take ownership
  of those files once with `alef adopt <path>` and drop the flag.
- **Two large generated diffs land at once, from two unrelated causes. Review both for removals and for files that
  stopped being emitted; do not skim either as progress.** A large diff that reads as healthy is how the worst
  defects in this release stayed invisible. (1) The snippet-ownership fix unfreezes a generated snippet surface that
  nothing has been able to update for some time — on one consumer tree 2,820 of 2,894 writes were being refused — so
  the diff is the accumulated drift of every refused run, and it is the first diff in which a deletion is possible
  again. (2) The snippet resource-release fix separately rewrites a large number of published Go, C# and WASM
  examples, which previously leaked the client they construct. Seeing both at once is expected; they are two things.
- **Commit `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml` if they are untracked.** `alef verify` now
  fails on this and prints the exact `git add` to run. See **Changed** for why an untracked record makes a green run
  on a warm machine certify a state no other checkout has.
- **The committed C header's content is now a function of the local build's feature set.** A `--no-default-features`
  build whose header is committed strips feature defines that a default build would have written, and nothing
  currently catches that. See **Changed**.

### Added

- **`alef adopt <path-or-glob>`**: take ownership of a pre-existing generated file so alef can regenerate it again.
  The write-time ownership guard refuses any pre-existing file it cannot prove it authored, which is correct but
  one-way: a file whose type became stampable only after it was committed carries no marker, so the write is
  refused, so the marker never lands, so it is refused forever. `crates/crawlberg-ffi/Cargo.toml` has never carried
  a marker in its entire history and had real fixes frozen out of it behind a warning nobody reads during a regen.
  `adopt` prints the full, untruncated diff between the file on disk and what alef would generate, and changes
  nothing without `--write`. Adoption stamps the marker onto the bytes already on disk and never writes generated
  content, so convergence happens on the next ordinary `alef generate`, in view of `git diff`. Formats that cannot
  carry a comment (`.json`) are adopted through the durable `.alef/` ownership record instead. It is deliberately
  not wired into `alef all` or any other command: no predicate over file content can tell "alef wrote this under an
  older release" apart from "someone hand-wrote this", because both are the same bytes.
- **config `[crates] cargo_lints`**: declare a per-crate `[lints.rust]`/`[lints.clippy]` table for generated binding
  crates. Previously inexpressible, so consumers hand-edited files headed `auto-generated by alef — DO NOT EDIT`
  and regeneration correctly restored declared content, silently making workspace `deny` levels inert in exactly
  the crates that most need them. `[lints] workspace = true` is not a substitute: all-or-nothing, and it pulls
  `unsafe_code = "deny"` which no FFI crate can accept. Covers the 11 Cargo.toml-emitting backends (ffi, node,
  python, php, jni, r, ruby, elixir, wasm, swift, dart); the six that emit no Cargo.toml are deliberately excluded.
  Where a backend already emits a builtin `[lints.rust]` block (elixir, swift, dart), a configured table merges
  into the same table rather than emitting a duplicate, which Cargo rejects.
- **`alef verify` reports frozen generated files**: a file alef intends to stamp, that exists on disk carrying no
  marker, can never be written by the ownership guard and so is frozen permanently. Previously this was invisible
  without running a generate. `collect_alef_hashes` cannot see such a file by construction — it only opens files
  that already carry a marker — so detection reuses the in-memory regeneration `verify` already performs to find
  missing files, intersected against what exists on disk unmarked. No heuristic and no added cost. Reported as its
  own section with the literal marker line to paste, never folded into stale or missing, because `alef generate`
  fixes those and cannot fix this. Scope is the ownership guard's freeze class only: create-once artifacts are
  emitted without a generated header and are deliberately excluded, so a hand-edited `Cargo.toml` is never reported.
- **eight create-once migrators**: artifacts emitted without a generated header are written once and never updated,
  so a repo scaffolded before a template fix keeps the broken content forever. Each migrator is pinned to the commit
  that fixed its template and repairs only content it can positively identify as alef's own stale output —
  `packages/dart/.pubignore`, `crates/*-wasm/package.json` exports, `crates/*-node/package.json` service export,
  `packages/java/checkstyle.xml` line length, the wasm `.cargo/config.toml` rustflags, and others. A migrator that
  cannot establish that provenance repairs nothing, because clobbering a hand-edit is far worse than a stale file.
- **fixture `docs.shows` gains `display`**: selects human-readable over debug formatting, matching the flag
  `iterate` already had. Without it Rust snippets always rendered `println!("{:?}", …)`, so documentation printed
  `Some(Text("Hello!"))` where a reader expects `Hello!`. Defaults to the previous behaviour.

- **readme**: expose each language README's generated public functions as structured `functions` template values
  with `name`, `rust_name`, `is_async`, and `documentation` fields. Names honor language exclusions, feature gates,
  ABI prefixes, Go type collisions, Ruby re-export names, and the centralized host-language naming policy.

- **`--version` carries build provenance**: the commit, the build time, and whether the tree was dirty. Three
  binaries built in one day all self-reported the same semver, and a defect that had already been fixed was
  investigated for hours against output from a binary that predated the fix. Generated output is evidence about the
  binary that produced it, not about the source, and until now nothing in that output identified the binary. A dirty
  build says so and names the commit it cannot be reproduced from; missing git metadata renders an explicit unknown
  rather than an empty field that would read as a clean build. `clean` is only as precise as the rerun-if-changed
  set; `dirty` and the sha are exact. **`--version` is now multi-line** — `-V` keeps the single bare semver line, so
  anything parsing a version out of `alef` should use `-V`.
- **breaking generated signatures are reported**: emitted public signatures are captured against a previous-run
  baseline, diffed, and each breaking change attributed to the callers that are not alef-owned. This warns rather
  than fails, because failing a regeneration on a change the consumer intends would be worse than the present
  silence; the value is the attribution. Zig only for now — other backends return no signatures, and a change
  detected for a language with no scan wiring warns rather than passing silently.
- **extraction warns when `#[serde(default…)]` and `impl Default` disagree for one field**: the effective value then
  depends on how the caller constructed the type, so a binding generated from one path silently contradicts the
  core when the other is taken. Alef read both and discarded the first — three writers assigned the same slot in
  sequence. The diagnostic stays silent unless both sides fold to fully concrete values: an `Unresolved` default
  means alef could not read a real `fn default()`, which is unknown rather than zero.

### Changed

- **`alef verify` now fails on a frozen file rather than warning.** This is a semantics change for consumer CI. The
  condition is permanent and self-perpetuating by construction — the guard refuses because there is no marker, and
  the marker can only arrive by writing the file — so no later run clears it and a warning is indistinguishable from
  one nobody reads. Remedy is `alef adopt <path>`.
- **the scaffold ownership record moved to a committed `.alef-ownership.toml`.** Ownership of a file whose format
  cannot carry a comment previously lived under `.alef/`, which alef itself writes into every consumer's
  `.gitignore` — so a fresh clone and a warm dev machine disagreed about which files alef owns, and CI refused
  writes a developer's machine permitted. Reads union the committed record with the legacy gitignored one, which is
  never written again, so upgrading does not turn every unmarkable file into a refusal at once. Entries migrate on
  the first authorised write of that path. Commit the file. Do not hand-add entries: use `alef adopt`.

- **formatting**: normalize the persistent FRB Cargo-cache helper with the repository Rust formatter.
- **test formatting**: normalize recently added FFI handle-registry, enum-conversion, and generated-hash regression
  fixtures with the repository formatter.

- **zig**: give each streaming adapter its own iterator struct type instead of naming it after the item type alone.
  Two streaming methods on the same opaque handle that yield the same item type (e.g. `crawl_stream` and
  `batch_crawl_stream`, both yielding `CrawlEvent`) collapsed into one shared `{ItemType}Stream` struct — whichever
  adapter emitted its struct first "won" the name, and the other adapter's wrapper method returned that same struct,
  whose `next()`/`deinit()` hardcode only the first adapter's `_next`/`_free` FFI symbols. That compiled and linked
  (both symbol sets exist in the C header) but handed the second adapter's stream handle to the first adapter's
  native functions — a runtime handle type-confusion bug, not a missing feature. Colliding adapters now each get a
  uniquely named struct derived from the adapter's own name. **Consumer-visible:** where a collision exists the
  shared `{ItemType}Stream` type is renamed to adapter-specific stream types. Zig code naming the old type explicitly
  must be updated. Types with a single streaming adapter
  keep their existing name.

- **write commands warn, and `alef verify` fails, when a required alef record is untracked.** Alef writes
  `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml`, tells the reader to commit them, and never stages
  them; both were untracked on two consumer repos. Under `--clean` that record is the only protection left on a
  hand-written scaffold file, and it is the input the orphan scan reads, so a fresh clone has neither the protection
  nor a correct picture and a green run on a warm machine certifies a state no other checkout has. **Action: commit
  `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml` if they are untracked in your repo.** `verify` prints
  the exact `git add` to run. Not auto-staged: mutating a user's index is a different licence from writing files, and
  in CI it would accomplish nothing.
- **the generated `build.rs` stamps a `#define` per `CARGO_FEATURE_*` into the C header.** cbindgen guards a
  declaration for every `#[cfg(feature)]` export, deriving the guards from the unfiltered API surface, while the Go
  glue is generated from the cfg-filtered one and the cgo preamble recorded neither — so the guarded declarations
  were exactly the ones the glue calls, and nothing anywhere defined the macros: 62 of 144 symbols invisible on one
  consumer, the whole C snippet lane dead on another. The defines are written after cbindgen emits the header, from
  literally what the cdylib was built with, so there is no second derivation to drift, and they reach Go, C snippets,
  the e2e Makefile and Zig at once. Guards stay load-bearing for a slim build: a disabled feature gets neither a call
  site nor a define. **Cost: the committed header's content is now a function of the local build's feature set.** A
  `--no-default-features` build whose header is committed strips defines that a default build would have written, and
  nothing currently catches that.
- **the snippet coverage ledger now proves alef's ownership of a generated snippet.** A snippet population predating
  marker support could never be updated — the write guard refuses a pre-existing file it cannot prove it authored,
  and the marker that would prove it is what the refused write was going to add. On two consumer trees that froze the
  entire generated snippet surface: 2,820 of 2,894 refusals on one. The ledger is a record rather than an inference,
  it is committed so a fresh clone behaves identically, and alef already unlinks files on its strength, so refusing
  to overwrite what it will happily delete was incoherent. The snapshot is taken before generation, or this run's own
  intentions would widen ownership to bare path identity. The first successful regeneration after upgrading produces
  a large snippet diff; see **Upgrading** for what to check in it.
- **kotlin**: hard keywords are escaped in every identifier path. Identifiers went through three parallel paths — one
  escaped nothing, one ran the escape on an already-PascalCased string so no all-lowercase keyword could match, and
  only the field-name path worked, so `fun object(...)`, `fun is(...)` and `when: String` reached consumers as parse
  errors. Backticks rather than renames: the DTO emitter writes `@JsonProperty` only when the Rust field carries
  `serde(rename)`, so renaming would silently move the wire key for every unrenamed field. `get`/`set` stay bare on
  purpose — they are soft keywords the grammar admits as `simpleIdentifier`.
- **pyo3**: one Python-visible name per keyword field. Five producers computed five different names for a single
  field: the getter published the bare keyword, the converter emitted `_rust.T(global=value.global)`, and a keyword
  field got no wire alias at all, shipping the escaped spelling as the JSON key. The escape now reaches the Python
  surface only; serde keeps the wire name and prefers the core type's own rename over deriving one from the escaped
  field. Where the Rust and Python escapes collide the Rust one wins — pyo3 strips `r#` when deriving the Python
  name, so `r#type` satisfies both while `type_` satisfies neither.
- **dart**: the trait emitter was the only Dart path calling `to_lower_camel_case` without the keyword escape, so
  `new` and `get` — the two commonest Rust trait method names — were emitted bare. They are now escaped, which
  renames them on the generated Dart trait surface.

### Fixed

- **go**: stop asserting a wire shape serde does not produce. Go encoded every `std::time::Duration` field with
  the `DurationMillis` helper, which writes serde's derived `{"secs":_,"nanos":_}` object. A field carrying
  `#[serde(with = "…")]` has a hand-written codec expecting a bare millisecond integer, so the derived shape is
  wrong for it and every config construction round-tripping through JSON failed with `invalid type: map, expected
  u64`. The IR could not tell the two apart — the extractor implemented five serde readers and none read
  `with`/`serialize_with` — so `FieldDef::serde_with` now records it. Reading all occurrences matters:
  `deserialize_with = "…"` contains `serialize_with = "…"` as a substring, so a first-match-only scan silently
  picks the read side. `api_has_duration_field` moves in lockstep, otherwise a crate whose only Duration fields are
  hand-coded gets an unused helper and an unused `encoding/json` import, which Go rejects.
- **java**: keep a vtable slot for a trait method returning an excluded type. `api_without_excluded_types` lost its
  `if !typ.is_trait` guard as incidental cleanup, so a bridge method whose signature named an `exclude_types` entry
  was pruned from the Java surface while the Rust vtable still declared its slot. Every function pointer after the
  dropped one then dispatched to the wrong method, with the last read running past the end of the struct. The prune
  is silent because it removes the method from the interface and adapter too, leaving all three Java files mutually
  consistent. Both emitters now take the slot list and its ABI order from one function, and generation fails when
  the declared and emitted layouts disagree. C# has the same unguarded prune and a positional vtable of its own; it
  is latent only because no consumer sets `[crates.csharp] exclude_types`.
- **bindings**: restore struct defaults the emitters dropped. Java decided twice, from two independent lists,
  whether a field carries a literal default — one governed boxing, the other the compact-constructor restore — and
  only the second was ever extended, so float defaults crossed the wire as `0.0`. Kotlin rendered a `f64` default of
  `1.0` as `Double = 1`, which does not compile, and emitted bare `NaN`/`inf`. Rustler decided what kind of default
  it had by sniffing rendered Rust text for `::` or a leading quote, collapsing every string default to `""`. All
  three now ask `typed_default`, and Java and Kotlin share one float renderer. **Breaking for Java consumers**: a
  `boolean` component carrying a `true` default becomes `Boolean`, since boxing is the only way to distinguish
  "not supplied" from the type-zero. Boxing applies only where the default differs from the zero.
- **e2e (C)**: reject an assertion whose leaf field the IR does not declare. The nested-accessor walk validated
  every intermediate hop and nothing at the leaf, defaulting to `char*` and synthesising `{parent}_{field}` as the
  symbol. Generation reported success and the failure surfaced only at `cc` time. Three existing mechanisms could
  not see it: `is_valid_for_result` is head-only by construction, splitting on `.` and inspecting only the first
  segment; the unavailable-field scanner looks for a comment this path never writes; and
  `ALEF_E2E_STRICT_FIELD_AVAILABILITY`, which arms the markers that are written, is set in no repository.
- **e2e**: resolve call names through the override chain. `CallConfig::function` is legitimately empty when a call
  names itself only per language, so sixteen sites reading it directly failed silently — adapter and IR lookups,
  `request_type`/`streaming_item_type` keys, and a `returns_void` classifier that bound a result from a void C#
  method on every registry call. Two resolvers now cover the two distinct questions, and a structural guard pins
  every remaining raw read against an allowlist keyed by source text rather than line number.
- **e2e (C, Zig)**: derive the C export prefix once. cbindgen writes `[export] prefix` as shouty-snake while every
  C and Zig emitter re-derived it with `to_uppercase`; the two diverge for any prefix with an internal word
  boundary, naming types absent from the header the snippet compiles against. Zig snippets also ran the closing
  `std.debug.print` onto the previous line.
- **snippets**: report per-language failures instead of one number. A run with 1753 failures emitted a start line, a
  finish line and a summary count, so a language failing every snippet was indistinguishable from one that passed.
  Java failed for an unrelated reason: session scratch moved outside the Maven source root, but nothing swept what
  older versions had written, and the leftovers are self-perpetuating — the consumer's own `mvn package` hook hits
  `duplicate class` and fails preparation for the whole language on every future run. The sweep is unconditional
  rather than tied to `--clean`, since needing a flag to get a correct run is a workaround.
- **snippets**: keep the mock harness out of published examples. C streaming and byte-buffer snippets published
  `create_client("test-key", NULL, …)` — a literal harness credential with no environment read. Unlike the other
  leaks this one was not blocked, because that string is not in the guard's marker list. Swift emitted the
  environment-reading constructor only when a fixture named a credential variable, and Elixir snippets called the
  module directly although the exported arity includes the client, naming a function that does not exist.
- **docs**: strip every `~keep` spelling. The stripper removed the marker's five bytes then chose between eating
  the following whitespace or one preceding character, so every variant with attached punctuation stranded it:
  `~keep:` left `.:`, `~keep,` left `.,`, and `(~keep)` — the most common broken form — left empty parens.
- **e2e (python)**: define the helpers the generated file calls. `_alef_e2e_text` has two independent callers but a
  single gate keyed on the second emitted both definitions, so a file whose only caller was the enum equality
  assertion shipped 22 undefined names.
- **codegen**: one snake-caser and one attribute scanner, not three. `error_gen` carried a third caser splitting
  before every uppercase letter, so `GraphQLError` became `graph_q_l_error` in C accessor symbols while the repo's
  declared derivation produced `graph_ql_error`; a fourth caser for screaming-snake shared the flaw, and one
  generated snippet pairs the two. `rust_type_kind_hints` reset its state on any line not starting an attribute, so
  a rustfmt-wrapped `#[derive(…)]` between a `#[repr(…)]` and its struct discarded the hint.
- **e2e/snippets (wasm)**: stop gating snippet availability on a codegen predicate. `function_is_exported`
  answers "should the plain-function generator emit a wrapper for this?" and returns `false` for trait-bridge
  register/unregister/clear functions precisely because the trait-bridge generator emits them instead. The snippet
  gate reused it to mean "can a snippet call this?", where `false` is flatly wrong — those functions are exported,
  from the generated `__alef_wasm_bridge_*` modules. A 0.61.0 regression with a committed positive control: 0.60.0
  emitted a valid snippet for the same fixture, with the correct import and JS name. It did not merely drop
  snippets, it aborted `alef all` before a byte was written, so an affected repo could not regenerate at all.
  Symbol resolution is fixed alongside the predicate: the gate needs a Rust identity while
  `overrides.wasm.function` legitimately holds the JavaScript spelling, so a symbol now resolves under either
  spelling and the bridge registry is searched beside the plain function surface. A name that resolves to nothing
  is reported as its own condition — folding it into "not exported" sends the reader to audit the wasm backend for
  what was only ever a misspelling in config.
- **cli/generate**: read a `#[cfg(...)]` attribute that rustfmt wrapped. The FFI header parity gate scanned
  attributes one line at a time, so a wrapped predicate was lost twice over — the single-line parser failed on the
  opening line, and the continuation lines, being neither attribute-prefixed nor function signatures, hit the state
  reset that clears the pending cfg. The export was recorded as unconditional and a correctly guarded header was
  reported as drifted, aborting `alef all`. The gate was unclearable by its own remedy: it advises running a cargo
  build so cbindgen regenerates the header, and the run had already done exactly that, successfully, moments
  before. An attribute the scanner cannot delimit is now recorded as an unparsed cfg rather than as no cfg, so the
  gate gets stricter here rather than looser.
- **cli/generate**: report every refused write rather than one phase's. A run writes through five independent
  phases, but the consolidated refusal summary was emitted from inside the scaffold writer, so it could only ever
  describe scaffolding — and `alef all` never called the reporter at all, dropping `refused_paths` from its four
  write sites. Measured across three real regenerations: 34 refused and 30 reported, 15,677 refused and 15,669
  rostered, 34 refused and 29 reported. The omitted paths are real binding sources. Worse than a wrong number: the
  summary tells the operator to review and adopt each path, so someone who works the printed list finishes
  believing they are done while those files stay permanently frozen.
- **backends/zig**: never report a typed error the binding cannot substantiate. A variant's FFI code comes only
  from an explicit `#[alef(error_code = N)]`, and no consumer declares one, so every zig binding resolved every
  failure to `_first_error(E)` — literally the first declared variant. Silently wrong error types rather than
  missing ones. The FFI layer was already honest here, sending `ALEF_FFI_UNKNOWN_ERROR` across the boundary; zig
  was the only backend that turned unknown into wrong. `_first_error` is removed rather than patched, because eight
  further emission sites used it with the identical defect — null constructor handle, null `to_json` pointer,
  stream start and next, trait-bridge clear and unregister, opaque-handle returns. An implicit `UnknownFfiError` is
  injected into every generated error set by the same mechanism that already injects `OutOfMemory`, so it coerces
  into any caller-supplied set; coded variants still dispatch per code, and only the `else` arm changed. Separately,
  `_first_error(anyerror)` never compiled at all: `@typeInfo(anyerror).error_set` is `null`, so `orelse unreachable`
  was comptime-evaluated, and two templates emitted exactly that. Five assertions across two files pinned the old
  behaviour — including one named `..._use_the_unknown_fallback` that asserted `_first_error`, which is not an
  unknown fallback — and were inverted rather than worked around.
- **e2e/snippets**: give generated markdown a provenance marker. Fixture snippets carried neither a marker nor an
  ownership record, so once written they were refused forever — 15,677 refusals in one consumer repo and 9,139 in
  another, dominated by the ~12,000 snippet `.md` files between them. `marker_header_syntax` excludes `.md` on the
  stated grounds that `readme::template` and `docs::render` both route content through
  `docs::render::with_html_header`; that is true of READMEs and docs pages and false of fixture snippets, which are
  assembled in `render_snippet_markdown` and never touch `docs::render`. The header now comes from that same
  emitter rather than a second producer. `marker_comment_style` is untouched — `.md` stays out of the *ownership*
  predicate, since adding it there would freeze every unmarked `.md` in every consumer repo. Note the placement has
  zero slack: front matter is 8 lines, the header lands on line 10, and the marker scan window is 10, so a ninth
  front-matter field would silently restore the deadlock with the marker still in the file. Files already committed
  without a marker are not unfrozen by this and still need adoption or regeneration from absent.
- **scaffold/ownership**: stop minting ownership from byte-equality alone. Four write paths recorded a file as
  alef-owned whenever its bytes already matched generated output, *before* any ownership check ran — the rejected
  content-equivalence predicate, relocated from a predicate into the record. A hand-written file that coincided with
  generated output silently acquired permanent overwrite permission, and the run that granted it changed nothing
  observable, which is why a test asserting only on file contents and the changed count passed throughout.
  Ownership is a fact about history, not about content; only `alef adopt` confers it now.
- **docs/c**: derive every documented C symbol from one helper. Docs published `{prefix}_{method}` while the FFI
  backend emits `{prefix}_{type_snake}_{method}`, so documented symbols did not exist, and the `this: AlefHandle`
  receiver was missing from documented signatures. A repo sweep found the symbol shape derived at roughly 262 sites
  — four of them independently inside docs alone — so patching the docs arm would have traded one divergence for
  two. Producers, docs and the streaming `_start`/`_next`/`_free` triple now route through `free_function_symbol` /
  `method_symbol` / `stream_adapter_symbol` in `codegen::c_consumer`.
- **backends/java, backends/kotlin**: reject a service `Finalize` entrypoint whose return type is not opaque. The
  FFI layer renders every non-opaque entrypoint return as an `i32` status code — `null_return = 1` on the error path
  — and no template carries a primitive value across the boundary. Java's representability gate nevertheless
  admitted the shape and then emitted `void`, and Kotlin would have declared `Int` against that `void`. Generation
  now fails on the shape instead, and Kotlin's `Finalize` return is the raw `AlefHandle` as `Long`.
- **scaffold/zig**: emit no test step when there is nothing to assert. `zig build test` exited 0 having run nothing,
  which is indistinguishable from coverage. The seed file and the `test_module`/`test_step` block now branch on one
  condition, so the step and the file it points at cannot drift apart; an empty surface fails with
  `error: no step named 'test'`. The fixture that should have caught this was empty, so the test validating the seed
  was validating the placeholder.
- **e2e/snippets**: keep mock-harness scaffolding out of published documentation. The zig snippet renderer called
  the test renderer, so published docs told readers to read `MOCK_SERVER_URL` and route through `/fixtures/<id>`.
  The existing scrub never covered this surface in any language — it swaps a placeholder in `fixture.input`, while
  these URLs are synthesised by the client-factory emitters from the fixture id at emission time. The guard is now
  applied at the single funnel every language and extension passes through, so a new backend inherits it.
- **e2e/codegen/rust**: emit an error branch for a fixture that expects an error. Rust was the only one of the
  fifteen snippet languages without one; it rendered `.expect("call failed")`, so the snippet documenting a failure
  panicked on it. Rust has no `try`, so this is a `match` on the `Result` with an `Err` arm.
- **hooks/check_project_mentions**: count `#[cfg(test)]` braces in code rather than in raw text. alef is a template
  generator and its comments are dense with Jinja — `{% endif %}` alone is one opener and two closers — so prose
  steered the exemption. Surplus closers ended the region early and reported test fixtures as violations; surplus
  openers extended a phantom region past the module's closing brace and silently hid every real violation in the
  production code below, leaving the gate reporting clean. Both directions are covered by a regression test.
- **backends/swift**: replace five regression files that had never been compiled. None was in the module tree, so
  Rust never saw them, and all sixteen of their assertions were `assert!(true)`. Wiring them in as written would
  have added no coverage. Rewritten against the private modules they actually cover.
- **bin_cli/verify**: scan every backend's output when collecting `alef:hash:` provenance. `VERIFY_SCAN_EXTENSIONS`
  omitted five backends outright, so `alef verify` reported those trees clean without ever opening a file in them —
  a passing verify was evidence of nothing for the languages it silently skipped. Dotfile stamps
  (`.gitignore`, `.gitattributes`, `.editorconfig`) were unreachable by construction on top of that, since
  `Path::extension` returns `None` for a name that is entirely a leading-dot stem; they are now matched by filename.
  The test that should have caught this was itself vacuous — its fixture wrote a stamp line but no hash line, and
  `collect_alef_hashes` requires both, so the sibling positive control asserted over an empty set. The fixture now
  emits a real stamped-and-hashed file, and a guard test pins that the walk actually collects what the fixture writes.

- **readme**: honour a configured `output_path`/`output_pattern` on the hardcoded fallback route, not only the
  templated one. `try_render_configured_readme` returns `None` in five distinct situations — no `template_dir`, a
  `template_dir` that does not exist, no entry for the language, no legacy YAML entry, or an entry whose template
  file is absent — and each one discarded the configured path along with the template, because path selection was
  reachable only from inside the templated route. A configured output path is not a property of a template and
  survives all five. Both routes now apply one precedence rule; the fallback previously composed its own, and the
  defect went unnoticed because the derived path usually agrees with the configured one. Agreement was coincidence.

- **backends/php**: emit `from_json` and the flat-field accessors in `.phpstub` output for tagged data enums. The
  runtime emits `from_json` unconditionally for every such enum plus a readonly property per flat field, while the
  stub declared neither — six enums in one consumer repo, three in another, two of them rendering as literally
  empty classes. The stub gate is `is_tagged_data_enum` alone and deliberately *not* the crate-level serde probe
  used for structs: the flat-enum mirror's serde derives are hardcoded in its template rather than gated, so keying
  the stub on the probe would reintroduce the same divergence in serde-less crates. Accessors are declared as
  properties rather than methods because ext-php-rs registers `#[php(getter)] fn get_x` as a property named `x`
  with no case conversion — the inverse of the struct path, which emits plain methods that land as `getX()`.

- **backends/magnus**: derive the async return annotation from one fact instead of three hand-maintained copies.
  `function_async_body.rs.jinja` is a single template serving both `has_error` arms and opens with a fallible
  `Runtime::new()?` in each, so building the tokio runtime makes an async binding fallible regardless of what the
  Rust signature declares. Two annotation sites disagreed with that: one hand-recomputed a subset of the `has_error`
  local already in scope four lines above it, dropping the `is_async` and `force_result_for_deser` terms.

- **docs**: stop emitting a Java constructor name the backend never generates. The docs carried two hand-written
  copies of the keyword-rename table, both wrong — they mapped `default` to `defaultOptions` and had no `new` arm
  at all, so an opaque type's default constructor reached `assert_valid_identifier` as the Java reserved word `new`
  and panicked, aborting the docs run. The table is now mirrored from the backend's `safe_java_method_name` with a
  test pinning the two together, and the duplicate copy is gone. Also corrects a cluster of `~keep` comments that
  asserted `Language::Jni` was unreachable — it is reachable today, and the files cited as proof say the opposite —
  and renames five tests whose names claimed the output had once been correct for a backend it never suited.

- **e2e/codegen, backends**: fail at generation time instead of emitting placeholder values that let a generated
  suite pass while testing nothing. Ten sites across eight backends silently fabricated output: the Ruby extension
  returned the literal `"[unimplemented: <fn>]"` (and `0`/`false`) for non-delegable functions, PHP, wasm-bindgen
  and the Elixir NIF did the same, `dart:ffi` mode dropped async functions from the API surface behind a comment,
  and the Rust e2e streaming path emitted nothing at all through unguarded `if let` arms. Two were worse than
  vacuous: the pyo3 capsule spliced `unreachable!()` into a value position, which compiles and then uncatchably
  panics the interpreter on first call, and the JNI `nativeRegister<Trait>` shim accepted a registration without
  ever calling `register_fn`, reporting success for a backend it never registered. Each now fails loudly —
  `compile_error!` where the backend already had that escape hatch, an explicit panic naming the crate, fixture
  and symbol elsewhere — and the pyo3 case raises a catchable `PyRuntimeError`.

- **e2e/fields**: derive field availability from the IR rather than from the hand-maintained `result_fields` TOML
  list, across fourteen backends. The list was wrong in both directions simultaneously in a real consumer repo —
  omitting a field that is exposed and listing one that a getter also exposes — so assertions were silently
  replaced by `skipped: field not available` comments. `FieldDef.binding_excluded` is now consulted first; it is
  not a proxy, being the same predicate `binding_fields()` uses to decide which fields the pyo3 backend gives a
  getter. Config and its sibling maps remain a fallback for names the IR has never seen. Note `#[serde(skip)]`
  never implies binding exclusion — only `#[doc(hidden)]`, `#[cfg_attr(alef, alef(skip))]` and `dyn Trait` fields
  do — and a control test now pins that, since a field can be absent from the wire format and still be exposed.

- **backends/rustler**: bound the visitor bridge's reply wait. `visitor_send_and_wait` blocked on `rx.recv()` with
  no timeout, so a host process that exited before replying held the NIF scheduler thread indefinitely; the
  trait-call path already had a watchdog and it was simply never retrofitted. The visitor channel carries no error
  slot, so the watchdog drops the sender and the existing disconnect path returns the method's default result,
  matching what the trait path already does on a closed channel.

- **backends/java**: resolve the libc `free` symbol lazily inside `freeHandlerResponse` instead of eagerly in the
  service class's static initializer. `SymbolLookup.loaderLookup()` only sees symbols reachable through libraries
  the classloader has loaded, and `free` is not emitted by alef's own FFI, so a service that never frees a handler
  response could fail to load at all. The sibling `malloc` lookup was already lazy; the asymmetry was the defect.

- **backends/zig**: emit the unwrap expression for a method or function returning `Option<OpaqueHandle>`. The match
  had arms for every other shape, so `Optional(Named)` fell through to a catch-all that returned the raw C value
  while the signature declared `?TypeName` — type-incorrect code that the vacuous generated test target concealed.

- **cli/generate**: skip languages with no binding backend instead of panicking. The build path already guarded
  this for docs-only targets (Rust, C); the generate path called `get_backend` unguarded on the same input.

- **scaffold/swift**: seed the generated test file with a real assertion. It emitted `XCTAssertTrue(true)`, which
  compiles and proves nothing; it now round-trips a serde DTO through `JSONEncoder`/`JSONDecoder` where the API
  surface allows, falls back to a type-resolution check, and keeps the bare placeholder only for an empty surface.

- **snippets/java**: write the validation scratch session outside the Maven source root. The generated `pom.xml`
  sets `sourceDirectory` to the project basedir, so the compiler plugin's `**/*.java` glob swept scratch snippet
  sources into the consumer's own build. `target/` is not a safe alternative for the same reason.

- **docs**: read signature contracts from the emitted binding instead of recomputing them per language. Java
  signatures stated the wrong `throws` contract on every method, so every rendered example failed to compile;
  Dart dropped the `Future<>` wrapper that its whole binding carries, and rendered optional parameters in
  positional rather than named syntax; Elixir dropped the receiver an instance function actually takes. Rust
  `Default`/`Clone` derives were documented as public API in languages that emit neither, error sections listed
  Rust enum variants rather than the generated exception classes, and integer and `Option` types were computed by
  a per-language formula that no two backends agreed on. An explicitly overridden return type is now authoritative
  and is no longer re-wrapped, which had turned a streaming `Stream<T>` into `Future<Stream<T>>`.

- **Java service bindings**: retain paired callback response deallocators and registration variant metadata while
  leasing service owners, and omit public functions whose signatures reference excluded types.

- **snippets/strict**: stop counting an explicit front-matter `level:` as a downgrade. `level:` is a validation
  contract — the author asked for exactly that level — and used to collapse into the same internal field a `<!--
  snippet:*-only -->` suppression comment uses, so a snippet that got exactly the level it declared was reported as a
  `strict`-failing `Downgraded` violation identical to one that suppressed validation below what the run requested. A
  declared `level:` that is fully honored now passes and carries a `Declared` `downgrade_reason`; a suppression
  annotation is unchanged and still fails strict, as does a declared level the environment or validator cannot
  actually reach.
- **snippets/strict**: extend the `max_level` capability-ceiling exemption to a validator's structural
  `achievable_level` gap. `php`, `ruby`, `elixir`, `bash`, and `r` cap `typecheck` down to `syntax` unconditionally —
  no checker is wired up for any of them, on any machine — but only `max_level` was exempted from `Downgraded`, so
  `validation_level = "typecheck"` plus `strict` was structurally unsatisfiable for any repo containing one of these
  languages, however healthy the environment was. Validators now declare whether an `achievable_level` gap is
  structural (permanent, exempted like `max_level`) or environmental (this run's machine only, e.g. a missing
  type-checker binary — still a genuine `Downgraded`); the five listed above declare theirs structural, and their
  affected snippets now report a capability-capped `Pass` instead of failing strict.
- **snippets/strict**: surface hard failures and session/preparation errors ahead of a strict downgrade bail, and
  name the results behind every strict-mode failure count. A run carrying both real `Fail`/`Error` results and
  `Downgraded` results previously reported only the downgrade count, because the strict downgraded check ran and
  bailed before the failure check further down was ever reached — a consumer investigating "N downgraded" never
  learned the run had failed outright. Every `ValidationResult` now carries a `downgrade_reason`
  (`Declared`/`Annotation`/`ValidatorCapability`/`Environment`), and the strict-failure and capability-capped-warning
  messages group by that reason as well as by language, so a consumer sees *why* a level differs, not just that it
  does.
- **snippets/sessions**: log a `tracing::error!` naming the target and language whenever
  `prepare_sessions_isolated` fails to prepare a validation session. Every snippet aimed at a failed target silently
  became a `SnippetStatus::Error` downstream with no other signal that the *target*, not the individual snippets, was
  what broke — this module had no `tracing::` calls at all before.
- **snippets/batching**: make the batch/fallback dispatch path observable. A language whose validator never overrides
  `validate_batch_in_session` (the default implementation always returns `None`) was still logged
  `Starting batched snippet validation`, then silently fell through to the per-snippet fallback with no matching
  `Finished` event — an observability gap in the dispatch path itself, not a signal about whether the validator ran
  or hung (a healthy, fully-passing language was exactly as silent there as a broken one). Validators now declare
  `supports_batching` upfront so a non-batching group never enters the batch codepath at all; a validator that does
  support batching but declines a specific group (rust declining to batch `Run`-level snippets) now logs an explicit
  fallback notice instead of a silent `continue`; and the per-snippet fallback dispatch itself now logs its own
  `language`-tagged `Starting`/`Finished` pair per language with a count and duration, so a `Starting`/`Finished`
  correlation by name works the same way for a fallback language as it does for a real batch. `run_validation` also
  now re-enters the caller's tracing span inside the rayon thread-pool closure, since `ThreadPool::install` always
  runs on a pool worker thread and span context is thread-local, not inherited across that boundary.
- **snippets/sessions**: purge stray top-level files left in a session's persistent `workspace_directory` (java,
  csharp, typescript) before running configured `before` hooks. That directory is deliberately reused across every
  snippet in a session and across every future run with an unchanged fingerprint, so compiled-artifact caches in its
  subdirectories survive between runs — but nothing ever removed the scratch source file each snippet's validate call
  writes at its top level, so it accumulated one leftover file per distinct snippet ever validated under that
  fingerprint. A consumer-configured `before` command that builds the whole module from `working_directory` (`mvn
  package`, for a Java session) runs once per session, before any of that run's own snippets are written, so the only
  way it could trip over bad scratch content was a leftover from a *previous* run — and one bad leftover then failed
  session preparation and stamped every snippet in the session `SnippetStatus::Error`, turning one bad snippet into
  an entire language going dark.
- **docs**: stop discarding every already-rendered API reference page when a later docs-stage step fails.
  `generate_docs_stage` renders the 15+ `api-*.md` pages plus `configuration.md`/`types.md`/`errors.md` before
  snippet discovery, snippet validation, CLI/MCP adoption checks, or llms/skills rendering ever run, but returned a
  single `Result<Vec<GeneratedFile>>` — so a failure in any of those later, unrelated steps (a strict snippet
  validation bail, an unmanaged `llms.txt`, a missing `docs.snippets.dirs` root) discarded the whole `Vec` and wrote
  nothing at all. A single strict-mode snippet failure could therefore silently freeze the entire published API
  reference at whatever version last validated cleanly, with no signal to the caller that anything was skipped.
  `generate_docs_stage` now returns `(Vec<GeneratedFile>, anyhow::Result<()>)`: callers write the pages unconditionally
  and only then propagate the error.
- **scaffold**: fix `detect_workspace_inheritance`, which never detected anything. It used
  `contents.parse::<toml::Value>()`, but `toml` 1.x's `FromStr for Value` parses a bare *value*, not a document, so
  it failed at `[workspace]` on every real Cargo.toml and silently returned an all-false result. Every binding-crate
  emitter that consults it (ffi, php, ruby, node, python, dart) therefore emitted a literal `version = "…"` instead
  of `version.workspace = true`, and likewise dropped `readme`/`keywords`/`categories`/`license` inheritance, so
  generated crates drifted behind workspace-wide bumps. The same mistake in the elixir scaffold silently yielded an
  empty feature list.
- **ffi/cbindgen**: emit `[defines]` feature keys unquoted — the `format!` sat inside a raw string so the key
  carried literal backslashes; cbindgen's `DefineKey::load` splits on `=` and trims but never unquotes, so no `#if`
  guard was emitted for any feature-gated export.
- **cli/build**: run the `ffi_dependent` stage even when an earlier independent group fails. The result loops used
  `let (stdout, stderr) = result?;`, returning on first failure and making that entire stage — go, java, csharp,
  kotlin_android, zig, jni — structurally unreachable.
- **cli/build**: supply default build recipes for swift, zig and gleam; `build_command_for` had no arms for them
  and fell through to `_ => "false"`.
- **registry**: return an error instead of panicking for `Language::C`; listing `"c"` in `[workspace] languages`
  aborted the run.
- **codegen/errors**: stop interpolating the error code into the exception message (pyo3 and napi), violating a
  documented invariant in the same file; the code now travels through a structured channel (`code: u32` on the
  generated Info classes). Note the leaked value is consumer-dependent — a repo that allocates no codes leaks an
  UNKNOWN sentinel uniformly, one that allocates leaks real codes — so the regression test asserts absence of any
  `[N]` prefix rather than a particular value.
- **codegen/errors**: restore newline separation between generated match arms; a nested `{%- if %}` inside
  `{%- for %}` collapsed the whole `match` body onto one line.
- **scaffold/ownership**: normalise the ownership-manifest key against `base_dir`. Callers disagreed on spelling —
  most commands pass an absolute `current_dir()`, the version-regen helpers pass `PathBuf::from(".")` — so the same
  file produced two keys and ownership established by one command was invisible to another.
- **scaffold/poly**: dedupe managed TOML arrays by decoded value rather than serialized text, and prune entries
  alef itself previously generated and no longer does. Pruning is provenance-gated so consumer-authored entries are
  never removed.
- **readme**: embed a provenance marker in generated READMEs so regeneration no longer depends on gitignored
  machine-local state.
- **docs/c**: document the real C ABI — handles render as the scalar handle type rather than invented per-type
  struct pointers, `bool` renders `int32_t`, a fallible void-returning function documents its `int32_t` status
  return, optional parameters no longer gain a second `*`, the error phrase is selected by return shape (`-1` /
  handle `0` / `NULL` / numeric `0`) instead of a blanket `NULL` claim, and every C type page states that the type
  name is documentation-only.
- **docs/go**: render static methods without a receiver, and pointer-wrap `Named` returns to match what the Go
  backend actually emits.
- **docs**: reject reserved words and malformed identifiers in generated signatures. The docs pipeline emitted a
  `new` constructor uniformly across languages with no check that the token was a legal identifier there — in Java
  and Dart `new` is reserved, so the documented signature was not parseable source.
- **e2e/tests**: emit a real assertion for a fixture whose only assertion is `not_error`, across python, php, java,
  csharp, swift, dart, elixir and typescript. Each backend treated `not_error` as needing no statement — correct in
  isolation — but when it was the only assertion the result was discarded entirely, and php additionally emitted
  `expectNotToPerformAssertions()`, which suppresses PHPUnit's own risky-test detector. Also stop emitting assertion
  helpers into files that never reference them, and derive the "has a usable assertion" decision from rendered
  content instead of a separately-maintained predicate that could drift.
- **e2e/kotlin, e2e/c**: fail generation instead of splicing a placeholder into an argument list. An unimplemented
  `TestBackendEmission` carries an `arg_expr` of literal comment text, which was pushed into the positional argument
  list unchecked; an unregistered trait bridge pushed a bare `null`.
- **backends/go**: derive snippet call shape from the extracted `FunctionDef` instead of re-asserting it in
  configuration, and infer `ptr(N)`'s type from the destination field rather than the literal.
- **backends/java**: serialize `Duration` fields as the real wire shape via paired Jackson converters, on both the
  record component and the builder setter.
- **backends/zig**: wrap an optional capsule return (`emit_function` matched only `TypeRef::Named`), use the
  correctly prefixed visitor-callbacks struct name, and back `VisitorHandle` with `u64` rather than `*anyopaque`.
- **snippets/zig**: emit a relative dependency path and a `fingerprint` in the generated `build.zig.zon`; Zig
  rejects an absolute `.path` outright.
- **backends/php**: sort constructor parameters required-before-optional in the runtime binding to match the stub,
  and apply the stub's `Duration` widening so the two agree on type and nullability.
- **backends/php, backends/extendr**: stop suppressing a generated enum variant factory when its name collides with
  an `enum_def.methods` entry; no backend forwards those methods into generated output, so the suppression dropped
  the factory with nothing replacing it.
- **backends/magnus, backends/php, backends/napi**: stop declaring in stubs what the binding generator does not
  emit. Each had a declaration generator and a binding generator independently deciding what exists; the stub side
  now consults the binding side's own predicate.
- **extract/reexports**: AND-combine a re-export or module `cfg` with an item's own instead of filling only when
  absent, so a type behind `#[cfg(feature = "a")]` re-exported through a `#[cfg(feature = "x")] pub mod` no longer
  loses `x`.
- **internal**: assert that every `.jinja` file on disk is present in its backend's template registration array.
  Template lookup resolves against a static array rather than the filesystem, so an unregistered template compiled
  fine and panicked at runtime; this happened three times in one day across two backends. The guard's first run
  surfaced 49 orphaned or superseded template files, all removed.
- **tests**: pin the working directory of `dart` and `kotlinc` child processes. Other tests mutate the
  process-global cwd into tempdirs that are then dropped, so an inherited cwd could already be deleted and the
  toolchain died at startup rather than reporting any result.

- **build observability**: emit centralized backend completion events with `duration_ms` and explicit success, failure,
  or skip outcomes for every configured language.

- **generation pipeline**: refresh cbindgen headers after generated FFI sources and backend post-build steps in both
  `generate` and `all`, then enforce source/header symbol parity before either command succeeds.

- **generated documentation**: remove poly's internal `~keep` token while preserving the surrounding public prose
  across every binding backend.
- **generated manifests**: replace hash-stamped Alef-owned TOML manifests with the current generated definition so
  stale dependencies and feature declarations cannot survive regeneration; continue refusing unmarked manifests.
- **FFI linting**: preallocate generated handle-request vectors so single-handle entrypoints pass crate-denied Clippy lints.

- **Zig snippets**: compile against the generated package's exported module so transitive imports declared by its
  `build.zig` remain available during validation.
- **build pipeline**: execute Gradle backends directly and make unsupported backend tools fail instead of reporting a
  successful no-op.
- **WASM snippets**: resolve local packages from wasm-pack's flat `pkg` output instead of a nonexistent `pkg/nodejs`
  subdirectory.
- **Go snippets**: construct generated DTO fields with the same optional/default pointer policy as the Go binding
  backend, preventing both pointer-to-value and value-to-pointer struct literal mismatches.
- Generate owned scalar handles for optional trait-bridge alias fields by cloning the configured handle, and reject
  non-Copy, non-Clone named field getters instead of silently returning null.
- Fix WASM manifests to enable configured binding-side feature gates by default, keeping exported factories available in ordinary `wasm-pack` builds.

- **Python snippets**: remove request types supplied by per-call native `from_json` overrides from public imports so
  the native class is imported exactly once and cannot shadow or be shadowed by the public type.
- **C snippets**: define standalone success guards and only rewrite the expected-result assertion, preventing error
  snippets from testing a result before its declaration or comparing scalar handles with pointer sentinels.
- **Java documentation**: escape Rust `\\u{...}` syntax before Java's early Unicode processing so generated Javadoc
  remains compilable.
- **Java trait adapters**: omit lifecycle overrides when a bridge has no configured super-trait, keeping generated
  adapters consistent with their managed interfaces.
- **Default constructor extraction**: preserve manually implemented `Default::default` as a generated static constructor
  across FFI, Python, PHP, R, and other method-based binding surfaces.
- **FFI default constructors**: retain canonical zero-argument `default` exports for lifetime-bearing owned values
  when conservative reference metadata is present, while continuing to exclude other borrowed returns and parameters.
- **Java visitor handles**: use the imported `List` type in generated cleanup tracking so strict Java lint does not
  report an unused import.
- **full generation convergence**: generate documentation snippets before rendering READMEs, so a clean `alef all`
  consumes the current run's snippets instead of requiring a second pass to add result-display statements.
- **Node declarations**: escape block-comment closers in Rust documentation before embedding it in generated
  TypeScript declaration comments.
- **snippets check**: run the configured audit and gap checks that `--help` already promised, scoped and gated to
  agree with `alef validate`'s existing snippet gate. Audit and gap checks see `docs.snippets.dirs` only —
  `inline_dirs` are prose pages whose fences are validated as snippets, never `--8<--` include targets — and a
  snippet counts as referenced when a `[crates.readme]` mapping, a generated-snippet coverage ledger, or a queried
  Astro content collection names it. Audit is skipped without a configured `docs_dirs`, and gaps are skipped
  without either `docs_dirs` or `required_languages`. Audit errors and structural gaps (missing include targets,
  missing required language variants, undocumented skips, unknown fence languages) fail the gate; unreferenced
  snippets remain a `strict`-only failure. A coverage manifest recording missing fixture/language cells stays a
  warning unless `strict`, as before, instead of failing reference resolution outright.
  **Newly fails:** an unparsable `docs.snippets.required_languages` entry is now an error rather than being
  silently dropped, matching `alef validate`.
- **Java errors**: align last-error dispatch with the shared FFI conversion, core, and panic taxonomy, and safely
  handle missing error context.
- **generate manifests**: reconcile Alef-owned generated TOML manifests before post-build processing, so newly
  generated binding dependencies are available without requiring a prior `alef scaffold` or `alef all` run while
  handwritten manifests remain untouched, and fail generation instead of continuing to dependent post-builds when
  the required scaffold manifest set cannot be produced.
- **C# service bindings**: invoke configurators through the native ABI, marshal named record parameters through
  owned scalar handles for configurators, registrations, and entrypoints, and propagate native conversion failures.
- **Java owned handles**: keep service and opaque owners closeable when transfer setup fails, contain handler upcall
  failures at the native boundary, and use the exact C ABI carriers for service metadata.
- **C# owned handles**: keep service and opaque owners closeable when transfer setup fails, lease service owners
  through registration calls, and defer trait-bridge cleanup until native release and active callbacks complete.
- **Go bytes**: pass the output pointer, length, and capacity required by every direct owned-byte return instead of
  treating an infallible byte function's integer status as a NUL-terminated buffer.
- **Go handles**: compare named parameter and return handles with the scalar zero sentinel in direct wrappers.
- **FFI scaffold**: declare `serde` directly in generated FFI crates now that the handle registry requires
  `serde::Serialize`, using the centralized template version and cargo-machete metadata rather than relying on the
  core crate's transitive dependencies.
- **FFI borrowed defaults**: restore free default-constructor exports that return owned lifetime-bearing values,
  storing them as serialized handles while continuing to exclude borrowed returns and borrowed-handle parameters.
- **documentation snippets**: read client credentials from each fixture's configured environment variable, falling
  back to the generic `API_KEY` name instead of publishing mock credentials in C, C#, Dart, Java, Kotlin, Python,
  Rust, and Swift examples.
- **WASM documentation snippets**: derive direct-call fixture eligibility from the target's exported function
  surface, recording unavailable imports as missing coverage while retaining client-wrapper recipes whose methods
  are reached through the resolved per-call or default factory.
- **java default values**: suffix integer defaults for boxed `Long` record components with `L`, so generated compact
  constructors compile while continuing to distinguish an absent value from an explicitly supplied zero.
- **FFI borrowed contexts**: restore owned lifecycle, field-accessor, owned-self method, and default-constructor
  exports for lifetime-bearing visitor contexts while continuing to reject APIs that pass borrowed handles across
  the ABI boundary. Non-`Send` contexts are stored as type-keyed serialized snapshots, preserving the registry's
  `Send` invariant and scalar handle ABI without erasing live visitor-context symbols.
- **e2e/ruby snippets**: bind collected streaming values through the configured result variable, keeping the
  assignment and subsequent `puts ...inspect` reference synchronized instead of binding an unused `chunks` variable.
- **e2e/elixir snippets**: bind collected streaming values through the configured result variable, keeping the
  assignment and subsequent `IO.inspect` reference synchronized instead of binding an unused `chunks` variable.
- **R default arguments**: only call a generated class's `$default()` wrapper when that class is actually eligible for
  extendr registration, preventing required options from referencing removed wrappers.
- **Go snippet validation**: preserve configured `GOMODCACHE` and `GOPATH` paths in the sanitized tool environment
  and derive Go's home-based defaults when they are not explicitly exported, so generated snippets can reuse
  available modules instead of failing before validation.
- **C documentation snippets**: keep configured client-method identities separate from prefixed ABI symbols, so
  adapter metadata resolves and real client/streaming examples are emitted; unresolved recipes now enter the
  missing-coverage ledger instead of compiling as successful diagnostic skip stubs.
- **readme tests**: align the structured function-surface template fixture with Minijinja's Boolean rendering.
- **zig visitor tests**: assert scalar-handle serialization through the configured FFI symbol instead of a
  hardcoded placeholder name.
- **e2e/zig visitors**: treat generated FFI result handles as scalar integers, using the zero sentinel instead of
  optional-pointer comparisons, captures, and unwraps while preserving pointer handling for returned JSON strings.
- **verify**: reject Alef-owned generated files whose header remains but whose `alef:hash` stamp is missing, so a
  mixed stamped/unstamped generated tree cannot pass freshness verification.
- **Python snippets**: bind successful non-void call results even when `docs.shows` or a presentation recipe consumes
  them, so generated examples display useful values instead of discarding the call result.
- **documentation snippets**: display successful non-void Rust, Swift, Zig, R, and Kotlin call results in generated
  examples; PHP already consumed these values.
- **FFI error header**: keep `AlefFfiErrorCode` reachable through generated cbindgen export filters and avoid repeated
  `ErrorError` tokens where an error type and variant meet in public C enum members.
- **FFI error enum members**: collapse consecutive repeated words inside the error type path, so a crate laying its
  error type out as `my_crate::error::Error` emits `MyCrateErrorNotFound` rather than `MyCrateErrorErrorNotFound`.
  The previous pass only elided the repeat at the type/variant boundary and left the path-internal stutter intact.
- **FFI error enum members**: namespace alef's five built-in codes with the project ABI prefix, so `None` is emitted as
  e.g. `SampleAlefNone`. cbindgen applies `[export] prefix` to the enum type but copies member names into the header
  verbatim, and C enum members are global identifiers — the bare names collided with platform headers (X11 defines
  `None` as `0L`) and with any second alef-generated library in the same translation unit.
- **kotlin errors**: let unnumbered error variants use the runtime fallback instead of panicking while generating
  Kotlin/Native bindings that mix explicitly numbered and fallback variants.
- **Ruby/Magnus errors**: reconstruct tuple error variants with positional Rust syntax for every binding
  representation that emits tuple variants, including adjacently tagged enums, while retaining struct syntax for
  named variants and bare syntax for unit variants.
- **generated-file provenance**: align hash extraction with the raw header window used by injection, including a stamp
  emitted at zero-based line 10 after Markdown frontmatter, and only strip exact generated stamp shapes immediately
  following an Alef header marker so hash-like body prose remains untouched.
- **zig errors**: let unnumbered error variants use the stable unknown-code fallback instead of panicking while
  generating bindings that mix explicitly numbered and fallback variants.
- **test fixtures**: keep the version-pin fixture aligned with the root-flat config shape and give the Go capsule
  fixture the complete borrowed-static ABI contract required by capsule validation.
- **dart/flutter_rust_bridge**: give FVM a persistent Alef cache when running
  `flutter_rust_bridge_codegen`, so clean regeneration worktrees reuse the installed Flutter SDK instead of
  downloading it again. Explicit `FVM_CACHE_PATH` and legacy `FVM_HOME` settings remain authoritative.
- **dart/flutter_rust_bridge**: reuse a persistent, crate-scoped Cargo target directory for FRB macro expansion.
  Clean regeneration worktrees now retain Cargo fingerprints, dependencies, proc macros, and build-script artifacts
  instead of recompiling the full Rust crate for every `cargo expand`; an explicit `CARGO_TARGET_DIR` still wins.
- **wasm scaffold**: publish a conditional `exports` map that resolves package self-imports to generated Node entrypoints
  while keeping the browser condition on the explicitly initialized web build. This lets generated snippets and e2e
  tests import the package by name after `wasm-pack` builds its target directories.
- **FFI errors**: replace unstable hash-derived domain error codes with explicit `#[alef(error_code = N)]` allocations,
  validate their public range and uniqueness, and emit a cbindgen-visible `AlefFfiErrorCode` enum. Unannotated variants
  now use the stable `Unknown = 2` fallback instead of accidentally creating a rename-sensitive ABI contract.
- **e2e/dart snippets**: stop emitting a call to the undefined `_fixtureUrl` helper in doc snippets for
  `client_factory` calls. The helper is defined only by the full e2e test-file emitter, never by the standalone
  snippet emitter, so every snippet constructing a client failed to compile with "The function '_fixtureUrl' isn't
  defined." Snippets now build the client with just the API key, matching the PHP, Ruby, Go, and TypeScript emitters,
  which likewise omit the mock-server `baseUrl` from their doc snippets.
- **Python snippets**: import every symbol the emitted snippet body references. Import candidates were
  computed only when a fixture group held at least one fixture *not* skipped for Python, but a docs
  snippet is emitted for every fixture regardless of skip status and lifts its import block out of that
  same rendered test file. A Python-skipped fixture therefore produced a snippet whose body called the
  configured client factory and constructed the request type while importing neither, and snippet
  validation failed with `unknown-name`. Candidates are now derived for every non-HTTP fixture and then
  pruned to the identifiers the emitted unit actually references, so nothing referenced goes unimported
  and nothing imported goes unreferenced.
- **C snippets**: define the `ALEF_TEST_SKIP` guard macro inside every emitted snippet that references it.
  A fixture declaring `[env] api_key_var` without a mock server renders an `ALEF_TEST_SKIP(...)` env guard,
  but the macro is declared only in the generated e2e runner header, which a standalone documentation
  snippet never includes — so the emitted translation unit failed to compile with a
  `call to undeclared function 'ALEF_TEST_SKIP'` error. The snippet-local definition returns `EXIT_SUCCESS`
  from `main` rather than the runner's bare `return`, which is valid only inside its `void test_*(void)`
  functions, and is `#ifndef`-guarded so an enclosing definition still wins.
- **csharp**: derive the element type of an array-valued field in an object initializer from the owning
  struct's `Vec<T>` field rather than hardcoding `List<string>`. Genuinely typed collections
  (`List<Message>`, `List<RerankDocument>`) were emitted as string lists, which does not satisfy the
  generated property, and scalar fields binding to `JsonElement` were emitted as bare literals. Both now
  route through the same per-element `JsonSerializer.Deserialize<T>` rendering that top-level array args
  already use, falling back to `List<string>` only when the element type is genuinely unresolvable.
- **wasm/typescript**: emit the raw value for a `#[serde(untagged)]` data enum field instead of an enum
  member reference. Such an enum serializes as the bare payload of whichever variant matched, so a
  string-typed instance is the JS value itself; treating it as `EnumType.Variant` turned an empty string
  into `WasmEmbeddingInput.`, a syntax error. Mirrors the representation gate the napi `.d.ts` dispatcher
  uses.
- **napi**: declare internally-tagged enums with newtype-of-struct variants as the flat optional-field
  object the napi glue actually emits, instead of a discriminated union keyed by the tuple field's
  synthetic `_0` name. The generated `.d.ts` previously leaked the internal field name as a literal `0:`
  property (e.g. `{ role: 'system'; 0: SystemMessage }`) and wrapped each variant as its own union member,
  when the compiled `#[napi(object)]` struct is actually one type with every variant's field present as an
  optional property (e.g. `{ role: 'system' | 'user'; system?: SystemMessage; user?: UserMessage }`).
- **napi**: dispatch the `.d.ts` enum arm on the enum's actual serde representation rather than on
  whether any variant carries a payload. `is_data_enum` gated on `serde_tag.is_some()` *and* at least
  one variant having fields, and was the arm's only switch, so two representations fell through to the
  plain string-enum branch and silently lost their wire shape. An internally-tagged enum whose variants
  are all unit variants serializes as `{"kind":"A"}`, not the bare string `"A"`; the glue generator
  gated on the same condition, so the binding conversion was wrong in the same way and both are
  corrected together. An untagged (`#[serde(untagged)]`) data-bearing enum serializes as a bare union
  of each variant's own shape and was likewise declared as a payload-free string enum, discarding every
  field. Externally tagged data enums remain unhandled and are tracked separately — declaring a shape
  the runtime does not produce would be worse than the current omission.
- **snippet typecheck validation**: stop reporting `effective_level: typecheck` for PHP, Ruby, and Elixir
  snippets that were never actually type-checked. Every level below `Run` ran the same syntax-only check
  (`php -l`, `ruby -c`, `Code.string_to_quoted`) while the validator declared `max_level: Run`, so a
  snippet referencing an undefined class, constant, or module passed as a `typecheck` result — the entire
  population of typecheck passes for these three languages was unverified. A validator can now report,
  through a new `achievable_level` hook, the deepest level its *current environment* actually backs,
  separately from `max_level`'s fixed per-language ceiling; a `typecheck` request for these three
  languages now reports `effective_level: syntax` and counts as `downgraded`, which a strict run treats
  as incomplete coverage instead of a pass. No zero-config real type-checker exists for any of the three
  that can safely analyze an isolated snippet: PHP's PHPStan/Psalm need the project's composer autoload
  or every legitimately external symbol reads as unresolvable, Ruby's Sorbet/RBS need project-wide
  `# typed:` sigils the generated snippets don't carry, and Elixir's Dialyzer needs an out-of-band PLT
  this harness doesn't build. Wiring one in with the project context it needs is left for a follow-up.
- **snippet typecheck validation**: apply the same `achievable_level` cap to Bash and R snippets. Both ran a
  syntax-only check below `Run` (`bash -n`, R's `parse(file = ...)`) while declaring `max_level: Run`, so a
  snippet referencing an undefined command or function likewise passed as an unverified `typecheck` result. A
  `typecheck` request for either language now reports `effective_level: syntax` and counts as `downgraded`.
  ShellCheck and R's `codetools::checkUsage`/lintr exist but aren't wired up here, so `typecheck` must not be
  claimed until they are.

- **C# e2e**: omit the illegal `private` modifier from file-scope generated visitor classes while preserving the
  same class shape when the visitor appears inside a nested test container.
- **go**: compare value receivers returned by `_from_json` against the scalar `AlefHandle` zero sentinel instead of
  `nil`, allowing generated non-opaque methods to compile under cgo.
- **Zig snippets**: import the configured binding module rather than the e2e release-package alias, keeping registry
  artifact naming separate from the public module identifier.
- **Dart snippets**: import the configured public library entrypoint instead of deriving the filename from the Rust
  crate name, while preserving the separately configured e2e dependency alias.
- **snippets**: let a fenced code block's own tag win over the file's front-matter `language:` during discovery. A
  markdown file with multiple fenced blocks in different languages (e.g. a `toml` config block followed by a `json`
  block) had every block forced onto the front-matter language, so the non-matching blocks were validated with the
  wrong toolchain and failed for reasons unrelated to their actual content. An unrecognized or absent fence tag still
  falls back to the front-matter language, then to the directory-derived language.
- **CLI**: `alef all` now synchronizes registry package versions before generation and reloads changed configuration,
  allowing stale Zig registry hash version prefixes to self-heal instead of aborting clean canaries.
- **napi**: omit `..Default::default()` from fully populated adjacent-enum constructors while retaining it for
  partial variants that still require defaulted fields.
- **rust snippets**: locate generated test-function boundaries without treating braces inside raw strings, escaped
  strings, or comments as the function terminator.
- **rust snippets**: declare the `tokio` dependency an async snippet needs. `rust/snippet_body.rs.jinja` emits
  `#[tokio::main]` for every async fixture, but the snippet carried no matching crate requirement, so the validator
  built a check project with nothing in `[dependencies]` and every async Rust snippet failed on E0433
  (`use of unresolved module or unlinked crate tokio`) and E0752 before any of the behaviour it demonstrates was
  checked. The requirement is pinned with `features = ["full"]`: `#[tokio::main]` lives behind tokio's `macros` and
  `rt-multi-thread` features, so a bare version line resolves the crate and still fails to compile.
- **zig snippets**: pass the include directories the build manifest declares to the reconstructed `build-exe`
  command. The session validator read the manifest for its module name and root source only and discarded the
  `addIncludePath` the same file declares, so a snippet reaching a `@cInclude` inside the binding failed with
  `C import failed ... 'header.h' not found` while `zig build` against that identical manifest succeeded — the
  harness was validating something other than what ships. An include path bound through a
  `b.option(...) orelse "<default>"`, the shape Alef's own `build_zig.jinja` emits, resolves to its default; any
  other expression is skipped rather than guessed at, since a wrong `-I` is worse than none.
- **e2e snippets**: use the configured exported error type instead of fabricating one from the crate name, and emit
  Go error values with the correct non-pointer shape.
- **scaffold**: derive Python, PHP, and FFI `.gitattributes` entries from the source crate directory, matching the
  binding crate paths Alef actually scaffolds when the configured package name differs from its Rust crate path.
- **zig scaffold**: emit the example with Zig 0.16's `std.Io` API and avoid an unused allocator binding, so the
  scaffolded example compiles with the supported Zig toolchain.
- **e2e/node**: import enum classes referenced as runtime values by generated typed-input builders, including enum
  fields discovered recursively from the IR rather than declared in per-language overrides.
- **validate**: stop treating vendored/fetched copies of a crate's `Cargo.toml` as authoritative when checking
  `Cargo.lock` versions. `alef validate versions` built a single name-to-version map from every `Cargo.toml` under
  the workspace root, keyed by package name alone. A frozen manifest left behind by dependency vendoring — a
  Rustler `vendor/` tree carried for offline builds, or a Mix `deps/` fetch of a published package that bundles its
  own native crate source — declares the same package name at whatever version was current when it was
  vendored/fetched, silently overwriting the live crate's entry in the map. Every other `Cargo.lock` in the repo
  that already matched canonical then compared against the stale vendored version and was reported as a
  `[MISMATCH]`, even though the lockfile agreed with the live source. `vendor` and `deps` directories are now
  excluded from both the `Cargo.toml` manifest scan and the `Cargo.lock` check, the same way `target`/`.git` already
  are. Genuine drift in a real (non-vendored) `Cargo.lock` still fails.
- **cli**: stop `generate`, `scaffold`, and `all` from silently rewriting `alef.toml`'s `alef_version`. The pin never
  gates generation and projects may coordinate it with external installer or workflow pins that generation cannot
  update; release version sync remains explicit.
- **validate**: stop descending into nested git checkouts that `.gitmodules` does not register. `alef validate
  versions` walked every directory under the workspace root when collecting `Cargo.toml`/`Cargo.lock` files,
  including a linked `git worktree` checked out inside the repo (e.g. under `.worktrees/`) — an independent checkout
  sitting at a different commit. Its manifests reported `[MISMATCH]` noise against the host repo's canonical
  version, could poison the version map for a package name it happens to share with the live tree, and — for a
  worktree mid-regeneration — could differ between two runs of the same command. A directory whose root carries a
  `.git` entry is now skipped, but **only** when `.gitmodules` does not register it as a submodule path: a
  registered submodule is a declared part of the repo's version surface and is still walked by both the manifest
  scan and the `Cargo.lock` check, so genuine drift inside a submodule keeps failing.
- **ffi**: return the scalar `0` sentinel from string-bridge parameter and UTF-8 guard failures when the exported
  ABI returns `AlefHandle`, instead of emitting `null_mut()` and producing uncompilable generated Rust.
- **zig snippets**: omit the parsed-result binding when generated assertions never reference it, avoiding an
  unused-local compile error while preserving the binding for assertions that consume the result.
- **java scaffold**: exclude Alef's `.alef` validation scratch directory from Maven Checkstyle scans without
  suppressing violations in user source files.
- **go**: represent generated FFI handles consistently as scalar `AlefHandle` values, including visitor and
  options conversion paths, and compare their failure sentinel against `0` instead of `nil`.
- **C examples and e2e**: render scalar handle declarations and absent-value sentinels as `AlefHandle`/`0` while
  retaining `NULL` for pointer-valued strings and other pointer parameters.
- **ffi**: honor configured extra Clippy allowances and allow `collapsible_if` in generated crates without raising
  their minimum supported Rust version.
- **e2e/ruby**: emit a fixture category's spec file whenever its fixtures render executable examples. The
  category-level gate in `ruby.rs` decided whether to emit the file using a predicate that omitted `is_streaming`,
  while the per-fixture branch in `spec_file.rs` decided what to put in it using one that included it. A category
  whose fixtures were all streaming was therefore dropped whole — no file — even though the ruby streaming emitter
  renders those fixtures fully. Nothing downstream notices an absent category: `alef verify` walks emitted markers,
  the empty-category check in `e2e/validate.rs` only fires when *every* configured language skips a category, and
  `fixture_inclusion` never consults an emitter's capability. Both callers now share one predicate so they cannot
  drift apart again, and a category that genuinely renders nothing executable is logged instead of vanishing.
- **generate**: run the converging formatting pass on `alef all`, so a full regen lands committable. `alef all`
  called `format_generated` with `Some(&changed_languages)`, which selects the single-pass branch; the convergence
  loop — written because poly's `.cs`/`.java`/`.json` engines are not single-pass idempotent, and documented as
  serving "the `alef all` path" — was therefore unreachable from the one command that regenerates everything. A
  regen left formatting drift that `poly fmt --check` rejected, `finalize_hashes` then stamped provenance over that
  drift, and a second `alef all` silently settled it — which is why regenerating twice produced changes the first
  run should have made. The language filter was also wrong for the workspace-wide `cargo sort -n -w` folded into
  that loop, which must cover crates the current run did not generate.
- **defaults**: carry the elements of a non-empty collection literal through the IR instead of discarding them.
  Every `vec!`/`hashmap!`/`hashset!` default collapsed to `DefaultValue::Empty`, so a Rust default of
  `vec!["noscript"]` reached the backends indistinguishable from `vec![]` and every binding emitted an empty
  collection — a silent cross-language behavioural divergence, not a cosmetic one. The guard that appeared to
  separate the two cases was dead code: both of its branches returned `Empty`. A new `DefaultValue::ListLiteral`
  carries the elements, and Rust, Python, Kotlin, Swift, Dart, C#, Elixir and the docs renderer emit them. Go and R
  deliberately keep falling back to the empty collection, because Go needs the element type spelled out
  (`[]string{…}`) and R's `c()` carries vector-coercion semantics of its own — guessing either risks a default that
  differs from the Rust one. A genuinely empty `vec![]` still lowers to `Empty`, and a literal containing any element
  that cannot be rendered self-containedly — notably a function call — falls back to `Empty` whole rather than
  lowering a partial list. **Consumer-visible:** a field whose Rust default is a populated collection now generates
  that collection as its binding default instead of an empty one.
- **snippets**: scope the shared validation timeout to validators that genuinely batch. `run_validation` handed a
  single wall-clock budget, keyed by language/session/level, to the per-snippet path — which is reached only by
  validators that spawn one toolchain process per snippet (every language except Rust below `Run`). The first
  snippet in a group could consume the whole budget, after which the runner short-circuited every remaining snippet
  with a synthetic `Timeout` error naming a `<language> validation batch` command that was never spawned. Docs runs
  therefore reported timeouts that varied with snippet ordering and machine speed rather than with the snippets.
  Each snippet on that path now receives the configured `timeout_secs`; group budgeting is retained in
  `validate_batches`, where one process really does cover N snippets. **Consumer-visible:** a language group's
  worst-case wall clock is now `snippets × timeout_secs` rather than `timeout_secs`, so a `timeout_secs` tuned
  against the old shared budget may need lowering.
- **capsules**: expose `shares_native_runtime` for Kotlin Android and enforce capsule ownership/ABI contracts during
  Go, Swift, Zig, and Kotlin Android generation instead of leaving those backends outside the validation gate.
- **e2e/zig**: emit streaming e2e tests instead of discarding them, and never drop a fixture category in silence. A
  hardcoded zig-only filter excluded every fixture whose resolved call declared `streaming = true`, although the zig
  streaming emitter is fully written; the exclusion is now narrowed to the case it was guarding — a streaming call
  with no `client_factory`, which zig cannot render because streaming is exposed only as a method on the handle. A
  category left empty by that filter previously hit a bare `continue`: no file, no log and no gate failure, because
  `alef verify`, the empty-category check in `e2e/validate.rs` and `fixture_inclusion` all still reported zig as
  included, so a consumer whose config routed a streaming call to zig received nothing and was never told. Such a
  category now emits a placeholder suite naming every dropped fixture, plus a warning, so an unemittable category is
  visible in the output tree instead of vanishing.
- **java**: box a config field whose serde default is a non-zero integer literal, so a caller's explicit `0` is no
  longer overwritten by the default. Java records carry no per-component defaults, so a default is restored in the
  compact constructor by testing the incoming value, which requires a value meaning "nothing was supplied".
  `Duration` and `#[serde(default)]` fields already boxed for that reason; a plain literal default did not, leaving
  `0` as the only available sentinel — `if (maxRedirects == 0) { maxRedirects = 10; }` silently gave 10 redirects to
  a caller who asked for none. The "must box" predicate now covers a non-zero literal default and is applied at
  every site that has to agree — record component type, `@Nullable`, the compact constructor's condition, and both
  the builder field and setter types — through one shared helper, because a disagreement between them emits a record
  and a builder declaring different types for the same field. Java-local by construction: Python, C# and Kotlin emit
  per-field initializers and need no sentinel. **Consumer-visible:** affected record components and builder setters
  change from the primitive type to its boxed form.
- Make generated parameter-conversion failures use the scalar zero sentinel whenever the exported FFI return type is
  `AlefHandle`, even when the source return metadata has a pointer-shaped fallback.
- **config generation**: report every Rust-binding field whose serde default function cannot be preserved in one
  actionable generation error instead of panicking on the first field. The diagnostic now identifies each owning
  type, field, and function and explains the public, unconditional static-method or literal-default remedies.
- **validate**: compare csproj `AssemblyVersion`/`FileVersion` in the 4-component .NET form rather than against raw
  canonical SemVer. The generator stamps both fields through `to_dotnet_assembly_version` — `1.17.0` becomes
  `1.17.0.0`, and a prerelease such as `1.9.0-rc.48` becomes `1.9.0.0` — because .NET rejects SemVer prereleases in
  those attributes. `alef validate versions` compared all four csproj fields against the raw canonical string, so the
  two assembly fields could never match and every consumer with C# enabled reported permanent mismatches on output
  alef itself is required to produce. Under `--exit-code` that is a permanently red release gate. `Version` and
  `InformationalVersion` carry the full SemVer and keep comparing raw, so the four-component form is still rejected
  there.

- **csharp**: only emit the `Register<Trait>` facade when the trait bridge declares a `register_fn`. The facade calls
  `NativeMethods.Register<Trait>`, which is generated solely for bridges that have a native register function, so a
  bridge configured without one produced a call to an undeclared member — `CS0117`, failing the whole package build.
  The unregister facade two lines below was already gated on `unregister_fn`, and Java (`gen_bindings/facade.rs`) and
  Go (`gen_bindings/mod.rs`) already gate on `register_fn`; C# was the sole outlier. Emission is narrowed rather than
  a declaration invented: no register-shaped symbol exists in the Rust exports, the cbindgen header or the built
  dylib, so declaring the extern would convert a build error into a runtime `EntryPointNotFoundException`.
  **Consumer-visible:** bridges without a `register_fn` no longer expose a `Register<Trait>` method that could never
  have worked; bridges with one are unaffected.
- **e2e/fixture**: accept a `skip.languages` id naming a known e2e target that isn't configured in this run, so a
  consumer holding a backend out of `[languages]` keeps its skip entries valid. Typo'd ids still fail, and `"ffi"`
  stays rejected because `Language::Ffi` maps to the `"c"` generator.
- **ffi/e2e/c**: keep options-field bridge functions, options, results, and visitor callbacks on the scalar
  generational `AlefHandle` ABI used by every ordinary managed value. The special options-field bridge bypass still
  emitted raw pointers after the 0.61 handle migration, while its JSON and visitor constructors returned scalar
  handles; generated C tests and snippets therefore failed strict compilation in both pointer-to-integer directions.
- **docs/snippets**: make a strict-mode failure actionable. `strict snippet validation failed for crate X: N
  validation(s) downgraded` reported only a total — no language, no snippet id, and no level transition — and the
  achieved level is not recorded in the emitted snippet frontmatter, so there was no other way to learn which
  snippets regressed or from what level. A real run reported 261 downgrades with no entry point to any of them.
  Strict failures (downgraded, unavailable, and failed/errored) now append a per-language tally with up to three
  sample ids and their `requested -> effective` transition, and say how many were elided. The validation report
  configured by `report_output` is also written *before* the strict bails rather than after: a run that fails
  strict mode is precisely the run whose report is needed, and emitting it afterwards meant the artifact was never
  produced in that case.
- **e2e/format**: make generated output reproducible when a formatter fails. Languages were collected into a
  `HashSet` and formatted in its iteration order, which is randomly seeded per instance, and the loop aborted on
  the first failure with `?`. Together those meant one failing language left a *different, arbitrary* subset of
  the remaining languages unformatted on every run: regenerating an unchanged tree produced different bytes.
  Observed on a consumer whose registry-mode Go test app cannot pass `go mod tidy` before its version is
  published — two consecutive `alef all --clean` runs differed in 143 files, the next pair in 47, all under
  `test_apps/`, with clang-format applied to a varying subset. Languages are now formatted in sorted order and
  every language is attempted before failures are reported, so the emitted tree no longer depends on ordering or
  on whether an earlier language failed. The run still fails, and now names every language that failed rather
  than only the first.
- **tests**: refresh Kotlin Android, Swift, and Zig snapshots so the checked-in expectations cover the integrated
  JNI path discovery, serde bridge grouping, numeric error dispatch, and fallible string ownership behavior.
- **ffi/scaffold**: declare every feature named by a `#[cfg(feature = "X")]` gate the codegen emits into the
  generated FFI crate. Cargo features are per-crate, so a wrapper whose `[features]` table forwards only
  `full = ["<core>/full"]` never defines `X` itself, even when `full` enables `X` on the core dependency. The
  emitted gate was therefore unsatisfiable under *every* feature selection: `rustc` reported it as an
  `unexpected cfg condition value` warning and silently dropped the item, while `cbindgen` — which does not
  evaluate the gate — kept declaring it in the header. The crate still built with exit 0, so the failure surfaced
  only downstream, as a link error or `dlsym` miss for every C-ABI consumer against a header that promises symbols
  the `cdylib` does not export. Features discovered from emitted gates are now declared as passthroughs and default
  ON, preserving the surface each gate was written to expose while keeping it switchable; `[crates.ffi]
  extra_features` keeps its documented declare-but-do-not-enable behaviour for mutually-exclusive alternatives such
  as a `wasm-http` backend, and a gate naming one of those entries does not promote it into `default`. This mirrors
  the feature collection the dart, wasm, swift, and extendr backends already perform.
- **e2e**: stop registry-mode dependency resolution from making a release unreachable. Registry-mode test apps pin
  the version the current run produces, so any post-generation step that resolves those manifests against a registry
  (`go mod tidy`, or a user `format` override shelling out to a resolver such as `bundle exec`) cannot succeed until
  that version is published — while publishing requires the run to finish first. The failure aborted
  `run_formatters` mid-stage, so `finalize_hashes` for the test apps, the orphan sweep, the stage cache write and the
  entire **docs** stage never ran: one unpublished package left the test apps unstamped and every generated docs page
  stale, and re-running could not converge. Formatting itself is unchanged and still aborts generation in every mode
  — poly and `mix format` need no registry, so they have no pre-release excuse. Only dependency resolution is
  deferred, and only under `DependencyMode::Registry`: `go mod tidy` is skipped and recorded, and a failing user
  `format` override is recorded rather than fatal. `run_formatters` now returns the deferred steps as
  `Vec<DeferredFormatting>`, which `alef all` reports once the pipeline has completed. `DependencyMode::Local` — the
  mode that actually gates correctness — is behaviourally identical and always yields an empty list.
  **API-visible:** `run_formatters` and `run_formatters_for_cached_paths` return `Vec<DeferredFormatting>` instead of
  `()`.
- **kotlin-android**: emit a handle wrapper class only for opaque types some visible top-level function returns.
  A type that is `is_opaque` but that nothing returns cannot be constructed from Kotlin at all, yet still got a
  `<TypeName>.kt` whose `close()` called `nativeFree<TypeName>` — a symbol the Bridge object never declares and the
  native JNI shim never implements, so the generated module failed to compile with `Unresolved reference` on a class
  no caller could have instantiated. Matches the reachability predicate the JNI shim generator and the Bridge
  destructor emitter already apply.
- **jni/scaffold**: inherit the core crate's configured feature set when Kotlin Android does not provide a
  backend-specific override, while preserving explicit per-backend features.
- **e2e/wasm (tests)**: cover the fully-excluded-category path through `WasmCodegen::generate`, not only through the
  renderer. The existing unit test called `render_wasm_excluded_category` directly, so it verified the renderer but
  never that `generate` invoked it — reintroducing the silent-drop regression left all seven wasm unit tests green.
  The new integration test drives `generate` end to end and fails when the category is dropped.
- **generate**: record scaffold-emitted manifest paths (`composer.json`, `package.json`) in a durable
  per-crate manifest and reclaim them, along with their package-manager lockfile siblings, when a later
  `alef all` run stops emitting them at that path. Every existing orphan-cleanup input (`write_lang_manifest`,
  the `generate-{lang}-ownership` stage) filters scaffold paths through `carries_alef_marker()`, which a
  `generated_header: false` manifest never satisfies, so `sweep_manifest_orphans` was always called with an
  empty `previous_paths` for these files and could never reach manifests left behind by a layout change, even under
  `--clean`. The marker-free route is confined to an explicit two-name allowlist of
  manifests that are structurally incapable of carrying a marker; a lockfile is never reclaimed on its own
  provenance, only as a cascade of its manifest being reclaimed.
- **generate**: refuse to write a `generated_header: true` scaffold/e2e file over pre-existing content that
  carries no `alef:hash:` marker, instead of stamping or overwriting it unconditionally. A plain `alef all
  --clean` run could silently claim and stamp hand-written files because the write path only ever asked "does this
  run want to emit here", never "has alef ever recorded owning this exact path". The check is marker-based
  rather than cache-based (`.alef/`'s manifests are gitignored local scratch space that does not survive a
  fresh clone or a cache-less CI job), so it holds durably across sessions without narrowing any legitimate
  regeneration. The check is skipped for paths alef cannot stamp (`.md` above all — no generated README has
  ever carried a marker), where a missing marker is not evidence of foreign content and enforcing would
  freeze regeneration permanently.
- **e2e/elixir**: emit `test/test_helper.exs` with `generated_header: true` instead of `false`. As a
  `generated_header: false` seed it never carried a marker even though alef legitimately re-authors it on
  every run, so the write path had no durable way to tell "alef's own unmarked output" apart from
  "hand-written foreign content" for it.
- **e2e/kotlin**: honor `fields_json_scalar` for fixture field paths that carry a virtual namespace prefix (e.g.
  `interaction.action_results[0].data`), and accept the same bracket-wildcard/de-indexed spellings already
  interchangeable in `fields_optional`. `field_is_json_scalar` compared the raw fixture path directly against
  the configured set, but `accessor()` strips the namespace prefix before building the field expression — so a
  `fields_json_scalar` entry configured against the stripped struct path (`action_results[].data`) never
  matched, and the field fell through to the plain `.orEmpty()` fallback. `.orEmpty()` is a `String?`
  extension undefined on the `Any?` a JSON-scalar field actually has, leaving the generated Kotlin e2e module
  uncompilable.
- **magnus**: apply a field's real `#[serde(default = "...")]` value instead of raising `missing required
  field` for it. Any `Named`-typed field without an `EnumVariant` typed default was treated as required,
  which also caught fields that carry a genuine callable default (`FunctionCall`/`PublicFunctionCall`) —
  silently dropping the default and forcing Ruby callers to pass the field explicitly, even though the
  generated `.rbs` stub still declared it optional. The default's return value is converted with `.into()`
  for the same reason as the wasm fix: Magnus mirrors `Named` types into their own `#[magnus::wrap]` struct,
  a distinct Rust type from the core one under the same short name.
- **wasm**: convert a `#[serde(default = "...")]` function's return value into the field's wrapper type in
  generated constructors. A defaulted field whose type is `Named` (e.g. `ssrf: SsrfPolicy`) is mapped to a
  distinct binding wrapper (`WasmSsrfPolicy`), but the constructor's fallback expression called the core
  default function directly without `.into()` — an `E0308` type
  mismatch that broke every wasm build with a defaulted, wrapped-type field.
- **e2e/wasm**: stop silently dropping a fixture category when every fixture in it is excluded for wasm (e.g. an
  entire `visitor` category skipped via `skip.languages`); emit a placeholder suite naming each excluded fixture and
  its reason, and log a warning, instead of generating no output at all.
- **e2e/r, e2e/c**: stop trimming only the actual side of an `equals` assertion. R wrapped the result in `trimws(...)`
  while emitting the fixture's expected literal verbatim, and C routed string equals through a helper that trimmed
  trailing whitespace off the actual value only; both made assertions against expected values with a legitimate
  trailing newline permanently unsatisfiable. `equals` now compares both sides exactly, matching every other
  generated language.

- **config/services**: accept registration variant `languages` maps and carry each canonical language's style,
  handler shape, and method prefix through extraction into service generation. Python decorator overrides now use
  their registered overload templates instead of failing after config parsing.
- **ffi/services**: carry an explicit response-deallocator callback beside every service handler callback and invoke
  it after copying the response but before fallible deserialization. Rust previously called process-global `free()`
  on host allocations, including C# `Marshal.StringToCoTaskMemUTF8` memory, which crosses allocator families and can
  corrupt the Windows heap. C#, Java, Go, and Zig service registrations now pass their matching deallocator, and the
  generated service wrappers use the scalar `AlefHandle` carrier consistently.
- **csharp/zig**: invalidate host wrappers immediately after a consuming native method returns, including error
  returns, so fluent builders cannot retain a stale owner while wrapping the replacement handle.
- **ffi/csharp/zig**: return every byte buffer through the owned pointer/length/capacity ABI, including infallible
  borrowed slices, so hosts copy exact binary data and free only the allocation transferred by the FFI layer.
- **java/service**: bind service constructors, owners, metadata, registration variants, and entrypoints with the
  scalar `AlefHandle` carriers and canonical C export names emitted by the FFI backend; marshal text metadata to
  native segments, invoke Panama handles with typed arguments, and validate the service symbols at load time.
- **extract/ffi**: recognize serde implementations that derive one direction and implement the other manually, so
  named parameters such as authorization configuration emit the FFI JSON constructor their Java and C# wrappers call.
- **config**: reject unknown keys in closed Alef configuration sections instead of silently discarding misspelled or
  misplaced settings; extension maps remain open where arbitrary names are part of the schema.
- **ffi/java**: always emit JSON constructors for serializable FFI types and declare matching Java lifecycle handles,
  including types reached through generated facade fields rather than direct function parameters.
- **go/visitor**: keep options-field visitor context and result types out of generic binding and method emission, so
  generated Go packages no longer call FFI symbols intentionally omitted for borrowed visitor-associated types.

- **kotlin-android**: locate the configured JNI crate by walking from the generated Gradle project, accept an
  explicit manifest override, and copy host libraries from the Cargo workspace target directory.

- **swift**: emit reachable opaque-handle type aliases even when no capsule mapping is configured, while avoiding
  duplicate declarations for client and capsule types.
- **e2e/dart**: emit trait-bridge stub factories in standalone snippets and avoid binding `Future<void>` calls to a
  result variable.
- **e2e/snippets**: prune previously generated snippet files that a later successful run no longer produces, using
  recorded generation ownership and language-scoped path differences without deleting hand-authored files.
- **pipeline/cache**: discover and re-stamp Alef-owned generated files omitted from a cache-hit language's in-memory
  path set, including files whose provenance hash line was previously stripped.
- **sync-versions**: keep C# assembly/file versions and the registry Rust test-app package version aligned with the
  consumer SemVer; .NET assembly fields now use the required four-component numeric form.
- **scaffold/poly**: lower configured exclusions separately for discovery gitignore semantics and hook glob matching,
  preventing nested fixture over-pruning and missed nested build/vendor directories.
- **docs/zig**: render function and enum-value identifiers in snake_case while retaining PascalCase type names in
  generated reference pages.
- **e2e**: use one normalized indexed-path convention across optional matching and eight accessor renderers, including
  TypeScript, Node, C#, Zig, Kotlin, Kotlin Android, Swift, and Dart.
- **e2e/go**: resolve fixture fields through their serde wire names so renamed DTO properties are populated instead
  of silently disappearing from generated Go literals.
- **e2e/typescript**: qualify inferred enum-field metadata by its owning type so an enum field cannot poison a
  same-named scalar field on another generated DTO.
- **snippets/strict**: treat a validator's declared `max_level` as a capability ceiling rather than a downgrade.
  A snippet that passes at its validator's maximum was reported as downgraded, and `strict` fails on any downgrade,
  so requesting a level that any validator caps below — `typecheck` with zig capping at `compile`, or toml/json/yaml
  capping at `syntax` — could never pass however healthy the environment was, and the only consumer workaround was
  lowering the level for every other language too. Such results now pass, carry a `capability_capped` flag, are
  counted in the run summary, and are surfaced through an explicit warning. Annotation-driven downgrades and
  environmental failures (unavailable toolchains, timeouts, errors) are unchanged and still fail strict.
- **jni**: request the core crate's configured feature set in the generated JNI `Cargo.toml`. Features were read only
  from `[crates.kotlin_android] features`, so a consumer that omits that key got a dependency on the core crate's
  default features while the generated shim still called into feature-gated modules — the crate then failed to compile
  with `E0433`. The lookup now falls back to the top-level `features`, matching every other binding scaffolder, and an
  explicit `[crates.kotlin_android] features` still wins.
- **extract**: apply a `#[cfg(feature = "…")] pub mod` gate to the items inside it regardless of which source file
  declares the module. Only `sources[0]` was scanned for module-level cfg attributes, but `sources` is an
  author-ordered list and the file holding the gated module is frequently not first. Every item under such a gate was
  recorded with no cfg, so backends that exclude items by cfg — notably wasm, whose target cannot compile
  non-wasm-safe modules — emitted calls into modules absent from their feature set.
- **ffi/all**: rebuild a missing or stale cbindgen header after all FFI source-writing stages and validate the refreshed
  declarations in the same `alef all --clean` run, instead of failing once and requiring a manual Cargo build before
  an identical second generation could succeed.
- **e2e/rust**: accept `crates.e2e.error_field_aliases` and apply the configured mapping when generated Rust tests
  assert fields on an error value.
- **jni**: honor `crates.jni.exclude_functions` when emitting top-level and instance-method shims, alongside the
  paired Kotlin and Kotlin Android exclusion lists.
- **scaffold/php**: emit exactly one composer.json per layout. The co-located layout now emits only the
  repository-root manifest; the package-directory copy is kept for the split layout, where it is the
  installable package. Both rendered the same composer `name`, so every co-located consumer carried a
  second, unreachable declaration of its own published package — Packagist reads the repository root, and
  consumer references target the class directory rather than the manifest.
- **ffi**: preserve registration methods' domain error types when a service owner is absent, and return the scalar
  zero sentinel when an opaque constructor rejects an enum discriminant.
- **e2e/snippets**: assert the whole rendered snippet document in the canonical-language test instead of a set of
  substring probes, so `level`, `requires` and `side_effect` are pinned again; a renderer emitting a bogus value for
  any of them previously satisfied the probes.

- **snippets/check**: build Rust snippet sessions with the crate's declared features in `alef snippets check`, the
  same merge `alef docs` already performs. The command dropped them, so the path dependency in the generated
  snippet-check manifest resolved with default features only and every snippet importing a feature-gated module
  failed with `unresolved import`.
- **e2e/snippets**: declare the `serde_json` dependency that generated Rust snippet bodies name. A `json_object`
  argument makes the Rust recipe emit `serde_json::from_str(…)`/`serde_json::from_value(…)`, but the snippet
  frontmatter and coverage ledger both reported `requires: []`, so the Rust snippet validator built its check
  project without the crate and every such snippet failed with `E0433: failed to resolve: use of undeclared crate
  serde_json`. Rust snippets now carry a `crate:serde_json` requirement, and the Rust validator resolves
  `crate:<name>` requirements into `[dependencies]` of the check project. Session configuration still wins: a crate
  declared under `docs.snippets.sessions.<target>.rust_dependencies` keeps its configured version and features, and
  a requirement Alef has no pinned version for fails with the config key to add instead of resolving silently.
- **ffi**: verify cbindgen declarations against the FFI source after writing it, so a stale-header failure leaves Cargo
  the new source it must rebuild instead of trapping generation in a rebuild loop; declaration matching also accepts
  formatted line breaks while retaining exact symbol boundaries, and includes exports from generated modules such as
  `service.rs` rather than treating their valid header declarations as removed.
- **codegen**: preserve public associated serde default providers as callable Rust paths instead of treating every
  `#[serde(default = "Type::function")]` as private and attempting structural JSON recovery, which failed when the
  owning configuration contained required nested named fields.
- **snippets**: stop the shared batch timeout from truncating to zero for the first validator in a batch.
  `remaining_batch_timeout` floored the time left via `Duration::as_secs`, so the near-full budget the very first
  caller sees (a few elapsed nanoseconds short of a whole second) rounded down to 0 and the freshly added
  zero-budget guard rejected every snippet in the batch without running any of them — a batch validated nothing,
  silently. The remainder now rounds up to the next whole second instead of down.
- **e2e/ruby**: compare `equals` assertions exactly. Ruby stripped both the actual and the expected value, which is
  symmetric and so never produced an unsatisfiable assertion, but it also made a genuine trailing-whitespace
  regression invisible in Ruby while every other backend compares exactly. String coercion is kept; normalization is
  not.
- **e2e/elixir**: stop normalizing the actual value in generated `equals` and `is_empty` assertions. `is_empty`
  emitted `String.trim(actual) == ""`, which passes for a whitespace-only value that Python's falsy check and
  TypeScript's length check both reject — so one fixture assertion disagreed across languages.
- **e2e**: compare `equals` assertions exactly in generated Python, PHP, Rust, TypeScript and WASM tests. The actual
  value was normalized with `.trim()`/`.strip()` while the fixture `expected` literal was emitted verbatim, so a
  fixture whose expectation legitimately ends in a newline could never be satisfied, and a genuine trailing-whitespace
  regression was silently absorbed. Neither side is normalized now.

- **magnus**: marshal `initialize`'s keyword types the same way as the accessors. The kwargs constructor
  converts each field with `<mapped type>::try_convert` and `Json` maps to `String`, so a `json_value`
  keyword promised a parsed document the constructor cannot accept — `try_convert` yields `None` on a Hash
  and the field silently falls back to its default rather than raising.
- **magnus**: declare `.rbs` attributes read-only. `attr_accessor` was emitted for every field of a defaulted
  struct, but the binding defines no writer for any field, so steep green-lit assignments that raise
  `NoMethodError`.
- **magnus**: declare each `.rbs` attribute as the accessor the extension actually emits. `Json` is mapped to
  `String` recursively, because the binding serializes it before Ruby sees it — `json_value` promised a parsed
  document that never arrives, so steep accepted `page.extracted_data["key"]` on a `String`. Nullability now
  follows the field's own optionality rather than the owning type's `has_default`, which had been nil-wrapping
  accessors that can never return nil.
- **e2e/rust**: share one field-aware containment predicate across `contains`, `contains_all`, `not_contains`, and
  `contains_any`, so enum and collection fields no longer emit assertions that fail to compile. Only `contains`
  handled those field kinds; the other three emitted a plain `.contains()` call that an enum does not have and
  that compares whole collection elements. `contains`'s own output is byte-for-byte unchanged.
- **snippets**: preserve extension-owned fixture descriptions while validating generated documentation language
  identities.
- **readme**: name the missing `crates.readme.snippets_dir` (or `snippets.<key>`) config again when a README
  template's `include_snippet` call references an undefined snippet mapping, instead of surfacing serde's generic
  "did not match any variant of untagged enum" message. Undefined values must be rejected before struct
  deserialization of the (String | {path, root}) snippet mapping, which fails silently on the underlying cause.
- **napi**: allow string-enum field literals alongside the nominal TypeScript enum using values derived from the
  canonical enum emitter.
- **kotlin-android**: treat a DTO as default-constructible only when every emitted constructor parameter has a
  Kotlin default, including transitive nested DTO defaults.
- **dart**: keep config parameters optional for serde function defaults by delegating to generated JSON
  construction, preserving the source default instead of synthesizing a zero value.
- **rustler**: JSON-encode default-valued records at public boundaries, report malformed payloads with context, and
  preserve async, error, and fluent-resource return shapes.
- **generate/verify**: stamp every emitted file carrying an Alef marker, including backends that template their own
  marker while intentionally leaving `generated_header` disabled.
- **ffi**: fail generation when generated FFI exports and the on-disk cbindgen header come from different runs.
- **ffi**: declare the callback-style streaming method wrapper's `client` parameter over the same scalar
  `AlefHandle` every producer of that client type returns, instead of a `TYPE *`/`const TYPE *` struct pointer.
  Every FFI producer hands out client types through `insert_handle`, never `Box::into_raw`, so the mismatched
  pointer parameter made the generated Rust and the generated C header describe two incompatible shapes for the
  same handle, and no caller holding a valid client handle could invoke a streaming method without a cast that
  was wrong by construction.
- **ffi**: declare enum `_free`/`_to_json`/`_to_string`/`_from_json` companions over the same scalar `AlefHandle`
  every producer returns for that enum, instead of a `TYPE *`/`const TYPE *` struct pointer. Every FFI producer
  (function returns, method returns, field accessors, and JSON constructors) hands out `Named` types — enums
  included — through `insert_handle`, never `Box::into_raw`, so the mismatched pointer signature made the
  generated C header describe two incompatible shapes for the same handle and broke consumer codegen (e.g. Go
  `cannot use ... as *_Ctype_struct_... value`) for any enum with data-carrying variants.
- **codegen/conversions**: use one tuple-variant predicate for enum definitions and `From` conversions so
  adjacently tagged tuple variants and untagged struct variants emit matching Rust syntax (#232).
- **ffi/service**: free the C-allocated response pointer before deserializing it in the generated service handler
  bridge, so a malformed response no longer leaks the buffer when parsing fails.
- **e2e/python**: stop emitting broken generated snippets/tests for `json_object` args configured with
  `options_via = "from_json"`. Construct via the type's plain kwargs constructor instead, stop importing that type
  from both the public module and the native bindings module in the same file, and bind the call result whenever
  the snippet template is going to print it instead of discarding the return value and printing an unbound name.
- **e2e/snippets**: give the Python, Go, Dart, and TypeScript snippet generators a crate-name-derived
  `"{PascalCase(crate name)}Error"` fallback for the error type they import/catch, instead of the bare literal
  `"Error"`, which almost never names a real generated type. Scoped to the four snippet emitters via a shared
  `snippet_error_type_name` helper — `ResolvedCrateConfig::error_type_name()` itself is unchanged and still
  defaults to `"Error"`, since 11 Rust-generating backends (extendr, rustler, wasm, php, magnus, ffi, swift, pyo3,
  jni, napi) consume it through `error_constructor_expr()` to generate Rust, and some consumer crates genuinely
  name their error type `Error`.
- **snippets**: enforce one timeout budget across every snippet in a validation batch and terminate timed-out
  toolchain process groups so descendant processes cannot keep docs generation alive.
- **cache**: serialize IR maps and sets in canonical order so unchanged inputs retain stable IR and backend cache hashes.
- **codegen/defaults**: stop emitting `#[serde(default = "path")]` functions as callable initializers in generated
  Rust (Magnus, PHP, NAPI, Rustler) — the named function belongs to the source crate, is not `pub`, and is frequently
  `#[cfg(feature = "serde")]`-gated, so the binding failed to compile with `E0425: cannot find function`. Generated
  Rust now recovers the field's real value by deserializing a minimal JSON stub through the source type's own
  `Deserialize` impl, the same mechanism `#[serde(default = "path")]` itself relies on. Where the owning type is not
  known, generation fails with the crate, type, field, and uncallable function named, rather than substituting the
  field type's zero value — which compiles and looks right while disagreeing with the source crate's configured
  `default_span()` is `1`; `u32::default()` is `0`).
- **validate versions**: discover nested C#, Dart, Zig, and Cargo lock manifests, validate all C# assembly version
  fields and every local lock package against its manifest, and normalize doubled path separators in diagnostics.
- **ffi**: preserve fully qualified streaming request types when emitting handle validation and lookup code.
- **generate/all**: stamp each successful generation stage before later work can fail, and defer standalone orphan
  cleanup until post-build succeeds while preserving non-header generator and scaffold outputs.
- **snippets**: isolate per-target validation-session preparation failures so healthy language sessions still run
  while strict validation reports the affected target as an error.
- **snippets/c/go**: model public FFI handles as scalar `AlefHandle` values and use zero as the invalid-handle sentinel.
- **pyo3/e2e-python**: unify the two independent `from_json` eligibility checks — pyo3's `#[pymethods]`
  injection/`.pyi` stub gate read crate-level serde availability only, while the e2e python snippet emitter's
  gate read per-type serde derives only — into one shared `pyo3_from_json_eligible` predicate
  (`src/codegen/conversions/helpers/eligibility.rs`) requiring per-type serde derives, crate-level serde
  availability, and core<->binding convertibility. The two gates could disagree for a crate whose types carry
  serde derives but whose Python binding crate lacks `serde`/`serde_json`, or vice versa; both call sites now
  delegate to the shared predicate instead of each re-deriving it.

- **fixtures/readme**: separate reader-facing fixture inputs, result presentation, and error intent from test data,
  and allow individual README snippet mappings to migrate between roots without breaking sibling mappings.
- **e2e/python**: emit unbound calls when a fixture only verifies that the call does not raise, avoiding unused-result
  Ruff failures during generation.
- **snippets/c**: derive list and map return ownership from the Rust IR and use the generated JSON-string ABI.
- **e2e/kotlin/swift**: make `not_empty` assertions distinguish nullable scalars from nullable containers.
- **e2e/rust/zig**: generate mutable JSON patch values and use a runtime Zig I/O provider for file inputs.
- **e2e/dart**: emit trait-stub wrapper factories when test backends come from call-level argument mappings.
- **e2e/typescript**: lower IR byte arrays and enum wire strings to their typed WASM values.
- **generate/snippets**: propagate mandatory post-build failures from `alef generate`, terminate descendant validator
  processes at the configured timeout, and report stable generated-file totals across content-identical passes.
- **snippets**: use built-in language recipes for declarative trait-bridge fixtures and resolve C registry operations
  from configured bridge identities through canonical ABI naming.
- **snippets/c**: generate compilable trait-bridge examples with IR-derived callbacks, initialized vtables, canonical
  registration calls, and owned userdata cleanup.
- **e2e/rust**: emit Clippy-clean mock-server route loading.
- **ffi**: return named values as generational handle tokens and resolve nested Rust type paths without duplicating
  the core module.
- **clean**: use the generated Kotlin Android Gradle wrapper and avoid invalid generic Dart and C# clean commands.
- **e2e**: keep ordinary generation from writing a fixture schema before validation succeeds.
- **snippets/docs**: emit reader-facing, fail-closed examples, recognize Astro content-collection references, and
  allow per-language README snippet roots.
- **sync-versions**: preserve README content unless regeneration is explicitly requested with `--regen`.
- **wasm**: delegate instance methods with borrowed named inputs to the Rust core and compile C environment shims
  only for `wasm32` targets.
- **ffi**: keep named field accessors on scalar handle tokens, fail closed for lifetime-borrowed types that cannot
  enter the process-global registry, preserve forward-compatible error taxonomy matching, and type empty handle
  acquisition lists explicitly.
- **ffi/csharp**: retain configured trait registration exports and matching P/Invoke declarations when visitor
  callbacks also bind the trait through an options field.
- **ruby**: exclude compiled native libraries, object archives, logs, and debug-symbol bundles from scaffolded source
  gems.
- **zig**: derive the scaffolded local FFI header include directory from the configured FFI output path.
- **snippets**: emit strict TypeScript DTO literals and optional accessors, prefix WASM imports, and deserialize
  Kotlin Android inputs using each argument's declared DTO type.
- **node**: export zero-argument adjacent-enum namespace constructors as callable functions instead of getters.
- **csharp**: emit formatter-stable imports, native calls, and sealed-union converters that pass `dotnet format`.
- **zig**: keep generated bindings silent, validate nullable native returns before dereference, and release owned native
  buffers even when Zig allocation fails.
- **swift**: emit JSON constructors for serializable types referenced by generated API signatures, including opaque
  configuration types used by setters.
- **node/python**: attach stable numeric taxonomy codes to generated native error conversions while preserving typed
  Python exception classes and generic fallbacks.
- **java**: emit native-library required symbols one per line so generated bindings satisfy Checkstyle line limits.
- **java/kotlin**: map stable native taxonomy codes to generated typed exceptions while retaining generic FFI error
  fallbacks.
- **go/zig**: map native failures to typed binding errors by stable numeric FFI taxonomy codes instead of parsing
  human-readable error messages.
- **Java/C#**: require either a shared-native-runtime contract or an explicit borrowed-static, ABI-compatible,
  no-destructor capsule contract before wrapping native pointers, and protect C# service calls and callback
  registrations with SafeHandle lifetime guards.
- **C#**: restore readable formatting and a valid `/// <summary>` doc comment block in the service templates whose
  SafeHandle lifetime guards were reflowed onto run-together lines, including a bare `</summary>` that made the
  generated source uncompilable.
- **ffi**: report stable, collision-checked per-variant error taxonomy codes while preserving reserved conversion and
  panic codes.
- **JNI/Kotlin Android**: transfer configured capsule values as their raw host pointers instead of boxed Rust
  wrappers, omit incompatible Alef destructors, and make generated opaque-handle closure synchronized and idempotent.
- **generate**: preserve unchanged generated files and modification times across clean regeneration, reconcile only
  manifest-owned orphans, keep handwritten scaffold files outside formatting and hashing, and retain managed hashes
  on generated JNI manifests.
- **verify**: evaluate fixture-snippet coverage from current fixtures and renderers, reject stale or malformed coverage
  ledgers and missing tracked snippets, and preflight `alef all` before generated-file writes.
- **IR**: attach serialization-compatible, deterministic error type, variant, and numeric code taxonomy metadata to
  every extracted error variant.
- **extract**: preserve serde function defaults and supported zero-argument default calls as explicit runtime
  providers instead of collapsing them to empty language defaults.
- **extract**: suppress inherent impl methods and generic-method diagnostics when their declaring type is excluded.

- Fixed indexed JSON-scalar metadata matching, simple-result enum containment, falsifiable collection containment,
  and JNI streaming exports for opaque owner handles without ordinary methods.

- Made Rust and Kotlin containment assertions respect the effective result type, including complex arrays and
  nullable JSON values, without applying whole-record debug matching from unrelated field metadata.
- Imported streaming request DTOs into generated Kotlin tests and verified Start/Next/Free JNI declarations and
  exports in both directions.

- **snippets**: fail E2E generation when an expected documentation snippet has no compatible built-in or
  extension recipe, including cached coverage manifests, instead of warning and crediting an incomplete corpus.
- **snippets**: use pyrefly as the sole built-in Python snippet type-checker, matching scaffolded project tooling and
  reporting it explicitly when unavailable.

- **cli**: report disk-scanned orphan candidates instead of deleting them. The reclaim rule infers "orphan" from
  "absent from this run's output", and three situations produce that absence — the emitter stopped emitting the
  file, the emitter failed to emit it, or it is a create-once seed alef emits only when absent. No clause separated
  them; the manifest clause certifies that the backend keeps books, not that this file was deliberately dropped. The
  marker likewise records when alef last wrote a file, not whether alef owns it, so the rule protected neglected
  trees and endangered current ones: on a tree where 159 of 160 files carry markers, the public entry point is a
  candidate. Only a positive assertion from the producer can separate "dropped" from "failed to emit", and no such
  record exists yet, so the disk-scan route now reports.
- **e2e**: do not prune orphaned snippets on incomplete coverage. The prune ran before the completeness gate, and
  that gate only defers its error, so ordering alone would not have helped. A snippet that merely failed to render
  is absent from `generated_paths` while its language is still expected — indistinguishable from a genuine orphan —
  so a transient generator failure unlinked published documentation. `orphaned_paths` stakes its whole safety
  argument on that gate having passed first.
- **cli**: never prune a `poly.toml` table a scoped run did not emit. The prune step read a path absent from this
  run's output as an empty array, so a scoped `--lang java` generate stripped the consumer's whole rule selection
  down to `select = []`, which every linter accepts and then checks nothing. Only a path this run did emit can
  testify that one of its values is gone.
- **cli**: record every language manifest `alef all` writes. The service-API, stub and public-API phases wrote files
  and stamped hashes without folding them into the language manifest, so a backend recorded a single path while 38
  generated modules sat unrecorded on disk — and absence from the manifest reads as "this backend emits one file".
  Both manifest writers now log the crate, language and path count, since a pathologically small manifest was
  previously indistinguishable from a backend that genuinely emits one.
- **docs**: emit every page before validation can fail the stage. Snippet discovery and validation were fused into
  one call that ran before the CLI, MCP, `llms.txt` and `SKILL.md` emitters, so any strict bail, gap failure or audit
  failure returned before those pages were pushed — and downstream, a page missing from that list is read as one
  alef no longer emits. The stage's documented promise that its file list survives a failure covered only the first
  pass.
- **docs**: stop idiom-translating rustdoc intra-doc link targets. `rust_links_to_plain` only degraded links whose
  text began with a backtick, so a plain-text intra-doc link fell through to `rust_paths_to_dot_notation`, whose
  blanket `::` → `.` substitution rewrote the link *target* into a relative Markdown link that resolves nowhere: 26
  MD057 errors across 13 reference pages on one consumer. Both passes were individually reasonable and wrong in
  composition. Links are now degraded only when the target is identifier-shaped and is not anchored, schemed,
  slash-bearing or `.md`/`.html`-suffixed, and tests pin both directions.
- **zig**: declare every error the emitted body can return. `wrapper_return_type` and `method_return_type` each
  carried their own list of which return shapes need `OutOfMemory`, and neither matched a bare opaque-handle return
  — while both body emitters unconditionally emit `if (_result == 0) return error.OutOfMemory` for exactly that
  shape, so four generated functions declared `error{HandleClosed}` and returned `OutOfMemory` and the binding did
  not compile. The shape is derived once for both callers, and an assertion after emission checks that the declared
  set admits every `error.X` the body returns.
- **csharp**: derive the null-check sentinel from the P/Invoke return type. A capsule return was declared
  `extern ulong` while the FFI crate exports it as a raw `*const T` — Dart and Zig both agree on the pointer — so
  the wrapper's `== IntPtr.Zero` check was correct and the signature was wrong. The declaration and its sentinel now
  come from one function that reads the declaration string itself. Affects only types listed in `capsule_types`,
  which is why the common path looked flawless on a tree that configures none.
- **dart**: derive a snippet's call shape from the binding's own predicate. The snippet emitter decided whether a
  config argument was named or positional from a hardcoded list of type-name substrings while the binding decided it
  from whether a default expression can be synthesized, so for six type families the binding declared named-optional
  and the snippet called positionally and did not compile.
- **e2e (kotlin)**: bind a streaming adapter's request before the snippet uses it. The docs-snippet emitter resolved
  the owner handle but never the declared request parameter, so the emitted snippet either called the receiver with
  no arguments or passed raw handle-config-contaminated JSON where the typed request belonged. `kotlin_android`
  delegates here and was affected too.
- **e2e**: honour `exclude_functions` in the snippet coverage ledger. The expected set was built without consulting
  per-language exclusions, so it expected fixtures the emitter is configured never to produce, recorded them as
  generated, reported `missing: []` while those paths were absent from disk, and the tracked-file check then killed
  the run on a discrepancy the ledger had declined to record. Absent tracked paths are now all reported with a count
  instead of bailing on the first, and are named as distinct from what `missing` explains.
- **snippets**: register alef's ownership of the coverage ledger at write time. The ledger is strict JSON so it can
  never carry a provenance marker, and nothing recorded ownership, so every rewrite was refused, the stale copy
  survived, and the tracked-file check then failed the run on contents alef had just been prevented from refreshing.
  `alef adopt` could not repair it either: with no marker it classified as a create-once seed, the category for files
  a human grows past a placeholder. The write guard and adopt now share one notion of ownership; hand-written files
  at unregistered paths are still refused.
- **snippets**: keep validator scratch inside the cache root and sweep it. An earlier pass moved scratch under
  `.alef/snippets/tmp` but left the sweeper looking in the working directory, so the safety net pointed at the empty
  set and files survived every run. Cleanup is now an RAII guard, because validators return from four kinds of place
  and the explicit calls covered some of them; its `Drop` retries, since a killed child can still be exiting and lose
  the race with `ENOTEMPTY` — which `TempDir` discards silently, making a leak indistinguishable from success.
- **scaffold**: exclude the e2e snippet output tree from poly discovery. `docs_snippets_excludes` read only
  `[workspace.docs.snippets] dirs`, so a consumer configuring `[crates.e2e.snippets] output` alone got no protective
  exclude for the tree alef writes generated snippet Markdown into. The hazard is latent — in every tree surveyed the
  two keys name the same directory — but the divergent-key case is real.
- **codegen**: name the owning type by its full path in a `compile_error`. The diagnostic spliced the crate from
  `rust_path` onto the type from `name`, dropping every intermediate module and rendering `demo::inner::Settings` as
  `demo::Settings`. A diagnostic that misnames the definition it points at sends the reader to the wrong place.
- **c**: pair a snippet's failure guard with the declaration it names. The guard was emitted by a positionally-blind
  pass that replaced any assert line, so a client-construction assertion above the call became a guard on a result
  variable above its declaration. A single walk now carries declaration state, so the guard cannot precede the
  declaration it reads and an out-of-order match becomes a generator diagnostic instead of published C. Output is
  byte-identical for every currently valid snippet.

### Removed

- **snippet validation**: remove the legacy path-only `alef snippets validate` command, the fail-whole-map session
  preparation API, and the report-dropping snippet artifact projection. Use configured `alef snippets check`,
  isolated session preparation, and `generate_snippet_report` so validation retains sessions, coverage, audits, and
  missing-generation diagnostics.

### Changed (BREAKING)

- **FFI handles**: ordinary opaque values now cross the C ABI as scalar, generational `AlefHandle` tokens with zero as
  the invalid sentinel. Regenerate every host binding and C consumer; pointer-shaped calls to constructors,
  accessors, serializers, streaming methods, and destructors are no longer compatible.
- **capsules**: Java, C#, JNI, and Kotlin Android capsule mappings must explicitly describe either shared-runtime
  ownership or borrowed-static ABI compatibility. Owned, refcounted, and WebAssembly-backed pointers remain
  fail-closed unless the configured host contract can preserve their lifecycle.

## [0.60.2] - 2026-08-12

### Fixed

- Kotlin E2E generation now constructs streaming adapters' declared request DTOs instead of passing primitive fixture
  arguments to typed owner methods, and cross-checks streaming native declarations against generated JNI exports.

- Generic documentation snippet generation now records calls with no effective function identity as missing coverage
  instead of emitting invalid empty calls and counting those files as generated.

- Generated Rust E2E assertions now use declared collection and enum field metadata for textual containment checks,
  avoiding invalid string arguments to `Vec<Named>::contains` and nonexistent enum `contains` methods.

- Restored structurally corrupted FFI, Go, Swift, Zig, and Node templates while preserving their allocator,
  ownership-transfer, concurrency, native-symbol, and runtime-export semantics.

- Generated FFI service modules now define their own panic guard, and generated Rust headers are placed before
  multiline inner attributes instead of being inserted inside their token trees.

- Snippet session cleanup now tolerates scratch directories removed concurrently and reports the exact path and
  operation for other filesystem failures, preventing opaque intermittent `ENOENT` failures in docs and snippet checks.

- E2E formatting now resolves generated language directories before changing the formatter working directory,
  preventing relative paths from becoming nonexistent doubled paths while rejecting formatter engine failures that
  older Poly versions only reported as warnings.

- Standard-library trait implementations on structs are no longer extracted as public binding methods, preventing
  methods such as `Debug::fmt` from producing lossy sanitized APIs and blocking generation.

- Generated Elixir test helpers now rely on `System.put_env/2` and no longer call the intentionally omitted
  `set_env` NIF.

- Generated Ruby bindings now use tuple constructor and match syntax for adjacently tagged positional enum variants.

- Kotlin assertions over optional JSON scalar fields now stringify safely instead of invoking string-only extensions
  on `Any?` values.

- Generated Rust documentation snippets now preserve every line inside multiline raw string literals instead of
  dropping or reindenting literal contents, and no longer retain the surrounding test module's closing brace.

- Generated Java bindings now honor explicit native-library path overrides before bundled resources and report every
  missing ABI symbol, the exported count, and the loaded path in one eager startup diagnostic.

- Generated agent skills now include required YAML `name` and `description` frontmatter when templates omit it.

- Alef now warns when the running CLI is newer than `alef.toml`'s pinned version, making the pin update visible
  before regeneration.

- Generated Java E2E assertions now retain statement separators when multiple assertions share a test method.

- Generated C# native declarations now retain data-enum handles while excluding unit enums and traits, preventing
  both missing live FFI declarations and calls to nonexistent JSON or destructor exports.

- Generated C E2E harnesses now propagate assertion failures into per-test results, return a failing process status,
  and report credential-gated tests as skipped instead of silently counting every invocation as passed.

- Kotlin JNI bridge declarations now exclude sanitized methods and only expose destructors for opaque handles returned
  by emitted functions, keeping every declared native method paired with a generated JNI export.

- Generated JNI shims now gate feature-dependent functions by their target-specific dependency feature sets, keeping
  disabled APIs out of Android builds while retaining enabled fallback implementations.

- Generated JNI manifests now inherit dependencies declared by the consumer workspace, keeping binding crates aligned
  with workspace dependency versions while retaining standalone fallbacks.

- Generated Python data-enum accessors now follow Serde's externally tagged wire shape, including bare-string unit
  variants and renamed payload keys, instead of assuming every enum contains a `tag` field.

- Generated Java opaque handles now make `close()` idempotent, clear native ownership before freeing, and reject method
  calls after close instead of allowing double-free crashes.

- Generated ownership headers now recommend `alef verify` directly instead of the deprecated no-op `--exit-code` flag.

- Go methods on value types now emit receiver-marshalling statements on separate lines, restoring valid generated Go
  syntax while preserving consumed-receiver ownership.

- Zig opaque-handle methods now convert C ABI integer booleans to Zig `bool`, preventing generated methods from
  returning an incompatible `i32` value.

- Generated Zig E2E tests no longer ignore `SIGABRT`, so allocator corruption and native aborts fail the suite instead
  of being silently suppressed.

- C FFI string-length companions now keep function-specific thread-local lengths, preventing intervening calls from
  turning a valid returned string into an out-of-bounds slice. Feature-gated opaque constructors now match their
  destructors and header declarations, while field accessors report conversion failures and document pointer ownership.

- JNI shims now contain panics across every Rust-owned JVM entrypoint and reject zero `jlong` handles before
  constructing Rust references, preventing unwinds and null-reference undefined behavior at the JNI ABI boundary.

- E2E regeneration now refreshes `fixtures/schema.json`, keeping consumer fixture schemas aligned with structured
  documentation metadata supported by the generator.

- Batched snippet validation now drains compiler output concurrently to prevent large-corpus pipe deadlocks, reports
  batch start and completion, keeps temporary workspaces under `.alef`, and removes timed-out legacy root-level
  scratch directories on subsequent validation runs.

- Snippet gap detection now discovers imported snippet content in Astro component files as well as MDX pages.

- `alef test` now fails when preconditions skip every explicitly requested language instead of reporting a vacuous
  success with zero executed suites.

- Snippet validation limits now cap the effective validation level instead of skipping the snippet when a stronger
  level is requested; bare fences remain unannotated and validate at the configured level.

## [0.60.1] - 2026-08-12

### Changed

- `alef verify` now fails on stale bindings or version drift by default; use `--report-only` for advisory output. The
  deprecated `--exit-code` flag remains accepted as a no-op for compatibility.

- Development version advanced to 0.60.1 for the strict snippet-validation and migration fixes.

### Fixed

- Serialize JVM-backed snippet validator integration tests so concurrent Java and Kotlin compiler startup cannot
  destabilize the JDK module image during the full test suite.

- Preserve line boundaries in generated JNI crate headers so Rust attributes, imports, and constants remain valid syntax.

- Keep FFI panic-guard and JNI unsafe-lint audits aligned with generated output.

- Emit every `not_contains` value in generated E2E assertions when fixtures use the plural `values` form.

- E2E fixtures can opt into preserving declared `mock_url` and `mock_url_list` values verbatim across generated
  language tests, so URL-policy and SSRF regressions exercise the address declared by the fixture rather than a
  substituted mock-server URL.

- Integration-test enum fixtures now initialize adjacent Serde content metadata, restoring full-suite compilation.

- E2E and registry test-app generation now fails when a configured formatter is unavailable or exits unsuccessfully,
  preventing noncanonical generated output from being reported as successful.

- Generated Ruby, PHP, R and Homebrew E2E error tests now assert the error value a fixture declares, matching it
  against either the error's message or its class name. PHPUnit's `expectException*` and testthat's `expect_error`
  combine message and class with AND, so both emit an explicit try/catch to express the disjunction.

- Generated Homebrew E2E error tests no longer interpolate the declared value inside a double-quoted `echo`, where a
  value containing `$` or a backtick would have been expanded by the shell.

- Generated Elixir E2E validation tests now call the operation under test when engine creation succeeds, instead of
  stopping after asserting `{:error, _}` on creation. Fixtures whose error is raised per-request rather than at
  construction previously asserted nothing and could never fail.

- Generated Elixir and Gleam E2E error tests now assert the error value a fixture declares, matching the reason's
  `inspect` rendering so a plain message and a typed atom or struct are both covered by one check.

- Generated Swift and Dart E2E error tests now assert the error value a fixture declares, matching it against either
  the error's description or its runtime type name. Swift's `catch` accepted any error and Dart used
  `throwsA(anything)`, so neither could distinguish the declared error from any other failure.

- Generated Java and C# E2E error tests now assert the error value a fixture declares, matching it against either the
  thrown exception's message or its type name. The existing expected exception type is unchanged; only the value check
  is added.

- Generated Kotlin and Kotlin Android E2E tests now dispatch streaming `owner_type` adapters as instance methods on the
  owner handle rather than as static facade calls, so the generated sources compile. The Kotlin backend had never
  ported the branch the Java backend already implements.

- Generated TypeScript and WebAssembly E2E error tests now assert the error value a fixture declares, matching it
  against either the error's message or its `name`. `.rejects.toThrow(regex)` only inspects the message, so the
  disjunction is expressed with a `toSatisfy` predicate; declared values are escaped for regex-literal context.

- Generated Go E2E error tests now assert the error value a fixture declares, matching it against either the error's
  message or its concrete type, instead of only checking that some error occurred.

- Generated C E2E error tests now fail when the call unexpectedly succeeds. Fixtures whose call used a
  `raw_c_result_type` emitted no error check at all, so the test passed regardless of outcome. Unmodeled result types
  fall back to asserting a non-zero `last_error_code()` rather than emitting nothing.

- Generated Zig E2E error tests now fail when the call unexpectedly succeeds. The previous shape wrapped the call in
  `catch { try testing.expect(true); return; }`, so a successful call skipped the catch entirely and the test passed
  having asserted nothing, while `expect(true)` was a tautology on the error path.

- Run the configured E2E formatter pipeline on cached test-app outputs so formatter and configuration updates converge
  without requiring a clean regeneration.

- Generated API examples now emit valid empty-map literals for Elixir and R, parameter fallback text is complete for
  unnamed values, and generated package READMEs carry generated-file headers.

- Generated reference pages now omit members gated by `cfg(test)` even when a binding enables an umbrella `full`
  feature.

- Generated Markdown tables now preserve square brackets inside code spans while escaping brackets that would form
  reference links, avoiding malformed API type cells.

- Generated parameter documentation now retains multiline rustdoc `# Arguments` descriptions and accepts all
  CommonMark unordered-list markers.

- C and Zig documentation snippet sessions now accept configured native include directories, and Swift validation
  tolerates SwiftPM binary directories that have not been created yet.

- Kotlin documentation snippets now implement configured visitor callbacks and attach them to conversion options.

- Rust documentation snippets now preserve raw-string delimiters and declare visitor feature requirements; docs
  validation enables the crate's configured Rust features for local snippet sessions.

- Go documentation snippets now emit visitor implementations and pass visitor-backed options to binding calls.

- Java documentation snippets now construct configured visitors and attach them through generated options builders.

- C# documentation snippets now instantiate configured visitor implementations and attach them to conversion options.

- C visitor documentation snippets now use only public binding APIs and omit test-harness JSON assertion helpers.

- Elixir documentation snippets now place visitor callbacks inside the conversion options argument.

- Dart documentation snippets now import and initialize the generated Rust library and dispose it on every exit path.

- Enum extraction now preserves Serde `content` metadata for adjacently tagged wire representations.

- Generated Go and Node bindings now preserve adjacent enum tag/content fields and reject unknown Go discriminants.

- Generated Node bindings now export adjacent enum unit values and payload factories through runtime namespaces.

- Generated Ruby and Elixir bridge enums now retain adjacent Serde tag/content shapes, including tuple payloads.

- Generated TypeScript visitor snippets now use lowercase wire actions, adjacent custom payloads, and trait-order code
  block arguments.

- Kotlin documentation snippets now import configured packages, shorten fully-qualified facade names, and only use
  coroutine entry points for asynchronous calls.

- PHP documentation snippets now enable strict types and load Composer dependencies before using generated classes.

- Ruby documentation snippets now require the load-path entry instead of the hyphenated gem distribution name.

- Zig documentation snippets now render directly as executable bodies, retain owned-result cleanup, and avoid test-only
  aliases and unmatched delimiters.

- Go documentation snippets now honor shared pointer-option configuration and print structured result fields.

- Generated-file manifests are deterministic and newline-terminated, empty manifests invalidate the cache, and
  `alef verify` now detects edits to individual generated file bodies.

- Rustler binding structs now raw-escape Rust keyword field names such as `type`, including constructors and tagged
  enum conversions.

- Generated Python and R visitor snippets now serialize callback actions with canonical lowercase wire tags and the
  adjacent `output` payload expected by visitor bridges.

- Generated WASM options-field visitor bridges now forward configured visitor handles instead of replacing them with
  `None` in input builders.

- Generated E2E assertions now preserve raw and whitespace-sensitive string semantics, enforce case-sensitive C#
  containment, validate content for `not_empty`, and fail null values instead of allowing vacuous matches.

- Snippet validation sessions now accept repository-relative native include paths, allowing Zig and C bindings that
  import generated C headers to compile without project-specific validator behavior.

- Swift snippet validation no longer reports an internal I/O error when SwiftPM's reported binary directory has not
  been materialized yet.

- Generated Elixir NIFs no longer expose a process-wide environment mutation helper, avoiding unsafe concurrent
  `setenv` access from BEAM scheduler threads.

- Generated Go wrappers no longer free marshalled value receivers after an owned-receiver FFI method consumes them.

- Generated FFI accessors no longer expose borrowed non-clone fields as owned handles that callers could invalidly free.

- Generated Swift trait protocols now document their concurrency contract, and their Rust `Send`/`Sync` assertions carry
  explicit safety invariants.

- Visitor callback payloads now use their reported byte length and an allocator-matched host destructor instead of
  reconstructing host allocations with Rust's global allocator; Go, C#, and Java callback tables provide the destructor.

- Generated cbindgen configuration now maps correctly escaped Rust feature predicates to prefixed C macros so
  feature-gated declarations remain guarded in public headers.

- Generated JNI service lifecycle, registration, and destructor entry points now catch Rust panics before they can
  unwind across the JVM ABI boundary.

- Generated Zig trait callbacks now copy returned strings into allocator-matched storage, free undispatched results,
  release callback strings through the matching allocator, and return owned error text through `out_error`.

- FFI and Java bindings now omit `free_bytes` when no generated API can produce its allocation metadata, preventing
  unrelated pointers from being mistaken for byte-result allocations.

- Infallible complex-return trait callbacks now consume and free host `out_error` diagnostics before returning a safe
  default on non-zero callback status.

- Options-field visitor callbacks no longer emit an unattached generic trait bridge whose public destructor could free
  shared host state independently of the live visitor handle.

- Every generated FFI entry point now clears stale thread-local error state before execution, while error and return
  metadata accessors preserve the state they report.

- C# visitor declarations now use configured FFI prefixes, and C#/Java options-field visitor bindings omit generic
  registry and byte-destructor symbols that their FFI library does not export.

- Attaching a visitor to an FFI options handle now transfers ownership into one synchronized object, preventing multiple
  independent mutexes from aliasing the same mutable visitor; Go wrappers honor the transfer.

- Zig options-field visitor helpers now call the actual generated visitor constructor with the correct callback and
  handle types instead of emitting a phantom trait-specific symbol and mismatched free contract.

- Snippet gap and audit commands now discover complete coverage ledgers beneath configured snippet roots, so generated
  output directories are trusted without hiding orphaned handwritten snippets.

- Generated FFI and JNI crate roots now contain Rust 2024 unsafe implementation lints, allowing consumers to inherit
  strict workspace lint policy without warnings from generated glue.

- Managed TOML scaffold manifests now carry Alef provenance, preserve unknown user tables during structured refresh,
  and participate in `alef diff`; write-once scaffold seeds remain untouched.

- Snippet coverage ledgers now reject tracked files that resolve outside their configured generated root, including
  symlink escapes.

- C documentation snippets now construct whole-input typed DTO handles from the public JSON API, preserve file-backed
  byte inputs, and free owned handles instead of omitting required arguments or passing placeholder nulls.

- Standalone C documentation snippets now construct every JSON argument with its declared ABI type and derive opaque
  return handles from extracted function metadata instead of call-name guesses or shared placeholder option types.

- Nested C result accessors now infer optional opaque handle types from extracted struct fields when no explicit
  `fields_c_types` override is needed, avoiding generation-time panics for authoritative IR shapes.

- Rust documentation snippet validation now checks compatible uncached cells as binaries in one Cargo invocation,
  while retaining per-cell diagnostics, cache entries, session isolation, and deterministic result ordering.

- Documentation snippets for expected-error fixtures now render idiomatic executable failure handling in C#, Dart,
  Elixir, Go, Java, Kotlin, Python, and Ruby instead of presenting failing calls as successful examples.

- Harness-only trait bridge fixtures now require extension-owned public documentation recipes instead of publishing internal test
  stubs, including when the test backend argument comes from global call configuration.

- Fixed Kotlin documentation snippets referencing undeclared typed input variables when fixtures do not define file presentation metadata;
  DTO JSON now uses centralized nested serde wire names and an idiomatic local mapper name.

- Snippet session regressions now use isolated .NET restore state and tolerate cold Windows toolchains without
  weakening validation.

- Snippet validation sessions now integrate configured package manifests and toolchain roots across Rust, TypeScript,
  Go, C#, Java, Kotlin, Dart, Python, Swift, and Zig, with isolated absolute caches and explicit Rust dependencies.

- Generated C snippets now use configured ABI prefixes, explicitly typed scalar and void return shapes, while Java snippets present
  returned values and selected fields; C Doxygen output also escapes nested comment delimiters.

- Elixir DTO typespecs now retain generated enum modules inside lists, and PyO3 single-variant enum constructors avoid
  warning-producing one-arm matches.

## [0.60.0] - 2026-08-10

### Fixed

- Fixture-generated snippets now request type-check validation instead of being downgraded to syntax-only skips.

- Fixture-generated documentation snippets now include canonical validation frontmatter without exposing test inputs or assertions.

- C FFI capsule returns now use const-null failure sentinels, keeping generated Rust pointer mutability consistent.

- C FFI streaming iterator `_next` functions now keep their full bodies inside the panic guard, producing valid Rust.

- Alef skip extraction now parses attribute structure instead of token substrings, supporting `#[alef::skip]` without
  misclassifying similarly named or feature-gated serde attributes, and positional enum/newtype fields now preserve
  field-level exclusion metadata.

- Alef scratch workspaces and caches are ignored recursively, preventing nested validation artifacts from appearing
  as consumer repository changes.

- Release verification now accepts Java documentation snippets that stage typed JSON before deserialization and keeps
  enum conversion helpers warning-clean under strict Clippy.

- WASM tagged-enum conversion now maps hidden or binding-excluded core variants to the binding default instead of
  trapping.

- C FFI byte-buffer ownership no longer relies on caller-controlled vector capacity metadata.

- JNI and Kotlin Android generation now selects cfg-gated function variants from the target feature set before
  deduplication, preventing disabled APIs from referencing absent core modules.

- Swift async return-type `Sendable` extensions now emit in canonical name order, keeping repeated isolated generation
  byte-for-byte deterministic.

- Kotlin/JNI opaque clients now serialize native calls with `close()`, make repeated and concurrent closes idempotent,
  and reject method or stream creation after close before entering JNI.

- Zig opaque and streaming handles now clear their nullable pointer before teardown, making repeated free/deinit safe and
  returning `HandleClosed` locally on use after teardown.

- Every generated Rust-owned C FFI entrypoint now contains Rust panics, including constructors, destructors,
  conversions, accessors, services, callbacks, traits, visitors, streams, bridges, and support helpers; contained
  panics use the existing thread-local error contract and return each signature's established failure sentinel.

- PHP flat data-enum tags are now read-only and JSON construction rejects unknown tags before infallible core
  conversion, preventing malformed or future variant tags from reaching generated panic arms.

- Rust documentation snippets now use public presentation inputs without leaking E2E mock-server environment or
  private-network setup, while generated E2E tests retain their runtime harness.

- Node and WASM snippet sessions now extend configured TypeScript project manifests, so declared local packages resolve
  during strict validation while stable validation workspaces still replace each snippet's source.

- Restore generated ownership headers when a managed backend file omits its inline header, keeping `alef:hash`
  verification and orphan cleanup active after regeneration.

- Domain-shaped E2E fixtures now require extension-owned documentation recipes instead of falling back to generic
  function-call generators that could emit invalid or test-harness snippets.

- Snippet gap and audit checks now discover current generated coverage ledgers directly from configured snippet roots,
  treating only ledger-backed generated files as references while continuing to report manual orphan snippets.

- Configured C#, Java, Node, and WASM snippet validation sessions now reuse stable project workspaces, preserving
  local package linkage and compiler state across snippets instead of creating an isolated project for every block.

- Generated Rust mock servers now emit valid fixture field types when loading documentation-rich fixtures.

- E2E fixture validation now accepts the complete structured documentation metadata model, including target paths,
  typed presentation arguments, file inputs, and result operations.

- Python visitor bridges now honor internally tagged return-action dictionaries such as
  `{"type": "custom", "output": "..."}` while retaining legacy externally tagged payloads.

- C documentation snippets now read configured file inputs into byte-array fields of typed DTO JSON.

- Zig documentation snippets now read configured file inputs into byte-array fields of typed DTO JSON.

- Swift documentation snippets now read configured file inputs into byte-array fields of typed DTOs.

- R documentation snippets now read configured file inputs into raw-vector fields of typed DTOs.

- Python, Rust, Node, and WASM documentation snippets now read configured file inputs into typed DTO byte fields.

- Ruby bindings now box converted named fields in data-enum variants when the Rust core field is boxed.

- Ruby and Elixir documentation snippets now read configured file inputs into typed DTO fields.

- Java and Kotlin documentation snippets now read configured file inputs into byte fields of typed DTOs.

- Go, C#, Dart, and PHP documentation snippets now read configured file inputs into native byte values inside typed DTOs.

- Go native DTO snippets now pass defaulted fields through pointers to match generated binding struct types.

- Go documentation snippets now inherit configured options DTO types when fixture recipes omit an inline type.

- Go documentation snippets now materialize absent typed DTO arguments as values and align native struct fields canonically.

- TypeScript documentation snippets now use imported enum members and safely destructure optional result collections.

- Fixture documentation presentations can replace inline test data with validated local-file inputs, and generated
  snippets no longer expose mock-server harness details.

- Dart and PHP documentation snippets now construct known DTO arguments with native typed constructors instead of
  JSON round trips.

- Go documentation snippets now construct known DTO arguments with native struct literals instead of JSON round trips.

- Rust and TypeScript documentation snippets now render display values, typed DTO inputs, and optional first-result
  collections using idiomatic, strict-mode-safe syntax.

- Go documentation snippet bodies now avoid a non-canonical blank line at the end of fenced source.

- Generated Go documentation snippets now match canonical `gofmt` layout, including imports and error blocks.

- C# documentation snippets now construct known DTO arguments with native object initializers instead of JSON round trips.

- Shared binding conversion regressions now keep test modules after production items for strict Clippy compatibility.

- Coverage-ledger side-effect metadata now uses its typed serialized representation without retaining an obsolete
  Markdown frontmatter renderer.

- Generated fixture snippets now keep validation metadata in the authoritative coverage ledger while rendering clean
  Astro-facing fenced Markdown with language titles and only explicitly configured user prose.

- Structured fixture presentations now preserve Rust `Result` unwrapping and import every TypeScript DTO referenced by
  docs-specific typed arguments, so result access and overridden inputs remain compilable.

- Generated Go documentation snippets now separate package and import declarations into gofmt-compatible lines.

- TypeScript E2E fixture regressions now retain formatter-clean protocol metadata initializers.

- PHP and Ruby now box converted values when binding DTO fields map to core `Option<Box<T>>` fields.

- Poly scaffolding now merges Alef defaults into an existing `poly.toml`, preserving custom tables, rules, excludes,
  and comments across clean regeneration while keeping repeated scaffold passes idempotent.

- Fixture documentation now supports typed input and argument overrides plus structured result presentation, allowing
  backend-owned snippets to render idiomatic field display and collection iteration without embedding language code.

- Fixture documentation can now select a safe relative snippet output path per configured target while retaining the
  shared topic/stem fallback.

- Snippet compile sessions now wire configured Rust crates, TypeScript packages, C headers, Swift packages, and Zig
  modules into isolated validator projects instead of using their manifests only for cache fingerprints.

- Snippet sessions now canonicalize configured package paths before changing subprocess directories; Rust scratch
  crates establish their own workspace boundary, and Swift compilation discovers package C-module maps.

- Binding-aware snippet sessions now resolve Go and Dart package manifests from their actual project roots, with
  regression coverage for local Go, C#, Dart, Java, and Kotlin dependencies.

- Strict documentation coverage now treats paths from a current, complete fixture-snippet ledger as authoritative
  references while rejecting missing files and stale ledger formats.

- Go mock-server integration fixtures now initialize the complete protocol fixture surface.

- Generated snippets now retain their binding target independently of the canonical fence language, allowing Node and
  WASM or Kotlin and Kotlin Android examples to resolve distinct validation sessions.

- PHP no longer reports complex map fields as non-settable when serde-backed `fromJson()` construction is available.

- Poly's successful "files reformatted" status no longer produces a non-fatal formatter warning during scaffolding.

- C documentation snippets now reuse engine-factory and byte-buffer call preparation, while C, Swift, and Zig
  streaming snippets reuse their binding-aware E2E call paths instead of leaving fixture-language coverage gaps.

- Snippet validation sessions now apply explicitly configured environment variables to setup and validation commands,
  allowing tool caches to work without inheriting ambient user environment state.

- Snippet side-effect policy now blocks execution only; syntax, compile, and type-check validation still run for
  network, process, install, and server examples.

- Snippet discovery now ignores Alef-owned metadata files such as `.alef-snippet-coverage.json`.

- R documentation snippets now render idiomatic package calls, and C, Swift, Zig, and WASM snippets support visitor
  fixtures through the same native callback and call-preparation paths as their E2E tests.

- Cached E2E generation now reloads and reports the persisted fixture-snippet coverage ledger, so missing language
  cells remain visible instead of becoming false-green on an unchanged rerun.

- Targeted `alef generate --lang ...` cleanup now stays within the selected language's owned output roots instead of
  deleting generated files belonging to other targets.

- Python documentation snippets now retain their binding imports, and Java snippets declare typed JSON arguments while
  preserving explicitly qualified service class names.

- Documentation snippets now omit unused Go, TypeScript/WASM, C#, and Swift imports, keep Go imports formatter-ordered,
  and release native C results before exit.

- Swift optional-vector fields now use the JSON bridge instead of emitting `Option<Vec<T>>`, which swift-bridge
  0.1.59 cannot parse.

- Fixture extensions can now render protocol documentation recipes from typed AsyncAPI operations and WebSocket
  sessions. Documentation generation no longer inherits E2E harness skip directives.

- Node/WASM and Kotlin/Kotlin Android snippets now keep distinct target output directories while sharing their canonical
  TypeScript or Kotlin frontmatter and fence validation, preventing cross-target fixture path collisions.

- Targeted E2E orphan cleanup now requires a current artifact inside a language subdirectory before sweeping it, so
  top-level scaffolding and documentation snippets cannot delete an otherwise ungenerated test suite.

- Qualified field paths now restore the surviving public type name after a same-name internal type is excluded, avoiding
  lossy `String` sanitization for optional public configuration fields.

- Ruby, PHP, Elixir, and Dart documentation snippets now render standalone HTTP requests for HTTP harness fixtures,
  using the same normalized request bodies, content types, headers, and cookies as E2E generation.

- Missing Dart prebuilt libraries are now a quiet no-op during ordinary development builds, while package creation
  retains an actionable warning that published consumers will need a local native build.

- Emit runnable Python and Rust documentation entry points and native expected-error handling for Node and PHP snippets.

- External DTO roots now preserve qualified type identity when names overlap with native API types, including field
  references and qualified field exclusions, without false unmatched-exclusion warnings or forced field removal.

- Generate standalone C and Zig documentation programs and render native expected-error handling for C, Swift, and Zig.

- Targeted E2E generation now derives orphan-sweep roots exclusively from current E2E artifacts, preventing
  snippet-only output from deleting valid language test suites.

- Fix generated documentation snippets for Go, Dart, Java, Kotlin, C#, and PHP to include standalone runtime wrappers,
  required imports, and non-test error handling.

- Generated snippet paths, fences, and frontmatter now share the validator's canonical documentation language identity,
  including TypeScript for Node and WASM and Kotlin for Kotlin Android, while coverage retains the configured target.

- Crate-level validation suppressions now downgrade configured lossy surface, unknown type, ambiguous JSON value, and
  backend stub path diagnostics during extraction and generation while unsupported public generics remain fatal.

- E2E fixture schema validation now validates each element of top-level JSON arrays independently, accepts numeric
  identifier prefixes and project-specific fixture payloads, and reports the failing fixture index.

- Brew and Homebrew fixture snippet targets now use shell documentation metadata and report unsupported recipes through
  exact per-language coverage exceptions instead of aborting snippet generation with a language-mapping error.

- Multipart fixture requests now share one request plan across the generic and Rust e2e clients. Schema-only
  uploads synthesize a real multipart body and boundary, while explicitly empty form data emits neither.

- Snippet audits now count README-configured snippet paths, including language redirects, and fixture side-effect
  metadata uses the canonical safe, network, process, install, and server taxonomy without collapsing mutations.

- Validate configured documentation snippets in binding-aware per-language sessions so compile and run checks resolve
  local generated packages and manifests, with sanitized one-time setup commands and explicit preparation failures.

- Every configured fixture-language pair, including extension-backed rendering, now participates in deterministic
  snippet coverage accounting. Missing documentation metadata, empty recipes, and incompatible renderers remain
  visible unless an exact user-facing documentation exception explains the difference.

- Lossy binding-to-core conversion now boxes named fields after converting their binding values.

- PHP optional struct setters now borrow wrapper values accepted by ext-php-rs and clone them into owned core fields.

- Java, Kotlin, Kotlin Android, and C# documentation snippets now reuse backend-native typed argument and
  setup generation while preserving client factories, coroutine or async calls, and imports without test harnesses.

- PHP, Ruby, Elixir, and Dart documentation snippets now reuse their backend-native argument,
  setup, client, visitor, and streaming call preparation without emitting test assertions or teardown.

- Node, WASM, and Go documentation snippets now reuse backend argument builders for typed setup,
  imports, client factories, async calls, and binding-native function names without test assertions or harness code.

- Python and Rust documentation snippets now reuse their backend test renderers, preserving typed options,
  enum values, optional arguments, client factories, JSON request objects, async calls, and mock-server setup.

- Generated Python snippets now preserve top-level indentation and import boundaries, retain
  synthetic optional mock-server URL arguments, and fail syntax validation instead of treating
  indentation/parser errors as missing dependencies.

- Render fixture-driven documentation snippets with setup calls, imports, recipe-aware options and enum constructors,
  omitted absent optionals, client factories, mock-server URLs, Python handle constructors, valid Python/Ruby/Rust JSON
  literals, Rust async calls, side-effect frontmatter, whole-input arguments, and Rust/C language aliases.

### Added

- **The CLI can compare handwritten snippets with fixture-generated equivalents without writing files.**
  `alef e2e snippets-migrate <existing-root>` reports identical, different, and unmatched files in stable text or JSON.

- **Fixture snippet generation can target an explicit subset of E2E languages.** Set
  `[crates.e2e.snippets].languages` to stage generated documentation alongside languages that
  remain handwritten; an empty list continues to inherit the E2E target list.

- **Swift, Zig, and C/FFI documentation snippets now reuse their typed e2e call rendering.** Generated examples
  preserve backend imports, argument setup, allocator and environment handling while omitting test assertions and
  teardown, and reject complex harness-only patterns with contextual errors.

- **E2E backends now own documentation snippet bodies.** Snippet orchestration resolves the registered
  language generator, passes the extracted type and enum registries through, wraps backend output in
  shared Markdown metadata, and reports unsupported or unknown languages explicitly.

- **Snippet checks now produce versioned, source-aware reports and enforce explicit validation policy.**
  Results distinguish requested from effective validation levels, report downgraded probes, classify
  side effects, sanitize validator environments, reuse a persistent content-hash cache for changed-only
  checks, parse MDX frontmatter, and fail on empty discovery or report write errors. (`src/snippets`,
  `src/cli/commands/snippets.rs`)

- **Strict snippet checks now fail when any discovered example is skipped.** This prevents a
  configured validation gate from succeeding with zero validated snippets. (`src/cli/commands/snippets.rs`)

- **Generated snippet imports, setup statements, and calls now retain required line breaks.**
  (`src/e2e/templates/snippets/call.jinja`)

- **E2E fixtures can now generate deterministic, tested documentation snippets.** Optional fixture
  documentation metadata, declarative capability requirements, safe collision-checked output under
  `[crates.e2e.snippets]`, and migration comparison APIs let projects replace handwritten examples
  incrementally while preserving existing e2e output when snippet generation is not configured.
  (`src/e2e/snippets`, `src/core/config/e2e`)

- **Snippet validation can be configured and enforced across a generated workspace.** Docs snippet
  configuration now distinguishes reusable snippet roots from handwritten pages containing inline
  fences, supports exclusions, strict coverage and side-effect policies, and configures cache and
  report paths. Newly scaffolded Poly configuration runs the strict aggregate snippet check when
  snippet inputs are present. (`src/core/config/output`, `src/docs`, `src/scaffold/languages/poly.rs`)

## [0.59.0] - 2026-08-09

### Added

- **The HTTP e2e fixture model now carries every middleware category a fixture can declare.**
  `HttpMiddleware` gained `lifecycle_hooks`, `openrpc`, `background_tasks`, `websocket`, and
  `authorization`, and the struct is now `deny_unknown_fields`. Previously an undeclared category
  was silently discarded at parse time, so no generator in any language could ever see it; an
  unmodelled category is now a hard parse error rather than an invisible omission.
  (`src/e2e/fixture.rs`)
- **Generated Rust HTTP e2e tests assert the response body and headers, not only the status code.**
  Header checks skip values the transport or a response-encoding layer computes for itself.
  (`src/e2e/codegen/rust/http.rs`)

### Fixed

- **Generated Rust HTTP e2e tests sent string request bodies wrapped in quotes.** A string body was
  emitted through `serde_json::to_string`, so the payload reached the server with a leading and
  trailing `"`. Form-urlencoded bodies gained two characters that shifted every field index,
  multipart bodies received the two-character sequence `\\r\\n` instead of CRLF, and deliberately
  malformed JSON payloads arrived as valid JSON strings. String bodies are now emitted verbatim;
  structured bodies are unchanged. (`src/e2e/codegen/rust/http.rs`)

## [0.58.3] - 2026-08-09

### Fixed

- **Linux CLI release archives now build on available GitHub-hosted x86_64 and arm64 runners.**
  (`.github/workflows/publish.yaml`)
- **The publish workflow now uses GitHub-hosted runners for release orchestration jobs.** This
  removes the unavailable `runner-medium` dependency from preparation, validation, release checks,
  asset upload, package publishing, and finalization so published releases can produce their
  downloadable CLI archives. (`.github/workflows/publish.yaml`)
- **TypeScript e2e nested-type discovery is deterministic when distinct Rust types share a short
  name.** Candidate resolution now uses the full Rust path as a stable tie-breaker, preventing
  generated WASM tests from changing with input order. (`src/e2e/codegen/typescript/test_file`)
- **PHP bindings now generate working setters for optional named-struct fields.** Setter signatures,
  native conversions, and generated type stubs consistently accept nullable wrapped structs instead
  of dropping or mistyping the assignment path. (`src/backends/php/gen_bindings`)
- **PyO3 trait bridges now preserve mutable callback updates and deserialize unit-enum returns correctly.**
  Async callbacks with `&mut` named parameters write an optional host-returned replacement back to
  Rust, protocol stubs expose that contract, and unit-only enums accept natural bare variant names
  without weakening struct-return validation. (`src/backends/pyo3`)
- **Snippet audits and coverage reports now recognize Astro MDX imports and `.mdx` documentation files.**
  MDX `Content` imports resolve relative to the importing page, and both audit and gap detection
  include `.mdx` alongside Markdown when checking references and fenced languages.
  (`src/snippets/audit.rs`, `src/snippets/gaps.rs`)
- **Generated Elixir e2e suites could leave the harness running as an orphan, hanging `mix test`
  after every test had already passed.** `test_helper.exs` spawned the harness with
  `Port.open({:spawn_executable, ...})` and never reaped it. Closing an Erlang port only closes the
  child's stdin; the harness runs `elixir -noshell` and never reads stdin, so it survives the port
  close, gets reparented to init, and keeps the stdout pipe it inherited from the test runner open —
  leaving the runner blocked on EOF indefinitely. The template now captures the harness's OS pid via
  `:erlang.port_info(port, :os_pid)` and reaps it in `ExUnit.after_suite`, `kill`-ing it and falling
  back to `kill -9` after a grace period. Only a harness this process spawned is touched — the
  existing `SUT_URL` guard still leaves an externally supplied harness untouched.
  (`src/e2e/templates/elixir/test_helper_server.exs.jinja`)

## [0.58.2] - 2026-08-09

### Fixed

- **Generated Swift e2e suites could leave the harness running as an orphan, hanging `swift test` and
  corrupting later runs.** `setUp` piped the harness's `standardOutput` without ever draining it — an
  undrained pipe blocks the child once the kernel buffer fills — and never assigned `standardError`
  at all, so the child inherited the test runner's stderr descriptor and could keep it open
  indefinitely. There was no `tearDown` anywhere, so nothing reaped the process. Both streams now go
  to `FileHandle.nullDevice`, and a new `tearDown` terminates and waits on the harness it spawned.
  Because `swift test` runs every class in one process and `setUp` only spawns when `SUT_URL` is
  unset, the spawning class also clears `SUT_URL` on teardown so the next class spawns its own
  harness rather than addressing the one just killed; an externally supplied `SUT_URL` is left
  untouched. An orphan surviving on the fixed port also silently redirected *other* languages' e2e
  suites at the wrong server, since every generated suite probes the port without verifying
  ownership. (`src/e2e/codegen/swift/test_file.rs`)

### Changed

- **The Rustler backend selected its handler-wrapper template by matching the registration method
  name against the literal `"route"`**, a product-specific string in generator core. It now
  dispatches structurally on the existing `HandlerShape` IR enum, emitting the context-object wrapper
  only for `HandlerShape::ContextObject`. That field existed but was never populated from
  configuration; `[[crates.services.registrations]]` entries now accept `handler_shape`
  (`"bare_callable"` — the default — `"context_object"`, `"request_response"` or
  `"introspect_params"`), resolved in the service extractor. Consumers relying on the old behaviour
  must set `handler_shape = "context_object"` on the affected registration. The wrapper template was
  previously unreachable in tests, because the fixture's registration was named `add_handler` and so
  never satisfied the name gate; it is now covered by a positive and a negative case.
  (`src/backends/rustler/gen_bindings/service_api`, `src/core/config/service.rs`,
  `src/extract/extractor/service.rs`)

## [0.58.1] - 2026-08-08

### Fixed

- **The Rustler backend generated an Elixir binding in which every request reaching a user handler
  hung forever.** Three defects compounded. (1) Chainable opaque wrapper methods returned the bare
  NIF `reference()` instead of re-wrapping in `%__MODULE__{ref: ...}`, because `returns_self` was
  computed as `is_static && returns_self` — excluding every receiver-based (`&mut self -> Self`)
  builder method, which is the only kind a builder chain uses. A builder therefore degraded from
  struct to bare reference on the first chained call. (2) An opaque metadata parameter was emitted
  bare into the registration tuple, passing the Elixir wrapper struct where the NIF decodes
  `rustler::ResourceArc<T>`. (3) The handler `GenServer` received on `handle_cast/2`, but the Rust
  bridge dispatches with a raw `send/2`, so `{:trait_call, ...}` fell through to the default
  `handle_info/2` and was silently discarded. Raw-send + `handle_info` is already the contract used
  by the scaffold, the e2e stubs and the Gleam trait bridge, so the two service-API templates were
  the outliers. (`src/backends/rustler`)
- **The Elixir e2e HTTP client JSON-encoded raw request bodies.** `render_call` unconditionally used
  Req's `json:` option, so a pre-encoded form or multipart payload was sent as a quoted JSON string
  and rejected by the server; multipart calls additionally never emitted a `Content-Type` header at
  all, since `ctx.content_type` was consulted only for the body decision. It now sends raw bodies via
  `body:` and falls back to `ctx.content_type` for the header. The content-type resolution added for
  Java in 0.58.0 is now a shared helper (`effective_content_type` / `is_raw_text_content_type` in
  `src/e2e/codegen/client`) rather than a third inline copy. (`src/e2e/codegen/elixir/http.rs`)
- **The Elixir e2e client silently dropped either request headers or cookies** when a fixture had
  both, by emitting two separate `headers:` options in one keyword list. They are now merged into a
  single option. (`src/e2e/codegen/elixir/http.rs`)
- **Generated Dart and WASM test files reordered their imports between otherwise identical runs.**
  Trait-import collection used a `HashSet`, and the transitive WASM nested-type walk returned a
  field-name-keyed `HashMap` in which two classes sharing a field name collided — which one survived
  depended on iteration order. Both now use `BTreeSet`, and the WASM walk returns a set of class
  names, which is all its only consumer needs. Generated `e2e/` output is byte-compared by CI, so
  this ordering must be deterministic. (`src/e2e/codegen/dart`, `src/e2e/codegen/typescript`)

### Added

- **`skip.languages` ids in fixtures are validated against the configured e2e target list.** An id
  that matched no real target silently disabled nothing, so the fixture kept running everywhere the
  author believed it was skipped. (`src/e2e/fixture.rs`)

## [0.58.0] - 2026-08-08

### Added

- **Kotlin value types bridge their instance methods through JNI shims**, so methods declared on a
  value type are now callable from Kotlin rather than being dropped at the binding boundary.
  (`src/backends/kotlin`)
- **Dart emits the FRB cfg-gate carry helper into `build.rs`**, carrying `cfg` gates through the
  flutter_rust_bridge codegen so gated items compile consistently. (`src/backends/dart`)

### Fixed

- **The Java e2e HTTP client now percent-encodes reserved characters in an embedded query and
  honours a form Content-Type declared only in a request header.** `java.net.URI.create` is
  RFC-2396-strict, so a fixture whose `request.path` embedded a raw query such as
  `?tags=a|b|c` threw `IllegalArgumentException` (lenient clients like Python and Node accept it).
  Separately, a fixture that declared `application/x-www-form-urlencoded` only in `request.headers`
  (leaving the request `content_type` field unset) had its string body JSON-encoded — the quoted
  body was then rejected by the server. The renderer now sanitizes the query segment and consults
  the header when deciding whether to send a raw body. (`src/e2e/codegen/java/http.rs`)
- **Zig e2e assertions stay on the raw JSON navigation path** instead of diverging onto a typed path
  that did not match the generated harness. (`src/e2e/codegen/zig`)
- **WebAssembly generation emits compilable conversions for delegating and payload-enum types.**
  (`src/backends/wasm`)

### Changed

- Updated the `jsonschema` crate to 0.49.7.

## [0.57.1] - 2026-08-07

### Fixed

- **The Dart module file no longer emits an unused `import 'traits.dart';`.** 0.56.0 added the
  import unconditionally so that a doc comment naming a trait (`[OcrBackend]`) would not trip
  `comment_references`. But the module file usually names no trait at all, so `dart analyze` then
  reported the import as `unused_import` — a hard lint failure in every consuming repo that
  regenerated on 0.56.0 or 0.57.0. The import is now emitted only when the generated body actually
  refers to one of the configured bridge trait names, which keeps both lints satisfied.
  (`src/backends/dart/gen_bindings/mod.rs`)

## [0.57.0] - 2026-08-07

### Changed

- **MSRV raised to 1.88.** The declared 1.85 floor was never real: `zip` 8.6 requires 1.88 and
  `criterion` 0.8.2 requires 1.86. Because `cargo upgrade` is MSRV-aware, the false floor made it
  propose *downgrades* (`libloading` 0.9→0.8, `zip` 8→7, `criterion` 0.8→0.7) instead of upgrades,
  so dependency maintenance had to route around it with `--ignore-rust-version`. Raising the floor
  also unlocks clippy's let-chain `collapsible_if` suggestions, applied across 868 sites in 250
  files. Consumers building alef from source now need Rust 1.88 or newer. (`Cargo.toml`)

### Fixed

- **Generated Elixir e2e/test_apps projects are now formatted by `mix format`.** `.ex`/`.exs` are
  excluded from poly's pass so `mix format` can own them, but `mix format` only ever ran in
  `packages/elixir` — the generated e2e and test_apps suites were therefore formatted by nothing at
  all and shipped exactly as the emitter wrote them, with calls left unwrapped well past the line
  limit. A `.formatter.exs` is now emitted next to the generated `mix.exs` (a bare `mix format` has
  no `inputs:` without one, so it refuses to run), and `mix format` runs over the directory as an
  Elixir residual alongside the existing `go mod tidy` one. `line_length` matches the binding
  package's `.formatter.exs` so every generated Elixir tree wraps identically; `import_deps` is
  deliberately omitted so formatting never depends on a fetched `deps/`.
  (`src/e2e/codegen/elixir.rs`, `src/e2e/format.rs`)

- Two redundant derefs in the PHP type-stub backend that were failing `poly lint` on main.
  (`src/backends/php/gen_bindings/type_stubs.rs`)

## [0.56.0] - 2026-08-07

### Changed

- **BREAKING: `FieldDef` gains a `version` field.** `alef(since = "...")` written on a struct
  field was parsed and immediately discarded — every other IR item (structs, methods, params)
  already carried a `VersionAnnotation`, but the field-level annotation had nowhere to land.
  `FieldDef` now has `pub version: VersionAnnotation` alongside the rest, so field-level `since`/
  `deprecated` metadata survives extraction and reaches backends. Any code that builds a `FieldDef`
  with an exhaustive struct literal — the pattern used ~300 times in this crate's own extractor and
  IR-construction tests, and likely used by any downstream code that constructs the IR directly
  rather than only reading it — now fails to compile with a missing-field error. Add
  `..Default::default()` to the literal (the field carries `#[serde(default)]`, so deserializing an
  older IR document is unaffected) or set `version` explicitly if you need to preserve field-level
  annotations. (`src/core/ir/items.rs`)

### Fixed

- **A boxed field on a struct-variant (named-field) enum arm now converts correctly in both
  directions.** wasm tagged-enum codegen already threaded `field.is_boxed` through the tuple-variant
  branch, but the named-field branch ignored it, so a `Box<T>` payload on a struct variant generated
  a conversion with no `Box::new`/deref — code that did not compile. Both branches now share
  `box_wrap_map_into`/`box_unwrap_map_into`/`box_unwrap_into` helpers, so tuple and struct variants
  wrap and unwrap boxed fields identically in both directions.
  (`src/backends/wasm/gen_bindings/enums.rs`)

- **A wasm type that drops a field during extraction no longer emits a delegating `Default` impl it
  cannot satisfy.** The delegating impl is `<core::T as Default>::default().into()`, which requires
  a `From<core::T>` able to carry every core field into the binding type; a field omitted from the
  binding (e.g. an unknown/sanitized type) makes that conversion impossible to generate correctly.
  Such types now fall back to `#[derive(Default)]` on the fields the binding actually has, matching
  the same core-to-binding convertibility check already used for the `From` impl itself.
  (`src/backends/wasm/gen_bindings/mod.rs`)

- **Generated Dart code passes `dart analyze` with zero warnings.** Three independent issues: `lib.dart`
  exported `traits.dart` but never imported it, so every `///` doc reference to a plugin trait was an
  unresolvable `comment_references` — `export` puts a name in downstream scope, not the exporting
  file's own scope, and doc-comment resolution only looks at the latter; `render_type` unconditionally
  added `import 'dart:typed_data'` even when the FRB typed-list import already superseded it, tripping
  `unused_import`; and the scaffolded `bin/download_libs.dart` reached into `lib/` via a relative
  `../lib/...` path instead of the package import. `lib.dart` now also imports `traits.dart`, the
  redundant `dart:typed_data` import is dropped once the FRB import is present, and
  `download_libs.dart` uses a `package:` import.
  (`src/backends/dart/gen_bindings/mod.rs`)

- **flutter_rust_bridge no longer emits calls to functions that were compiled out.** FRB is not
  feature-aware: it generates bindings straight from `lib.rs`, so a function behind a `#[cfg(...)]`
  gate that a reduced feature set (e.g. Android's trimmed OCR backend list) compiles out still gets a
  generated call site, which fails to build. A new post-build step,
  `PostBuildStep::CarryFrbCfgGates`, reads the `#[cfg(...)]` gates directly off `lib.rs` and rewrites
  the frb-generated glue to carry the same gates, via `carry_lib_rs_cfg_gates_into_frb_generated`.
  (`src/backends/dart/frb_rewrite/cfg_gates.rs`, `src/backends/dart/frb_rewrite.rs`,
  `src/cli/pipeline/commands/build.rs`)

- **The generated PHPStan stub declares a getter for every binding field, matching the extension it
  describes.** The real ext-php-rs extension emits a getter for every field unconditionally (a
  `for field in binding_fields(&typ.fields)` loop in `structs.rs`), including fields with no
  constructor-param support. The stub used to skip some of those, so PHPStan reported a false
  "undefined method" on a getter call that works fine at runtime. The stub's getter loop now mirrors
  the extension's exactly, including the `?string` return type Json/untagged-enum getters always
  serialize to regardless of the field's own optionality.
  (`src/backends/php/gen_bindings/type_stubs.rs`)

- **Generated Python stubs annotate `Json` fields as `str`, not `dict[str, Any]`.** `Pyo3Mapper::json()`
  maps `TypeRef::Json` to Rust `String`, so the field is always a JSON-encoded string at the pyo3
  boundary — the stub previously advertised `dict[str, Any]`, a type the runtime value never actually
  has. Stubs now declare `str`, and the now-unneeded `from typing import Any` import tied to that
  annotation is dropped so it doesn't trip ruff's `F401` (the `from_native` converters are still the
  only remaining source of `Any`). (`src/backends/pyo3/gen_bindings/types.rs`)

## [0.55.8] - 2026-08-07

### Fixed

- **`serde` attributes hidden behind `cfg_attr` are honoured again, so enum wire names under a
  conditional `rename_all` are correct.** `extract_serde_rename_all` unwrapped `cfg_attr` with
  `Attribute::parse_nested_meta`, which silently gave up when the condition was anything more
  complex than a bare ident or `feature = "x"` — so a `#[cfg_attr(any(feature = "serde", feature =
  "metadata"), serde(rename_all = "snake_case"))]` enum was extracted as having no `rename_all` at
  all. That was harmless until 0.55.7 changed the Java backend's no-`rename_all` fallback from
  lowercasing the variant to emitting it verbatim: the two agree for single-word variants
  (`Auto` → `auto`), so the missing attribute only became visible once the fallback changed, at
  which point the generated Java sent `Auto` to a core that deserialises `auto` and every call
  failed with ``unknown variant `Auto` ``. The condition is now parsed structurally as a
  `syn::Meta` (handling `any`/`all`/`not` and nesting), nested `cfg_attr` is unwrapped
  recursively, and the bare and `cfg_attr` paths share one walk. The predicate itself is still
  never evaluated — alef cannot know which features a downstream build enables, so every inner
  attribute is treated as if it applied unconditionally.
  (`src/extract/extractor/helpers/attributes.rs`)

## [0.55.7] - 2026-08-07

### Added

- **The Swift bridge crate's injected FFI dependency accepts per-target overrides.** A new
  `[crates.swift] ffi_target_dep_overrides` list — `cfg`/`features`/`default_features`, the same
  shape as `target_dep_overrides` — moves the secondary `*-ffi` dep out of the flat `[dependencies]`
  table into one `[target.'cfg(...)'.dependencies]` block per predicate, with the default gated on
  `cfg(not(any(...)))`. Until now `ffi_features` could only apply to a single ungated dep line, and
  because Cargo unifies features across every edge to a package, an unconditional `full-no-heic`
  pulled `sceptre-ocr-ort` onto iOS even where the core dep asked only for `android-target`,
  tripping the mobile `compile_error!` guards; xberg carried a hand-written post-regen patch that
  every local `alef all` reverted (#370). The FFI and core target entries are merged into one
  globally sorted list, since cargo-sort orders all target tables per manifest, not per dependency.
  Empty by default, so a config that sets only `ffi_features` is byte-identical.
  (`src/core/config/languages/swift.rs`, `src/backends/swift/gen_rust_crate/cargo.rs`,
  `src/backends/swift/gen_rust_crate/mod.rs`)

### Fixed

- **Go e2e harness import no longer collides with a reserved keyword.** The harness derived its
  import alias from the last segment of the module path with no sanitization, so a module ending in
  `/go` (e.g. `.../packages/go`) emitted `import go "..."` — and `go` is a reserved word, so the file
  failed to compile with `missing import path`. Aliases are now routed through a new `go_ident`
  helper that escapes reserved keywords and invalid identifiers (`go` → `go_`).
  (`src/core/keywords.rs`, `src/e2e/codegen/go.rs`)

- **`alef update`/`upgrade` no longer corrupt a pnpm project's `package.json`.** The default Node
  recipes ran bare `pnpm up -r` (and `pnpm up --latest -r -w`). With pnpm's default
  `auto-install-peers`/`dedupe-peer-dependents`, `pnpm up` promotes the optional peer deps of
  installed packages (e.g. napi-rs's `@emnapi/core`, `@emnapi/runtime`, `@octokit/core`, `typanion`)
  into the project's own `dependencies` and stamps them with the *workspace* version — so every
  update rewrote `package.json` with bogus, version-mismatched dependencies. Both recipes now pass
  `--config.auto-install-peers=false --config.dedupe-peer-dependents=false`, so only the real,
  declared dependency ranges are bumped. (`src/core/config/update_defaults.rs`)

- **PHP streaming methods are emitted in adapter-declaration order.** The PHP backend collected the
  streaming method keys into an `AHashSet` and then *iterated* it to emit the `#[php_impl]` methods —
  the only place in the backend where a hash container drove output order. ahash seeds itself per
  process, so regenerating an unchanged tree could swap two streaming methods in the generated Rust
  binding, producing a spurious diff and an intermittently red `alef verify` freshness gate. The keys
  are now an order-preserving, deduplicated `Vec` built from `config.adapters`, matching the
  config-declared order every other PHP emitter already uses.
  (`src/backends/php/gen_bindings/rust_bindings.rs`,
  `src/backends/php/gen_bindings/types/structs.rs`)

- **The scaffolded PHP `composer.json` declares a PHPUnit constraint that is installable on the PHP
  version it claims to support.** The generated manifests paired `"php": ">=8.2"` with
  `"phpunit/phpunit": "^13.1"`, but PHPUnit 13 requires PHP >= 8.4.1 — so `composer install` could
  not resolve on 8.2 or 8.3, and Dependabot, which resolves Composer against the declared platform
  floor rather than the runtime PHP, failed on every run in the consumer repos. The constraint is now
  `^11.5 || ^12.0 || ^13.1`, letting Composer pick the newest major the actual PHP supports.
  (`src/core/template_versions.rs`)

- **Java enum wire names now match serde's actual no-`rename_all` fallback.** The Java backend's
  tagged-discriminator and simple-enum generators lowercased the variant name (`listitem`) when an
  enum had no `#[serde(rename_all)]`, but serde with no rename attributes emits the PascalCase
  variant name verbatim (`ListItem`). Generated `json_name` values — and the matching
  `excluded_variants` handling — now fall through to the same verbatim behavior as every other
  backend. (`src/backends/java/gen_bindings/types/enums.rs`)

- **NAPI tagged-enum discriminator wire names now match the declared `#[serde(tag = ...)]`
  contract.** (#218, @thisislvca)

- **NAPI tagged-enum sanitized fields no longer drop data or emit non-compiling conversions.**
  #218 (@thisislvca) fixed the tagged-enum discriminator wire names but its sanitized-field
  handling had follow-on gaps: an unreachable `optional` branch inside the `sanitized` arm meant
  `field_conversion_from_core` was always called with `optional: false`; checking `f.optional`
  before `f.sanitized` meant an `Option<Vec<(String, String)>>` field never reached the sanitized
  path in either direction; gating on any `Vec<_>` shape (rather than the specific
  `Vec<Vec<String>>` shape actually handled) could emit a `format!("{:?}", …)` assigned to a
  `Vec<_>`-typed field, which does not compile; and the core→binding direction re-parsed a
  rendered `"name: expr"` string with `strip_prefix`/`replace` instead of composing an expression
  directly. Sanitized `Vec<Vec<String>>` (optional and non-optional) and `Map<String, String>`
  fields now convert correctly in both directions; every other sanitized shape keeps the
  pre-#218 `Default::default()` / `None` fallback, which always compiles.
  (`src/backends/napi/gen_bindings/methods.rs`,
  `src/codegen/conversions/helpers/field_fragments.rs`)

- **Generated `[target.'cfg(...)'.dependencies]` tables are ordered the way `cargo-sort` expects.**
  cargo-sort enforces table order, not just entries within a table: target-cfg blocks sort
  alphabetically by the raw cfg predicate, byte-wise. Every generator emitted the default
  `cfg(not(any(...)))` branch first, which is only coincidentally correct — `not(` sorts after
  `all(` but before `target_os`, so an `all(...)` override (xberg's macOS-Intel target) produced an
  unsorted manifest that `cargo sort --check`, and hence `poly lint`, rejects. A new
  `join_sorted_target_dep_blocks` sorts the default branch together with every override, and the
  FFI, JNI, Dart and shared `render_core_dep_with_overrides` (python/node/ruby/php/elixir) emitters
  all route through it. Separately, the wasm template emitted `[dev-dependencies]` ahead of its
  trailing `getrandom` target block; it now comes after. (`src/scaffold/mod.rs`,
  `src/scaffold/languages/ffi.rs`, `src/scaffold/languages/jni.rs`,
  `src/backends/dart/gen_rust_crate/cargo.rs`, `src/backends/wasm/gen_bindings/cargo.rs`)

- **The Swift bridge crate's `Cargo.toml` emits `[build-dependencies]` before `[lints.rust]`.** The
  manifest format string placed the lints table between the target-cfg blocks and
  `[build-dependencies]`, which is not the section order `cargo-sort` accepts, so the generated
  manifest failed `cargo sort --check`. (`src/backends/swift/gen_rust_crate/cargo.rs`)

- **`sync-versions` leaves unpublished manifests at their own version.** The release version was
  stamped onto every manifest the pipeline globbed, including `publish = false` workspace members —
  compatibility shims that exist only to keep a path dependency resolvable — and npm `package.json`
  files marked `"private": true`. Neither is ever published, so the churn was pure noise in every
  release diff. `publish` is now parsed properly by `manifest_is_publishable`: absent, `true` and
  `["some-registry"]` all stay publishable and only the literal `false` is skipped. Both that check
  and the new `package_json_is_private` fail open — a missing or unparsable manifest counts as
  publishable — so an odd manifest shape cannot silently freeze a real crate's version.
  (`src/publish/workspace.rs`, `src/cli/pipeline/version_workspace.rs`,
  `src/cli/pipeline/version_core.rs`, `src/cli/pipeline/version.rs`)

- **The PHP, NAPI and wasm emitters use field-init shorthand instead of a redundant `x: x`.** All
  three built struct literals with an unconditional `format!("{}: {}", name, expr)`, so any field
  whose expression is just its own name came out as a `clippy::redundant_field_names` violation —
  which is why xberg's generated crates carry a file-level allow for it: 423 sites in php, 318 in
  wasm, 18 in node. Each emitter now compares the field name against the expression and emits the
  bare name when they are equal, porting the guard the PyO3 backend already had (and why its count
  is zero). A field whose type genuinely needs a cast or wrap keeps its full `field: expr` form.
  (`src/backends/php/gen_bindings/types/structs.rs`, `src/backends/napi/gen_bindings/methods.rs`,
  `src/backends/wasm/gen_bindings/types.rs`)

- **Nested `Json` maps to `JsonElement` at any depth in the generated C# DTOs.**
  `csharp_type_for_dto_field` matched only bare `Json`, `Map<_, Json>` and `Option<Json>` and then
  fell through to `csharp_type`, which maps `Json` to `string` — so `Vec<Value>` became
  `List<string>`, reintroducing the exact "Cannot get the value of a token type 'StartObject' as a
  string" failure the function's own doc comment says it exists to prevent. It now recurses through
  `Optional`, `Vec` and `Map`, reusing the same wrapping formats as `CsharpMapper`'s
  `optional`/`vec`/`map` combinators, so non-Json types still resolve exactly as `csharp_type` does.
  The Java `resolve_field_type` doc comment is corrected in the same pass: it claimed unknown
  `Named` types are replaced with `JsonNode` when the backend actually emits `Object`, so the doc
  was wrong, not the code. (`src/backends/csharp/type_map.rs`,
  `src/backends/java/gen_bindings/types/shared.rs`)

- **Generated `From` impls carry only the clippy allows they can actually trigger.** Every emitted
  impl had an unconditional `#[allow(clippy::redundant_closure, clippy::useless_conversion)]` —
  ~1435 sites across xberg's four generated crates, half of it duplicating a crate-level allow. A
  new `needs_clippy_allow` scans the assembled field/statement/argument fragments for `(|` and for
  `.into()`/`Into::into`, and each allow is emitted only when its lint can fire, matching how
  `needless_update` was already gated. Two of the underlying closures are removed rather than
  suppressed: an optional `Arc` core wrapper now emits `.map(std::sync::Arc::new)` instead of
  `.map(|v| std::sync::Arc::new(v))`, and a newtype over an identity/tuple-passthrough `Named` type
  drops the no-op `.into()`. (`src/codegen/conversions/helpers/clippy_allow.rs`,
  `src/codegen/conversions/binding_to_core/render.rs`,
  `src/codegen/conversions/core_to_binding/render.rs`,
  `src/codegen/conversions/binding_to_core/wrappers.rs`,
  `src/codegen/templates/conversions/binding_to_core_impl.jinja`,
  `src/codegen/templates/conversions/core_to_binding_impl.jinja`)

- **A boxed opaque field converts to `Box<T>` in both directions.** `field.is_boxed` was ignored on
  both opaque paths. Binding→core moved the opaque wrapper's `Arc<T>` handle out of `.inner`
  directly, overwriting the `Box::new` applied upstream and yielding `Option<Arc<T>>` where the core
  struct declares `Option<Box<T>>`; core→binding nested the value as `Arc<Box<T>>`. Both branches
  now deref-clone the shared value and rebox it. The core→binding unbox rewrite also matched its
  input by exact string equality against `val.<field>.map(Into::into)`, so any other producer
  silently skipped the deref; it is structural now — strip the `<field>: val.<field>` prefix, unbox,
  then re-apply whatever the rest of the expression already did. Boxed struct fields had no test
  coverage in either direction. (`src/codegen/conversions/binding_to_core/render.rs`,
  `src/codegen/conversions/core_to_binding/render.rs`)

- **`mix format` is the sole formatter for generated Elixir; poly no longer touches `.ex`/`.exs`.**
  poly's pure-Rust Elixir formatter rewrites valid, mix-compliant source — `|>` pipe continuation
  drops from 6 spaces to 4, multi-line struct/map field continuation collapses to flush-left — and
  then its own `--check` reports the corrupted result as clean, so no freshness gate caught it: 247
  generated Elixir files in xberg drifted with every gate passing. Fixing the templates could not
  work, because poly re-corrupted correctly indented input on every run. Both the `--fix` and
  `--check` poly invocations now pass `--exclude **/*.ex --exclude **/*.exs`, and `mix deps.get`
  followed by `mix format` runs as an Elixir residual on a partial regen and once after the
  full-regen convergence loop. `mix` joins `required_formatters` whenever Elixir is targeted, so a
  missing binary warns loudly rather than silently leaving the output unformatted — silent skipping
  is what let this hide in the first place. (`src/cli/pipeline/format.rs`)

## [0.55.6] - 2026-08-06

### Fixed

- **The Dart native loader downloads and caches the library again on a cold cache.** alef had
  two divergent implementations of the same injected `_alefResolveExternalLibrary` prologue: a
  hardcoded `format!` in `frb_rewrite::external_library_loader` and
  `dart_init_prologue_replacement.jinja`, rendered into the generated bridge crate's `build.rs`.
  Within alef's own pipeline the `format!` variant always wins — a `post_build` FRB regeneration
  clobbers `build.rs`'s patch before `FrbDartSealedVariants` runs — and that variant only ever
  *read* the versioned cache, so a cache miss threw `StateError` even though
  `nativeDownloadAndCacheLibrary()` was defined and exported for exactly that case. Both call
  sites now render the one template, which keeps the `format!` variant's improvements
  (absolute-path `dlopen`, the `Platform.script` package-root fallback, the descriptive miss) and
  restores the download-on-miss step ahead of the `StateError`.
  (`src/backends/dart/templates/dart_init_prologue_replacement.jinja`,
  `src/backends/dart/frb_rewrite/external_library_loader.rs`,
  `src/backends/dart/gen_rust_crate/cargo.rs`)

- **`build.rs`'s embedded loader searches for the library that is actually built.** The bridge
  crate emitted at `packages/dart/rust/` is `<source>-dart`, so its cdylib is
  `lib<source>_dart.dylib` — but the source crate name was passed as the candidate stem, leaving
  the embedded loader looking for a `libhtml_to_markdown_rs.dylib` that no build produces. Only
  reachable when a consumer builds the bridge crate outside alef's pipeline, where it silently
  degraded every bundled-native lookup into a cache lookup.
  (`src/backends/dart/gen_rust_crate/mod.rs`)

- **The loader's "not found" message now names the actual environment variable.** The override
  was suggested as an escaped `\$nativeLibDirEnv`, so Dart printed the identifier rather than
  interpolating it and the reader was told to set a variable whose name was never given. The
  lookup also repeated the variable's value as a string literal instead of reading the
  `nativeLibDirEnv` constant, leaving two places that had to agree on it.
  (`src/backends/dart/templates/dart_init_prologue_replacement.jinja`)

## [0.55.5] - 2026-08-06

### Fixed

- **The CLI release now includes a Windows binary.** The publish matrix built only
  linux-x86_64, linux-aarch64 and macos-arm64, while the archive step's `.zip` branch and
  its `disable-cache` toggle were already written for Windows — the matrix entry was simply
  missing. `xberg-io/actions/install-alef` therefore found no asset on a Windows runner and
  fell back to `cargo install --git --tag`, building alef from source on every Windows job that
  installs it: 441s, 550s and 651s in html-to-markdown's three Windows Python e2e jobs alone.
  (`.github/workflows/publish.yaml`)

## [0.55.4] - 2026-08-06

`v0.55.2` and `v0.55.3` were tagged and pushed but never published to crates.io — the
`Publish` workflow only triggers on `release: types: [published]`, and no GitHub release
was created for either tag (see the `publish-flow` fix below). Their fixes are folded into
this section, in the order they actually landed, since 0.55.4 is the first version anyone
could actually install.

### Fixed

- **`nativeFree<Owner>` calls now pascal-case an acronym owner in the generated Kotlin JNI
  client's `close()`.** `close()` built the free-function name from the class name verbatim
  (`nativeFreeGraphQLRouteConfig`), while every other JNI emission site pascal-cases the
  owner via `to_pascal_case` (`nativeFreeGraphQlRouteConfig`) — so `close()` on any client
  type whose name contained an acronym called a native function that was never registered.
  `close()` now derives `free_name` from `to_pascal_case(class_name)`, matching the bridge's
  `external fun` declaration and the Rust JNI export.
  (`src/backends/kotlin/gen_bindings/jni_emitter/client_class.rs`)

- **Generated FFI free functions compile under edition 2024.** `free_function_header.jinja`
  emitted `pub extern "C" fn ...` for the generated `_free` shims; edition 2024 requires an
  `extern "C"` function containing raw-pointer or FFI-unsafe operations to be written as
  `unsafe extern "C" fn`, so every generated FFI binding with a free shim failed to compile.
  The template now emits `pub unsafe extern "C" fn`.
  (`src/backends/ffi/templates/free_function_header.jinja`)

- **The generated `poly.toml` is now poly-canonical when it is written.** `toml_array` hard-coded a
  4-space indent while its doc-comment claimed to emit "taplo's canonical multi-line form" — taplo
  uses 2 — and several inline arrays carried inner padding (`select = [ "correctness", … ]`). The
  freshly written file therefore never matched the committed one, so the byte-equality skip in
  `write_scaffold_files_with_overwrite` never fired and `poly.toml` was rewritten on every run in
  every repo. What normally hid this is the post-generation `poly fmt --fix` pass repairing it
  afterwards — but that runs after post-build, stubs, README, e2e and docs, so an abort in any of
  those leaves the raw file behind (observed on xberg, where the run died in the Dart FRB
  post-build), and the partial-regen paths never pass the repo root to poly at all. The emitter now
  matches taplo, and `poly.toml` is handed to poly immediately after it is written rather than many
  fallible stages later.
  (`src/scaffold/languages/poly.rs`, `src/cli/pipeline/generate/scaffold.rs`)

### Added

- **`[tools.mix]` is emitted for repos with an Elixir binding.** poly has no native Elixir formatter
  and `tree-sitter-elixir` ships no `indents.scm`, so poly reindented `.ex`/`.exs` with a hand-rolled
  query that modelled only `do…end` and `fn…end`; every other construct was re-emitted at column 0
  and poly then fought `mix format` indefinitely. Declaring the catalog tool hands the language to
  `mix format` — poly ≥0.19.6 drops its own reindenter when a runnable catalog formatter owns the
  language. (`src/scaffold/languages/poly.rs`)

## [0.55.1] - 2026-08-05

### Fixed

- **Generated Rust e2e harness compiles under edition 2024.** `tests/common.rs` called
  `std::env::set_var` at three points to publish the mock-server URLs, which edition 2024 made an
  unsafe function, so every integration-test binary failed to build with `error[E0133]` and the
  whole Rust e2e suite was uncompilable (seen on liter-llm). The calls are now wrapped in `unsafe`
  with a SAFETY note: they run inside the `OnceLock` initializer, before any test thread exists.
  (`src/e2e/codegen/rust/mock_server/common_module.rs`)

## [0.55.0] - 2026-08-05

### Changed

- **Python: a field whose name matches a method is an attribute again, not a bound method.** When a
  core type declared both a public field and a same-named inherent method, the PyO3 backend emitted
  a `#[pyo3(get)]` getter *and* a `#[pymethods]` wrapper. The wrapper is registered last and kills
  the getter, so `config.providers` silently returned a bound method while the generated stub and
  the constructor keyword both promised a list. The method wrapper is now skipped and the attribute
  wins, matching every other binding. Any caller written against the accidental `config.providers()`
  spelling must drop the parentheses.

### Fixed

- **A field and a same-named method no longer collide in the Go, Ruby, Swift and C# backends.** The
  same defect already fixed for the FFI (0.54.1) and WASM (0.54.2) backends, in four more emitters
  an earlier survey wrongly cleared. Go emitted both into one struct (`field and method with the
  same name Providers` — a hard compile error); Ruby emitted a duplicate inherent method
  (`error[E0592]`), a duplicate `define_method`, and an RBS `DuplicatedMethodDefinition` that failed
  `steep`; Swift and C# admitted the collision but had no live instance downstream. Each backend now
  emits the field and skips the method. A parameterized method of the same name is still emitted.
- **alef's own CI is green again.** The e2e PHP composer tests hardcoded real downstream project
  names, which `check_project_mentions.py` forbids — alef must stay project-agnostic — failing
  `no_project_name_special_casing_in_enforced_files` on all three platforms.

## [0.54.2] - 2026-08-05

### Fixed

- **Generated FFI code is clean under edition 2024's stricter lints.** Two more consequences of
  0.54.0's edition bump: the `ffi_set_out_error` helper nested `if let Ok(cs) = …` inside a null
  check, which edition 2024 rejects as `collapsible_if` now that let-chains are stable; and the
  error-method emitter wrote raw-pointer dereferences and `CString::from_raw` as bare statements
  inside `unsafe extern "C"` bodies, which `unsafe_op_in_unsafe_fn` — on by default in 2024 — turns
  into a hard `error[E0133]` for any error type declaring methods. Consumers lint generated crates
  with `-D warnings`, so both broke their builds.
- **The WASM backend no longer emits a duplicate binding for a field and a same-named method.**
  Mirroring the FFI fix in 0.54.1: the field-getter and method-wrapper loops both emitted
  `pub fn <name>` into one `#[wasm_bindgen] impl`, so a type with a `providers` field and a
  `providers()` method failed to compile with `error[E0592]`. The method wrapper is skipped when a
  field getter of that name was already emitted, leaving the getter as the callable surface. A
  survey of the other backends found napi, php, jni, go, dart and java unaffected.

## [0.54.1] - 2026-08-05

### Fixed

- **The generated FFI error accessors compile under edition 2024.** 0.54.0 moved generated Rust
  crates to edition 2024 and converted the FFI templates to `#[unsafe(no_mangle)]`, but the sweep
  missed `error_gen`'s shared emitter, which builds the `status_code`, `is_transient`, `error_type`
  and `error_type_free` functions from Rust string literals rather than templates. Those four kept a
  bare `#[no_mangle]`, which edition 2024 rejects (`unsafe attribute used without unsafe`), so any
  repo with a core error type failed to build its `-ffi` crate after regenerating on 0.54.0.
- **A field and a method sharing a name no longer emit a duplicate FFI symbol.** The field-accessor
  and method-wrapper emitters each minted `{prefix}_{type}_{name}` with no collision check, so a
  type with both a `providers` field and a `providers()` method produced two definitions of the same
  `#[unsafe(no_mangle)]` function (`error[E0428]`). The method wrapper is now skipped when a
  same-named field accessor was already emitted, which keeps the existing symbol and its semantics.

## [0.54.0] - 2026-08-05

### Added

- `crates.readme.languages.<name>.snippet_language` lets a README language borrow its code
  snippets from a differently-named snippet directory (e.g. an `ffi` README pulling examples
  from a `c/` snippet root, since the FFI binding's usage examples are C code and a consumer
  repo already maintains one `c/` snippet set rather than a duplicate `ffi/` one). Defaults to
  the language's own code, so existing configs are unaffected. Only applies to
  `include_snippet(language)` calls using the current README's own language variable — a
  template calling `include_snippet` with an explicit literal (e.g. `include_snippet("python")`)
  is unaffected.

### Changed

- Generated Rust crates (e2e `Cargo.toml`, scaffolded FFI crates) now declare `edition = "2024"`
  instead of `"2021"`, matching every other scaffolded language crate.

### Fixed

- **The generated PHP e2e `composer.json` uses the configured namespace verbatim as its PSR-4
  prefix.** The autoload key was re-derived from the *composer package name* by splitting it on
  `-` and upper-camel-casing each part, so `xberg/html-to-markdown` produced the three-segment
  prefix `Html\To\Markdown\` while the emitted PHP declared the one-segment `namespace
  HtmlToMarkdown;`. The prefix never matched, Composer never autoloaded the facade class, and
  every PHP e2e test failed with `Class "…\HtmlToMarkdown" not found`. A namespace that really
  does contain separators (e.g. `Xberg\Crawlberg`) is still preserved as written.
- **Generated Dart FRB loader code derives its `package:` URIs from `pubspec_name`.** The package
  segment was reconstructed from the bridge crate's file stem (`<crate>_dart` → `<crate>`), so a
  repository whose Dart package is named differently from its Rust crate emitted
  `package:html_to_markdown_rs/src/native_loader.dart` for a package actually named `h2m`. Every
  Dart e2e test failed to load with `Not found: 'package:…/src/native_loader.dart'`. This affected
  the loader import, both `Isolate.resolvePackageUri` calls, and the `dart run …:download_libs`
  hint. The bridge output directory stays crate-derived, since that is a Rust output path.
- The C FFI backend's static-constructor, string-parameter, and trait-bridge registration
  templates now compile under edition 2024. Three emitters (`ffi_opaque_constructor_header.jinja`
  and the `service_api_*`/`registration_variant` templates) still wrote a bare `#[no_mangle]`,
  which edition 2024 rejects outright (`unsafe attribute used without unsafe`). Several
  trait-bridge templates also dereferenced raw pointers (`&*vtable`) and called the `unsafe fn`
  `ffi_set_out_error` without an explicit `unsafe { }` block, which edition 2024 now warns on
  (`unsafe_op_in_unsafe_fn`) even inside an `unsafe fn` body. No generated symbol name, signature,
  or behavior changed.
- Generated shebang scripts keep their executable bit across a regen. `poly fmt` rewrites changed
  files via atomic rename, which resets the mode to `0644`, so every full regen silently stripped the
  bit from the scripts poly reformatted (`run_tests.php`, `download_ffi.sh`, `mvnw`, `gradlew`) and
  poly's own `file-safety` hook then rejected the next commit. The formatting pass now snapshots
  executable modes beforehand and restores any the formatter dropped.
- The generated `credo` pre-commit hook runs `mix deps.get` before `mix credo --strict`. poly runs
  hooks from a staged snapshot outside the repo and Elixir resolves dependencies strictly
  project-locally into a gitignored `deps/`, so credo's own package was missing there and every
  commit touching `.ex`/`.exs` files failed with "Unchecked dependencies for environment dev". The
  snapshot persists between runs, so the fetch is a one-time cost.

## [0.53.1] - 2026-08-04

### Added

- `[workspace.poly] lint-workspace` controls the generated `poly.toml`'s `[lint] workspace` setting.
  Repos whose CI installs only a subset of toolchains need `poly lint` to skip its whole-project
  phase; that setting previously existed only as a hand-edit to the generated file, which the next
  scaffold run silently dropped. Omitting the key emits no `[lint]` table, leaving poly's own
  default in force, so existing output is unchanged.

### Fixed

- A crate-local `Result` alias declared in one module is now honoured by functions in other modules.
  Extraction walks a crate file by file and replaced the alias hint map on every file, so the alias
  from `error.rs` was discarded before the module using it was resolved and its functions fell back
  to `anyhow::Error`. Hints now accumulate across a crate's modules and are reset once per crate, so
  a crate without its own alias no longer inherits the previous crate's error type.
- Swift e2e: a `count_min` assertion on an optional `Vec<Named>` field of a first-class parent DTO
  no longer emits `field()?.count ?? 0`. `emit_vec_struct_serde_getter` collapses that shape to a
  whole-field `-> String`, so the Swift side sees a `RustString` and the generated test failed to
  compile. The countable-vs-JSON-bridged classifier now mirrors the getter emitter's optional split.

## [0.53.0] - 2026-08-04

### Changed

- **An unresolvable README or docs snippet is now a hard error instead of a silent placeholder.**
  `crates.readme.snippets_dir` and `workspace.docs.snippets.dirs` entries that do not exist on disk
  are rejected up front, naming the config key and both the configured and resolved path; a snippet
  reference that cannot be resolved fails the run instead of emitting
  `<!-- snippet not found: ... -->` into the output. The README path previously never failed at all,
  so that placeholder shipped verbatim to package registries while `alef readme` reported success.
  The configured-directory check runs even when no template references a snippet, so a stale path
  cannot hide behind a template that happens not to use the filter.

  **Breaking:** repositories whose snippet references are already broken now fail `alef readme` /
  `alef docs` until the missing snippet files are added or the references removed.

### Fixed

- **Closing code fences in generated API reference docs are no longer tagged with a language.**
  `replace_fence_lang` appended the language to every line starting with a fence, including the
  closing one, turning ` ``` ` into ` ```rust ` and reopening the block instead of closing it. This
  corrupted every `**Example:**` block rendered from a doc comment.
- **A generic `Result<T>` alias now yields the crate's real error type in generated signatures.**
  Hint extraction was gated on the alias having no generic parameters, so the idiomatic
  `pub type Result<T> = std::result::Result<T, MyError>;` was skipped and signatures fell back to
  the placeholder `anyhow::Error`, rendered as a nonexistent `Error` type.
- **Magnus tagged-enum predicate methods emit Ruby booleans.** The value was interpolated through
  minijinja, which stringifies a bool Python-style, producing `def system? = True` — parsed by Ruby
  as a constant lookup, so any predicate call raised `NameError`.
- **Scaffold `.cargo/config.toml` `[env]` structured values render valid TOML booleans.** The
  `relative` flag was interpolated straight from a bool through minijinja, which stringifies it
  Python-style as `True`/`False` — invalid TOML that broke `cargo` on any scaffold using a
  structured env entry (e.g. the Ruby `preferred-ruby.sh` path). The value is now emitted as a
  lowercase `true`/`false` literal.
- **Generated Kotlin Android `build.gradle.kts` no longer stamps a downstream issue reference into
  every consuming project.** The release JNI guard's explanatory comment carried a cross-project
  issue link that no other repository can resolve; the technical rationale is retained.

## [0.52.0] - 2026-08-04

### Added

- **WASM binding crates can declare additional opt-in core features.** The new
  `[crates.wasm].extra_features` list emits each entry as a generated binding-crate feature that
  forwards to the matching core-crate feature without enabling it by default. This supports
  hand-written WASM modules whose `#[cfg(feature = "...")]` gates are not visible in Alef's extracted
  API surface.

### Fixed

- **Swift bindings link the Rust staticlibs by explicit `.a` path.** The generated `Package.swift`
  linked them via a bare `.linkedLibrary(...)`; with both `lib<name>.a` and `lib<name>.dylib` present
  in `target/`, ld64 preferred the `dynamic_lookup` dylib, so swift-bridge glue symbols (e.g.
  `__swift_bridge__$<Type>$_free`) were never linked and the swift test bundle failed to `dlopen`.
  The scaffold now emits a `resolvedStaticLib` helper and links the two Rust staticlibs by absolute
  `.a` path so the linker cannot substitute the sibling dylib.
- **PHP e2e/test_apps autoload path follows the crate move.** The generated composer autoload
  pkg-path defaulted to the historical `../../packages/php`, stale since 0.51 relocated the PHP
  source to `crates/<pkg>-php`; it now derives from the configured php crate output path (falling
  back to `packages/php` when unconfigured).

## [0.51.2] - 2026-08-04

### Fixed

- **Swift e2e: `Option<Vec<Named>>` fields no longer emit non-compiling `.count` assertions.**
  Fields like `elements: Option<Vec<Element>>` are natively bridged by swift-bridge as
  `Optional<RustVec<T>>`, not JSON-bridged to `RustString`. The e2e classifier previously
  treated every optional Vec field as JSON-bridged, so `count_min`/`count_equals`/`min_length`
  assertions emitted `<accessor>().toString().count` against `RustVec<T>?`, which does not
  compile ("value of type 'RustVec<Element>?' has no member 'toString'"). Classification now
  matches the real getter shape used by the Swift binding generator.
- **PHP e2e `composer.json` accepts guzzle 7 or 8.** The generated `require-dev` constraint was
  pinned to `^7.0`, which hard-fails `composer install` against a `composer.lock` that already
  resolved `guzzlehttp/guzzle` to `8.0.0`. The constraint is now `^7.0 || ^8.0`.

## [0.51.1] - 2026-08-04

### Fixed

- Generated Ruby wrappers no longer publish binding types into the global `Object` namespace.
  The previous `Object.const_set` loop exported every module (e.g. `Parser`) globally, colliding
  with unrelated gems such as `parser` (`TypeError: Parser is not a module`). Generated types now
  stay namespaced under their binding module; consumers reference them qualified.

## [0.51.0] - 2026-08-03

### Changed

- PHP userland classes and stubs now honor `[crates.output] php`, co-locating with the generated
  composer.json in the crate (unset config unchanged: `packages/php/`).

## [0.50.0] - 2026-08-03

### Added

- **Configurable logging across alef and its generated bindings.** All of alef's own diagnostics now
  flow through `tracing` (with `error!`/`warn!`/`info!`/`debug!`/`trace!` levels) instead of raw
  `eprintln!`/`println!`, filterable via `-v`/`-vv`/`-q`/`RUST_LOG`. Generated Rust binding glue logs
  host-callback failures through `tracing::warn!` and generated Java bindings through
  `java.lang.System.Logger`, so consuming libraries configure verbosity through their own logging
  setup. Genuine machine-readable command output (JSON reports, schema, diffs, listings) stays on
  stdout through a single sanctioned output helper.
- **A clippy print-guard forbids raw print macros on production code paths.** `print_stdout` and
  `print_stderr` are denied crate-wide (enforced by `poly lint` and the pre-commit hook); the few
  legitimate stdout sites (the output helper, report modules, e2e harness, and test code) carry a
  narrow `#[allow]`.

### Changed

- **Verbosity is reconciled to a single channel.** `-v` now raises the log level to `debug` and `-vv`
  to `trace` (previously `-v` did not change the level); the separate `DispatchContext.verbose` flag
  was removed and its per-file detail folded into `debug!`.
- **Generated Rust crates gain a `tracing` dependency** when trait bridges are present, sourced from
  the centralized version registry (Renovate-managed). The WASM `__log_host_failure` JS-console helper
  was removed in favor of a Rust-side `tracing::warn!`; consumers wanting browser output wire a wasm
  tracing subscriber.

### Fixed

- **Swift `RustBridgeC` target now emits a real object file.** The Swift backend declared
  `RustBridgeC` as a compiled SwiftPM target over a directory that held only `RustBridgeC.h`, with no
  translation unit. `swift build` tolerated the header-only target, but Xcode's XCBuild expected a
  `RustBridgeC.o` and failed to link, breaking every Xcode/iOS consumer of the published SPM package.
  The backend now also emits a minimal `RustBridgeC.c`, so a real object file is produced
  (html-to-markdown#449).

## [0.49.0] - 2026-08-01

### Added

- **Swift binding: `ffi_features` config knob.** The swift-bridge Rust shim's injected FFI-crate
  dependency (`<crate>-ffi`) can now be emitted with `default-features = false` and an explicit
  feature list via the new `[crates.<name>.swift] ffi_features` field. Previously this secondary
  dependency was always emitted in plain `{ version, path }` form, inheriting the FFI crate's default
  features with no way to drop cross-compile-hostile features (e.g. `heic` via `libheif-sys`, whose
  `build.rs` cannot satisfy `pkg-config` under cross-compilation). The primary core dependency's
  `features` / `excluded_default_features` / `target_dep_overrides` do not reach this injection.
  Empty (the default) preserves the previous plain form.

## [0.48.8] - 2026-07-29

### Fixed

- **Swift e2e `.count` assertions no longer emit uncompilable `RustString` accesses.** The Vec-field
  classifier in `build_swift_first_class_map` had dropped the `f.optional` disjunct, so optional
  `Vec<Named>` metadata fields (`headings`/`favicons`/`hreflangs`) — which the swift-bridge layer
  JSON-bridges to a `-> RustString` getter with no `.count` — were recorded as countable and emitted
  `headings()?.count`, failing to compile. Restore the disjunct: optional vecs are skipped while
  non-optional vecs (`urls`, `nodes`, `tables`) stay countable.

## [0.48.5] - 2026-07-27

### Added

- **Generated Zig e2e projects now expose a dedicated `smoke` build step** (`zig build smoke`) that
  runs `smoke_test.zig` in isolation, outside the serial test chain, as a fast published-package
  sanity check. Zig 0.16's `zig build` has no `--test-filter`, so the isolation is wired as its own
  build step with its own `RunStep` over the same compiled binary; it is emitted only when a
  `smoke_test.zig` fixture exists, so no dead step is generated.

### Fixed

- **The Dart flutter_rust_bridge loader is now upgraded in place when a stale one was injected by an
  older alef.** The marker-based idempotency check previously froze any already-injected loader
  forever, so a binding shipped with a cache-unaware loader never picked up the fix on regeneration:
  the download script populated the versioned cache, but the frozen loader never looked there. A file
  carrying the loader marker but not the current-template sentinel (`nativeCachedLibPath()`) now has
  its injected region replaced with the current template while preserving the original `init` body.
- **Zig e2e dependency resolution now treats repeated-character fill hashes (`AAAA…`) as placeholders,
  not just the explicit `STALE_HASH_REGENERATE` marker.** Such fills — used to keep `build.zig.zon`
  syntactically valid before a release exists to hash — were being emitted as real dependency hashes,
  failing `zig build` with a hash mismatch. They now fall through to the cache/network/omit-hash path.
  The heuristic requires a run of at least 16 identical characters, so it cannot misfire on a genuine
  base64 content multihash.

## [0.48.4] - 2026-07-27

### Fixed

- **C# NuGet packing no longer fails on a missing `runtime.json`.** `scaffold_csharp` now emits
  `packages/csharp/<Namespace>/runtime.json.template` alongside the csproj — the file the csproj's
  `RequireRuntimeJson` target has always required but that nothing ever generated, so every consumer
  `dotnet pack` errored. The template carries NuGet's RID-fallback graph (one `<PackageId>.runtime.<rid>`
  dependency per enabled published RID, plus `linux-musl-*` `#import` fallbacks) with a literal
  `{{VERSION}}` placeholder that CI substitutes before pack.
- **The generated Maven pom's enforcer floor no longer exceeds the CI runner's Maven version.**
  `MAVEN_CORE` (which feeds `<requireMavenVersion>`) had been renovate-bumped to `3.9.16`, above the
  `3.9.11` GitHub-hosted runners ship, so `enforce-maven` failed during publish. It is now a fixed
  compatibility floor (`3.6.3`) with the `renovate:` annotation removed so it is not auto-bumped again.

## [0.48.3] - 2026-07-26

### Fixed

- **Magnus RBS stubs now emit the real owning class for `Self`-returning methods instead of the
  `json_value` fallback.** When a type is managed by another codegen pass (e.g. a service owner
  type that is `binding_excluded`) it is still emitted as a `class` stub here, so builder-style
  methods and constructors returning `Self` (resolved to the owning type during extraction) must
  reference that class. A new `substitute_excluded_types_except_owner` never substitutes the owner
  type, restoring `-> App`-style return types (regression from 0.42→0.48).
- **Generated Go service templates no longer leave errors assigned to the blank identifier**, so
  they pass `golangci-lint` with `errcheck.check-blank = true`. The background `Run()` goroutine and
  the TCP readiness probe's `conn.Close()` in `service_start_background.jinja`, and the error-branch
  `json.Marshal` in `service_handler_registry.jinja`, now check their errors explicitly.

## [0.48.2] - 2026-07-26

### Fixed

- **A full regen (`alef all`) now converges to a zero-drift tree** instead of needing 2-3 manual
  `poly fmt --fix` passes downstream. `poly fmt --fix <root>` now loops to a fixed point (bounded
  at 3 passes, detected via `poly fmt --check`) — some poly-bundled engines (`.cs`, `.java`,
  `.json`) were not single-pass idempotent on freshly generated output.
- **Rust crates are no longer left rustfmt-dirty after a full regen.** A workspace-wide `cargo fmt
  --all` now runs (best-effort, skipped with a warning when `cargo`/`rustfmt` are unavailable),
  folded into the same convergence loop as `poly fmt` so any drift it introduces is reconciled by
  the next pass.
- **Cargo-sort now covers every crate in the workspace on a full regen, not just the languages
  that happened to be generated.** The old per-language cargo-sort residuals only ran for
  wasm/ffi/ruby/elixir/R, and the workspace-wide (`-w`) variant only ran when the ffi target was
  present — leaving python, node, php, swift, and dart binding crates unsorted and tripping poly's
  own bundled cargo-sort check. A full regen now runs a single `cargo sort -n -w` at the repo root
  covering the whole workspace regardless of target languages (partial/single-language regens keep
  the existing per-language residuals unchanged).

### Changed

- `format_generated`'s full-regen path (`only_languages = None`, used by `alef all`) now converges
  `poly fmt`, `cargo fmt`, and workspace-wide `cargo sort` together in one bounded loop instead of a
  single `poly fmt` pass plus fixed per-language residuals.

## [0.48.1] - 2026-07-26

### Fixed

- **Generated C# `.csproj` no longer embeds a downstream project name.** The thin meta-package
  `.csproj` template carried a comment referencing a specific consumer project's issue tracker
  (`xberg #1280`), leaking a downstream project name into every generated csproj and tripping alef's
  project-agnosticism enforcement. The internal issue references are removed from both the source
  doc comment and the emitted csproj comment.

## [0.48.0] - 2026-07-26

### Added

- **cbindgen C headers are formatted by poly.** When an FFI target is present, the generated
  `poly.toml` enables poly's `clang-format` catalog tool (`[tools.clang-format] enabled = true`) and a
  canonical `.clang-format` is scaffolded, so `poly fmt` and the pre-commit hook format the
  build-time-generated `crates/*-ffi/include/*.h` headers consistently across repos.
- **Per-language lint defaults extended so consumer repos can drop identical `[crates.lint.*]`
  overrides:** ruby runs `bundle install` before rubocop, and elixir runs `mix deps.get` before credo.

### Changed (BREAKING)

- **Removed the hidden `--format` flag** from `alef generate` / `all` / `init` / `e2e generate` /
  `test-apps generate`. Formatting always runs, delegating to `poly fmt` whenever poly is on PATH; when
  poly is absent, generation now warns and continues (emitting unformatted output) instead of aborting.
- **ktfmt (`--kotlinlang-style`) is now the single Kotlin formatter** for both the `kotlin` and
  `kotlin_android` backends (was `gradle ktlintCheck`), and the **Swift default formats only `Sources`**
  (not `Tests`). Both match what every consumer repo already overrode to; regenerating changes the
  Kotlin, Kotlin-Android, and Swift lint commands.

### Fixed

- **Swift: generated `Package.swift` links `libbz2`** at both the dev and artifactbundle sites, fixing
  undefined `_BZ2_bzDecompress*` symbols in the RustBridge target.
- **Python `.pyi` enum stubs no longer emit `# noqa: PYI029`** on the generated `__str__`/`__repr__`
  stubs. PYI029 is not enabled in the generated ruff config, so ruff flagged the suppression itself as
  an unused directive (`RUF100`).
- **Kotlin-Android `build.gradle.kts` formatting cleaned up.** The host-JNI `else` branch keeps a
  single-spaced trailing `// linux` comment (was double-spaced), and the
  `mavenPublishing { configure(...) }` call wraps its multi-line `AndroidSingleVariantLibrary(...)`
  argument onto its own line.
- **Ruby `.rbs` stubs no longer reference undeclared types (`steep RBS::UnknownTypeName`).** Streaming
  methods now declare `Enumerator[<ItemType>]` from the adapter's real item type instead of an
  undeclared `<Method>Iterator`, and any signature referencing a binding-excluded or opaque
  (`alef(skip)`) type substitutes the declared `json_value` alias.
- **Ruby `.rbs` trait-typed parameters/returns now reference the interface name.** A parameter or
  return whose type is a trait was emitted with the bare trait name (e.g. `DocumentExtractor`), but
  traits are surfaced only as host-implementable `interface _TraitName` declarations, so `steep`
  failed with `RBS::UnknownTypeName`. Such references are now substituted to their `_`-prefixed
  interface name.
- **Python PyO3 trait-bridge Protocol methods with numeric returns are typed `Iterable` (#203).**
  A Protocol method is implemented by the host and its return extracted by the bridge, so typing it
  with the parameter rule (e.g. `Vec<Vec<f32>>` → `list[list[float]]`) rejected NumPy values the
  bridge already accepts, forcing a `.tolist()` at every call. Numeric `Vec` returns now render as
  `Iterable`; only numeric leaves widen, and parameters and ordinary function stubs are unchanged.

### Removed

- **PMD/CPD dropped from the generated Java package.** PMD ran the built-in `quickstart` ruleset
  (the emitted `pmd-ruleset.xml` was never referenced by the `pom.xml`), and PMD/CPD mostly fought
  alef-generated code. The `pmd` workspace hook, the `maven-pmd-plugin` build plugin and its
  `pmd.skip`/`cpd.skip` publish-profile properties, and the scaffolded `pmd-ruleset.xml` are all
  removed. `checkstyle` continues to run as before.
- **ktlint removed entirely from generated Kotlin and Kotlin-Android projects.** ktfmt is the single
  Kotlin formatter, and ktlint's rule set fought ktfmt's output on generated code. The
  `org.jlleitschuh.gradle.ktlint` gradle plugin (and its `ktlint {}` config block), the `ktlint`
  `poly.toml` workspace hook (`gradle ktlintCheck`), and the `ktlint_standard_*` `.editorconfig`
  overrides are all removed from both backends.

## [0.47.2] - 2026-07-25

### Fixed

- **Generated Go binding is cgo-safe again.** The `// If linking fails … cannot find -lxberg_ffi …` note
  was emitted directly above the `/* #cgo … */` preamble, so cgo fed it to the C compiler
  (`error: unknown type name 'If'`, stray backtick) and every cgo build failed. The note is now
  separated from the cgo preamble by a blank line.

## [0.47.1] - 2026-07-25

### Fixed

- **C# meta-package is thin again (fixes NuGet HTTP 413 on publish).** The generated
  `packages/csharp/<Namespace>/<Namespace>.csproj` packed the entire native closure via
  `<None Include="runtimes/**">`, pushing the `XbergIo.Xberg` meta package past NuGet's size limit
  (HTTP 413; regressed since rc.37 — the per-RID split from #1280/rc.35 had slimmed it). The template
  now packs only `runtime.json` — the RID-fallback graph rendered from `runtime.json.template` by CI —
  plus the managed assembly, and adds a `RequireRuntimeJson` pre-pack target that hard-errors if
  `runtime.json` is missing. Native closures continue to ship in the per-RID
  `<PackageId>.runtime.<rid>` packages.

## [0.47.0] - 2026-07-25

### Fixed

- **Python `__all__` now honors `exclude_functions`.** An excluded function leaked into the generated
  `__init__.py` `__all__` even though it was correctly kept out of the `.api` import list — most
  visibly an excluded `*_async` variant whose sync sibling was already dropped. The undefined name in
  `__all__` tripped pyrefly's `bad-dunder-all` (now enforced by `poly lint .`) and would break
  `from <pkg> import *`. The `__all__` builder applies the same exclude filter as the import list.

## [0.46.0] - 2026-07-25

### Added

- **poly is now the single lint orchestrator: `poly lint .` invokes the external linters poly does not
  bundle.** The generated `poly.toml` emits a `workspace = true` hook per configured language for the
  tools poly has no built-in engine for — pyrefly (Python type-check), rubocop + steep (Ruby),
  golangci-lint (Go), checkstyle + pmd (Java), ktlint (Kotlin/Android), `dart analyze` (Dart), and
  credo (Elixir). Each runs once over its package directory, discovers its own native config
  (`.rubocop.yml`, `.golangci.yml`, `checkstyle.xml`, `.credo.exs`, `analysis_options.yaml`, …), and is
  skipped gracefully when its toolchain is absent. The existing `pyrefly` hook gains `workspace = true`
  so it actually runs during `poly lint .` (previously it only fired on git pre-commit). Downstream
  repos can drop their per-language lint tasks in favour of `poly lint .`.

### Changed

- **Python generated `pyproject.toml` no longer declares a `ruff` dev-dependency.** poly bundles ruff
  for lint+format, so a standalone `ruff` in the dev group is redundant; only the `pyrefly`
  type-checker (which poly does not provide) remains.
- **Python `poly.toml`: `[lint.python.ruff]` now uses an explicit `select` allowlist** instead of
  `select = ["ALL"]` minus an ignore list. Enabling every rule then suppressing the noise meant each
  ruff release could silently start firing a new deny-by-default copyright-header rule
  family) on generated bindings. The scaffold now selects the rule families we want; families that were
  only ever carried to be fully ignored (`COM`, `FBT`, `FIX`, `TD`, `PD`, `EM`, `TRY`, `BLE`) are no
  longer selected, and `ignore` is trimmed to the in-family sub-rules that remain relevant.

## [0.45.0] - 2026-07-25

### Added

- **Go backend: download-at-consume native distribution.** Published Go modules no longer require
  native libraries inside the module (module zips only contain the git tag's files; `.lib/` stays
  gitignored). The generated `cmd/setup` tool replaces `cmd/download_ffi`: it downloads the platform
  FFI library from the GitHub release into a versioned user cache
  (`os.UserCacheDir()/<name>/go/<version>/<platform>`), verifies its SHA-256 sidecar, and writes a
  machine-local, gitignored cgo link shim (`<name>_cgo_link.go`) with absolute `-L`/`-rpath` flags
  into the consumer's package. The binding exports a per-version `RequireNativeSetup_<version>`
  sentinel referenced by the shim, turning shim/module version skew into a compile error.
  `embed_ffi.go` now embeds only `include/*` so `go mod vendor` carries the C header; `go generate`
  runs `cmd/setup -lib-dir .lib`; test-app run defaults use `go run <module>/cmd/setup` instead of
  the copy-module-out-of-cache workaround.

### Fixed

- **PHP: registry-mode e2e `composer.json` now declares the userland PSR-4 autoload.** Only the
  `Local` dependency mode emitted the `"autoload"` section mapping the binding's PHP namespace to
  the local `packages/php/src/`, so registry-mode test apps could not resolve the userland classes
  layered over the native ext-php-rs extension — every test failed with `Class not found` even after
  PIE installed the extension. Both modes now emit the mapping via a shared helper.

## [0.44.0] - 2026-07-24

### Fixed

- **Swift: link the C++ standard library in the generated `Package.swift`**: the Rust staticlib pulls in
  C++ dependencies (onnxruntime, tesseract, ClipperLib) whose C++ ABI symbols (`__cxa_throw`,
  `__gxx_personality_v0`, `__cxa_guard_acquire`, …) were left undefined at the SwiftPM link step, so
  consuming a published Swift package failed to link. Both the in-tree and the published
  `.binaryTarget` root manifests now link `c++` on Apple platforms and `stdc++` on Linux,
  platform-conditionally.

- **Renovate now actually maintains the generated dependency version pins**: the `renovate.json`
  regex customManager targeted a stale path (`crates/alef-core/src/template_versions.rs`, gone
  since the crate went root-flat in 0.18.0) and required `// renovate:` marker comments that no
  const carried, so it bumped nothing. The path is corrected to `src/core/template_versions.rs`
  and every auto-bumpable const now carries a `datasource`/`depName` marker. An explicit top-level
  `"enabled": true` re-enables the repo (a closed onboarding PR had left it flagged disabled). Pins
  no longer drift stale, which is what was driving Dependabot churn (jackson, guzzle, junit, …) in
  the generated `/packages/*` and `/e2e/*` directories of consumer repos.

- **Generated dependency versions are fully centralized in `template_versions.rs`**: several
  versions were hardcoded outside the registry and had drifted. The Java scaffold `pom.xml` (a raw
  `format!` string, also a jinja-templates rule violation) is converted to a Minijinja template and
  sources every version from `template_versions::maven`/`toolchain` (fixing stale jackson `2.21.2`,
  junit `5.11.4`, and six maven-plugin pins). The Java e2e pom template (`org.jetbrains:annotations`,
  `maven-antrun-plugin`), Python e2e (`pytest`/`pytest-asyncio`/`pytest-timeout`/`setuptools`), Gleam
  e2e (`gleam_http` range), Dart scaffold (`http`, `crypto`), and Rust e2e (`serde`/`serde_json`/
  `tokio`) now all draw from the central consts.

- **Renovate marker datasources corrected so no pins error out**: the Dart pins used
  `datasource=pub`, but Renovate's Dart datasource id is `dart` — the invalid id produced
  "Missing datasource" / "Unsupported range strategy" warnings and blocked those bumps. The Gradle
  plugin pins (`ktlint-gradle`, `gradle-versions-plugin`, `gradle-maven-publish-plugin`) resolve
  from the Gradle Plugin Portal rather than Maven Central, so a `registryUrls` package rule points
  their maven lookups there (fixing the `ktlint-gradle: no-result` lookup failure). Renovate has no
  CRAN datasource, so the `rextendr` pin is now manually tracked (marker removed) rather than
  emitting a "Missing datasource" warning, and the custom manager carries an explicit
  `rangeStrategy: replace`. The Ruby gem pins used pessimistic (`~>`) constraints that
  Renovate's regex custom manager cannot bump — the `ruby` versioning then logged an
  "Unsupported range strategy" warning and produced no update — so those markers are removed
  and the gems are tracked manually (their `~>` floors already admit newer releases at
  `bundle install`).

## [0.42.1] - 2026-07-22

### Added

- **Node (NAPI): the ergonomic `/service` module re-exports the native value types it wraps**: the
  generated `service.ts` exported only the service class, so consumers (and e2e harnesses) importing
  from `<pkg>/service` could not reach the `Method` enum or `RouteBuilder` the service API expects —
  a `Method`-is-`undefined` `TypeError` at runtime. The service module now also re-exports the native
  value types referenced by the service surface, skipping its internal aliased self-import.

- **Ruby (Magnus): ABI-aware native extension loading and staging**: the generated `native.rb`
  wrapper now resolves the compiled extension through `RbConfig` — searching ABI-specific candidates
  (`lib/<ext>/<ruby_version>/...`) across `DLEXT`/`DLEXT2`, with the legacy flat path as a fallback —
  and raises a `LoadError` listing every expanded candidate when none match. The Ruby packager stages
  native libraries under `lib/<ext_name>/<ruby_abi>/...`, deriving the ABI from `RbConfig`'s
  `ruby_version` unless `RUBY_ABI` is set. This makes alef the canonical place for multi-ABI Ruby
  distribution. A `RUBY_ABI` override is now trimmed and rejected when blank, and a failing `ruby`
  invocation surfaces its stderr for diagnosability.

### Fixed

- **Elixir e2e: ExUnit test names are bounded to stay under the 255-character limit**: a fixture with
  a long description produced a computed test name (`test {describe} {description}`) of 255+ characters,
  which ExUnit rejects with `SystemLimitError`, failing the whole suite at compile time. The description
  portion of the test name is now truncated on a UTF-8 char boundary to keep the full name under the
  limit; each describe wraps a single test, so names remain unique.

- **`[crates.exclude].fields` now applies to external type roots**: fields hidden globally were only
  pruned from the primary crate's surface, so a field on an externally-extracted DTO root could pull
  in a colliding foreign type and fail merge with a same-name host conflict. Excluded fields are now
  applied to each external type root before its DTO roots are expanded, matching the behavior on the
  primary surface.

## [0.39.0] - 2026-07-20

### Added

- **WASM: configurable `wasm-opt` pass via `[crates.wasm].wasm_opt`**: the generated wasm binding
  `Cargo.toml` hard-coded `[package.metadata.wasm-pack.profile.release] wasm-opt = false`, so
  wasm-pack always skipped the size-optimization pass. A new `wasm_opt` field (a list of `wasm-opt`
  flags, e.g. `["-Oz"]`) is now emitted as `wasm-opt = [...]` when set, letting large wasm builds
  stay under CDN per-file size caps. Defaults to empty, which still emits `wasm-opt = false` — the
  historical behavior is unchanged for consumers that don't set it.

## [0.38.4] - 2026-07-20

### Fixed

- **Ruby (Magnus): `&mut self` methods on opaque types are now bound**: the module-init
  registration loop unconditionally skipped every `RefMut`-receiver method, so an opaque type whose
  methods all take `&mut self` (e.g. a tree-sitter `Parser` with `set_language`/`parse`/`parse_bytes`/
  `reset`) was exposed to Ruby with zero callable methods. Opaque wrappers are `Arc<Mutex<T>>` and
  their instance methods already delegate through the lock, so these methods now register. The gate
  stays scoped to opaque types — non-opaque by-value DTOs have no delegating wrapper and are
  unchanged.
- **Ruby (Magnus): a `Bytes` parameter now decodes from a Ruby `String`**: the generated `.rbs`
  advertised `String` but the wrapper took `Vec<u8>` (a Ruby `Array`), so a `bytes` argument such as
  `parse_bytes(source)` could not be called with a String. Magnus `Bytes` params now take
  `magnus::RString` and copy into a `Vec<u8>` before the core call, matching the advertised contract.
- **Swift: `count_min`/`count_equals` assertions on an opaque-parent `Optional<RustVec<T>>` field**
  now count the decoded array directly instead of `.toString().count`, which counted characters of a
  JSON string rather than elements.
- **Swift: DTO `CodingKeys` now honor serde `rename`/`rename_all`**: a discriminant field renamed at
  the serde layer (e.g. `call_type` serialized as the wire key `type` on a tool-call variant) is now
  decoded from its wire key instead of throwing `keyNotFound` on a key the payload never contains.
- **Swift: `Optional<Vec<struct/tagged-enum>>` accessors no longer double-encode**: the
  `getter_vec_enum_string_optional` template encoded each element to a JSON string and collected an
  array of JSON strings, which Swift's strongly-typed `[T]?` `init(from:)` rejected with a
  `typeMismatch` (expected object, found string). The accessor now serializes the field directly via
  `serde_json`. The non-optional `Vec<String>` path (which strips quotes element-wise) is unchanged.
- **Elixir: the e2e generator no longer appends an `_async` suffix to streaming entry points**
  (e.g. `chat_stream`), which produced calls to a nonexistent `chat_stream_async/2`
  (`UndefinedFunctionError`). The binding was always correct; only the generated e2e was wrong.
- **Internal: removed downstream project-name references from enforced source files** (a Swift
  forwarder test's error-type fixture and a conversions doc-comment example) so the
  project-agnostic guard passes.

## [0.38.3] - 2026-07-20

### Fixed

- **Go: free-function name no longer collides with a same-named type**: when a Rust crate exposed
  both a free function and a struct that mapped to the same Go PascalCase identifier (e.g.
  `model_info` / `ModelInfo`), the Go backend emitted both `func ModelInfo(...)` and
  `type ModelInfo struct`, which the Go compiler rejects as a redeclaration. Free functions whose
  Go name collides with a generated type name are now `Get`-prefixed (`GetModelInfo`); the type name
  and the underlying C FFI symbol are unchanged.
- **Ruby (Magnus): enum data-variant `Map` fields flattened to `String` now round-trip via JSON**:
  Magnus collapses a `Map` field on an enum data-variant to a JSON `String` DTO field, but the
  generated `From` impls still emitted the `HashMap::into_iter().map(...).collect()` template,
  producing uncompilable Rust (`into_iter` on `String`). Such fields now round-trip via
  `serde_json`. Struct `Map` fields (which Magnus keeps as native `HashMap`) are unaffected.
- **Swift: free functions returning a `String`-backed enum no longer emit an invalid initializer**:
  the forwarder used the struct positional-init template (`EnumType(_rb_obj)`) for enum returns,
  but a `String`-backed enum only synthesizes `init(from:)`. Enum returns now decode via the enum's
  `RawValue` initializer, matching the existing enum-typed DTO-field pattern.

### Added

- **Multipart request-body synthesis for TestClient-driven languages**: the shared `http_call`
  driver (Go, Zig, Gleam) now synthesizes a `multipart/form-data` request body from the handler's
  object body schema when a fixture declares that content type but carries no explicit body —
  matching the Python/Ruby/TypeScript generators. Previously these languages emitted an empty
  request body, so the core rejected multipart upload fixtures with 422 before the handler ran.

## [0.38.1] - 2026-07-19

### Fixed

- **`alef all --clean` now poly-formats root-level generated files**: the full-regen format pass
  only ran `poly fmt --fix` over each language's package directory, so generated files that live
  outside every package dir — `poly.toml`, `.cargo/config.toml`, and the docs/skills output — were
  never formatted and failed `poly fmt --check` in consuming repos (0.38.0 regression). A `--clean`
  run now formats the whole base directory.

## [0.38.0] - 2026-07-19

### Added

- **`[crates.ruby] required-ruby-version` config**: the scaffolded gemspec's
  `required_ruby_version` constraint is now configurable per repo. Unset, it defaults to
  `">= 3.2.0"` (see Fixed).
- **`[[workspace.poly.hooks-sources]]` passthrough**: external git-sourced pre-commit hook
  sources (e.g. an `ai-rulez` validation hook pinned by `git` + `revision`) are now modeled in
  `[workspace.poly]` and rendered as `[[hooks.sources]]` blocks in the generated `poly.toml`, so
  consumers relying on such hooks no longer have to hand-edit the generated file (which regen
  would clobber). Empty by default — output stays byte-identical when unused.

### Fixed

- **Ruby gemspec no longer pins `< 4.0`**: the scaffolded gemspec hardcoded
  `required_ruby_version = [">= 3.2.0", "< 4.0"]`, blocking `gem install` on Ruby 4.x. It now
  defaults to `">= 3.2.0"` (no upper bound). Affects every repo with a Ruby binding.
- **Elixir positional JSON-encoded NIF args now handle `nil` and pre-encoded strings**: the
  positional constructor arg (e.g. `create_engine/1`) unconditionally re-encoded via
  `Jason.encode!`, so a `nil` default config encoded to `"null"` and a pre-encoded JSON string
  (the documented `Jason.encode!(%Struct{})` form) double-encoded — both rejected by serde at the
  NIF boundary. The generated wrapper now forwards `nil` and binaries as-is and encodes native
  terms, mirroring the keyword-arg path. Affects all rustler bindings.
- **Generated Rust e2e test code is now clippy-clean under `--all-targets`**: the `min_length`
  assertion emitted `x.len() >= 1` (trips `clippy::len_zero`); for `n == 1` it now emits
  `!x.is_empty()` and keeps `len() >= n` for `n > 1`. The generated mock-server `Child` singleton
  is annotated `#[allow(clippy::zombie_processes)]`.

### Changed

- **Dependencies bumped to latest**: `syn` `2` → `3` and `jsonschema` `0.46` → `0.48`. The `syn` 3
  upgrade restructured `ItemImpl.trait_` (3-tuple → `(Path, For)`) and `Receiver` (`reference`/
  `mutability` → `kind: ReceiverKind`); the Rust-source extractor was adapted accordingly. No
  change to generated output.

## [0.37.2] - 2026-07-19

### Fixed

- **Swift e2e `.length`/`.count` assertions on JSON-bridged collections no longer emit
  uncompilable `.count`**: a length/count/size assertion whose collection leaf is a swift-bridge
  scalar `RustString` getter — an `Option<Vec<T>>`, `Map`, or `Vec<Vec<_>>` field, which bridges
  to a single JSON string with no `.count` — generated `<collection>()?.count`, which does not
  compile. Such assertions are now skipped with a "not available on result type" comment,
  matching the go/csharp/java backends. Countable `RustVec` getters (plain `Vec<T>`) are
  unaffected and still emit `.count`.

## [0.37.1] - 2026-07-19

### Fixed

- **Elixir streaming NIFs now compile**: the generated Rustler streaming start NIF
  (`crawl_stream`/`batch_crawl_stream`-style methods on an opaque resource) cloned the
  `Arc<RwLock<Handle>>` and called the core stream method on it, which does not exist
  (`E0599`). Streaming codegen now read-locks and clones the inner handle first, matching the
  non-streaming opaque method path.
- **Swift `Option<Vec<serde-struct>>` getters on opaque parents no longer collapse to `String`**:
  an optional `Vec` of a serde-deriving struct on an opaque (non-first-class) parent was
  JSON-degraded to a single `RustString` getter while the constructor kept a real
  `Optional<RustVec<T>>`, so `.field()?.count` did not compile. The getter now returns
  `Option<Vec<T>>` (matching the constructor and the opaque element accessors); the JSON
  degradation is retained only for first-class Codable parents whose Swift decoder needs it.
- **Project-agnostic fixtures**: renamed real downstream project names used as sample fixtures in
  `src/core/ir/surface.rs` and the C# e2e test-app generator to neutral names, restoring the
  project-mention guard to green.

## [0.37.0] - 2026-07-19

### Added

- **`custom_modules` entries for backends that ignore them are now flagged** (#183): `alef generate`
  emits a warning when `[custom_modules].<lang>` carries entries for a language whose backend never
  consumes them (`node`, `wasm`, `go`, `java`, `csharp`). Only pyo3, ffi, php, magnus, rustler, and
  extendr read `custom_modules`; entries elsewhere silently did nothing. The warning names the
  language and, for wasm, points at `[crates.wasm].custom_rust_modules` — the knob that actually
  declares hand-written Rust modules. The misleading `custom_rust_modules` doc comment (which
  claimed `[custom_modules].wasm` adds TypeScript re-exports) is corrected.
- **`alef verify` flags hash-inconsistent trees** (#184): verify now reports when the generated tree
  carries more distinct `alef:hash` values than there are generating crates — the signature of a
  partial regeneration where some files were regenerated and others left with an older hash. The
  check is host-independent (it never recomputes the inputs hash), so partial regens are caught at
  commit time regardless of environment. Surfaces under `--exit-code`.

### Changed (BREAKING)

- **Generation fails fast when a required formatter is missing** (#184): `alef generate` and
  `alef all` now abort up front if `rustfmt`, `poly`, or (for languages with a cargo-sort residual:
  wasm/ffi/ruby/elixir/r) `cargo-sort` is not on `PATH`, instead of warning and emitting
  differently-formatted, host-dependent output. The error names each missing tool and how to install
  it. This makes generation deterministic modulo the config; install the listed tools to proceed.
- **Generated node/e2e dependency bumps**: `@napi-rs/cli` `^3.6.2` → `^3.7.3` (devDependency and the
  default build command), `@types/node` `^22.10.2` → `^26.0.0`, and `vitest` `^4.1.5` → `^4.1.10`.

### Fixed

- **`alef generate --lang <one>` no longer deletes other languages' output** (#178): the orphan
  sweep computed its keep set from the filtered language but widened its roots unconditionally
  (always including `packages/wasm` and `packages/typescript`), so a filtered run deleted every other
  binding's still-valid generated files. Filtered runs now scope the sweep roots to the requested
  languages' own directories; unfiltered `alef all` behavior is unchanged.
- **`alef all` no longer deletes the generated docs reference tree based on host state** (#184): the
  set of reference pages `generate_docs_stage` emits varies with the host (CLI/MCP source presence,
  doc-language subset), so a host that regenerated fewer pages let orphan cleanup delete the
  committed pages it did not produce. Committed pages under `[docs].reference_output` are now
  protected from orphan cleanup.

## [0.36.2] - 2026-07-13

### Fixed

- **Generated test apps had four runtime-breaking defects when run against published packages**:
  - **C# registry test app referenced the wrong NuGet id**: `render_csproj` emitted
    `<PackageReference Include="{project_name}">` (the C# assembly/namespace, e.g. `Xberg`) instead
    of the published NuGet id from `[crates.csharp].package_id` (e.g. `XbergIo.Xberg`), so
    `dotnet restore` failed with `NU1101: Unable to find package`. The registry-mode reference now
    resolves `package_id` → namespace → project name.
  - **Go test app's `go.mod` was an incomplete dependency graph**: only `github.com/stretchr/testify`
    was required, with none of its transitive deps, so `go test` aborted demanding `go mod tidy`.
    `render_go_mod` now emits testify's pinned indirect deps (`go-spew`, `go-difflib`, `yaml.v3`) as
    an `// indirect` block so the app builds offline without a manual tidy.
  - **Dart test app never fetched its native library**: the `download_libs` invocation had been
    dropped on the false premise that natives ship via pub.dev (they exceed pub.dev's 100 MB cap and
    are fetched from the GitHub release). Restored: the run config derives the under-test package name
    and runs `dart run <pkg>:download_libs` between `pub get` and `dart test`, so `RustLib.init()`
    finds the native.
  - **WASM/node test apps shipped a stale JS lockfile across `--clean`**: `pnpm-lock.yaml` pinned an
    older version than `package.json` wanted, tripping pnpm's `minimumReleaseAge` supply-chain gate.
    JS lockfiles (`pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`) are no longer preserved across
    `--clean` for `node`/`wasm`, so the post-generate `pnpm install --lockfile-only` regenerates them
    fresh; non-JS locks are still preserved.

## [0.36.1] - 2026-07-13

### Fixed

- **`alef docs` over-documented `#[cfg(feature = "…")]`-gated items for feature-restricted bindings**:
  the reference-docs generator rendered the full extracted API surface without evaluating each
  binding's effective feature set, so a binding whose feature set excludes a gate (e.g. the wasm
  binding — `wasm-target`, which does not enable `tree-sitter`) still documented the gated types,
  struct fields, enum variants, and functions, diverging from the surface the binding actually
  compiles. `generate_lang_doc` now filters the surface through the new
  `ApiSurface::with_cfg_filtered_deep` — which drops cfg-gated *members* (fields, enum variants,
  variant fields), not just top-level items — using each backend's real effective feature set
  (Swift/Dart force-enable every cfg-referenced feature minus `excluded_default_features`; other
  backends use their configured feature list). `cfg_feature_satisfied` gains three-valued (Kleene)
  evaluation with full `all`/`any`/`not` and nested-predicate support, and keeps any item whose gate
  depends on an unresolved non-feature leaf (e.g. `target_arch`), so target-conditional items are
  never wrongly dropped.

## [0.34.7] - 2026-07-10

### Fixed

- **dart native loader emitted unparsable Dart (`\${...}` instead of `${...}`)**: the
  `StateError` raised on a full native-library cache miss escaped `${nativeCacheDir() ...}` and
  `${nativeAssetUrlBase()}` as a literal `\$` instead of real Dart string interpolation. The stray
  backslash meant the enclosing single-quoted string terminated early at the nested
  `'<unresolved cache dir>'` literal, producing bare identifiers (`unresolved`, `cache`, `dir`)
  that fail to compile in every consumer of `frb_generated.dart`. Fixed in
  `frb_init_prologue_replacement`; added a regression test asserting real interpolation.
- **e2e shebang scripts lost their executable bit after formatting**: the scaffold writer chmods
  generated shebang scripts (e.g. `run_tests.php`) to `0o755`, but the subsequent `poly fmt --fix`
  pass in the e2e formatter rewrites them via atomic rename, resetting the mode to `0o644`. The
  generated suites then committed a non-executable `run_tests.php`, which trips the
  `check-shebang-scripts-are-executable` file-safety hook downstream. `run_formatters` now
  re-asserts the shebang chmod after every formatter pass, so shebang e2e scripts stay executable.

## [0.34.5] - 2026-07-09

### Added

- **dart native loader**: the Dart backend now generates a runtime loader that fetches the
  platform-matched native from the package's GitHub Release (version-pinned, SHA-256 verified)
  into a versioned user-cache dir on first use, instead of bundling all-platform natives in the
  published package. Adds a shared `native_loader.dart` helper, a cache-resolution loader stage
  that errors actionably on a full miss (naming the asset URL and the `download_libs` / env-var
  escape hatches), and the `crypto` dependency for SHA-256 verification.

### Fixed

- **cargo-machete false positives on binding scaffolds**: the R (extendr), Dart, and Ruby crate
  manifests declare `async-trait` — and Ruby additionally declares `tokio` — for trait-bridge
  support, but a synchronous trait bridge (e.g. a visitor) never imports them in the generated
  shim, so `cargo-machete` flagged them as unused and failed `poly lint`. Each generator now adds
  the emitted-but-unused dependency to its `[package.metadata.cargo-machete]` ignored list: R gains
  the stanza (it previously emitted none), Dart appends `async-trait` (its bridge genuinely uses
  `tokio`), and Ruby appends `async-trait` plus `tokio` when the bridge carries no real async. This
  removes the need to hand-patch the generated manifests after regeneration.

## [0.34.4] - 2026-07-09

### Fixed

- **java visitor codegen**: the upcall `FunctionDescriptor` for visitor callbacks now declares
  `ValueLayout.JAVA_INT` as its return layout, matching the `int`-returning `handleVisit*` bridge
  methods and the `int.class` `MethodType`. It previously emitted `ValueLayout.JAVA_LONG`, so the
  Java Linker rejected every visitor upcall stub with `IllegalArgumentException: Wrong method
  handle type: (MemorySegment×5)int`, making `withVisitor(...)` unusable — even a no-op visitor
  threw before any callback ran. The `JAVA_LONG` parameter layouts for genuine i64 arguments
  (e.g. `depth`, `index_in_parent`) are unchanged. Mirrors the `JAVA_INT` return layout the
  lifecycle/JSON-convention trait-bridge stubs already use.

## [0.34.3] - 2026-07-09

### Fixed

- **magnus (Ruby) codegen**: a non-variadic, infallible, synchronous free function whose
  parameters require fallible serde deserialization — a non-opaque `Named`, `Vec<Named>`, or
  sanitized `Vec<String>` param — now emits a `Result`-returning wrapper that `Ok(...)`-wraps
  the core call, instead of a stub whose `?`-based argument conversion failed to compile in a
  non-`Result` body (`E0277`). Surfaced by `max_sim_score(&MultiVectorEmbedding,
  &MultiVectorEmbedding) -> f64` and `max_sim_rank(...) -> Vec<LateInteractionMatch>`. Scoped
  strictly to this previously-broken case: variadic / error-returning / async functions keep
  their existing codegen path unchanged.
- **rustler (Elixir) codegen**: same-named NIF entries — a real definition plus its crate-root
  re-export under a narrower `cfg` (e.g. `max_sim_score`, gated `any(presets, late-interaction)`
  in its module and re-exported under `presets`) — are now collapsed via
  `dedup_same_name_functions` before re-gating. Emitting both produced two same-named
  `#[rustler::nif]` items whose cfgs overlap, which rustler auto-discovers and rejects at
  `on_load` with "Duplicate NIF entry". The other single-surface and Rust-cfg-gated backends
  already deduplicated; the native NIF generator was the last to only re-gate.

## [0.34.2] - 2026-07-08

### Fixed

- **dart scaffold**: the generated `.pubignore` now excludes native library binaries
  (`*.so`, `*.dylib`, `*.dll`) in addition to `lib/src/native/`. The FRB build stages the
  compiled library (every platform in CI) into `lib/src/<module>_bridge_generated/`, which
  is not covered by the `lib/src/native/` rule and pushed the published archive past
  pub.dev's 100MB cap (269MB observed). Native binaries are fetched at install time by
  `bin/download_libs.dart`, so none belong in the pub archive.
- **swift e2e codegen**: `count_min` / `count_equals` assertions on a scalar-string leaf no
  longer emit `.toString()?.count`. `.toString()` yields a non-optional Swift `String`, so
  optional-chaining `?.count` onto it failed to compile ("cannot use optional chaining on
  non-optional value of type 'String'"); such targets now take `.count` directly.

## [0.34.1] - 2026-07-08

### Fixed

- **codegen**: generated binding→core struct conversions now survive additive core
  changes. Every public-field `From<Binding> for Core` literal (and the lossy
  method-body and mirror-crate constructor literals in the magnus, php, dart, and
  swift backends) ends with `..Default::default()` whenever the core type
  implements `Default` — previously the trailer was emitted only when a field was
  skipped at generation time. A field added to a core config struct after
  generation now falls back to its core default instead of breaking every
  generated binding except napi with `E0063: missing field`, until the bindings
  are regenerated. Currently-mapped fields are still assigned explicitly, so
  existing conversions behave identically. `CODEGEN_FORMAT_VERSION` is bumped to
  `2` so `alef verify` re-stamps existing bindings with the forward-compatible
  literals.

## [0.34.0] - 2026-07-07

### Fixed

- **verify**: stop reporting every binding stale after unrelated changes. The inputs hash
  (`compute_inputs_hash`) no longer folds in the alef crate version (`ALEF_REV`) — a dedicated
  `CODEGEN_FORMAT_VERSION`, bumped only on output-affecting codegen changes, replaces it — and it
  now hashes a canonical, normalized serialization of `alef.toml` rather than its raw bytes. As a
  result, crate version bumps, comment/whitespace/key-order edits, and CRLF/LF differences no longer
  invalidate freshness. Source paths are normalized (repo-relative, forward-slash) before hashing.
  Adds `alef verify --verbose`, which prints the computed vs. embedded hash for each stale file.
- **scaffold (dart)**: emit `packages/dart/.pubignore` excluding bundled native libraries and
  development directories (`android/`, `ios/`, `blobs/`, `lib/src/native/`, `rust/`, `example/`,
  `test/`), so `dart pub publish` stays under pub.dev's 100 MB archive limit. The runtime
  `download_libs` script fetches the correct platform library from the GitHub release at install time.
- **e2e (swift)**: bind Vec-of-opaque accessors to a local before indexing
  (`let _vec = result.results(); _vec[0].tables()`) to prevent a use-after-free crash when
  swift-bridge releases the parent `RustVec` temporary mid-expression.
- **e2e (swift)**: emit `<expr>.toString().count` for scalar and optional-chain String
  count/emptiness assertions (previously skipped), parenthesize the optional form as
  `(… .count ?? 0)`, and bind `let result =` for `not_error` contract fixtures.

### Changed

- **rustler**: the generated `native.ex` `nif_versions` list is now driven by
  `[crates.publish.languages.elixir].nif_versions` (previously a hardcoded `["2.16", "2.17"]`),
  keeping the RustlerPrecompiled declaration in lockstep with packaging and the CI build matrix.

## [0.33.0] - 2026-07-07

### Changed

- **docs**: emit deprecation notices as Starlight-compatible `:::caution[…]` asides
  instead of mkdocs-Material `!!! warning "…"` admonitions, so generated reference
  pages render correctly under Astro Starlight. Reference pages stay `.md` (no other
  mkdocs-only syntax is generated), so type signatures with `<`, `{`, `[` need no
  MDX escaping.

### Fixed

- **docs (cli)**: expand `#[command(flatten)]` args in struct-like enum-variant
  commands. The CLI-doc generator handled `flatten` only on struct-derived commands,
  so subcommands defined as enum variants (e.g. a CLI whose `extract`/`batch` variants
  flatten an `ExtractionOverrides` args struct) emitted an opaque struct row instead of
  the flattened flags. A shared `process_command_field` helper now expands flattened
  args inline on both the struct and enum-variant paths.

## [0.32.11] - 2026-07-07

### Fixed

- **scaffold**: the generated repo-root `poly.toml` now emits the `[hooks.builtin]`
  keys `lint`/`fmt` instead of `polylint`/`polyfmt`, matching the current poly
  config schema. 0.32.10 fixed alef's own committed `poly.toml`, but the generator
  still emitted the old keys, so every downstream regen (e.g. xberg) reverted the
  config to a form poly rejects (`unknown field 'polyfmt'`). Fixed the emitter in
  `scaffold::languages::poly` and its tests.

## [0.32.10] - 2026-07-07

### Fixed

- **config**: rename the `[hooks.builtin]` keys `polylint`/`polyfmt` to `lint`/`fmt`
  in `poly.toml` to match the current poly config schema. The old keys made poly
  fail to load its config (`unknown field 'polyfmt'`), which broke the
  `poly-validate` CI job.
- **zig**: correct the trait-bridge complex-return test to assert the pass-through
  path. In the Zig trait-bridge ABI every complex return (`Bytes`, `Vec<T>`, struct,
  enum, Map) is a pre-serialized JSON `[*c]const u8` that the host impl returns
  directly, so the fallible thunk hands it back via `@constCast` rather than
  re-serializing with `std.json.fmt` (which zig 0.16 cannot apply to
  `[*c]const u8`). The codegen (shipped in 0.32.9) was already correct; the test
  still asserted the old `std.json.fmt` path. Test-only change, no codegen change.

## [0.32.6] - 2026-07-05

### Fixed

- **dart**: mirror→core conversions of `Vec<primitive>` fields now emit
  `.collect::<Vec<_>>()` instead of a bare `.collect()`. In a core struct literal
  that ends with `..Default::default()`, the field's expected type does not
  propagate through `.collect()` to pin the `x as _` cast target, so rustc
  reported `error[E0282]: type annotations needed` (e.g. `crawlberg::CrawlConfig`
  `retry_codes: Vec<u16>` from mirror `Vec<i64>`). Turbofishing resolves the
  `FromIterator` target eagerly so the element type is inferred. Applied to the
  single- and nested-`Vec` struct-field arms and the enum-variant field arm,
  matching the core→mirror direction.

## [0.32.5] - 2026-07-05

### Changed

- **java**: scaffolded Maven packages no longer wire the Spotless Maven plugin
  or emit `eclipse-formatter.xml`; Java formatting is delegated to `poly` while
  Checkstyle remains focused on correctness checks.

### Fixed

- **rustler**: plugin trait registration stubs now include the
  `implemented_methods` parameter, matching the native Rust NIF signature and
  avoiding load-time arity failures.
- **kotlin-android**: generated JNI dispatchers are public so public native
  registration methods do not expose an internal parameter type or trigger JVM
  symbol name mangling.
- **swift-e2e**: `count_min` assertions over opaque scalar method-call fields
  now convert `RustString` values to Swift `String` before checking `.count`.
- **zig-e2e**: generated tests convert returned C string pointers with
  `std.mem.span()` before JSON parsing, formatting, or byte-length assertions.

## [0.32.4] - 2026-07-05

### Added

- **php**: `package_entry_filenames` now resolves the PHP public facade class
  file (`<ExtensionNamePascal>.php`, emitted in the public-API pass) so an
  extension's `public_api_additions` attaches to it, matching the existing
  Python/Ruby wiring. Go/Dart/Node emit their entry file in a different pass and
  remain a documented no-op.

### Fixed

- **trait-bridge**: sync infallible bridge methods no longer swallow host
  failures silently. A raised/thrown host callback is logged with the wrapper
  and method name before the default value is substituted (value-returning
  methods) or the call is discarded (unit methods), so a fabricated default —
  e.g. a zero token count that reads as "fits any budget" — is no longer
  indistinguishable from a real result. Covers pyo3, napi, magnus, php, wasm
  (console.error), jni (including the host-error envelope text the dispatcher
  already marshals), rustler, extendr, the csharp primitive-return adapter,
  and the ffi null-slot/null-result edge defaults.
- **go**: generated cgo trampolines recover host panics instead of crashing
  the process, logging to stderr and returning the zero value (fallible slots
  marshal the panic text through `outError`). The invalid-handle paths —
  including the four plugin lifecycle slots — log and marshal `outError`
  instead of fabricating `1` as a return value.
- **dart**: the block_on shim logs and returns the default when an infallible
  host callback panics, instead of aborting the calling thread via `expect`.
- **java**: sync infallible trait methods now match the vtable slot signature
  exactly. Primitive/unit returns use the direct-value convention (the previous
  JSON-convention upcall stubs mismatched the C slot — a wild pointer write
  plus the status code read back as the return value — breaking such methods on
  every call); infallible `Char`/`Path` slots no longer declare a phantom
  `outError`, and infallible `Optional<non-primitive>`/`Bytes` slots declare no
  out-pointers at all, mirroring `c_return_convention`.

## [0.32.2] - 2026-07-04

### Fixed

- **swift**: first-class DTO method wrappers (`{type}_{method}_from_json`) now
  honor owned and optional parameters. Optional params are declared as
  `Option<T>` in the wrapper signature (mirroring the extern block's
  `!needs_json_bridge` guard) instead of a bare `T`, and `String`/`Named` call
  args are borrowed only when the core parameter is a reference (`is_ref`).
  Methods taking owned `String` or `Option<T>` params (e.g.
  `Response::set_cookie` / `set_header`) previously failed to compile (E0308).

## [0.32.1] - 2026-07-04

### Fixed

- **napi**: async JS handlers are now awaited in the generated handler bridge.
  The threadsafe-function return type is `Either<Promise<HandlerReturn>,
  HandlerReturn>`, so a handler that returns a thenable routes to the `Promise`
  arm (awaited on the Rust side) and a plain object routes to the value arm —
  supporting both sync and async handlers. Previously a Promise return
  serialized to `{}` and dispatch failed with a missing-field error. Adds a
  `HandlerReturn` newtype implementing `ValidateNapiValue`/`TypeName`, because
  `serde_json::Value` cannot satisfy the `Either`/`Promise` bounds directly.
- **jni**: the generated handler-bridge struct and trait-object storage now use
  `jni::refs::Global<jni::objects::JObject<'static>>` instead of the
  `jni::objects::GlobalRef` alias. In jni 0.22.4 `GlobalRef` and the whole
  `jni::objects::*` reference-type re-export are `#[deprecated]`, so the old
  emission tripped deprecation errors under `-D warnings` in generated bindings.
- **swift**: DTO Unit-returning method wrappers no longer bind `let __value =
  ...?` when the ok type is `()`, which tripped `clippy::let_unit_value` in
  generated bindings under `-D warnings`.
- **pyo3** (#174): `.pyi` stub field annotations that shadow a builtin (e.g. a
  field named `bytes`) are now qualified as `builtins.bytes` for both the field
  and `__init__` signatures, and `gen_stubs` auto-imports `builtins` — fixing a
  `mypy --strict` `valid-type` error. Salvages the #173 regression test onto
  main (the `binding_fields` converter filter has been present since 0.31.0).

## [0.32.0] - 2026-07-04

### Added

- **pipeline**: `transform_scaffold_files` extension hook, letting extensions
  post-process generated scaffold files before they are written.

### Fixed

- **jni**: trait-bridge registration now dispatches. The kotlin-android bridge
  object wraps the host in a generated `<Trait>JniDispatcher` (suspend
  interface methods are bridged via `runBlocking`), and the generated Rust
  bridge routes every trait method through its JSON `dispatch` entry point —
  previously registration discarded the object and no plugin call ever reached
  the host. Rust-defaulted methods and the `Plugin` lifecycle hooks get the
  same presence-guarded forwarding as the other dynamic backends (#170).
- **swift**: first-class DTO instance methods now emit real dispatch instead of
  being excluded/crashing. The Swift side serializes `self`, calls a generated
  Rust wrapper extern, and decodes the JSON result; the Rust wrapper
  deserializes into the **core** type (not the serde-less swift-bridge wrapper
  newtype), converts `Path` params to `PathBuf`/`&Path`, and uses swift-bridge's
  unlabeled arguments + `RustString` return. Both the extern block and the Rust
  wrapper are emitted for non-opaque types (previously nested in the `is_opaque`
  branch, so the Swift calls referenced Rust wrappers that were never generated).
  Also fixes `Renderer` trait-bridge dispatch.
- **zig**: complex trait-vtable return types are serialized to JSON and handed
  back as a caller-owned, NUL-terminated C string via `out_result`, replacing a
  placeholder that silently wrote null. Uses the Zig 0.16 `std.json.fmt` API.
- **csharp**: `Register{Trait}(impl)` now delegates to `Register`, which calls
  the native `Register{Trait}` — previously it stored the bridge but never
  registered it natively (a silent no-op).
- **rustler**: opaque resources are stored behind `Arc<RwLock<T>>` so `&mut self`
  methods (e.g. `Registry::extend_from_dir`) mutate the held value in place
  through a write lock instead of returning `Not implemented` (or, worse,
  mutating a throwaway clone). Reads take a read lock; all lock acquisitions
  recover from poison (`unwrap_or_else(|e| e.into_inner())`) to avoid crashing
  the BEAM.
- **napi**: TypeScript service wrappers call the `native{UpperCamel}` methods the
  Rust `#[napi]` glue actually exposes (`nativeRun`/`nativeIntoRouter`), not the
  bare `run`/`intoRouter` which do not exist on the native class.

### Removed

- **pyo3**: dropped the never-rendered `trait_bridge/bridge_function.jinja`
  placeholder template and its registration.

## [0.31.2] - 2026-07-04

### Fixed

- **pyo3**: field-less `_from_native_*` options converters (types whose fields
  are all binding-excluded, e.g. `App`, `GraphQLRouteConfig`) now name their
  parameter `_native` and emit a bare `return X()`, so the unused parameter no
  longer trips ruff `ARG001` in the generated `options.py`.
- **pyo3**: the visitor `Protocol` stub's "Optional methods…" note is now gated
  on `emit_docstrings`, so the default no longer emits a docstring into the
  generated `.pyi` (ruff `PYI021`/`PYI013`).

## [0.31.1] - 2026-07-04

### Fixed

- **jni**: complete the `needless_borrows_for_generic_args` fix from 0.31.0.
  The 0.31.0 change only touched the inline Optional-JSON marshaller; the
  `string_to_jstring(env, &s)` warnings in generated shims actually originate
  in the return templates. Pass the owned `String` by value there too
  (`return_optional_string`, `return_json`, `streaming_shims`).

## [0.31.0] - 2026-07-04

### Added

- **config**: `[workspace.poly.pyrefly-sub-configs]` — a glob → error-code map
  emitted as extra `[[tool.pyrefly.sub-config]]` blocks in the generated
  `pyproject.toml` (alongside the built-in `api.py` block), so extensions can
  suppress type-checker errors on generated modules whose runtime-reconciled
  pyo3 boundaries a static checker cannot follow.

### Fixed

- **pyo3**: `_from_native_*` options converters now reference only the fields
  the `@dataclass` declares (via `binding_fields`), no longer passing
  binding-excluded fields (`methods_joined_cache`, `headers_joined_cache`,
  `lifecycle_hooks`, `di_container`, …) as keyword arguments — which raised
  `unexpected-keyword` at type-check time and `TypeError` at runtime.
- **codegen**: extra clippy allows (`[workspace] extra_clippy_allows`) are now
  filtered against the backend's default allow block emitted above them, so a
  lint that is already allowed is not re-emitted — clearing clippy's
  `duplicated_attributes` lint under `-D warnings`.
- **codegen**: `clippy::redundant_field_names` is now in the crate-level allow
  block of the php, pyo3, napi, wasm, and dart backends, silencing pre-existing
  warnings in generated binding crates under clippy 1.95.
- **jni**: the `Optional` return marshaller no longer borrows the owned
  serialized `String` when calling `string_to_jstring` (`&s` → `s`), clearing a
  `clippy::needless_borrows_for_generic_args` warning in every generated JNI
  shim.
- **ffi**: the generated `build.rs` capsule header fixup now emits direct
  `header.replace(...)` statements instead of a `for` loop over an array
  literal, clearing a `clippy::single_element_loop` warning when a crate
  exposes a single capsule pointee type.
- **pyo3**: `options.py` now imports `Any` whenever `_from_native_*` converters
  are emitted (their `native: Any` parameter), not only when a `TypeRef::Json`
  field is present, fixing an `unknown-name` type-check error.

## [0.30.19] - 2026-07-04

### Fixed

- **swift**: `Vec<opaque-handle>` getters on an opaque parent type now bridge as
  a real `Vec<T>` (e.g. `ExtractionResult.results()` yields
  `RustVec<ExtractedDocument>`) instead of `Vec<String>`, so opaque-element
  accessors such as `.mimeType()`/`.content()` resolve. JSON degradation of a
  `Vec<Named>` getter to `Vec<String>` is now gated on the containing type being
  a first-class Codable struct rather than on the element type, keeping the two
  code paths (`gen_bindings` DTO classification and `gen_rust_crate` extern/getter
  emission) in lockstep via a shared `compute_first_class_dto_names` helper.
- **trait-bridge**: dynamic-backend bridges (pyo3, magnus, php, napi, wasm,
  rustler, extendr) now forward Rust-defaulted trait methods to the host
  object when it implements them, falling back to the genuine Rust default
  body otherwise. Previously a host implementation of a defaulted method
  (e.g. `supports_table_detection`, `process_document`) was silently ignored
  and the Rust default always won (#167).
- **trait-bridge**: generated host surfaces (Python `Protocol`, Ruby `.rbs`,
  PHP `interface`, Elixir behaviour, Node `.d.ts`) now match the runtime
  contract: Rust-defaulted methods are no longer required members (documented
  as optional instead), Elixir behaviours gain `@optional_callbacks` plus the
  lifecycle callbacks, and Node plugin interfaces declare the optional
  lifecycle hooks. Bridges treat a missing `initialize`/`shutdown` as a no-op
  instead of failing registration. On magnus the bridge no longer invokes
  `initialize` — which is the Ruby constructor — on host objects (#166).
- **pyo3**: plugin `Protocol` config parameters are now typed as the public
  options dataclass the package exports, and the bridge passes that type to
  the host, so an implementer typed against the public API conforms to the
  Protocol (#165).
- **rustler**: behaviour `@callback` specs now declare natively-marshalled
  struct params as `map()` instead of the stale JSON `String.t()` (#168).

## [0.30.18] - 2026-07-03

### Added

- **extension**: `Extension::public_api_additions` is now honored for **Ruby**,
  not just Python. `package_init_filename` is generalized to
  `package_entry_filenames(language, &ResolvedCrateConfig)`, which resolves each
  language's package entry file — including dynamic conventions like Ruby's
  `lib/<gem_name_snake>.rb` — so an extension can wire its public API into the
  gem entry. Additions remain append-only with exact-line de-dup and still do
  not feed the generation-inputs hash (`alef verify` unaffected). Languages
  whose entry file is produced outside the public-API pass continue to be a
  silent no-op.
- **hooks**: `alef all`, `alef scaffold`, and `alef init` now run `poly hooks
  install` after scaffolding, wiring poly's pre-commit + commit-msg git hooks
  (polylint, polyfmt, file_safety, the `cargo` builtin — clippy / cargo-sort /
  machete / deny — and the conventional-commit hook) from the generated
  `poly.toml`. Best-effort and idempotent: a no-op when `poly` is absent or the
  target is not a git repository.

### Changed

- **format**: generated code is now formatted by the `poly` (polylint) CLI as a
  single system dependency — one `poly fmt --fix` pass replaces the previous ~19
  per-language formatter shell-outs (ruff, oxfmt, rubocop, php-cs-fixer, gofmt,
  google-java-format, ktfmt, swift-format, dart, gleam, zig, shfmt, …). poly is
  invoked as a subprocess rather than compiled in, keeping alef's build lean and
  its dependency tree unchanged; a missing `poly` binary is a best-effort no-op.
  The scaffolded `poly.toml` drives lint, format, cargo interop
  (clippy/sort/machete/deny), and the pre-commit + commit-msg hooks. A residual
  `cargo sort` still runs at generation time for workspace-excluded binding
  crates so `alef verify` stays hash-stable.

## [0.30.17] - 2026-07-03

### Fixed

- **swift**: getters returning `Vec<T>` or `Option<Vec<T>>` where `T` is a
  serde-serializable struct now JSON-decode each bridged element. The Rust
  bridge serializes such collections to `Vec<String>` (per-element JSON) or a
  single JSON `String`, but the generated swift wrapper previously emitted
  `.map { try T($0) }`, which only compiles for scalar `RustVec<RustString>`
  getters and left the binding uncompilable. It now decodes with `JSONDecoder`
  (per-element for `Vec<T>`, whole-array for `Option<Vec<T>>`). Fixes generated
  bindings for core types such as `CellChange`, `PageRange`, `PageSignals`,
  `LayoutDetection`, and `PageInfo`.

## [0.30.16] - 2026-07-03

### Added

- **extension**: new `Extension::public_api_additions(api, cfg, language)`
  hook. Extensions can now contribute raw lines to a package's public-API
  init file (e.g. Python's `__init__.py`) during public-API generation, once
  per resolved language. Returned lines are appended verbatim with exact-line
  de-duplication so re-runs are idempotent; the extension owns all language
  semantics (imports, `__all__` merges). The default implementation returns an
  empty list. The appended content does not feed the generation-inputs hash,
  so `alef verify` is unaffected.

## [0.30.15] - 2026-07-03

### Fixed

- **config**: scaffold language-specific tests (`test_scaffold_python`,
  `test_scaffold_node`, and 12 others) no longer fail after
  `feat(scaffold): emit canonical rustfmt.toml`. `rustfmt.toml` is a
  repo-level file like `poly.toml`; the `language_files` test helper now
  filters it out so file-count assertions in language-specific tests remain
  stable. The `crates/alpha/Cargo.toml` fixture in the
  `sync_versions_patches_dep_tables_on_version_change` test now includes a
  minimal `src/lib.rs` stub so `cargo update --workspace --offline` no longer
  prints a "no targets specified in the manifest" error to the test output.

- **cli**: `alef sync-versions` no longer regenerates test_apps/ and scaffold
  files by default, which was causing ~20min hangs on large repos. The command
  now only updates version fields in manifests and alef.toml; regeneration is
  the responsibility of explicit `alef generate`, `alef all`, or `task
  alef:generate` invocations. Use `--regen` flag to opt into the old behavior
  (expensive, not recommended for routine version syncs).

### Added

- **poly**: `[workspace.poly.typos]` in `alef.toml` now feeds typos
  spell-checker allowlists into the generated `poly.toml`. Declare
  `[workspace.poly.typos.extend-words]` and
  `[workspace.poly.typos.extend-identifiers]` (each a `word = "word"` table)
  to preserve repo-specific allowlists across every `alef all` regeneration.
  Previously, `alef generate` clobbered hand-edited `[lint.typos.*]` sections
  in `poly.toml`; those customisations must now live in `alef.toml` under
  `[workspace.poly.typos]` (fixes #66, enables #67).

- **config**: resolve `[[crates.source_crates]]` from the cargo registry via
  `from_registry = true`. When set, each `sources` entry is treated as relative
  to the crate's published source root (resolved through `cargo metadata`)
  instead of a workspace-relative sibling path, making regeneration hermetic in
  worktrees, CI, and fresh clones. Default (`false`) behavior is unchanged.

## [0.30.14] - 2026-07-03

### Fixed

- **swift**: fix the `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV
  (called out as still-open in 0.30.13). A `Vec<Named struct>` getter on a serde
  type was emitted as an opaque `RustVec<Table>`, which swift-bridge cannot
  marshal safely — dereferencing it (e.g. `.tables().count`) crashed at runtime
  with SIGSEGV. Such getters are now bridged as a JSON `Vec<String>` (mirroring
  the existing `Vec<Named enum>` handling), yielding a countable, safely
  marshaled swift collection.

### Added

- **scaffold**: honor per-target core-dependency overrides in the scripting
  bindings (#164).

### Changed

- **style**: apply canonical poly formatting (rustfmt `max_width = 120`, taplo,
  oxc) across the jni/kotlin emitters, `deny.toml`, `renovate.json`, `.mcp.json`,
  and the e2e fixture schema.

## [0.30.13] - 2026-07-02

### Fixed

- **swift**: revert the broken Option-wrapping of non-optional JSON-bridged
  `Vec<T>` extern-block return types (introduced in 0.30.10). The wrapper
  declared `Option<String>` while the impl returned bare `String`, producing an
  E0308 type mismatch that failed every consuming swift binding's compile. The
  swift codegen now emits consistent `String`/`String`. (Does not address the
  separate `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV.)

## [0.30.12] - 2026-07-02

### Added

- **scaffold**: the poly scaffold now also emits a canonical repo-root `rustfmt.toml`
  (`max_width = 120`, alef-managed). poly's Rust formatter defers to rustfmt's own
  config discovery (matching `cargo fmt`), so this pins the width both tools use;
  without it rustfmt falls back to its 100 default. Every alef-managed repo
  standardizes on 120 to match poly's global `line_length` default.

## [0.30.11] - 2026-07-02

### Added

- **config**: `[workspace] extra_clippy_allows` — a string list of additional clippy lints
  to allow in every generated Rust binding file. Entries may be bare lint names
  (`"single_match"`) or `clippy::`-prefixed (`"clippy::single_match"`); both forms are
  accepted and normalised internally. The configured lints are merged (union,
  de-duplicated; defaults first, extras appended) with each backend's built-in default
  allow-list, and a single extra `#![allow(...)]` attribute is emitted after the defaults.
  When the list is absent or empty the generated output is byte-identical to the previous
  behaviour. Affected backends: pyo3, napi, magnus, php, rustler, extendr, wasm, dart,
  swift.

  Example:

  ```toml
  [workspace]
  extra_clippy_allows = ["single_match", "collapsible_match"]
  ```

## [0.30.10] - 2026-07-02

### Fixed

- **pyo3**: exclude capsule types from `_rust`-qualified return annotations. Capsule types (both raw
  round-trip and `ConstructFrom`) resolve to a host type imported from another package (e.g.
  `tree_sitter.Parser`), not a native pyclass. Qualifying them with `_rust.` in a free function's
  return annotation produced an attribute (`_rust.Parser`) that no longer exists, raising
  `AttributeError` at import on Pythons with eager annotations (<3.14). They are now excluded from
  `return_type_names`, consistent with how they are special-cased elsewhere in api.py generation.
- **swift**: nil-safe accessor for non-optional JSON-bridged `Vec<T>` fields. Wrapping such a field in
  `Option<>` makes swift-bridge emit the nil-checked accessor, matching sibling accessors, so a null
  bridged pointer degrades gracefully instead of segfaulting. Defensive fix; the underlying
  null-pointer root cause is not yet confirmed.

### Changed

- **chore**: consolidate the typos allowlist into `poly.toml` and drop dead configs.

## [0.30.9] - 2026-07-02

### Fixed

- **codegen/ffi**: complete the service-owner forward-declaration fix from 0.30.8. The new
  `api.services` loop filtered by `exclude_types`, but a service owner is `binding_excluded` by
  construction and therefore always in that set — so the owner (`App`) was still dropped and the
  `typedef struct {PREFIX}App {PREFIX}App;` never emitted. Service owners are now forward-declared
  unconditionally (their `{PREFIX}{Service}Opaque.inner` pointer references them regardless of
  exclusion). Regression test tightened to mark the owner `binding_excluded`.

## [0.30.8] - 2026-07-02

### Fixed

- **codegen/ffi**: the C header no longer references an undeclared service-owner type. The cbindgen
  forward-declaration pass iterated `api.types`/`enums`/`errors` but not `api.services`, so a service
  owner (e.g. `App`) emitted as the opaque `inner` pointer of its `{PREFIX}{Service}Opaque` handle
  (`{PREFIX}App *inner`) had no `typedef struct {PREFIX}App {PREFIX}App;` — cbindgen then failed the
  downstream C/Go build with "unknown type name". Service owners are now forward-declared too
  (filtered by `exclude_types`). Declaring the owner in `[workspace.opaque_types]` is not required.
- **sync-versions**: three alef-emitted version sites were left at the prior version on every bump.
  - Root `Package.swift`: the `.binaryTarget` artifactbundle URL
    (`releases/download/vX.Y.Z/…`) was only updated via the `v__ALEF_SWIFT_VERSION__` placeholder,
    which is gone after the first sync — so subsequent bumps left the concrete tag stale (downstream
    `from: "X.Y.Z"` consumers fetched the wrong artifact). Now rewrites the concrete
    `releases/download/vX.Y.Z/` segment too, matching the shape `verify_versions` already checks.
  - C# `.csproj`: `<InformationalVersion>` was never rewritten (only `<Version>` was). Both are now
    bumped.
  - Ruby native (Magnus) crate `packages/ruby/ext/*/native/Cargo.toml`: the core-crate dependency
    pin (`<core> = { version = "X.Y.Z", path = "…" }`) drifted because this crate is not a workspace
    member and the workspace dep-pin pass never saw it. The pin now tracks the workspace version.

## [0.30.7] - 2026-07-02

### Fixed

- **codegen/pyo3**: `_to_rust_*` converters dropped all cfg-gated fields from the Rust constructor
  call (filter was `f.cfg.is_none()`). Feature-gated fields such as `UrlExtractionConfig.crawl`
  (gated on `any(feature = "url-ingestion", feature = "url-config-types")`) ARE compiled into the
  pyo3 `#[new]` constructor, so omitting them left them unset. Added `cfg_present_for_pyo3`
  (mirroring the `.pyi` stub's `cfg_present_for_pyo3_stub`): keep fields with no cfg or whose cfg
  resolves to present in the native pyo3 build (feature gates, `not(target_arch = "wasm32")`, or
  `any(...)` of those), while still dropping genuinely platform-specific fields.
- **maven**: pin jackson to `2.19.0`. jackson 2.20+ adopted a 2-component scheme (2.20/2.21/2.22)
  only partially on Maven Central (jackson-core/databind 2.22 and any x.y.0 return 404), breaking
  generated Java/Kotlin e2e dependency resolution. `2.19.0` is fully present across all five jackson
  artifacts.

## [0.30.6] - 2026-07-02

### Fixed

- `core_to_binding_convertible_types` false-negative: types whose only non-convertible binding
  fields are excluded from the backend surface (e.g. wasm `exclude_types`) were wrongly removed
  from the convertible set. The function now accepts `excluded_field_types: &[String]` and skips
  those fields in the predicate. All non-wasm backends pass `&[]`; the wasm backend passes its
  `exclude_types` list so structs with core-only omitted fields are correctly convertible.
- Wasm `gen_struct` emitted the delegating `impl Default` unconditionally for `has_default` types
  without checking convertibility, causing E0277 when `From<core::T>` was not generated.
  Non-convertible `has_default` wasm structs now correctly keep `#[derive(Default)]` instead.

## [0.30.5] - 2026-07-02

### Fixed

- **codegen/pyo3**: suppress delegating `Default` impl for types absent from `core_to_binding_convertible_types`. The struct generator emitted a delegating `impl Default` (calling `<core::T as Default>::default().into()`) for every `has_default` type, but `gen_from_core_to_binding` is only emitted when a type passes `can_generate_conversion`. A type with `has_default=true` whose fields include an unconvertible nested type received no `From<core::T>` impl, causing E0277 in the pyo3 backend (e.g. `ServerConfig`). Fixed by adding `emit_delegating_default_for_types: Option<&AHashSet<String>>` to `RustBindingConfig` and pre-computing the eligible set in the pyo3 backend before the type loop.
- **codegen/wasm**: apply `source_crate_remaps` inside `gen_delegating_default_impl`. When a `core_crate_override` remaps the leading crate segment (e.g. `spikard` → `spikard_http`), the delegating `Default` body used the raw `rust_path` verbatim, emitting `<spikard::ServerConfig as Default>::default().into()` instead of `<spikard_http::ServerConfig as Default>::default().into()`, causing E0433 in wasm. Fixed by calling `apply_crate_remaps` on the qualified path in `gen_delegating_default_impl` and threading `source_crate_remaps` through `RustBindingConfig`.

## [0.30.4] - 2026-07-02

### Fixed

- **defaults**: unwrap `Some(inner)` Rust defaults instead of collapsing them to `Empty`.
  `expr_to_default_value` had no `Some(...)` case in the `Expr::Call` arm, so `Option` fields with a
  `Some(literal)` default (e.g. `document_max_size: Some(50 * 1024 * 1024)`,
  `extraction_timeout_secs: Some(60)`) rendered as the type's zero value — Dart's `documentMaxSize`
  became `0`, truncating fetched documents to 0 bytes. The extractor now recurses into `Some(inner)`
  so the inner literal surfaces in synthesized default-config literals across every backend that
  emits them (dart/php/swift/…).
- **php**: map cfg-gated fields the binding keeps in the `From<binding>` conversion for core. The
  enum-tainted `From<binding>` generator unconditionally skipped every cfg-gated field, letting
  `..Default::default()` fill it. PHP keeps cfg-gated fields in the binding struct
  (`strip_cfg_fields_from_binding_struct = false`), so real values (`ExtractionConfig::keywords`,
  `UrlExtractionConfig::crawl`) were silently dropped on the PHP→core conversion. The skip is now
  gated on `strip_cfg_fields_from_binding_struct`, mirroring the standard `render.rs` path.
- **wasm**: infallible trait-bridge result conversion now returns `Option`. The `unwrap_or_default`
  branch chained `.and_then` on the `Option<String>` from `.as_string()` but the closure returned a
  `Result`, failing to compile (`E0308` expected `Option`, found `Result`; `E0425` unknown `e`). The
  closure now uses `.ok()`, fixing infallible trait methods that return enums/collections
  (`backend_type`, `processing_stage`, `supported_languages`, `dimensions`).
- **wasm**: add `--allow-multiple-definition` to the scaffolded `wasm32` rustflags.
  `wasm32-unknown-unknown` has no unified libc, so multiple C deps each ship functionally-equivalent
  libc stubs (tree-sitter's shim defines `__assert_fail`; a WASI-built Tesseract bundles
  wasi-libc `assert.o`/`atexit.o`) that `wasm-ld` rejects. The emitted `.cargo/config.toml` now
  passes first-def-wins linking, a no-op unless duplicates exist.
- **e2e/dart**: clear process-global plugin registries in `tearDownAll` to prevent a cross-isolate
  deadlock. Each Dart test file runs in its own isolate, but the Rust plugin registries are
  process-global; a file that registered a Dart-backed plugin left its `DartFnFuture` callback in the
  registry after its isolate died, and a later file's isolate deadlocked (30s timeout) invoking the
  dead callback via `block_on`. The generator now emits a `clear<Registry>()` call for each
  `register_*` backend fixture present in a file, taking the Dart e2e suite from 27 to 78 passing.

## [0.30.3] - 2026-07-01

### Changed

- **scaffold**: bump the generated e2e Java `jackson-databind` version (`JACKSON_E2E`) from
  2.18.2 to 2.22.0, matching the main jackson pin so regenerated e2e poms carry the security
  update instead of drifting from a manually-bumped dependency.
- **scaffold**: fold generated-test-code lint allowances into the emitter — `A001` and `N801`
  added to `TEST_IGNORES` (generated e2e tests take an `input` param shadowing the builtin;
  generated plugin trait-bridge stub classes aren't CapWords), and `I001` added to the
  `options.py` per-file-ignore. Consumer repos no longer need repo-specific `[workspace.poly]`
  overrides for these.

## [0.30.2] - 2026-07-01

### Added

- **config**: a `[workspace.poly]` section in `alef.toml` for repo-specific poly.toml overrides —
  extra `exclude` globs and cross-engine `per-file-ignores` that the scaffolder merges into the
  generated `poly.toml`, so repo-local lint suppressions survive regeneration.

### Changed

- **scaffold**: emit a single repo-root `poly.toml` that drives lint, format, git hooks, and
  commit-message policy, replacing `.pre-commit-config.yaml` and the per-tool config files
  (`[tool.ruff]`, `[tool.mypy]`, `phpstan.neon`, `.php-cs-fixer.dist.php`, `.lintr`, `.typos.toml`,
  `.rumdl.toml`). Python type-checking moves from mypy to pyrefly. The emitted config excludes
  Jinja templates from poly (reformatting them corrupts `{{ }}` placeholders) and carries
  generated-test-code lint allowances so regenerated e2e/test-app suites stay clean.

### Fixed

- **pyo3**: strip the Rust raw-identifier prefix in `.pyi` constructor params — PyO3 exposes a
  field declared `r#type` to Python as `type`, but the stub emitted `r#type` verbatim (invalid
  Python that ruff cannot parse). The `#[new]` signature keeps `r#` to compile.
- **pyo3**: drop the duplicate OptionsField trait-bridge parameter from the `.pyi __init__` stub.
  The field was emitted both as a regular param and as the dedicated bridge kwarg, producing a
  duplicate parameter; the stub now filters the bridge field out, mirroring `#[new]`.
- **pyo3**: drop the redundant closure when wrapping a zero-argument sync core call in
  `py.detach`. `py.detach(|| xberg::list_supported_formats())` tripped `clippy::redundant_closure`
  and failed `clippy -D warnings`; zero-arg calls now pass the function path directly
  (`py.detach(xberg::list_supported_formats)`). Calls that capture arguments keep the closure.
- **php**: generate the correct return type for `serde(default = "...")` helpers on fields whose
  core type is mirrored into a binding DTO. The helper returned the core type (e.g.
  `crawlberg::SsrfPolicy`) while the field is rendered as the crate-root mirror, so the generated
  php crate failed to compile (`expected SsrfPolicy, found crawlberg::SsrfPolicy`). The helper now
  returns the mirror and converts the core value via `.into()`.

## [0.30.1] - 2026-06-29

### Fixed

- **tests**: normalize docs-stage generated path assertions across Windows and Unix.
- **java**: always generate `ByteArraySerializer.java`. The generated ObjectMapper registers
  `new ByteArraySerializer()` unconditionally, but the class was only emitted when a record had a
  non-optional `Bytes` field — leaving a dangling reference that fails to compile for packages
  without one. It is now emitted unconditionally, matching `JsonUtil`.

## [0.30.0] - 2026-06-29

### Added

- **docs**: add a template-driven docs stage for API, CLI, MCP, `llms.txt`, agent skills, and
  snippet validation. Repos can configure generated reference output, required local templates for
  `llms.txt` and grouped skill files, static Clap/rmcp source extraction, and docs-specific snippet
  checks. Alef now warns on explicit skipped docs inputs such as missing configured sources or
  unavailable snippet toolchains while avoiding noisy warnings for unset optional docs layers.

- **snippets**: `typecheck` validation level. Ordered between `compile` and `run`, it statically
  type-checks a snippet without executing it, and for compiled languages without needing the native
  library. Each language runs its strict static checker: `python -m mypy`, `tsc --noEmit`,
  `cargo check`, `go vet`, `javac -Xlint:all -Werror`, `dotnet build -warnaserror`,
  `swiftc -typecheck -warnings-as-errors`, `kotlinc -Werror`, `dart analyze --fatal-infos`, and
  `cc -fsyntax-only -Wall -Werror`. This catches dual-representation mistakes (a config field typed
  against a flattened union alias that rejects the documented data-enum constructor) that
  `py_compile` and a lenient compile cannot see. A matching `snippet:typecheck-only` ceiling
  annotation sits alongside `syntax-only` and `compile-only`. mypy is optional: when it is not
  installed the Python snippet is reported as unavailable rather than failing.

### Fixed

- **napi**: give the generated streaming `WORKER_POOL` tokio runtime a 16 MB worker stack, so a
  deep consumer future does not overflow the default (~2 MB) worker stack and abort with `SIGBUS`.
- **pyo3**: provision an enlarged worker-thread stack on the generated module's async runtime.
  pyo3-async-runtimes' default multi-thread runtime gives workers a small (~2 MB) stack, which a
  deep consumer future (e.g. a multi-stage OCR pipeline) overflows — aborting the whole process
  with `SIGBUS`. The `#[pymodule]` init now installs a `tokio` runtime with a 16 MB
  `thread_stack_size` before the first `future_into_py`.
- **pyo3**: serialize `dict`/`list` values for JSON (`serde_json::Value`) config fields in the
  generated `api.py` converters. PyO3 cannot expose a settable `serde_json::Value` field, so the
  binding stores such fields as `str`, while the public dataclass and `.pyi` stub type them as
  `dict[str, Any]`. The converter forwarded the dict straight through, so the documented dict form
  raised `TypeError: 'dict' object is not an instance of 'str'` at runtime; it now `json.dumps`es a
  dict/list (passing `str`/`None` through unchanged).
- **pyo3**: re-point each re-exported exception's `__module__` at the public package in the
  generated `exceptions.py`. The classes are the native ones (`create_exception!` sets their
  module to the compiled `_native` extension), so tracebacks and `repr()` previously read
  `_native.DownloadError` instead of the public name, and the exceptions were not picklable under
  their public path. `exceptions.py` now reassigns `__module__` for every name in `__all__`
  (tree-sitter-language-pack issue #147).
- **codegen**: generate compiling binding→core conversions for core structs that have private
  (`pub(crate)`) fields. Such a struct cannot be built with struct-literal syntax from a foreign
  crate — neither by naming the private field nor by patching it with `..Default::default()` — so
  the conversion now seeds the core type's `Default` (which fills the private fields inside the
  defining crate) and assigns only the public fields onto it. The strategy is centralized in a
  shared helper used by the pyo3/napi/wasm/extendr/rustler/magnus generator, the Dart mirror crate
  generator, and the PHP enum-tainted conversion path; when the core type has private fields but no
  `Default`, a `compile_error!` guides the author to derive `Default`. A new `has_private_fields`
  flag on struct IR records the condition during extraction.
- **php**: marshal owned (by-value) native-struct callback parameters by value rather than
  dereferencing them as a borrow (`(*input)` does not type-check on an owned `core::T`), and stop
  emitting the native-object return fast-path — a PHP `#[php_class]` binding struct implements
  `FromZvalMut` (for `&mut T`) but not `FromZval` (for `T`), so the bridge keeps the JSON return
  path that is well-defined for PHP.
- **pyo3**: marshal owned (by-value) native-struct callback parameters into the host's native
  binding object via `From<core::T>`, the same way borrowed ones already were. A trait method that
  takes a serde struct by value (e.g. an extraction-input envelope) previously passed the raw
  `core::T` across the Python boundary, which has no `IntoPyObject` and failed to compile.
- **pyo3**: when a core `register_*` free function shares its name with a trait bridge's
  `register_fn`, emit only the bridge's duck-typed registration. The function loop no longer also
  emits the auto-wrapped core version, which collided (`E0428`) with the bridge definition and no
  longer type-checks against a registry that takes `Arc<dyn Trait>`.
- **pyo3**: the generated Python package now type-checks clean under `mypy`. Data-enum config fields
  are annotated against their public class (so `EmbeddingConfig(model=EmbeddingModelType.plugin(...))`
  is accepted) instead of a flattened union alias that shadowed the class; constructors accept the
  public dataclass/dict for factory parameters; data-enum `__init__` signatures match the runtime
  `#[new]`; `Json` maps to `dict[str, Any]`; and the duplicate `clear_*` registry stub is no longer
  emitted twice.
- **napi**: substitute binding-excluded types (e.g. `InternalDocument`) with `JsonValue` in the
  `.d.ts` host-interface signatures. Referencing a type that is never emitted produced an undefined
  TypeScript name; the runtime bridge marshals such values as JSON, so `JsonValue` is the faithful
  stand-in and `tsc --strict` is clean.
- **magnus**: apply the same excluded-type substitution (to `json_value`) in generated `.rbs`
  interfaces and skip re-declaring a bridge `clear_*` function that is already exposed as a registry
  function, so `rbs validate` no longer reports an undefined type or a duplicated method definition.

- **node/wasm**: require Node 22 or newer in generated npm package
  manifests, and keep Python package generation on Python 3.10 or newer.

- **e2e/dart**: resolve `config` JSON object helper types from compatible
  call overrides so generated tests use concrete helpers such as
  `createExtractionConfigFromJson`.

- **wasm**: filter cfg-gated struct fields with the WASM backend's active feature set so
  inactive fields are omitted and active fields are generated consistently across structs,
  constructors, accessors, and conversions.

- **r**: keep cfg-gated struct fields when the R backend's configured feature set enables
  them, and align R wrapper exports with the classes registered in `extendr_module!`.

- **scaffold**: let managed `.cargo/config.toml` render an explicit
  `rustc-wrapper`, and make the R Rust crate honor curated feature sets the
  same way as WASM by disabling core default features and declaring cfg
  passthrough features without enabling them by default.

- **r**: merge crate-level `extra_dependencies` into the generated R Rust
  crate so external DTO conversion impls can depend on sibling Rust crates
  such as `crawlberg`.

- **elixir**: render known generated public DTO fields in struct typespecs as
  their concrete module types instead of falling back to `map()`.

- **swift**: filter host Swift bindings with the same effective cfg feature set
  as the generated Rust bridge crate, including default cfg passthrough
  features.

- **swift**: wrap method-shim DTO returns for `Option<&T>` and `Vec<T>`, and
  pass `&Path` method parameters as borrowed paths instead of owned `PathBuf`s.

- **pyo3/magnus/wasm**: delegate generated binding defaults for defaultable
  DTOs to the core Rust `Default` impl so omitted nested config fields keep
  semantic core defaults.

- **extract**: support root-scoped external DTO source crates so host bindings
  can expand typed config graphs from sibling crates without exposing sibling
  functions or importing sibling language packages.

- **extract**: preserve explicit field `type_rust_path` values and reject
  same-name types from different crates, while keeping binding-excluded fields
  out of include-list expansion.

- **go/java**: avoid callback return local-name collisions in generated trait
  bridges when a method parameter is named `result`.

- **ffi**: keep cbindgen forward declarations for live binding DTOs when cfg-gated
  skipped duplicates leave older entries in Alef's excluded type-path map.

- **dart**: suppress ordinary trait-bridge lifecycle wrappers so FRB only sees the generated
  `{Trait}DartImpl` registration surface.

- **e2e**: emit typed single-call `json_object` inputs for Dart, Swift, and R so unified
  `extract(input, config)` fixtures pass their `ExtractInput` payload instead of defaulting it away.

- **pyo3**: include Pyo3-present cfg-gated fields in generated `.pyi` constructor stubs so native
  signatures and type stubs agree for typed nested configs such as `UrlExtractionConfig.crawl`.

- **dart**: normalize trailing whitespace in FRB-generated Dart files, including `*.freezed.dart`
  files that `dart format` leaves unchanged.

- **e2e**: prefer configured config DTO types when rendering Dart `config`
  JSON objects, preventing fallback helpers such as `createConfigFromJson`.

- **e2e**: include WASM nested DTO imports reached through `json_object`
  element types, such as per-input file configs nested under extract inputs.

- **elixir**: JSON-encode default-typed single DTO parameters before calling
  Rustler NIFs, matching the NIF boundary used for unified extract inputs.

## [0.29.4] - 2026-06-27

### Changed

- **tooling**: extend the `no-project-special-casing` pre-commit hook to reject the `xberg` and
  `crawlberg` downstream product names (case-insensitive, including camelCase and separator
  variants), and consolidate the brand allowlist so the `xberg-io` org namespace and the `xberg.io`
  domain stay permitted while `xberg-io/xberg` and bare `xberg` mentions are still caught. Neutralize
  the `xberg`-named Java/enum test fixtures to generic sample names.

### Fixed

- **e2e**: keep public Ruby and Elixir test calls on configured method names and
  resolve `$mock_url` placeholders inside typed JSON-array arguments across
  generated language e2e suites.

- **e2e**: resolve `$mock_url` placeholders for Ruby object arrays, Elixir typed
  object arguments, and Kotlin/PHP typed object setup while allowing Elixir e2e
  calls to target keyword-opts public facades.

- **e2e**: avoid Elixir typed-object variable collisions and align Kotlin typed
  object mock URL fallbacks with the generated mock-server harness.

- **node**: remove downstream internal DTO names from generated trait-bridge
  return-value comments.

- **ffi**: honor `[crates.ffi].exclude_types` when generating `cbindgen.toml`.
  Excluded Rust-only helper DTOs are now omitted from the header prelude forward
  declarations and emitted in `[export].exclude`, keeping C and cgo headers from
  leaking types that the FFI layer does not expose.

- **java/kotlin-android**: route configured trait-bridge lifecycle functions through the generated
  bridge APIs instead of also emitting ordinary FFI wrappers. This keeps raw Rust functions such as
  `register_document_extractor` from shadowing typed host interfaces (`IDocumentExtractor`,
  `IRenderer`) with dangling `DocumentExtractor`/`Renderer` parameter types or JSON-string JNI
  declarations.

## [0.29.3] - 2026-06-26

### Fixed

- **java/kotlin-android**: honor per-language `generate.async_wrappers = false` when emitting
  Java `CompletableFuture` helpers and Kotlin Android suspend convenience wrappers. This keeps
  bindings that want a single canonical method name from leaking extra `fooAsync` entrypoints while
  still preserving Rust functions that are themselves named `*_async`.

- **java (scaffold)**: derive the `maven-source-plugin` source include from the Maven group's first
  path segment instead of a hardcoded `dev/**`. After the `dev.kreuzberg` → `io.xberg` rebrand,
  generated sources moved to `io/<group>/…`, so the stale `dev/**` include matched nothing, the
  source jar came out empty, and Sonatype Central rejected the deployment with "Sources must be
  provided but not found in entries". The include now tracks the group (`io/**` for `io.xberg.*`).

## [0.29.2] - 2026-06-26

### Fixed

- **java**: read i32-returning FFM downcall results as `(int) (long)` instead of `(int)`. Since all
  integer FFM layouts are promoted to `JAVA_LONG` (for JBR Win64 Panama compatibility), the downcall
  handle returns `long`; casting the `invoke(...)` result straight to `(int)` forced an illegal
  `long → int` `asType` conversion that threw `WrongMethodTypeException` at the call boundary. This
  broke every byte-result method (e.g. `speech`, `fileContent`) and the trait-bridge
  register/unregister/clear lifecycle calls. The call sites now narrow via `(int) (long)`, matching
  the canonical pattern already used for `last_error_code`.

- **swift**: encode enum-typed struct field getters to match how the Swift side decodes each enum
  kind. Tagged enums (some variant carries data, e.g. `AssistantContent`) are serialized with
  `serde_json::to_string` of the source value and decoded via `JSONDecoder` — the discriminant-only
  bridge wrapper's `.to_string()` previously dropped the payload and returned an unquoted name (e.g.
  `Text`), which `JSONDecoder` rejected with "The given data was not valid JSON." Unit enums (all
  variants fieldless, e.g. `FinishReason`) keep returning their bare serde raw value via the wrapper's
  `.to_string()`, which Swift reconstructs with `Type(rawValue:)`; serializing those to JSON would
  emit a quoted string the rawValue init cannot parse.

- **elixir**: keep async NIF symbols suffixed internally while exposing async free functions under
  their original public names in the high-level Elixir facade. Generated modules now expose
  `extract/1` and `extract_batch/1` when the Rust API names are `extract` and `extract_batch`, while
  still delegating to `Native.extract_async/2` and `Native.extract_batch_async/2`.

- **magnus**: register suffixed async helper functions under their original public Ruby names. Ruby
  bindings now expose canonical methods such as `extract` and `extract_batch` even when the generated
  native helper functions are named `extract_async` and `extract_batch_async`; RBS stubs use the same
  public names.

### Removed

- **napi: stop generating the legacy `packages/typescript` wrapper package.** The napi backend no
  longer emits the `packages/typescript/src/index.ts` re-export barrel or its `bridges/*.ts` files;
  the native package (`crates/{lib}-node`, published with its own `index.d.ts`) is the canonical
  TypeScript surface, and `packages/node` is the modern package directory. `generate_public_api` for
  the napi backend now falls back to the default (no-op), and the existing orphan sweep removes any
  previously generated `packages/typescript/` tree on the next run. Version sync/checks and the e2e
  node package fallback now reference `packages/node` instead of the legacy `packages/typescript`.

### Added

- **e2e: support typed JSON-object arguments and `$mock_url` placeholders inside request DTOs.**
  Generated e2e tests now resolve non-array `json_object` argument types from per-argument metadata
  (`element_type`, and `go_type` for Go) before falling back to call-level `options_type`, so calls with
  separate request/config DTOs can be generated correctly. Structured JSON args can also embed
  `$mock_url`, which is replaced at test runtime with the fixture's mock-server URL.

- **e2e: accept fixture-level args, config, and route mocks in validation.**
  The embedded fixture schema now matches Alef's fixture model for per-fixture argument overrides,
  top-level `config`, `mock_response`, `setup`, `env`, and HTTP fixtures. Fixture loading mirrors
  top-level `config` into `input.config` before generation, and semantic missing-field validation now
  respects fixture-level `args`.

## [0.29.0] - 2026-06-26

### Fixed

- **pyo3 (Python): qualify builtin containers shadowed by a data-enum variant factory name.**
  A data enum with a `List` variant emits a `def list(...)` `@staticmethod` factory, which shadows the
  builtin `list` within the class body — so a sibling factory annotated `entries: list[MetadataEntry]`
  resolves to the factory and mypy rejects the `.pyi`
  (`Function ... is not valid as a type [valid-type]`). Factory annotations now qualify a shadowed
  builtin container (`list`/`dict`/`set`/`tuple`/`frozenset`/`type`) as `builtins.<name>[...]`, and the
  stub emits `import builtins` when referenced.

- **java: promote all integer FFM `FunctionDescriptor` layouts to `JAVA_LONG` for JBR Win64 Panama
  compat.** JetBrains Runtime's Panama linker casts every descriptor layout to `OfLong` internally, so
  any sub-64-bit integer layout (`JAVA_BYTE`/`JAVA_SHORT`/`JAVA_INT`) threw
  `ClassCastException: OfIntImpl cannot be cast to OfLong` at `NativeLib` class load and corrupted
  `TreeCursor` FFM calls. `java_ffi_type`, `service_api`, the enum-discriminant layout, the
  `LAST_ERROR_CODE` descriptor, and the visitor/trait-bridge/registration callback descriptors now
  emit `JAVA_LONG` for bool, 8/16/32-bit ints, and enum discriminants. `java_ffi_return_cast` emits
  compound narrowing casts (`(int)(long)`, `(short)(long)`, `(byte)(long)`) and the primitive-result
  templates no longer double-wrap them in parens. Generated `FunctionDescriptor`s now contain zero
  sub-64-bit integer layouts.

- **swift: add a runtime rpath to the generated `Package.swift` so the FFI dylib loads at runtime.**
  The `RustBridge` target emitted only `-L` (compile-time search). Because the FFI dylib's
  install_name is `@rpath/lib…dylib`, the consumer (and any test bundle linking the target) needs an
  `LC_RPATH` or `swift test` aborts with `dlopen … Library not loaded: @rpath/libhtml_to_markdown_ffi.dylib`.
  The manifest now derives the Cargo target dir absolutely from `#filePath` (CWD-independent, like the
  Zig/C e2e generators) and adds the rpath for both the release and debug profiles via the
  swiftc-native `-Xlinker -rpath -Xlinker <dir>` spelling (swiftc rejects `-Wl,-rpath,<dir>`). The e2e
  Swift package inherits the rpath transitively through this target.

- **extendr (R): skip per-variant factory constructors whose fields cannot cross the extendr input boundary.**
  A tagged data enum (e.g. `NodeContent`) generates a `_factory_<variant>` `#[extendr]` constructor per
  struct variant. When a variant field was a Named DTO (`grid: TableGrid`) or `Vec<DTO>`
  (`entries: Vec<MetadataEntry>`), the constructor took it *by value*, which the `#[extendr]` proc-macro
  cannot accept (`error[E0277]: T: TryFrom<&Robj> not satisfied`) — extendr derives `TryFrom<&Robj>` only
  for `&T`, never owned `T`, and has no R-list conversion for `Vec<DTO>`. `gen_extendr_enum_variant_constructors`
  and `extendr_enum_variant_constructor_registrations` now skip such variants (predicate
  `extendr_factory_param_is_constructible`); those variants remain constructible via the enum's `from_json`
  factory.

- **extendr (R): exclude methods with R-incompatible `Vec`/`Option<Vec>` params from `#[extendr]` impls.**
  Method filtering only dropped methods with bare-enum or bare owned-struct params; it missed
  `Vec<struct>`, `Vec<enum>`, `Vec<Vec<_>>`, and `Option<Vec<_>>` params. extendr generates no
  `TryFrom<&Robj>` for those, so the proc-macro failed downstream with
  `error[E0277]: T: TryFrom<&Robj> not satisfied` (e.g. `Vec<MetadataEntry>`). The two method-filter
  sites in `gen_bindings/mod.rs` now also apply the existing `is_extendr_native_incompatible` param
  check (already used for free functions), so such methods are omitted from the impl block.

- **php: per-variant constructor boxes `Box<T>` fields.** The flat-data-enum factory
  (`gen_flat_data_enum_variant_constructors`) emitted `field: field.clone().into()` for a variant
  field whose core type is `Box<T>`/`Option<Box<T>>` (Named `T`), which fails to compile (no
  `From<Binding> for Box<Core>`). It now wraps the converted value in `Box::new(...)` (or
  `.map(Box::new)` when optional), using the `VariantConstructor::boxed` flags — mirroring
  `flat_enum_binding_to_core_field_expr` and the shared `variant_field_init`.

- **magnus: per-variant constructors no longer collide with tagged-enum modules.** Tagged data enums
  are represented on the Ruby side as a `module <Name>` interface with per-variant `Data.define`
  classes, but the per-variant-constructor feature also emitted a Rust `module.define_class("<Name>")`
  with singleton factories. At load the `.so` defined the class first, so the pure-Ruby `module <Name>`
  raised `TypeError: <Name> is not a module` and the extension failed to load. Tagged data enums now
  skip the Rust factory class entirely — the class/singleton registration (`module_init`), the Rust
  `_factory_*` methods (avoids unused-method `-D warnings`), and the `.rbs` singleton stubs are all
  gated on `serde_tag.is_none()`. Construction for tagged enums goes through the variant `Data` classes
  (`<Name>Basic.new(...)`) and `from_hash`; non-tagged data enums keep their factory constructors.

### Added

- **Exception handling architecture guide and cross-language pattern documentation.** Added comprehensive
  `EXCEPTION_HANDLING.md` documenting exception/error handling patterns across all 15 language bindings
  (Python, Node.js, Ruby, PHP, Go, Java, C#, Elixir, WebAssembly, Dart, Swift, Kotlin Android, R, Zig, C FFI).
  Covers issue #147 (Python exception class identity), type identity preservation, error code standardization
  (1000+), and implementation checklists for new bindings. Ensures consistency across polyglot bindings.

- **CI resource optimization guide.** Added `CI_RESOURCE_OPTIMIZATION.md` documenting optimization strategies
  for large polyglot codebases (300+ grammars) on resource-constrained GitHub-hosted runners. Covers concurrency
  tuning (CLONE_CONCURRENCY=8, GENERATE_CONCURRENCY=2), sharding across parallel jobs, memory monitoring,
  and troubleshooting. Resolves exit-code 143 (SIGTERM) resource exhaustion issues.

- **PyO3 exception handling pattern documentation.** Enhanced `src/backends/pyo3/gen_bindings/errors.rs` with
  detailed cross-language exception handling patterns and core principle that exception class/type identity
  raised by native code must match the type exposed by public API. Reference for all polyglot backends.

### Trait-callback host returns accept the native binding object across the dynamic backends

  (pyo3, magnus, php, extendr).** Host-implementable trait callbacks already received native
  arguments (#142/#143), but the return value was still marshalled through a mapping/JSON path that
  rejected the binding's native result object even though the generated host interface advertised
  that type. Each dynamic backend's return path now tries the native object first
  (`extract::<Binding>()` / `TryConvert` / `FromZval` / `ExternalPtr` unwrap) and converts via
  `From<Binding> for Core`, falling back to the existing dict/array/hash/JSON path. The native path
  is gated on the binding→core conversion actually being generated (`convertible_types`), and extendr
  additionally gates on extendr-representability so non-representable rich types keep the JSON path. A
  shared `native_marshalled_struct_returns` classifier mirrors the param-side allowlist. On pyo3 the
  Protocol method also changes from `async def` to `def`, matching the `spawn_blocking` bridge that
  never awaited it. Resolves #153.

### Fixed

- **Per-variant constructors now box `Box<T>` fields.** When a data enum's struct variant has a
  field whose core type is `Box<T>`/`Option<Box<T>>` for a Named `T` (e.g. `CrawlEvent::Page {
  result: Box<CrawlPageResult> }`), the generated `_factory_<variant>` constructor emitted
  `result.into()`, which fails to compile because there is no `From<Binding> for Box<Core>`. The
  factory path now wraps the converted value (`Box::new(result.into())`, or
  `result.map(Into::into).map(Box::new)` for the optional case), mirroring the existing
  `From`/`Into` impl path (`conversions::binding_to_core::render`). The `is_boxed` flag is carried
  on `VariantConstructor` (parallel to `params`) and threaded into `variant_field_init`, so the
  pyo3, magnus, and extendr per-variant factories all box correctly.
- **pyo3 (Python): type stubs declare per-variant data-enum constructors.** The `.pyi` stub for a
  tagged data enum now emits a `@staticmethod` per data-carrying variant — `def circle(radius: float)
  -> Shape: ...` — between the tag attribute and the `__str__`/`__repr__` dunders, so type-checkers and
  IDE autocomplete see the `Shape.circle(...)` factories the runtime binding already exposed. The
  declared name is the public host name (`#[pyo3(name = "<snake>")]`), each param maps through the
  stub's `python_type` mapper, and the return type is the enum. Optional params — naturally optional
  fields and those promoted because they follow an optional one — render as `T | None = None`, matching
  the runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **magnus (Ruby): RBS stubs declare per-variant data-enum constructors.** The `.rbs` stub for a
  tagged data enum was an empty `class Shape ... end`; it now declares a singleton method per
  data-carrying variant — `def self.circle: (Float radius) -> Shape` — so RBS sees the
  `Shape.circle(...)` factories the runtime binding registers via `define_singleton_method`. The
  declared name is the bare snake_case host name, each param maps through the stub's `rbs_type`
  mapper, and the return type is the enum. Optional params — naturally optional fields and those
  promoted because they follow an optional one — render as the nilable `?T name` form, matching the
  runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **php: type stubs declare per-variant data-enum constructors.** The IDE/PHPStan stub for a tagged
  data enum (lowered to a flat PHP class) was an empty `final class Shape {}`; it now declares a
  static factory per data-carrying variant — `public static function circle(float $radius): Shape` —
  so PHPStan and IDEs see the `Shape::circle(...)` constructors the flat class exposes at runtime. The
  declared name is the camelCase host name (`to_php_name`), each param maps through the stub's
  `php_type` mapper (optional fields become `?T $x = null`), and the return type is the enum class.
  Variant selection is shared with the runtime binding via `collect_variant_constructors`, so unit /
  tuple / `binding_excluded` / sanitized-field variants and hand-written method collisions are skipped
  identically.
- **pyo3 (Python): enum-variant payloads accept the public dataclass/dict.** A data-enum
  per-variant constructor (e.g. `EmbeddingModelType.llm(...)`) now coerces a config-DTO payload the
  same way struct fields are coerced, so passing the public `LlmConfig` dataclass — or a plain
  `dict` — builds the variant instead of raising `TypeError: 'LlmConfig' object is not an instance
  of 'LlmConfig'`. Previously the generated factory demanded the compiled `#[pyclass]` instance
  while the package re-exported the pure-Python `@dataclass` for the same name, so the two never
  matched. A payload field whose type is a dataclass-backed config DTO — directly, or as a
  `list`/`dict`/`Optional` of one — is now generated as `&Bound<PyAny>` and routed through the
  module-level `__alef_coerce_dto` helpers (dataclass via `dataclasses.asdict` / dict / JSON-native
  → serde into the core type). Renamed fields round-trip with full fidelity: a per-DTO
  `__ALEF_WIRE_*` schema rewrites dataclass field names to serde wire names, honoring both
  `#[serde(rename)]` and `#[serde(rename_all)]` and recursing through nested DTOs, sequences, maps,
  and optionals — wire names are sourced from the same centralized naming transform the Python
  `_to_rust_*` converters use. Native re-exported return types stay compiled and are left untouched;
  the config-vs-native-return classification is shared with `__init__.py` import routing as a single
  source of truth (xberg #1165).

## [0.1.0 – 0.28.1] - 2026-04-09 – 2026-06-25

Early development history (592 releases through 0.28.1) has been trimmed to keep
this file small. The full per-version changelog is preserved in the git tags and
GitHub releases: <https://github.com/xberg-io/alef/releases>
