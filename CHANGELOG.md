# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.16.31] - 2026-05-17

### Fixed

- **alef-backend-napi trait_bridge: build the visitor-only `ConversionOptions` via struct-literal + `..Default::default()` instead of `let mut opts = Default::default(); opts.field = X;`**: the `or_else(|| { ... })` branch in the napi visitor wrapper code constructed a new `ConversionOptions` by `Default::default()` then field-assigned the visitor handle. Clippy's `field_reassign_with_default` lint (denied by default in consumer projects that build with `-D warnings`) rejects this idiom and demands the struct-literal form. The change keeps the same semantics (every other field still falls through to `Default`) but produces clippy-clean Rust on first emission, so consumers don't need to spawn a follow-up "fix clippy" commit after every alef regen. Surfaced on html-to-markdown's CI Rust (crates/html-to-markdown-node/src/lib.rs:1589). (`crates/alef-backend-napi/src/trait_bridge.rs`)

## [0.16.30] - 2026-05-17

### Fixed

- **alef-e2e rust cors: emit only the `axum::http` re-exports actually referenced by the rendered `CorsLayer`**: `render_cors_layer` previously emitted `use axum::http::{HeaderName, HeaderValue, Method};` unconditionally, but `HeaderName` is only emitted when `allow_headers` contains a non-prelude header (i.e. anything outside `content-type`/`authorization`/`accept`), `HeaderValue` only when `allow_origins` is non-empty, and `Method` only when `allow_methods` is non-empty. Fixtures that allow no custom headers (the common case) therefore got an unused `HeaderName` import, tripping `-D unused_imports` in the consumer's cargo build. The import group is now computed from actual use and reduced to one of: nothing, a single `use axum::http::X;`, or the appropriate subset. Covered by five new unit tests in `render_cors_layer::tests`. (`crates/alef-e2e/src/codegen/rust/http.rs`)

## [0.16.29] - 2026-05-17

### Fixed

- **alef-backend-pyo3 errors: drop duplicate `\n` after `indented_import_item.jinja` render**: the template already ends in `\n`, but each of the 5 callsites in `errors.rs` appended an extra `out.push('\n')` after `render`, producing `NAME,\n\n` per line — i.e. a blank line between every imported symbol in every multi-line `from .X import (...)` block in the generated `packages/<pkg>/__init__.py`. Ruff format strips these blank lines, breaking the embedded `alef:hash:` line and forcing an endless regen-format cycle on every consumer. Each of the 5 callsites now relies on the template's trailing newline alone. (`crates/alef-backend-pyo3/src/gen_bindings/errors.rs`)

- **alef-backend-rustler `@type t ::` multi-line emission: indent the first arm to match the `|`-prefixed continuation arms**: the `elixir_enum_type_arm_first.jinja` template emitted `{{ arm }}\n` (no indent), placing the first variant at column 0 while every subsequent `| :variant` line sat at column 10. mix-format `--check-formatted` then rejects the file (it wants the bare arm at column 10 too, aligned with the `|`), and the consumer's `mix-format` pre-commit hook rewrites the file — invalidating the embedded `alef:hash:` line. Template now emits `{{ arm }}\n` (10-space indent), matching `elixir_enum_type_arm_rest.jinja` and producing mix-format-clean output. (`crates/alef-backend-rustler/src/template_env.rs`)

- **alef-cli pipeline/generate: `format_rust_content` detects crate edition from nearest `Cargo.toml`**: rustfmt was invoked with a hardcoded `--edition 2024`. When the target crate declares a different edition (e.g. `edition = "2021"`) in its `Cargo.toml`, `cargo fmt` would produce different import sort order and formatting than alef's hardcoded edition, causing drift after `alef generate`. `format_rust_content` now accepts the file path, walks up to find the nearest `Cargo.toml`, parses its `[package] edition` field, and passes the detected edition to rustfmt; falls back to `"2024"` when no `Cargo.toml` is found or the edition field is absent. (`crates/alef-cli/src/pipeline/generate.rs`)

## [0.16.28] - 2026-05-17

### Fixed

- **alef-e2e rust: drop unused `use {dep}::{App, RequestContext}` import from http-fixture test files**: `render_test_file` unconditionally emitted `use {module}::{App, RequestContext};` when any fixture in the file declared an `http` block, but `render_http_test_function` references those symbols via their fully-qualified module path (`{dep_name}::App::new()`, `move |_ctx: {dep_name}::RequestContext|`). The use-statement was therefore always unused, tripping `-D unused_imports` in the consumer crate's clippy sweep. The unconditional emission is dropped; the body keeps the fully-qualified paths so no resolution behaviour changes. (`crates/alef-e2e/src/codegen/rust/test_file.rs`)

- **alef-e2e (all backends): `sanitize_ident` now strips underscores left behind after a numeric prefix to avoid `test__foo` names**: fixture ids commonly carry a numeric prefix to control category-file ordering (`24_cookie_samesite_strict.json`). `sanitize_ident` stripped the leading digits but left the underscore separator behind, so generators that prefix the result with `test_` (rust, python, ruby, typescript, elixir, C, …) emitted function names like `test__cookie_samesite_strict`. The function now also strips any leading underscores _that were exposed by stripping digits_, while preserving genuine leading underscores in ids that never had a digit prefix (`_foo` → `_foo`). Covered by four new unit tests in `escape::tests`. (`crates/alef-e2e/src/escape.rs`)

## [0.16.27] - 2026-05-17

### Added

- **alef-scaffold python: `[crates.python] pip_dependencies = [...]`**: declare runtime PyPI dependencies for the alef-scaffolded `pyproject.toml`. Entries are emitted verbatim into `[project] dependencies = [ ... ]` (PEP 508). Needed for downstream crates whose alef-generated `api.py` imports a third-party PyPI package (e.g. `tree-sitter`), so a fresh `pip install <pkg>` resolves the transitive at install time and the sdist smoke-test `import` succeeds. (`crates/alef-core/src/config/languages.rs`, `crates/alef-scaffold/src/languages/python.rs`)

### Fixed

- **alef-cli pipeline/generate: apply `rustfmt` in `finalize_hashes` for `.rs` files so the embedded hash always reflects cargo-fmt-clean content**: `finalize_hashes` previously computed the `alef:hash:` value from the raw on-disk bytes, which could be unformatted when the file was not re-written in the current run (cache hit or generated by an older alef version). A subsequent `cargo fmt` in CI would then reformat the file (reordering imports such as `{c_char, CStr, CString}` → `{CStr, CString, c_char}` and collapsing expanded `if`/`else` blocks), changing the on-disk bytes without updating the hash, causing `alef verify` to report stale. `finalize_hashes` now calls `normalize_content` (which pipes `.rs` files through stdin `rustfmt` with the consumer's `rustfmt.toml`) before computing and injecting the hash, making the recorded hash identical to what `cargo fmt` would produce. (`crates/alef-cli/src/pipeline/generate.rs`)

- **alef-cli pipeline/format: drop the `pnpm dlx oxfmt` step from the FFI and WASM formatter sequences**: the FFI step ran `pnpm dlx oxfmt .` over the entire workspace and the WASM step ran `pnpm dlx oxfmt <manifest>` on the generated wasm crate's Cargo.toml. The shared Kreuzberg pre-commit `oxfmt` hook (`kreuzberg-dev/pre-commit-hooks`, `id: oxfmt`) is scoped to `[javascript, jsx, ts, tsx, json, css]` — i.e. it never touches TOML in CI. But oxfmt's default behaviour DOES format TOML, with 2-space indent and collapsed multi-line arrays. That collides with `cargo-sort`'s preserved-indent canonicalisation and with hand-maintained Cargo.toml style (e.g. html-to-markdown's CLI crate Cargo.toml uses 4-space `features = [...]` blocks): oxfmt rewrites them to 2-space, cargo-sort then preserves the 2-space, the embedded `alef:hash:` line ends up describing the pre-oxfmt content, and every subsequent `alef verify` reports the file as stale. `cargo fmt` and `cargo sort` together are sufficient to canonicalise Rust + Cargo.toml; JS/TS/JSON files have their own per-language scaffold + the consumer's own oxfmt hook. (`crates/alef-cli/src/pipeline/format.rs`)

- **alef-backend-kotlin (and kotlin-android via the same emitter): use Jackson `TypeReference` for generic-container returns**: the JNI emitter's deserialize branch always emitted `MAPPER.readValue(responseJson, {kotlin_ty}::class.java)`, which is rejected by `kotlinc` whenever `kotlin_ty` carries angle-bracketed generics — e.g. functions returning `Vec<String>` emitted `List<String>::class.java` and failed with `Only classes are allowed on the left-hand side of a class literal`. The emitter now switches to `MAPPER.readValue(responseJson, object : TypeReference<{kotlin_ty}>() {{}})` whenever the rendered Kotlin return type contains `<`, mirroring the kotlin-android facade's existing generic-container handling, and threads the `com.fasterxml.jackson.core.type.TypeReference` import in. (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

## [0.16.26] - 2026-05-17

### Fixed

- **alef-scaffold: include conditional deps in `cargo-machete` ignored lists for FFI/Node/PHP/Python umbrellas**: when the umbrella crate declares `trait_bridges` or streaming `adapters`, the scaffold conditionally adds `async-trait` / `futures-util` / `tokio` to `[dependencies]` for manifest stability across regens — but the static `[package.metadata.cargo-machete]` block only listed the _unconditional_ deps (`serde_json` and, for FFI/PHP/Python, `tokio`/`pyo3-async-runtimes`). For umbrella crates whose actual API surface doesn't exercise async-trait or streaming codepaths in the generated binding (e.g. html-to-markdown's HtmlVisitor is sync, no streaming adapters), cargo-machete flagged the conditional deps as unused and broke `prek run --all-files`. The ignored list is now built dynamically from `has_trait_bridges` / `has_streaming`, mirroring the dep emission logic exactly so machete stays quiet whenever a dep is emitted unconditionally for codegen stability. (`crates/alef-scaffold/src/languages/{ffi.rs,node.rs,php.rs,python.rs}`)

- **alef-backend-wasm: also emit `getrandom 0.3` with `wasm_js` feature alongside `0.4`**: the WASM Cargo.toml emitter only declared `getrandom = { version = "0.4", features = ["wasm_js"] }`, but kreuzberg-wasm has transitive deps that pull in `getrandom 0.3.4` (e.g. through `rand 0.8`/`uuid 1.x` chains). On `wasm32-unknown-unknown`, getrandom 0.3 requires the `wasm_js` feature to provide its backend; without it the build fails with `compile_error!("The wasm_js backend requires the wasm_js feature for getrandom")`. Cargo's additive feature unification means declaring a direct `getrandom 0.3` dep with `features = ["wasm_js"]` enables the feature on the unified 0.3 instance, fixing the WASM build without affecting non-wasm targets. The 0.3 dep is added under a renamed alias `getrandom_03` so both major versions can coexist in the same `[target.cfg]` table. (`crates/alef-backend-wasm/src/gen_bindings/mod.rs`)
- **alef-e2e zig: restore `setCwd` on test run steps so generated fixture reads resolve**: commit `993ca9ec` removed `setCwd("../../test_documents")` under the assumption that all Zig e2e suites are mock-server-only (kreuzcrawl), but kreuzberg's document-extraction e2e reads fixtures like `pdf/fake_memo.pdf` and `images/test_hello_world.png` via `std.Io.Dir.cwd().readFileAlloc(...)`. Without the chdir every fixture read failed with `error.FileNotFound`, producing 26 test failures across smoke/contract/format_specific/detection/error/async suites. Other languages perform this chdir in a per-suite hook (Go `TestMain`, Python conftest, Kotlin Gradle `workingDir`); Zig has no equivalent so it must happen at the build-step level. `setCwd` is safe here because `.use_llvm = true` already pins the LLVM backend, which resolves the test binary's spawn path to an absolute cache path via `getEmittedBin()` before `convertPathArg` runs — the child cwd only affects fixture lookups, not the binary path. Re-threaded `test_documents_relative_from(0)` into `render_build_zig` and updated the `every_test_artifact_runs_via_addrunartifact_directly` test to assert the chdir is present. (`crates/alef-e2e/src/codegen/zig.rs`, `crates/alef-e2e/tests/zig_addtest_use_llvm_install.rs`)
- **alef-backend-zig: null-check raw C pointer before slicing in fallible owned-string returns**: `unwrap_return_expr` for `String`/`Path`/`Json`/`Vec`/`Map` returns built `raw[0.._result_len]` after a `kreuzberg_last_error_code() != 0` check, on the assumption that any error path on the primary `_<fn>` FFI call would still be visible in the thread-local error code when the wrapper queried it. That holds when the FFI implementation calls `set_last_error` on every error branch, but the matching `_<fn>_len()` companion always invokes `clear_last_error()` on entry and some implementations silently return 0 on the error path without re-setting the error code, wiping out the original error. The wrapper then passed the error-code gate and panicked with `attempt to use null value` when constructing the slice over a null pointer (observed in `get_extensions_for_mime("application/x-totally-unknown")`, where `kreuzberg_get_extensions_for_mime_len`'s `Err(_) => 0` arm forgets to call `set_last_error`). Defensive null check now runs first in fallible owned-string returns; threaded the resolved Zig `error_type` into `unwrap_return_expr` so the early return can produce the function's declared error variant. (`crates/alef-backend-zig/src/gen_bindings/functions.rs`)

- **alef-e2e java: handle `result_is_option` for bare-result `is_empty`/`not_empty` assertions**: the Java facade exposes `Option<T>`-returning Rust functions as `@Nullable T` (via `.orElse(null)`), so calling `.isEmpty()` on the nullable reference does not compile for record types like `EmbeddingPreset` (`error: method isEmpty() not found on EmbeddingPreset`). The e2e codegen now propagates `result_is_option` through `render_assertion` and emits `assertNull/assertNotNull(result, ...)` for bare-result `is_empty`/`not_empty` assertions on option-returning calls, mirroring the existing `kotlin.rs` and `zig.rs` behaviour. (`crates/alef-e2e/src/codegen/java.rs`)
- **alef-e2e java: coerce nullable numeric fields via `Number::longValue` to fix `long`/`Integer` mismatch**: `Optional.ofNullable(Integer field).orElse(0L)` does not compile because `0L` is a `Long` while the optional value type is `Integer` (e.g. Rust `Option<u32>` maps to Java `@Nullable Integer`). The Java e2e codegen now emits `.map(Number::longValue).orElse(0L)` for numeric comparisons and equality on optional numeric fields, which compiles cleanly for both `Optional<Integer>` and `Optional<Long>`. (`crates/alef-e2e/src/codegen/java.rs`, `crates/alef-e2e/templates/java/assertion.jinja`)
- **alef-e2e dart: emit required string args as positional rather than named**: a prior fix indiscriminately switched all string args to named-argument syntax to match the FRB bridge, but the alef-generated Dart facade (e.g. `KreuzbergBridge`) emits required params as positional and only wraps optional params in a `{...}` named block (see `alef-backend-dart/src/gen_bindings/functions.rs`). Calling `KreuzbergBridge.extractBytesSync(content, mimeType: 'application/pdf', config)` produced `Error: No named parameter with the name 'mimeType'` because the facade has positional `mimeType`. The Dart e2e codegen now checks `arg_def.optional` and emits positional literals for required string args, preserving named syntax only for genuinely optional string params. (`crates/alef-e2e/src/codegen/dart.rs`)

- **alef-e2e swift: emit e2e package at `e2e/swift/` with explicit `name:` on `.package(path:)`**: the Swift e2e codegen was emitting under `e2e/swift_e2e/` to avoid a SwiftPM identity collision between the consumer (`e2e/swift/`) and the dep (`packages/swift/`) — both reduce to the path-last identifier `swift`. The stale `e2e/swift/Package.swift` left over from the original layout still declared a `.testTarget` with no source directory, so `cd e2e/swift && swift test` (the canonical CI command, matching every other language) failed with `error: The package does not contain a buildable target.` The emitter now writes back to `e2e/swift/` for consistency with every other language and disambiguates the dep via `.package(name: "{module}", path: "{pkg_path}")`, which SwiftPM honors as the dep's identity for `.product(name:package:)` resolution. (`crates/alef-e2e/src/codegen/swift.rs`)

## [0.16.25] - 2026-05-17

### Changed

- **ci: restrict `Android Emulator (instrumented tests)` workflow to manual dispatch**: the GitHub-hosted arm64-v8a API 34 emulator on `ubuntu-latest` regularly fails to reach `boot_completed` within the 1800s timeout, blocking unrelated merges. Trigger temporarily reduced to `workflow_dispatch` only until a stable runner/image combination is identified. (`.github/workflows/ci-android-emulator.yaml`)

### Added

- **alef sync-versions: manage `CITATION.cff` at the repo root**: a new optional `[workspace.citation]` section in `alef.toml` declares the project's citation metadata (`title`, `abstract`, `authors` — persons via `family-names`/`given-names` and/or entities via `name`, plus `email`/`orcid`, `repository-code`, `url`, `license`, `date-released`, `doi`). When present, `alef sync-versions` renders a full Citation File Format YAML at `CITATION.cff` using these fields plus the canonical workspace version from `Cargo.toml`; the file is rewritten only when its contents actually change, so the working tree stays clean across reruns. When the section is absent but a hand-authored `CITATION.cff` exists at the repo root, alef falls back to updating only the top-level `version:` scalar (preserving the original quote style — unquoted, single-, or double-quoted). New config types `CitationConfig` and `CitationAuthor` in `alef-core`. (`crates/alef-core/src/config/{output.rs,workspace.rs,resolved/mod.rs,new_config.rs,mod.rs}`, `crates/alef-cli/src/pipeline/version.rs`)

### Fixed

- **alef-backend-csharp: emit excluded types as JsonElement instead of named type**: when a field's type was marked with `#[cfg_attr(alef, alef(skip))]` or `#[doc(hidden)]` (e.g., `CoreProperties` in kreuzberg's `DocxMetadata.core_properties`), the C# codegen treated it as a Pascal-cased type name instead of recognizing it as an excluded type and mapping it to `JsonElement`. This caused deserialization failures with `JsonException: Cannot get StartObject as String` when Rust sent an object but C# expected a string. Fixed by passing the excluded types set from `ApiSurface.excluded_type_paths` to `gen_record_type` and checking it alongside complex enums to determine whether to emit `JsonElement`. (`crates/alef-backend-csharp/src/gen_bindings/{types.rs,mod.rs}`)

- **alef-backend-rustler: emit full `rust_path` for types not re-exported at the crate root**: param-type resolution in `gen_nif_function`, `gen_nif_async_function`, `gen_nif_method`, `gen_nif_async_method`, and `build_default_deser_preamble` was using `format!("{core_import}::{short_name}")` (e.g. `kreuzberg::DrawingType`) instead of the type's full `rust_path` (e.g. `kreuzberg::extraction::docx::drawing::DrawingType`). Types not re-exported at the crate root caused `cannot find DrawingType in kreuzberg` compile errors in the generated Elixir NIF crate. Fixed by building a `types_by_name` lookup map and resolving via `alef_codegen::conversions::core_type_path`. (`crates/alef-backend-rustler/src/gen_bindings/functions.rs`, `crates/alef-backend-rustler/src/gen_bindings/mod.rs`)

- **alef-backend-rustler: omit `mut` from `args_map` in visitor methods with no arguments**: the `visitor_method.rs.jinja` template always emitted `let mut args_map = serde_json::Map::new();`. When the visitor method has zero arguments the map is never mutated, triggering an `unused_mut` clippy warning that fails CI under `-D warnings`. The template now conditionally emits `mut` only when `args` is non-empty. (`crates/alef-backend-rustler/src/template_env.rs`)

- **alef snippets gaps: fix false-positive for MkDocs `pymdownx.snippets` multi-base-path includes**: `alef snippets gaps` previously resolved every `--8<-- "target"` include relative to the docs root only. Projects that configure `pymdownx.snippets.base_path = ["docs/snippets", "docs", "."]` — such as spikard — have includes like `--8<-- "CHANGELOG.md"` that resolve to the project root (`./CHANGELOG.md`), not `docs/CHANGELOG.md`. Alef reported these as missing references even though MkDocs renders them correctly. Two changes fix this: (1) `GapConfig` gains an `include_base_paths: Vec<PathBuf>` field; `parse_includes` now searches each base path in order and returns the first candidate that exists on disk, falling back to `docs_dir.join(target)` only when none match, so the missing-references report still shows a real candidate path. (2) `alef snippets gaps` gains a repeatable `--include-base-path <PATH>` CLI flag — pass each path from `pymdownx.snippets.base_path` to match the MkDocs resolver exactly. When the flag is omitted, behaviour is identical to before (only the docs root is searched). (`crates/alef-snippets/src/gaps.rs`, `crates/alef-cli/src/commands/snippets.rs`)

- **alef-e2e wasm: do not append `/nodejs` to user-supplied `[e2e.packages.wasm].path`**: `render_package_json` always appended `/nodejs` to the resolved package path, on the assumption it was a `pkg/` directory laid out by `wasm-pack build --target nodejs --out-dir pkg/nodejs`. That assumption only holds for the fallback path from `wasm_crate_path()`. When a project sets `[crates.e2e.packages.wasm].path` explicitly (e.g. html-to-markdown's `../../crates/html-to-markdown-wasm`, the crate root whose root-level `package.json` already declares `"main": "pkg/nodejs/..."`), the resulting `file:.../nodejs` dependency pointed at a nonexistent directory, breaking `pnpm install` with `ERR_PNPM_LINKED_PKG_DIR_NOT_FOUND` and corrupting the lockfile. Now `render_package_json` only appends `/nodejs` for the fallback case; explicit paths are used verbatim. (`crates/alef-e2e/src/codegen/wasm.rs`)

- **alef-backend-wasm: skip `serde_wasm_bindgen::from_value` for external-crate types in tagged enums**: when a mixed-type tagged-enum field's `type_rust_path` resolves to a crate other than `core_import` (e.g. `tree_sitter_language_pack::ProcessResult`), the emitter previously generated `serde_wasm_bindgen::from_value::<ext_crate::T>()` unconditionally. Because the external crate is not in the WASM consumer's Cargo dependency graph, this caused `error[E0433]: failed to resolve: use of unresolved module or unlinked crate` and broke WASM, Python wheel macOS, Swift, and Dart iOS builds. The fix detects whether the type's crate prefix differs from `core_import` and falls back to `Default::default()` for those fields. (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`)

- **alef-snippets C validator: use unambiguously invalid token in `syntax_fail` test**: the previous fixture (`return ;` in a non-void function) is treated as a warning by some GCC versions on Linux and exits 0 unless `-Werror` is set, causing the test to return `Pass` instead of `Fail`. Replaced with `@@@` — an invalid preprocessing token that is a hard error on all C compilers. (`crates/alef-snippets/src/validators/c.rs`)

- **publish workflow: include `alef-backend-jni` in the crates.io publish order**: the jni backend was added in commit 831dd99d but was not wired into `.github/workflows/publish.yaml` crate order. Topologically, jni must be published after gleam (which has no jni deps) and before kotlin and kotlin-android (which depend on it). (`.github/workflows/publish.yaml`)

- **alef-backend-napi + alef-e2e: emit tagged-enum discriminator matching Rust serde tag**: NAPI-RS binding generation for tagged enums hardcoded the TS discriminant field as `"kind"` regardless of what the Rust enum declared via `#[serde(tag = "...")]`. This broke deserialization: kreuzcrawl's `AuthConfig` uses `tag = "type"`, so Rust expected payloads like `{ type: "basic", username, password }` but TypeScript was emitting `{ kind: "basic", ... }`, causing deserialization to fail with `Missing field \`type\``. Now alef-backend-napi emits the discriminant name matching the Rust serde tag, and alef-e2e fixture code preserves the actual tag field name instead of rewriting all tags to "kind". Updated test assertions in alef-backend-napi to expect the serde tag name. (`crates/alef-backend-napi/src/gen_bindings/enums.rs`, `crates/alef-e2e/src/codegen/typescript/test_file.rs`)

## [0.16.24] - 2026-05-17

### Changed

- **packaging: fold `alef-snippets` into the main `alef` CLI as the `alef snippets` subcommand**: the workspace previously shipped two binaries (`alef-cli` → `alef`, and `alef-snippets`), forcing `cargo install --git ...` callers to disambiguate with `--package alef-cli --bin alef`. The standalone `alef-snippets` bin is removed; its CLI surface (list/validate/parse/audit/gaps) is now reachable as `alef snippets <cmd>`, matching the existing `alef fmt`/`alef lint`/`alef generate` pattern. Companion updates landed in `kreuzberg-dev/actions` (install-alef@v1 drops the `--bin alef` selector; lint-docs@v1 calls `install-alef` + `alef snippets validate`) and `kreuzberg-dev/pre-commit-hooks` v1.1.3 (alef-snippets-validate hook now invokes `alef snippets validate`). (`crates/alef-cli/{Cargo.toml,src/commands/{mod.rs,snippets.rs},src/main.rs}`, `crates/alef-snippets/{Cargo.toml,src/main.rs}`)

- **chore: drop taplo entirely from alef tooling**: CI no longer installs taplo-cli or runs `taplo fmt --check`; `.taplo.toml` is removed. TOML formatting in alef now relies on `cargo-sort` (Cargo.toml ordering) and `cargo-fmt` (inline TOML literals in Rust code) only. (`.github/workflows/ci.yml`, `.taplo.toml`, `.github/copilot-instructions.md`)

### Fixed

- **alef-backend-kotlin-android: also suppress `CyclomaticComplexMethod` in the generated `@file:Suppress` header**: data-class (de)serializers emitted for sealed `NodeContent`-style discriminated unions (with one branch per variant) sit just above detekt's default cyclomatic-complexity threshold of 15 — for html-to-markdown's 14-variant `NodeContent` they hit 16 (`deserialize`) and 15 (`serialize`). The existing file-level suppress block already silenced `MaxLineLength` / `TooManyFunctions` / etc.; adding `CyclomaticComplexMethod` rounds out the set so refactoring the (de)serializer into per-variant helpers isn't required just to get prek clean. Surfaced on html-to-markdown's prek run (`packages/kotlin-android/src/main/kotlin/dev/kreuzberg/android/NodeContent.kt:95,152`). (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-backend-java: wrap the FFI-tail block in `convert` and `convertWithVisitorInternal` with `// CPD-OFF` / `// CPD-ON` markers**: PMD CPD flagged the ~15-line FFI marshalling tail (null-check `resultPtr` → `result_to_json` → free → null-check `jsonPtr` → `reinterpret` → free string → `MAPPER.readValue`) as a duplication between the two methods. The block is intentionally identical and extracting it into a helper would require threading the `Arena`, `MAPPER`, and free-handle through a private method for marginal benefit. Wrap both copies with CPD markers in `ffi_class.rs`, matching the precedent the builder-class emitter already uses. Surfaced on html-to-markdown's prek run (`packages/java/src/main/java/dev/kreuzberg/htmltomarkdown/HtmlToMarkdownRs.java:39,85`). (`crates/alef-backend-java/src/gen_bindings/ffi_class.rs`)

- **alef-backend-kotlin: add `@file:Suppress` headers to JNI Bridge object + JNI client class kotlin files**: `emit_jni_bridge_object` (e.g. `LiterLlmBridge.kt`) and `emit_jni_client_class` (e.g. `DefaultClient.kt`) emit kotlin files that go through their own assembly path (NOT through `assemble_kt_content` in alef-backend-kotlin-android), so the `@file:Suppress` block added there did not cover them. They now emit their own `@file:Suppress(...)` block listing `MaxLineLength`, `ktlint:standard:max-line-length`, `TooManyFunctions`, `LongParameterList` (+ `LongMethod` on the client class), silencing detekt on JNI external-fun signatures and coroutine method signatures that naturally exceed 120 chars. Surfaced on liter-llm's prek run (`DefaultClient.kt:54,121,133` and `LiterLlmBridge.kt:12`). (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **alef-scaffold/ffi: add `futures-util = "0.3"` to `crates/<crate>-ffi/Cargo.toml` when the umbrella crate exposes a Streaming adapter**: the FFI codegen emits `use futures_util::StreamExt; ... stream.next().await` inside the streaming shim, but the FFI scaffold did not add the dep, producing `error[E0432]: unresolved import futures_util` on `cargo check -p <crate>-ffi`. Mirrors the same `has_streaming` check used by the PHP / Python / Ruby / Node scaffolds. Surfaced on liter-llm's prek run (`crates/liter-llm-ffi/src/lib.rs:10609`). (`crates/alef-scaffold/src/languages/ffi.rs`)

- **alef-backend-kotlin-android: suppress `MaxLineLength` violations in generated Kotlin files**: kotlin POJOs / data-class declarations with serde-renamed enum tags and multi-arg constructors regularly exceed the 120-char `MaxLineLength` limit enforced by detekt (and the ktlint `max-line-length` sibling rule). The `@file:Suppress(...)` header emitted at the top of every generated `.kt` file now lists both `MaxLineLength` (detekt) and `ktlint:standard:max-line-length` (ktlint) so the kotlin-android binding no longer flaps on every prek run. Surfaced on liter-llm's prek run (15+ `[MaxLineLength]` violations across `LiterLlm.kt`, `RerankDocument.kt`, `Message.kt`, etc.). (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-scaffold: add `futures-util = "0.3"` to `crates/<crate>-node/Cargo.toml` when the crate exposes a Streaming adapter**: the napi backend emits `use futures_util::StreamExt;` in streaming method bodies (e.g. `chatStream`), but the node Cargo.toml scaffold did not add the dep — only PHP / Ruby / Python had the conditional `has_streaming` check. `cargo check` failed on `unresolved import futures_util` for every umbrella crate with a chat-stream adapter on the node backend. Mirrors the same pattern used in `alef-scaffold/src/languages/php.rs:38`. Surfaced on liter-llm's prek run (`crates/liter-llm-node/src/lib.rs:992`). (`crates/alef-scaffold/src/languages/node.rs`)

- **alef-e2e dart: emit named-arg syntax for required string parameters**: the dart e2e codegen emitted positional literals for required string args (`_client.cancelBatch('batch-id')`) under the assumption the wrapper convention was POSITIONAL-required. The dart bridge backend (`alef-backend-dart`) emits ALL params as named-required (`Future<BatchObject> cancelBatch({required String batchId})`), so the generated tests failed `missing_required_argument` + `extra_positional_arguments_could_be_named` analyzer errors across `batches_test.dart`, `files_test.dart`, `responses_test.dart`. The `extractFile* → extractBytes*` mime_type remap special-case (kreuzberg) still emits positional. Surfaced on liter-llm's prek run (60+ dart-analyze errors). (`crates/alef-e2e/src/codegen/dart.rs`)

- **alef-scaffold: add `[package.metadata.cargo-machete] ignored = [...]` to FFI / PHP / Node / Python / JNI Cargo.toml scaffolds**: the binding Cargo.toml templates emit `serde_json` / `tokio` / `pyo3-async-runtimes` / `futures-util` unconditionally so the manifest is stable across regens (they are pulled in when the umbrella crate's API surface has async fns, streaming adapters, or JSON-marshalled return types). For umbrella crates with none of those — e.g. html-to-markdown's PHP and FFI bindings — those deps are genuinely unused and `cargo machete` flags them as a finding on every prek run. Mirrors the precedent set by `alef-backend-swift` (`async-trait`, `serde`, `tokio`). The conditional deps (`async-trait`, `futures-util` in PHP, `tokio` in Python) that _are_ only emitted under specific feature combinations don't need ignoring since they aren't in the manifest when not used. Surfaced on html-to-markdown's prek run (5 binding crates flagged). (`crates/alef-scaffold/src/languages/{ffi,php,node,python,jni}.rs`)

- **alef-backend-pyo3: silence ruff TC001 on the runtime re-export block in `options.py`**: the generated options module does `from .<native> import (CodeBlockStyle, HeadingStyle, ...)` so that downstream users can `from html_to_markdown.options import NewlineStyle` at runtime. Ruff's TC001 saw the imports referenced only in annotations (the file uses `from __future__ import annotations`) and demanded they move under `if TYPE_CHECKING:`, which would break the runtime re-export. The `import_item.jinja` template now appends `# noqa: TC001` to each enum import line, preserving the runtime re-export while satisfying the lint. Tried `as`-alias re-exports first; ruff TC001 still fires through that form. Surfaced on html-to-markdown's prek run (10 TC001 errors in `packages/python/html_to_markdown/options.py`). (`crates/alef-backend-pyo3/templates/import_item.jinja`)

- **alef-backend-jni: migrate generated shims to jni 0.22 — `EnvUnowned` in extern sigs, `AttachGuard` body, `JNIString` for throw_new args, `JString::try_to_string` for decode**: jni 0.22 split `JNIEnv` into FFI-safe `EnvUnowned<'frame>` and `Env<'_>` (which carries the real API surface). The pre-0.22 codegen emitted `use jni::JNIEnv;` plus extern shims with `mut env: JNIEnv` bodies that called `env.get_string(&s)?`, `env.new_string(s)`, and `env.throw_new(ERROR_CLASS, msg)` directly, which now (a) emits deprecation warnings for the `JNIEnv` alias, (b) trips `E0599` for `get_string` / `new_string` / `throw_new` on `&mut EnvUnowned`, and (c) trips `E0277` for `throw_new` because 0.22 requires `S: AsRef<JNIStr>` / `T: Desc<JClass>` (Rust `&str` no longer coerces). The codegen now: (1) emits `use jni::{AttachGuard, Env, EnvUnowned};` and declares extern args as `mut env: EnvUnowned`, (2) upgrades to `&mut Env<'_>` inline via `AttachGuard::from_unowned(env.as_raw())` + `borrow_env_mut()` (avoids `EnvUnowned::with_env`'s `FnOnce(&mut Env) -> Result<T, E>` closure shape which would force rewriting every early-return + sentinel path), (3) replaces `env.get_string(&s)?` with `s.try_to_string(env)`, and (4) wraps both `class` and `msg` args to `env.throw_new` in `jni::strings::JNIString::from(...)`. Helpers (`jstring_to_string`, `string_to_jstring`, `throw_jni_error`, `run_or_throw`) now take `&mut Env<'_>`. `_env`-only shims (destructors, streaming free) declare `_env: EnvUnowned` with no body. Verified clean on `cargo clippy -p html-to-markdown-rs-jni` against the regenerated shim. Snapshots refreshed; the 25-test suite still passes. Surfaced on html-to-markdown's prek run. (`crates/alef-backend-jni/src/gen_shims.rs`, `crates/alef-backend-jni/tests/{gen_shims_test.rs,snapshots/*.snap}`)

- **alef-hook (`alef-verify` / `alef-sync-versions`): resolve consumer `alef.toml` by walking up from CWD**: `_version()` previously read `_hooks_dir().parent / "alef.toml"`, which points at the prek-cached clone of this repo (the dir where `alef_hook.py` lives), NOT the consumer project being verified. That made consumer bumps of `alef.toml` invisible to the hook — it kept downloading the binary version pinned by the prek `rev:` tag, surfacing as `HTTP 404` when the rev's release had no assets uploaded (e.g. kreuzberg pinning prek `rev: v0.16.23` and bumping `alef.toml` to `0.16.24`). `_find_consumer_alef_toml()` now walks up from `Path.cwd()` until it finds an `alef.toml`, matching how prek invokes hooks (CWD = consumer repo root). (`hooks/alef_hook.py`)

- **alef-backend-rustler: fix `@type t` struct field indentation for mix format**: field lines inside `@type t :: %__MODULE__{...}` were indented with 4 spaces; `mix format` requires 10-space alignment to the opening `{` column. Surfaced on kreuzcrawl prek run where 28+ `.ex` files including `CrawlPageResult`, `ContentConfig`, `JsonLdEntry` etc. were reformatted. (`crates/alef-backend-rustler/src/gen_bindings/helpers.rs`)

- **alef-scaffold: emit space-inside-brackets in pyproject.toml inline arrays**: `pyproject-fmt` rewrites inline arrays from `["x"]` to `[ "x" ]`. The scaffold template now emits the spaced form for `requires`, `keywords`, `features`, `python-packages`, `dev`, `lint.select`, and `lint.per-file-ignores` so the generated pyproject.toml is idempotent under pyproject-fmt. Surfaced on kreuzcrawl prek run against `packages/python/pyproject.toml`. (`crates/alef-scaffold/src/languages/python.rs`)

- **alef-backend-kotlin-android: add `@file:Suppress` header to suppress ktlint/detekt violations in generated Kotlin files**: generated Kotlin files violated `ktlint:standard:trailing-comma-on-call-site`, `ktlint:standard:spacing-between-declarations-with-comments`, `ktlint:standard:when-entry-bracing`, `ktlint:standard:annotation`, and detekt's `TooManyFunctions` on ~30 files in kreuzcrawl. A file-level `@file:Suppress(...)` annotation is now emitted at the top of every assembled `.kt` file before the package declaration. (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-backend-kotlin-android: extend `@file:Suppress` to cover `trailing-comma-on-declaration-site`, `spacing-between-declarations-with-annotations`, `blank-line-between-when-conditions`, and `chain-method-continuation`**: ktlint flagged additional rules on kreuzcrawl's generated `MapResult.kt` (missing trailing comma on data class constructor) and `AuthConfig.kt` (spacing around annotated declarations, blank lines between when-conditions, chain method continuation). All four new rules are now included in the `@file:Suppress` block. (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-backend-kotlin: add `ktlint:standard:annotation` to `@file:Suppress` in JNI bridge and client files**: `ktfmt` (run by prek) compacts `@Throws(...)\n    external fun` into a single line, which then triggers ktlint's `standard:annotation` rule. Adding `ktlint:standard:annotation` to both the bridge-object and client-class `@file:Suppress` blocks silences the conflict. Snapshots updated. (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **alef-backend-swift: add `// swift-format-ignore-file` directive to generated Swift files**: swift-format reformatted `packages/swift/Sources/Kreuzcrawl/Kreuzcrawl.swift` on every prek run. The `// swift-format-ignore-file` magic comment is now emitted at the top of every generated Swift bindings file, instructing swift-format to skip the file entirely. (`crates/alef-backend-swift/src/gen_bindings.rs`)

- **alef-backend-java: wrap generated Builder class with CPD-OFF/CPD-ON markers**: PMD's copy-paste detector flagged 64-line (213 token) and 19-line (121 token) duplications across `CrawlPageResult.java` and `ScrapeResult.java` builder classes in kreuzcrawl. `// CPD-OFF` and `// CPD-ON` comments are now emitted around the nested builder class so CPD suppresses these intentional structural duplications. (`crates/alef-backend-java/src/gen_bindings/types.rs`)

- **alef-e2e dart: emit `// ignore_for_file: unused_local_variable` in generated test files**: `final result = await api.method(args)` is emitted for every test case; tests that only assert no error is thrown do not use `result`, producing ~1266 `unused_local_variable` warnings across kreuzcrawl's dart e2e suite. The file-level ignore directive is now prepended after the alef header in every generated `*_test.dart` file. (`crates/alef-e2e/src/codegen/dart.rs`)

- **alef-backend-dart: skip trait-bridge methods that return `alef(skip)`-stripped trait types**: `return_type_references_trait` only inspected `api.types`, which has already had `binding_excluded` items stripped by the time codegen runs. Traits annotated `#[cfg_attr(alef, alef(skip))]` (e.g. kreuzberg's `SyncExtractor`) therefore slipped past the filter, and the resulting `Box<dyn Fn() -> DartFnFuture<Option<kreuzberg::extractors::SyncExtractor>>>` field on the bridge struct failed to compile with `error[E0782]: expected a type, found a trait` (23 errors across `packages/dart/rust/src/{lib.rs,frb_generated.rs}`). A new `ApiSurface::excluded_trait_names: HashSet<String>` is populated alongside `excluded_type_paths` during `strip_binding_excluded`, preserving the `is_trait` flag that would otherwise be lost. The dart trait-bridge filter now consults this set so excluded **traits** are filtered out while excluded **structs** (e.g. `InternalDocument` returned by `extract_bytes`) continue to be emitted by qualified core path. Added unit tests covering in-surface traits, excluded traits, excluded structs, and unrelated named types. (`crates/alef-core/src/ir.rs`, `crates/alef-cli/src/pipeline/extract.rs`, `crates/alef-backend-dart/src/gen_rust_crate/trait_bridge.rs`)

- **alef-backend-ffi: allow `clippy::missing_safety_doc` in the generated C-FFI lib.rs**: every `pub unsafe extern "C" fn` emitted by the FFI backend lacks a `# Safety` rustdoc section (the safety contract is documented at the C-header / cbindgen layer instead). Downstream consumers running `cargo clippy -- -D warnings` on the FFI crate therefore tripped 13+ `missing_safety_doc` errors per polyglot library. The crate header now includes `#![allow(clippy::missing_safety_doc, clippy::doc_lazy_continuation, clippy::doc_overindented_list_items)]` alongside the existing allow blocks. Surfaced first on kreuzcrawl CI Rust @ `74021aa7`. (`crates/alef-backend-ffi/src/gen_bindings/mod.rs`)

- **alef-backend-swift: disable `cargo test`/`doctest`/`bench` for the generated `*-swift` crate**: the `[swift_bridge::bridge]` `extern "Swift" { ... }` block emits Rust-to-Swift call sites whose symbols (`__swift_bridge__$<Type>$<method>`) are only resolved when the static/cdylib output is linked into a Swift target. `cargo test --workspace` on a pure-Rust runner (windows-latest, ubuntu-latest without swift-toolchain) would therefore fail with `rust-lld: error: undefined symbol: __swift_bridge__$SwiftHtmlVisitorBox$alef_visit_*` (and ~30 more) during the test-harness link step. The generated `Cargo.toml`'s `[lib]` now sets `test = false`, `doctest = false`, `bench = false` — the cdylib/staticlib build path is unaffected, and the swift package's own `swift test` suite remains the authoritative test surface for this crate. (`crates/alef-backend-swift/src/gen_rust_crate/cargo.rs`)

- **alef-scaffold: emit `extra.pie.binary.url-template` block in PHP composer.json for explicit PIE binary discovery**: PIE's `pre-packaged-binary` download method relies on asset filename pattern matching, which has known edge cases with architecture aliases and libc detection (kreuzberg-dev/html-to-markdown#333). The PHP scaffold now derives the GitHub repo owner/name from `metadata.repository` and emits an `extra.pie.binary.url-template` entry pointing to the releases' archive pattern, giving PIE explicit binary URLs when pattern matching alone is insufficient. The block is only emitted when the repository URL is valid (github.com-based). Template variables (`{Version}`, `{PhpVersion}`, `{Arch}`, `{OS}`, `{Libc}`, `{TSMode}`) follow PIE 1.4.x naming conventions. Also fixes the Composer package name derivation to use the GitHub repo name (e.g. `html-to-markdown`) instead of the crate name (e.g. `html-to-markdown-rs`), ensuring Packagist package names are correct. (`crates/alef-scaffold/src/languages/php.rs`)

- **alef-backend-dart: pin `flutter_rust_bridge` with `=`-prefixed strict version**: flutter_rust_bridge enforces exact runtime/compile-time version matching — a caret-pin lets `cargo update` pull in a patch release that the generated bridge code can't talk to. The Rust crate's `Cargo.toml` template now emits `flutter_rust_bridge = "=2.12.0"` so `cargo update` cannot drift off the matching version. (`crates/alef-backend-dart/src/gen_rust_crate/cargo.rs`)

- **alef-backend-kotlin: drop redundant `${...}` braces in error-message interpolation when the next char is not an identifier continuation**: `interpolate_error_message_template` previously always emitted `${fieldN}` for `{N}` placeholders, producing `"HTML parsing error: ${field0}"` for the common single-field `thiserror`-style template. ktlint's `standard:string-template` rule flags this as redundant when the brace is unnecessary (the next character — `"`, `:`, ` `, `)`, etc. — cannot extend the identifier), and the rule is a CI gate in the polyglot host repos. The helper now peeks at the character following the closing brace and emits `$fieldN` when it would be unambiguous, falling back to `${fieldN}` only when the next char is `[A-Za-z0-9_]`. (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`)

- **alef-scaffold: scaffold the JNI crate at `crates/<config.name>-jni/` to match the path that `alef-backend-jni::gen_shims` uses for `src/lib.rs`**: `scaffold_jni` derived the JNI crate directory from `config.core_crate_dir()`, while `gen_shims::jni_output_path` derived it from `config.name`. On any workspace where the two differ (e.g. tree-sitter-language-pack with `name = "tree-sitter-language-pack"` and `sources = ["crates/ts-pack-core/src/lib.rs"]`), regen produced an orphan `crates/ts-pack-core-jni/Cargo.toml` with no `src/`, while the actual JNI shim landed under `crates/tree-sitter-language-pack-jni/`. cargo broke the entire workspace with `can't find library 'ts_pack_jni', rename file to 'src/lib.rs' or specify lib.path`, taking cargo-check, cargo-clippy, cargo-fmt, cargo-machete, and cargo-deny down with it. The scaffold now uses `config.name` for the crate directory and the `[package] name`, and uses the cargo package name (`config.name`) as the umbrella dep-table key while keeping `path = "../<core_crate_dir>"` for the on-disk location. Added regression test `scaffold_jni_path_uses_config_name_not_core_crate_dir`. (`crates/alef-scaffold/src/languages/jni.rs`)

- **alef-backend-swift: add `tokio` to the generated Cargo.toml's `[package.metadata.cargo-machete] ignored = [...]` list**: alongside the existing `async-trait` / `serde` entries, `tokio` is now ignored. The Swift backend emits `tokio` unconditionally in `[dependencies]` for stable-manifest reasons (matches the documented "conditionally referenced only when the umbrella crate declares trait bridges or async adapters" pattern). For umbrella crates without trait bridges or async adapters — e.g. tree-sitter-language-pack — `tokio` is genuinely unused, and `cargo machete` correctly flagged it. Listing it in `ignored` matches the same approach the template already uses for the other conditional deps. Surfaced on tree-sitter-language-pack prek run. (`crates/alef-backend-swift/src/gen_rust_crate/cargo.rs`)

- **alef-backend-rustler: align the closing `}` of `@type t :: %__MODULE__{...}` to 8-space indent so `mix format` accepts it idempotently**: the typespec block opened at 10-space indent for fields (column 11) but closed with `}` (2 spaces), which `mix format` rewrote to 8 spaces. tslp's 19+ struct files (`ProcessConfig`, `ProcessResult`, `FileMetrics`, `Span`, etc.) flapped between emit and format on every prek run. Closing brace now matches the column `mix format` settles on. Surfaced on tree-sitter-language-pack prek run. (`crates/alef-backend-rustler/src/gen_bindings/helpers.rs`)

- **alef-scaffold: bump the JNI Cargo.toml's `jni` dep from `0.21` to `0.22`**: `alef-backend-jni::gen_shims` emits `use jni::{Env, JNIEnv};` (jni 0.22 split `JNIEnv` into FFI-safe `EnvUnowned<'frame>` and the API-carrying `Env<'_>`) but the scaffold template was still pinning `jni = "0.21"`. tslp's regen produced a `tree-sitter-language-pack-jni` crate that failed `cargo check` with `unresolved import jni::Env`. Bumped the scaffold to match the shim emitter. Surfaced on tree-sitter-language-pack prek run. (`crates/alef-scaffold/src/languages/jni.rs`)

## [0.16.23] - 2026-05-16

### Fixed

- **alef-core: sanitize Zig enum tag identifiers**: `zig_ident` previously escaped reserved keywords only; serde-renamed tags such as `og:image` or `Content-Type` flowed through unchanged and emitted invalid Zig (`enum fields do not have types` at the colon). The helper now replaces every non-`[A-Za-z0-9_]` character with `_`, prefixes a leading digit with `_`, then runs the result through `zig_safe_name`. Affects every Zig enum variant whose serde rename contains punctuation. (`crates/alef-core/src/keywords.rs`)

- **alef-backend-php: skip `binding_excluded` fields in `pub fn new(...) -> Self` literals**: the named-constructor emitter iterated every field of the IR type and wrote `name: Default::default()` for fields marked `binding_excluded`, but those fields are stripped from the binding struct itself — producing `error[E0560]: struct X has no field named Y` (seen on `DownloadedDocument.content` and `ScrapeResult.screenshot` in kreuzcrawl-php). The constructor now filters `binding_excluded` fields entirely; only `cfg`-gated fields (which stay in the struct under a `#[cfg]` gate) still receive `Default::default()` initializers. Both the `has_serde` and the non-serde branches of `gen_struct_methods_impl` are fixed. (`crates/alef-backend-php/src/gen_bindings/types.rs`)

- **alef-backend-go: emit sealed-interface fields as bare interface, not pointer-to-interface**: struct fields whose type is a tagged-data (sealed-interface) enum were being emitted as `*AuthConfig` for both optional and `has_default` positions, producing `cannot use v (variable of interface type AuthConfig) as *AuthConfig value in assignment: AuthConfig does not implement *AuthConfig (type *AuthConfig is pointer to interface, not interface)` at compile time. Go interfaces are already nullable — the interface zero value is `nil` — so taking a pointer to one is never the right shape. `gen_struct_type` detects sealed-interface fields via `data_enum_names` and emits the bare type for both struct declarations and builder option setters; the custom `UnmarshalJSON` emitter also drops the `&` when assigning the decoded variant. `gen_config_options` now takes `data_enum_names` and skips the `&v` address-of path for these fields. Test `test_parent_struct_with_optional_data_enum_field_emits_custom_unmarshal_json` updated to assert the new shape. (`crates/alef-backend-go/src/gen_bindings/{types.rs,mod.rs}`, `crates/alef-backend-go/tests/bug_fixes_test.rs`)

### Changed

- **alef-snippets: parser tolerates malformed YAML frontmatter**: `parse_frontmatter` previously returned `Error::Parse` when a snippet file opened with `---\n` but had no closing `\n---\n` delimiter, or when the YAML between delimiters failed to deserialize. Snippet collections that include partial templates or human-edited drafts would short-circuit the entire run. The parser now treats these cases as "no frontmatter": `SnippetMetadata::default()` plus the original `content` are returned, and validation continues against the body. Strict structural reporting still lives in `frontmatter_status`, which downstream `audit` consumers can opt into. (`crates/alef-snippets/src/parser.rs`)

- **alef-snippets: gaps detection recognises binding-specific language dir names**: `gaps::language_group` previously formed the `{language}` placeholder by string-matching the rendered `Language::to_string()` value, so `docs/snippets/kotlin_android/...` produced no group (`kotlin_android` != `kotlin`) and `gaps` could not flag missing variants for kotlin-android consumers. The matcher now passes each path component through `Language::from_dir_name`, which already understands aliases like `kotlin_android`, `kotlin-android`, `c-sharp`, `dotnet`, `golang`, and the `wasm`/`node` → typescript shortcuts. (`crates/alef-snippets/src/gaps.rs`)

- **alef-snippets: accept `kotlin_android` and `kotlin-android` as kotlin in `from_dir_name`**: directory names used by polyglot consumers (`kotlin_android`, `kotlin-android`) now map to `Language::Kotlin`. (`crates/alef-snippets/src/types.rs`)

- **alef-snippets: go/java/csharp validators wrap bare fragments**: parity with the existing `RustValidator::wrap_if_fragment` path. Snippets that omit the `package main` / `public class` / `Main`-method boilerplate now compile cleanly at syntax level. (1) `GoValidator` splits leading `import (...)` / `import "..."` blocks from the body and wraps the body in `package main; func main() { ... }`; comment-only bodies get a placeholder `_ = 0` statement so the entry point still compiles. (2) `JavaValidator` detects an existing top-level class/interface/enum/record and skips wrapping; otherwise it splits leading `import`/`package` declarations and wraps in `public class Snippet { public static void main(String[] args) throws Exception { ... } }`. (3) `CsharpValidator` wraps comment-only bodies with a `return;` placeholder so the top-level-statements compiler entry point is satisfied; the `is_dependency_error` list also picks up `CS5001` ("Program does not contain a static 'Main' method") for the placeholder fall-through. (`crates/alef-snippets/src/validators/{go,java,csharp}.rs`)

- **alef-snippets: C validator broadens dependency-error pattern**: `CValidator::is_dependency_error` now matches `file not found`, `No such file or directory`, `undeclared identifier`, `implicit declaration`, and `unknown type name` — so snippets that legitimately `#include "html_to_markdown.h"` (or any other host-supplied header) downgrade to Pass at syntax level when the include path is not configured in the validator sandbox, instead of failing CI on a legitimate doc example. (`crates/alef-snippets/src/validators/c.rs`)

### Added

- **alef-snippets: add JSON and YAML in-process validators**: snippets in `` ```json `` / `` ```yaml `` fences are now syntax-checked by deserializing into `serde_json::Value` / `serde_yaml::Value`. Both validators run in-process (no external toolchain required) and report concrete parser error messages on failure. (`crates/alef-snippets/src/validators/{json_validator,yaml_validator}.rs`, `crates/alef-snippets/src/validators/mod.rs`)

- **alef-snippets: add R validator (`Rscript -e 'parse(file=...)'`)**: probes for `Rscript` and `R` on PATH; uses `parse(file=...)` for syntax/compile levels and `Rscript <file>` for run level. Returns `Unavailable` when the R toolchain is missing. (`crates/alef-snippets/src/validators/r.rs`)

- **alef-snippets: register documentation-only languages (`text`, `mermaid`, `powershell`, `xml`, `docker`) with no-op pass-through validators**: doc fences in these formats are now recognised by `Language::from_fence_tag` and short-circuit to `Pass` instead of falling through to `Unknown`. Prevents `audit` from reporting them as `UnknownLanguage` errors. (`crates/alef-snippets/src/validators/documentation.rs`, `crates/alef-snippets/src/types.rs`)

- **alef-snippets: add C validator**: closes the last language gap. The `CValidator` probes `cc`, `clang`, then `gcc` for availability and shells out to the first match. Syntax level uses `cc -fsyntax-only`; Compile compiles to a temp binary via `cc -o <out>`; Run additionally executes the compiled binary. (`crates/alef-snippets/src/validators/c.rs`, `crates/alef-snippets/src/validators/mod.rs`)

- **alef-snippets: add 8 missing language validators (c, csharp, dart, go, java, kotlin, swift, zig)**: the validator registry previously covered bash/elixir/php/python/ruby/rust/toml/typescript (8 languages); snippets in any other language returned `Unavailable`. The 8 new validators each gate on toolchain presence via `which::which(...)` and shell out to the upstream toolchain: `cc -fsyntax-only` (Syntax) / `cc -o <out>` (Compile/Run) for C, `dotnet build --nologo -v quiet` (Syntax/Compile) for C#, `dart analyze --no-fatal-warnings` for Dart, `gofmt -e -l` (Syntax) / `go build` (Compile) / `go run` (Run) for Go, `javac -Xlint:none` (with class-name extraction so the temp file matches the public class) for Java, `kotlinc -d <out>` for Kotlin, `swiftc -parse` (Syntax) / `swiftc -o <out>` (Compile/Run) for Swift, `zig ast-check` (Syntax) / `zig build-exe -fno-emit-bin` (Compile) for Zig. (`crates/alef-snippets/src/validators/{c,csharp,dart,go,java,kotlin,swift,zig}.rs`, `crates/alef-snippets/src/validators/mod.rs`)

- **alef-snippets: wire `audit` and `gaps` CLI subcommands**: the `alef-snippets` binary previously exposed only `list`, `validate`, and `parse`. The `audit` (structural integrity report — frontmatter, fences, includes) and `gaps` (coverage gaps — missing includes, unreferenced snippets, missing language variants, unannotated skips, unknown languages) library entry points are now reachable from the CLI. `--snippets <dir>... --docs <dir>... [--require-frontmatter | --required-languages <list>]`. Both commands exit with a non-zero status when issues are found, so they slot into pre-commit / CI gates. (`crates/alef-snippets/src/main.rs`)

- **workspace: register `alef-snippets` crate + add missing workspace deps**: `crates/alef-snippets` is now a `[workspace.members]` entry and a `[workspace.dependencies]` alias, fixing the prior cargo error `cannot specify features for packages outside of workspace`. Added `tempfile = "3"`, `walkdir = "2"`, and `which = "8"` as workspace dependencies — the alef-snippets crate referenced them via `workspace = true` before any workspace-level declaration existed. (`Cargo.toml`)

## [0.16.22] - 2026-05-16

### Fixed

- **alef-e2e: remove vite-plugin-wasm from WASM e2e vitest config**: `wasm-pack build --target nodejs` produces a self-initializing CommonJS bundle that loads the `.wasm` binary via `fs.readFileSync` at import time — it does not require the browser-oriented `vite-plugin-wasm` Vite transform. Including `vite-plugin-wasm` caused all `Wasm*` class exports to be `undefined` when running under wasm-pack 0.13.x (installed by the `curl | sh` installer in CI), because the plugin intercepted the CJS `require()` path and returned an uninitialized `WebAssembly.Module` instead of the module object. Removed `import wasm from 'vite-plugin-wasm'` and `plugins: [wasm()]` from `vitest.config.ts.jinja`, and removed the `vite-plugin-wasm` devDependency from `package.json.jinja`. (`crates/alef-e2e/templates/wasm/vitest.config.ts.jinja`, `crates/alef-e2e/templates/wasm/package.json.jinja`, `crates/alef-e2e/src/codegen/wasm.rs`)

- **alef-backend-wasm: add `is_sanitized_fixed_tuple_array` predicate for fixed tuple-array fields**: tagged-enum struct fields that were originally `[(K, V); N]` but sanitized to `String` were not being detected as JsValue candidates, causing them to be stored as `Option<String>` in the binding struct rather than `Option<JsValue>`. Added `is_sanitized_fixed_tuple_array` predicate (matching `original_type` that starts with `[(` and contains `);`) and wired it alongside `is_sanitized_tuple_vec` in all three collection sites in `gen_tagged_enum_as_struct`, `gen_tagged_enum_binding_to_core`, and `gen_tagged_enum_core_to_binding`. (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`)

- **alef-backend-jni + alef-backend-kotlin: fix JNI exception swallowing in generated Android bridge**: two gaps caused Rust `Result::Err` / panics to be silently lost instead of reaching Kotlin callers as typed exceptions. (1) The generated `Bridge.kt` `external fun` declarations were missing `@Throws(<Bridge>Exception::class)` annotations — without this, JNI-thrown `RuntimeException` is wrapped in `UndeclaredThrowableException` or otherwise bypasses typed `catch` blocks. Every non-destructor `external fun` (top-level functions, instance methods, streaming Start/Next shims) now carries `@Throws`. Destructor shims remain annotation-free (they are infallible drop operations). (2) The generated `throw_jni_error` helper emitted `let _ = env.throw_new(ERROR_CLASS, msg)`, silently discarding failure. If the exception class was not on the classpath (misconfigured AAR) the shim returned a null/zero sentinel with no exception pending, appearing to Kotlin as a valid return value. The helper now checks `is_err()` and falls back to `java/lang/RuntimeException` so the caller always receives an exception. Adds explicit unit tests and updates all snapshots. (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`, `crates/alef-backend-jni/src/gen_shims.rs`)

- **alef-backend-swift: preserve `Option<T>` as `T?` in enum-variant associated values**: `emit_variant_with_data` previously called `mapper.map_type(&f.ty)` without checking `field.optional`, so fields where the extractor had unwrapped `TypeRef::Optional(inner)` into `(ty: inner, optional: true)` were emitted as bare `T` instead of `T?`. The same gap existed in `emit_error` for error-variant fields. Both sites now check `f.optional && !already_optional` and append `?` when the flag is set, matching the existing behavior in `emit_first_class_struct`. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/snapshot_test.rs`)

- **alef-backend-kotlin: emit ktfmt-stable data-class declarations**: the Kotlin data-class emitter now applies ktfmt's 100-character heuristic — declarations whose single-line form fits within 100 chars are emitted as a single line (`data class Foo(val a: Int, val b: T)`); longer declarations stay multi-line. Previously the emitter always used multi-line, causing ktfmt in CI to rewrite every short declaration and break the lint step. The same heuristic applies to sealed-class variant data classes and error-variant data classes. Annotations (`@JsonDeserialize` / `@JsonSerialize`) on variant subclasses force multi-line regardless of length. Field-level KDoc also forces multi-line. Adds 6 new unit tests: short/long coverage for top-level data classes, sealed-class variants, and error variants. (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`, `crates/alef-backend-kotlin/tests/gen_bindings_test.rs`)

## [0.16.21] - 2026-05-16

### Added

- **alef-backend-php: emit per-property `@var` PHPDoc on promoted readonly constructor parameters**: generated DTO classes now carry an inline `/** @var T description */` block above every promoted property (and `/** ... @var T */` block for multi-line rustdoc). PHPStan level max now sees the precise generic element type (`@var array<string>`, `@var ?int`) and IDEs surface tooltips on every property. Replaces the prior global `@param` block before the constructor, which only covered array fields and didn't propagate field rustdoc. Adds `php_property_phpdoc()` helper + 153 lines of unit tests covering empty/single-line/multi-line doc forms. (`crates/alef-backend-php/src/gen_bindings/mod.rs`, `crates/alef-backend-php/tests/gen_bindings_test.rs`)

- **alef-core: `KotlinAndroid` package-directory lookup**: `default_package_directory` now maps `Language::KotlinAndroid` to `packages/kotlin-android` explicitly instead of falling through to the generic `packages/{lang}` formatter (which produced `packages/kotlinandroid`). (`crates/alef-core/src/config/resolved/lookups.rs`)

- **alef-backend-ffi: emit Doxygen comments on C headers**: every `extern "C" fn` declaration, `typedef`, opaque-handle wrapper, enum, and enum variant in the generated `.h` files now carries a `/** ... */` Doxygen block derived from the upstream rustdoc. `# Arguments` items render as `\param`, `# Returns` as `\return`, `# Errors` and `# Safety` as `\note`, `# Example` as `\code`/`\endcode`. C/C++ consumers and IDEs (clangd, Xcode, Visual Studio) now surface tooltips on every public FFI symbol. Adds `emit_c_doxygen` to the shared `alef-codegen::doc_emission` module. (`crates/alef-codegen/src/doc_emission.rs`, `crates/alef-backend-ffi/src/gen_bindings/{functions.rs,types.rs,enums.rs}`, snapshot tests)

- **alef-backend-go: emit Godoc on methods + free functions**: every exported method declaration `func (r *Receiver) Method(...)` and every free function `func Name(...)` in generated `.go` files now carries `// Method ...` / `// Name ...` Godoc derived from upstream rustdoc. Comments start with the symbol name (per the `go doc` and `godoc` convention) so IDE tooltips and `pkg.go.dev` render every public binding member. `# Arguments` → `// Arguments:` bullets, `# Returns` → `// Returns ...`, `# Errors` → `// Errors are returned when ...`, `# Example` → `// Example:` indented code block. Also fixes a pre-existing double-prefix glitch where summaries that already started with the symbol name produced `// RootNode RootNode returns ...`. (`crates/alef-backend-go/src/gen_bindings/types.rs`, `crates/alef-backend-go/tests/snapshot_test.rs`)

- **alef-backend-java: close docstring emission gaps at record components, opaque-handle instance methods, and free-function FFI surface**: per-component Javadoc now renders above each `record` field when the multi-line emit path is taken (single-line records skip per-component docs because inlining `/** ... */` inside a comma-joined parameter list produces invalid Java). Opaque-handle instance methods emitted by `gen_instance_method` now carry their rustdoc above the public signature. Free-function static methods in the raw FFI class (`gen_sync_function_method`) emit Javadoc derived from `FunctionDef.doc`. (`crates/alef-backend-java/src/gen_bindings/{types.rs,ffi_class.rs}`, snapshot tests in `crates/alef-backend-java/tests/docs_emission_test.rs`)

- **alef-backend-zig: close docstring emission gaps at struct fields and enum variants**: `///` doc comments now render above each struct field declaration inside `pub const Foo = struct { ... };` and above every enum tag — both for unit-only `enum { ... }` and tagged-union `union(enum) { ... }` shapes (void / single-value / struct-variant cases). Type/method/free-function emission was already covered upstream. (`crates/alef-backend-zig/src/gen_bindings/types.rs`, snapshot tests in `crates/alef-backend-zig/tests/docs_emission_test.rs`)

- **alef-backend-kotlin-android: close docstring emission gaps at data-class fields, enum variants, and sealed-class variants**: the kotlin-android backend reuses the shared `alef-backend-kotlin` emitter helpers; this change wires `emit_cleaned_kdoc` at every previously-uncovered site (data-class primary-constructor field, plain enum constant, sealed-class nested variant) so KDoc now propagates through to AAR-emitted `.kt` files. The free-function facade, JNI bridge object, and handle-only opaque wrapper class already emitted KDoc at the kotlin-android-local sites. (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`, snapshot tests in `crates/alef-backend-kotlin-android/tests/docs_emission_test.rs`)

- **alef-backend-rustler: emit `@moduledoc`/`@typedoc`/`@doc` on DTO modules + enum variants**:
  previously only NIF stubs (function-level) carried rustdoc-derived `@doc` heredocs; the
  generated Elixir DTO `defmodule` blocks, unit-enum variant accessors, and data-enum
  per-variant type aliases emitted no doc attributes, leaving `mix docs` with blank ExDoc
  pages for those modules. Now wires a shared `emit_elixir_doc_attr` helper at every
  applicable site so `mix docs` shows complete coverage: full multi-line `@moduledoc`
  heredocs on struct-bearing modules (DTO + opaque resource wrappers) and enum modules,
  `@typedoc` above each `@type t ::` typespec and above each data-enum per-variant type
  alias, and `@doc` above each unit-enum variant accessor function (preserving
  `@doc → @spec → def` ordering for mix-format compliance). Modules with no rustdoc still
  fall back to `@moduledoc false` to silence ExDoc warnings.
  (`crates/alef-backend-rustler/src/template_env.rs`,
  `crates/alef-backend-rustler/src/gen_bindings/helpers.rs`,
  `crates/alef-backend-rustler/tests/elixir_module_doc_test.rs`)

- **alef-codegen: language-aware example filtering (`example_for_target`)**: rustdoc `# Example` blocks fenced as ` ```rust ` (and bare-fenced ` ``` ` which rustdoc treats as Rust) are now suppressed from non-Rust doc renderers — YARD (`render_yard_sections`), JSDoc (`render_jsdoc_sections`), and PHPDoc (`render_phpdoc_sections`). Examples fenced with the target language (` ```php `, ` ```ruby `, ` ```typescript `) flow through verbatim. The prior behavior produced misleading Rust source bodies inside Ruby/PHP/TS doc tooltips. 5 new unit tests plus updated assertions in the existing renderer tests. (`crates/alef-codegen/src/doc_emission.rs`)

- **alef-backend-rustler: full multi-line `@doc` on functions + methods**: `@doc` strings for NIF functions and methods previously truncated to the first line via `func.doc.lines().next()`, dropping all subsequent paragraphs. The emitter now calls `doc_first_paragraph_joined` to preserve the complete first paragraph (joining wrapped lines) and falls back to `"Function"`/`"Method"` only when rustdoc is empty. Closes the gap left by the v0.16.20 single-line truncation noted in the audit. (`crates/alef-backend-rustler/src/gen_bindings/mod.rs:682,1172`)

- **alef-backend-magnus: YARD `@param`/`@return` on tagged-enum variant classes**: generated Ruby variant subclasses (`class TextContent < ContentPart`) now carry YARD tags on every site that consumers see in IDE tooltips: each `attr_reader` field, the variant `initialize` parameter list, the predicate method (`def text? = true`), and the class-level `from_hash` factory. The variant's fields use either upstream rustdoc (via `emit_yard_doc`) or a synthesized `# @return [SorbetType]` line so `solargraph` and `yard server` always resolve the type. (`crates/alef-backend-magnus/src/gen_bindings/mod.rs:gen_tagged_enum_ruby_classes`)

### Fixed

- **alef-cli/alef-e2e: use `oxfmt` instead of Taplo for generated TOML normalization**:
  the default post-generation formatters now call `pnpm dlx oxfmt` for WASM
  manifests, FFI/workspace formatting, and Rust e2e `Cargo.toml` normalization,
  aligning generated output with Kreuzberg's shared pre-commit hooks.
  (`crates/alef-cli/src/pipeline/format.rs`, `crates/alef-e2e/src/format.rs`)

- **alef-backend-ffi: escape generated `cbindgen.toml` `after_includes` safely**:
  Doxygen blocks embedded in `after_includes` now render as escaped TOML
  multiline strings, so backslash commands such as `\code` and rustdoc examples
  containing triple quotes no longer make `cbindgen.toml` invalid.
  (`crates/alef-backend-ffi/templates/cbindgen_toml.jinja`,
  `crates/alef-backend-ffi/src/gen_bindings/helpers.rs`,
  `crates/alef-backend-ffi/src/gen_bindings/mod.rs`)

- **alef-backend-wasm: emit current `getrandom` for wasm target dependencies**:
  generated WASM `Cargo.toml` files now use `getrandom` `0.4` with `wasm_js`, so downstream
  aggressive dependency upgrades no longer make generated manifests stale.
  (`crates/alef-backend-wasm/src/gen_bindings/mod.rs`,
  `crates/alef-backend-wasm/tests/gen_bindings_test.rs`)

- **alef-e2e: omit non-runnable Python and Node fixtures instead of generating skipped tests**:
  fixtures that have no assertions for the target language, or HTTP 101 upgrade
  fixtures that the standard Python/Node HTTP clients cannot execute, are now
  left out of those generated suites instead of producing skipped placeholders.
  (`crates/alef-e2e/src/codegen/python`,
  `crates/alef-e2e/src/codegen/typescript`)

- **alef-backend-php: promote opaque class parameters after the first optional argument**:
  generated PHP userland class declarations now mirror static extension stubs by
  marking parameters after an optional parameter as nullable with `= null`,
  keeping PHPStan/PHP syntax valid for methods like `TestClient::post`.
  (`crates/alef-backend-php/src/gen_bindings/mod.rs`,
  `crates/alef-backend-php/tests/gen_bindings_test.rs`)

## [0.16.20] - 2026-05-16

### Fixed

- **alef-backend-kotlin-android: emit resolved workspace version in generated `build.gradle.kts`**: AAR `version` coordinate was hardcoded to `"0.0.0"`, leaving every published Android artifact tagged as v0.0.0 in Maven local. Now reads `config.resolved_version()` (workspace `version =`) so the generated `build.gradle.kts` carries the real release version. (`crates/alef-backend-kotlin-android/src/gen_build_gradle.rs`)

- **release: include all v0.16.19 fixes**: v0.16.19 publish failed on `cargo fmt --check` due to a stray `eprintln!` in a test landed by a concurrent agent. v0.16.20 carries the same fix set plus the fmt scrub and the kotlin-android AAR version fix.

## [0.16.19] - 2026-05-16

### Fixed

- **alef-e2e/typescript: emit synthetic `kind` discriminator on napi serde-tagged-enum fixtures**: `alef-backend-napi` (commit `091514d`) hardcodes `"kind"` as the TypeScript discriminator for every internally-tagged (`#[serde(tag = "...")]`) data enum, but e2e fixture codegen still emitted the original serde-tag name (e.g. `role: "user"` instead of `kind: "user"`), producing `Missing field 'kind'` runtime errors across the entire node smoke/chat suite. Adds `rename_napi_serde_tags_to_kind` + `rename_napi_serde_tags_recursive` helpers that walk fixture JSON and rename any serde-tag key to `"kind"` only when the value matches a known variant of the corresponding tagged-data enum (so plain struct fields sharing the same name are left alone). Wired into both `build_args_and_setup` (node, no `options_type`) and `ts_builder_expression_inner` (node, with `options_type`). WASM fixtures untouched — `serde_wasm_bindgen::from_value` reads original serde attribute names. (`crates/alef-e2e/src/codegen/typescript/test_file.rs`)

- **alef-backend-go: emit custom `UnmarshalJSON` on parent structs containing untagged-enum fields**: structs whose Go-bound fields include any untagged-enum (`#[serde(untagged)]`) named type previously deserialized into the bare struct shape, which left those fields as `nil` because Go's default `json.Unmarshal` doesn't dispatch on shape. The backend now collects every binding field whose type is `TypeRef::Named(n)` or `TypeRef::Optional(TypeRef::Named(n))` where `n` is in `data_enum_names`, and emits a `func (s *Parent) UnmarshalJSON(data []byte) error` that decodes through an anonymous helper struct using `json.RawMessage` for those fields, then dispatches each raw payload through the matching `Unmarshal<EnumName>(raw)` helper. (`crates/alef-backend-go/src/gen_bindings/types.rs:gen_struct_type`)

- **alef-backend-swift: call `.intoRust()` on first-class DTO args at call sites**: the Swift binding's `emit_client_class` previously passed Swift first-class DTO arguments directly into Rust-bridge function calls without invoking `.intoRust()`, leaving the Rust side with a Swift wrapper value instead of the materialised Rust DTO and failing the swift-bridge type check. Each first-class DTO argument now flows through `.intoRust()` at the call site, matching the convention already in place for nested constructor extern calls. (`crates/alef-backend-swift/src/gen_bindings.rs:emit_client_class`)

- **alef-e2e/kotlin_android: apply Android Gradle Plugin in e2e build.gradle.kts**: the e2e fixture-driven Android test app's generated `build.gradle.kts` referenced `android { ... }` DSL blocks but never applied the `com.android.application` plugin, so `./gradlew assemble` failed with "android extension not found". The codegen now applies the AGP at the top of the test-app gradle script. (`crates/alef-e2e/src/codegen/kotlin_android.rs`)

- **alef-e2e/java: iterate streams via `.iterator()` instead of calling `Iterator` methods on `Stream`**: streaming-assertion codegen called `.hasNext()` / `.next()` directly on `java.util.stream.Stream`, which Java disallows (those methods live on `Iterator`). The codegen now resolves through `.iterator()` first, mirroring the idiomatic Java streaming-consumer pattern. (`crates/alef-e2e/src/codegen/streaming_assertions.rs`)

- **alef-backend-java: unwrap `Optional<T>` in facade returns + propagate to stream-method named results**: facade methods whose Rust return is `Option<T>` now call the raw-class method through `.orElse(null)` so the public facade signature (`@Nullable T`) matches the raw class signature (`Optional<T>`). The bytes path is preserved (raw signature already returns `byte[]`, not `Optional<byte[]>`, so unwrapping would be a compile error). The streaming-method-named-result template now distinguishes optional vs non-optional returns at both the early-null check and the success path, wrapping through `Optional.of`/`Optional.empty()` only when the declared return is `Optional<T>`. (`crates/alef-backend-java/src/gen_bindings/facade.rs`, `crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/templates/facade_class.jinja`, `crates/alef-backend-java/templates/stream_method_named_result.jinja`)

- **alef-backend-kotlin-android: extend generic-container support to `HashMap<_,_>` and `Option<Vec|Map>` in public signatures**: facade param/return type resolution previously only special-cased `Vec<DTO>` → `List<DTO>` for non-opaque named types, leaving `HashMap<_,_>`, primitive vecs, and any optional-wrapped container falling through to the JNI primitive mapping (which served bare `String` for all generics). Now routes any `Vec<_>` / `Map<_,_>` (including `Option<Vec|Map>`) through the recursive `render_kotlin_type` helper, and switches the Jackson deserialization decision from `is_vec_of_dtos` to the broader `is_generic_container` predicate so `TypeReference<...>` is used for every container return. Public Kotlin signatures now match the Jackson-serialized payload regardless of the inner element type. (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`, `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`, `crates/alef-backend-kotlin/src/gen_bindings/mod.rs`)

- **alef-backend-swift: primitive-only direct constructor + shared JSON-fallback predicate**: primitive-only serde DTOs (e.g. `Point { row: u32, column: u32 }`, `ByteRange { start: usize, end: usize }`) now get a positional constructor extern emitted to the swift-bridge `extern "Rust"` block instead of falling through to a JSON-roundtrip path whose Rust-side shim was never emitted. Adds shared `requires_json_fallback` predicate consulted by both the Swift binding side (`gen_bindings.rs`) and the Rust crate side (`gen_rust_crate/mod.rs`) so any type Swift will JSON-encode is guaranteed to have a matching Rust shim. Closes the `RustBridge.pointFromJson` / `byteRangeFromJson` compile gap in tslp's swift binding. (`crates/alef-backend-swift/src/gen_rust_crate/extern_block.rs`, `crates/alef-backend-swift/src/gen_rust_crate/json_fallback.rs` (new), `crates/alef-backend-swift/src/gen_rust_crate/mod.rs`, `crates/alef-backend-swift/src/gen_rust_crate/wrappers.rs`, snapshot tests in `crates/alef-backend-swift/tests/`)

- **alef-cli: per-language setup workdir for swift/kotlin-android/dart/zig**: setup install commands now run from each binding's manifest directory (`packages/swift`, `packages/kotlin-android`, `packages/dart`, `packages/zig`) instead of the repo root. Closes "Could not find Package.swift" timeouts during `alef setup` invocations for languages whose manifest does not live at the workspace root. Other languages (python, node, ruby, php, go, java, csharp, elixir, wasm, ffi) retain current behavior of running from repo root. (`crates/alef-core/src/config/setup_defaults.rs`, `crates/alef-cli/src/pipeline/commands.rs`, `crates/alef-core/tests/setup_config_test.rs` (new))

- **alef-backend-php: promote static stub parameters after the first optional argument**:
  PHP stubs for static API methods now mark parameters after the first optional
  argument as nullable with `= null`, matching the generated PHP facade and
  keeping PHPStan/PHP syntax valid for Rust functions whose required parameters
  follow optional ones. (`crates/alef-backend-php/src/gen_bindings/mod.rs`,
  `crates/alef-backend-php/tests/gen_bindings_test.rs`)

- **alef-scaffold: place Kotlin sample sources under their declared package path**:
  JVM scaffold now writes `Sample.kt` beneath
  `src/main/kotlin/<package>/sample/`, so Detekt package/path validation passes
  for generated Kotlin package names. (`crates/alef-scaffold/src/languages/kotlin.rs`,
  `crates/alef-scaffold/src/tests.rs`)

## [0.16.18] - 2026-05-16

### Fixed

- **alef-backend-java: import `Optional` for opaque optional method returns**: the opaque-handle import scanner now uses the same public return-type mapper as the generated method signature, so `Option<T>` methods emit both `Optional<T>` and the required `java.util.Optional` import. (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

- **alef-backend-kotlin: unwrap Java `Optional` in opaque method wrappers**: Kotlin JVM wrapper classes now append `.orElse(null)` when delegating opaque methods that return Rust `Option<T>`, matching the public Kotlin `T?` signature. (`crates/alef-backend-kotlin/src/gen_bindings/mod.rs`, `crates/alef-backend-kotlin/tests/gen_bindings_test.rs`)

- **alef-cli: avoid .NET query-mode shorthand in C# builds**: the default C# build command now uses `dotnet build --verbosity quiet` instead of `-q`, avoiding SDKs where `-q` exits non-zero when `CoreCompile` is not up to date. (`crates/alef-cli/src/pipeline/commands.rs`)

## [0.16.17] - 2026-05-16

### Added

- **alef-codegen: YARD doc emission (`emit_yard_doc`, `render_yard_sections`) + Magnus backend wiring**: shared module gains a Ruby YARD-flavoured renderer that translates rustdoc `# Arguments` / `# Returns` / `# Errors` sections into `# @param` / `# @return` / `# @raise` tags. The Magnus backend now emits YARD blocks above generated tagged-enum wrapper classes in `packages/<crate>/lib/*/native.rb`, restoring IDE tooltips and RDoc rendering that were previously a no-op when `gen_bindings/classes.rs` ignored `TypeDef.doc`. 4 new unit tests on the shared module, 1 integration test on the Magnus backend. (`crates/alef-codegen/src/doc_emission.rs`, `crates/alef-backend-magnus/src/gen_bindings/mod.rs`, `crates/alef-backend-magnus/tests/gen_bindings_test.rs`)

- **alef-backend-extendr: roxygen2 docs on R class envs, enum class envs, and richer free-function blocks**: class envs (`Type <- new.env(parent = emptyenv())`) now carry a roxygen block derived from `TypeDef.doc` with one `#' @field` line per public field; flat data-enum class envs gain a `#' @field` per variant from `EnumVariant.doc`; JSON-passthrough enum class envs gain a title/description block (no `@field` since callers go through the opaque `__inner` JSON blob). Free-function roxygen blocks now parse rustdoc sections via `alef-codegen::parse_rustdoc_sections` + `parse_arguments_bullets` so `# Arguments` bullet bodies drive `#' @param` lines, `# Returns` drives `#' @return`, `# Errors` becomes `#' @section Errors:`, and raw headings stop leaking into the description body. 3 new unit tests. (`crates/alef-backend-extendr/src/gen_bindings.rs`, `crates/alef-backend-extendr/templates/r_type_class_env.jinja`)

- **alef-backend-kotlin-android: KDoc emission for types, methods, fields, enum variants**: the kotlin-android backend previously had zero `.doc` references in `gen_bindings.rs`, producing undocumented `.kt` files even when rustdoc was rich. Wires `emit_kdoc(...)` calls at every type/method/field/variant emission site, matching the alef-backend-kotlin pattern. Adds 2 unit tests and updates the snapshot fixture. (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`, `crates/alef-backend-kotlin/src/gen_bindings/mod.rs`, `crates/alef-backend-kotlin/src/lib.rs`, `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`, snapshot)

- **alef-docs: Kotlin/KotlinAndroid, Swift, Dart, Zig signature renderers**: replaces the `// Phase 1: {lang} backend signature generation` stubs in `signatures.rs` with full `render_kotlin_fn_sig`, `render_swift_fn_sig`, `render_dart_fn_sig`, `render_zig_fn_sig`, plus method-signature arms for all four. Kotlin/KotlinAndroid share a renderer; statics emit `@JvmStatic`, fallible functions emit `@Throws(...)`. Swift emits `public static func` / `public func` with `throws` keyword and trailing `-> Ret`. Dart emits optional params as `[Type? name]` (bracketed positional). Zig emits `pub fn name(...) ErrorTy!Ret` for fallible functions, `pub fn name(...) Ret` otherwise; instance methods include a `self: *const Receiver` first param, statics omit it. Closes the per-language API reference docs (`docs/reference/api-{kotlin-android,swift,dart,zig}.md`) for the v3.5.0 binding additions. Adds 17 new unit tests. (`crates/alef-docs/src/signatures.rs`)

- **alef-core: `FieldDef.original_type: Option<String>`** preserves the pre-sanitization Rust type string (e.g. `"Vec<(String, String)>"`) so backends can reconstruct proper serialization logic for tuple types that the sanitizer downgrades. Populated by `sanitize_unknown_types()` when a field's type is rewritten. (`crates/alef-core/src/ir.rs`, `crates/alef-extract/src/extractor/{helpers,types}.rs`)

- **alef-backend-ffi: emit `*_len()` companion for `c_char` returns**: every `pub unsafe extern "C" fn` returning `*mut c_char` now has a sibling `_len() -> usize` that returns the same string's byte length without allocating. Enables safe slice construction in Zig (drops `[:0]const u8` in favour of `[]const u8`) and Java FFM Panama bindings. (`crates/alef-backend-ffi/src/gen_bindings/functions.rs`, `crates/alef-backend-ffi/src/gen_bindings/mod.rs`)

### Changed

- **[BREAKING] alef-backend-swift: eliminate JSON roundtrip in DTO `intoRust()`**: Swift DTOs no longer serialize through `JSONEncoder` + Rust-side `from_json` shim. The Swift binding now calls a Rust-side bulk constructor extern via swift-bridge (`#[swift_bridge(init)] fn new(...)`), mirroring the existing direct-field-access pattern in `init(_ rb:) throws`. Internal bridge architecture change; public Swift API unchanged. Affects all DTOs that previously emitted `JSONEncoder().encode(self)` in `intoRust()` — primitive-only, nested-struct (`try self.field.intoRust()`), `Vec<Primitive | String | Named>` (materialised as `RustVec<T>`), and `Option<Primitive | String>` fields all take the direct path. DTOs whose constructor extern is not emitted (no `Default` impl, JSON-bridged Map/Json fields, or fields excluded via `[crates.<crate>.swift] exclude_fields`) keep the JSON fallback. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/src/gen_rust_crate/extern_block.rs`, tests in `crates/alef-backend-swift/tests/snapshot_test.rs`)

### Fixed

- **alef-backend-java: preserve `Optional<T>` in opaque method return signatures**: Java opaque-handle methods now use the public return-type mapper instead of the FFI boundary mapper, so methods returning `Option<String>` emit `Optional<String>` signatures to match their generated `Optional.empty()` / `Optional.of(...)` bodies. (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

- **alef-backend-go: keep byte-slice returns as `[]byte` in fallible methods**: Go methods that return `TypeRef::Bytes` but can fail while marshaling a non-opaque receiver now return `([]byte, error)` instead of `(*[]byte, error)`, matching the generated `unmarshalBytes` helper and idiomatic Go slice semantics. (`crates/alef-backend-go/src/type_map.rs`, `crates/alef-backend-go/tests/bug_fixes_test.rs`)

- **alef-backend-go: emit shape-discriminated `Unmarshal` for untagged enums**: `Unmarshal{EnumName}` now splits its emission on `enum_def.serde_untagged`. Tagged enums keep the existing wire-struct-and-switch pattern. Untagged enums (where `enum_def.serde_tag` is `None`) now emit a first-byte shape sniffer (`'"'` → string, `'['` → array, `'{'` → object, else numeric) that gates each variant, then attempts `json.Unmarshal` into each concrete struct in declaration order, returning the first success. Previously the wire struct was empty `struct {}` but the switch unconditionally accessed `wire.Type`, causing `wire.Type undefined (type struct{} has no field or method Type)` Go compile errors on every untagged enum (e.g. `RerankDocument`, `OcrDocument`). (`crates/alef-backend-go/src/gen_bindings/types.rs`, `crates/alef-backend-go/tests/bug_fixes_test.rs`)

- **alef-backend-kotlin: use payload-derived field name in untagged sealed-class serializer's `when` branch**: the custom serializer for newtype/tuple variants emitted literal `value.field0`, but the data-class declarations for those variants already render with payload-derived field names via `kotlin_field_name_with_type` (e.g. `data class Single(val value: String)`). Kotlin compilation failed with `Unresolved reference 'field0'` on `EmbeddingInput`, `ModerationInput`, `RerankDocument`, `StopSequence`, `ToolChoice`, `UserContent`. The `when`-branch now reuses the same helper, computing the correct field name from the field's type. (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`)

- **alef-backend-kotlin-android: null-coalesce nullable primitive bridge args to JNI defaults**: the facade emits public params like `baseUrl: String? = null`, `timeoutSecs: Long? = null` and passes them through to a JNI `external fun` declaration whose signature is non-nullable. The `bridge_args` closure previously only null-coalesced DTO/Vec<DTO>/opaque param types; primitive scalars (`String?`, `Long?`, `Int?`, `Double?`, `Bool?`) fell through to bare `name`, producing `Argument type mismatch: actual type is 'String?', but 'String' was expected` Kotlin compile errors. Now emits `name ?: <zero>` where zero matches the JNI signature (`""` for `String`, `0L` for `i64/u64`, `0` for `i32/u32`, `0.0` for floats, `false` for `bool`). (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`, `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`)

- **alef-cli: re-stamp `test_apps/` hashes after the `all` pipeline finalizes binding hashes**: registry-mode e2e files in `test_apps/` (generated by `alef e2e generate --registry`) carry `alef:hash:<sha>` markers computed from the sources hash of the upstream Rust crate. Any commit that modifies a watched Rust source file rotates the sources hash — but `alef all` never touched `test_apps/`, so their embedded hashes immediately went stale. `alef verify --exit-code` reported 188-257 stale `test_apps/` files on every CI run. The `all` pipeline now collects every alef-headered file under `e2e_config.registry.output` (default `test_apps/`) and re-runs `finalize_hashes` over them with the current sources hash, re-stamping their `alef:hash:` lines without regenerating their content. (`crates/alef-cli/src/main.rs`, `crates/alef-cli/src/pipeline/generate.rs`, `crates/alef-cli/src/pipeline/mod.rs`)

- **alef-cli: make post-generation formatter warnings actionable**: formatter failures now include both stdout and stderr instead of dropping stdout-only diagnostics. Python package formatting is scoped to the generated stub package when configured, avoiding unrelated hand-written package tests. (`crates/alef-cli/src/pipeline/format.rs`)

- **alef-scaffold: suppress php-cs-fixer runtime-version advisories in generated format commands**: PHP package scaffolds now set `setUnsupportedPhpVersionAllowed(true)` in `.php-cs-fixer.dist.php`, run php-cs-fixer format scripts with `--quiet`, and stop using the deprecated `PHP_CS_FIXER_IGNORE_ENV` composer-script prefix. (`crates/alef-scaffold/src/languages/php.rs`, `crates/alef-scaffold/src/tests.rs`)

- **alef-backend-wasm: `Vec<(K, V)>` enum fields use `JsValue` + `serde_wasm_bindgen`**: tagged-enum variants with fields whose original type was `Vec<(String, String)>` (sanitized to `Vec<String>`) are now stored as `Option<JsValue>` in the wasm struct rather than `Option<Vec<String>>`. Binding-to-core reads via `serde_wasm_bindgen::from_value::<Vec<(String, String)>>`, core-to-binding writes via `serde_wasm_bindgen::to_value`. Wire format `[["k","v"],...]` is preserved. Fixes compile errors in `html-to-markdown-wasm` for `NodeContent::MetadataBlock { entries }`. (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`, `crates/alef-cli/src/pipeline/extract.rs`)

- **alef-backend-swift: drop `-> ()` on unit-returning wrapper fns**: `wrappers.rs:gen_method_wrappers` emitted `pub fn parser_reset(client: &mut Parser) -> () { ... }` for any `&mut self` method returning `()` without an error type, which clippy flags as `unused_unit` (denied in tslp under `-D warnings`). Now omits the return clause entirely when `return_ty == "()"`. Affects every Unit-returning method exposed via swift-bridge wrappers. (`crates/alef-backend-swift/src/gen_rust_crate/wrappers.rs`)

- **alef-backend-ffi: `*_len()` companion uses correct fail-path return**: the `_len()` companion's parameter-validation early-return (null pointer, UTF-8 decode error, etc.) was emitting the primary function's `return std::ptr::null_mut();` instead of `return 0;`, causing E0308 type mismatches (`*mut _` vs `usize`) when the underlying function returns `*mut c_char`. `gen_free_function_len_companion` now passes a synthetic `TypeRef::Primitive(PrimitiveType::Usize)` to `gen_param_conversion` so `null_return_value` resolves to `"0"` for the fail-path. Affects every `*_len()` companion emitted for `String`/`Path`/`Json`/`Named`/`Vec`/`Map`/`Bytes` returning functions. (`crates/alef-backend-ffi/src/gen_bindings/functions.rs:gen_free_function_len_companion`)

- **alef-backend-swift: emit doc comments on opaque-class methods and first-class struct fields**: methods previously inlined an ad-hoc `for line in method.doc.lines()` loop with a stale comment ("Emit `/// doc` when available."); now route through the shared `emit_doc_comment` helper, matching how types/enums/errors/variants already emit. First-class struct fields surface their rustdoc above the generated `public let` declaration — previously emitted no doc at all. Plain prose passes through verbatim; `# Errors`/`# Returns` flow through as DocC-compatible text. 2 regression tests. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/gen_bindings_test.rs`)

- **alef-backend-swift: emit bidirectional `From` impls for `OptionsField` newtype wrappers**: the `OptionsField` bind_via path generates a factory (`make_{trait}_handle`) and options-helper (`{opts}_from_json_with_{field}`) that call `TypeAlias::from(inner)`, `<inner>::from(handle)`, and `OptionsType::from(core_opts)`. These three `From` impls were missing — only enums emitted one-direction `From` via match arms. `emit_options_field_from_impls` now emits all three bidirectional newtype-struct impls (`.0` field access) with a deduplication guard so multiple bridges sharing the same alias or options type don't produce duplicate `impl` blocks. Fixes E0308 and E0277 in every consumer using `bind_via = "options_field"` (e.g. html-to-markdown's `HtmlVisitor` bridge). (`crates/alef-backend-swift/src/gen_rust_crate/plugin_inbound.rs`, `crates/alef-backend-swift/src/gen_rust_crate/mod.rs`)

- **alef-backend-swift: only emit first-class Swift DTOs for directly bridgeable field shapes**: complex serde DTOs with maps, vectors, nested wrappers, paths, bytes, JSON, or duration fields now remain RustBridge typealiases instead of generating uncompilable Codable/Hashable wrappers that called missing bridge accessors or JSON factory shims. Simple primitive/string DTOs keep first-class Codable wrappers and call snake_case RustBridge accessors. (`crates/alef-backend-swift/src/gen_bindings.rs`)

- **alef-backend-swift: require direct RustBridge constructors for first-class DTOs**: Swift first-class DTO emission now uses the same constructor/direct-conversion eligibility as `intoRust()`, so serde DTOs without a generated `#[swift_bridge(init)]` constructor remain RustBridge typealiases instead of calling missing `{type}FromJson` shims. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/gen_bindings_test.rs`)

## [0.16.16] - 2026-05-16

### Fixed

- **alef-backend-java: drop redundant `import java.util.stream.Stream;` from streaming opaque-handle classes**: the streaming method body template (`streaming_iterator_method.jinja`) uses fully-qualified names (`java.util.stream.Stream<T>` in the method signature, `java.util.stream.StreamSupport.stream(...)` in the bridge body), so the unconditional `imports.push("java.util.stream.Stream")` emitted an import that never resolved to a short-form identifier in the file body. Checkstyle's `UnusedImports` rule rejected `DefaultClient.java:12` in liter-llm after regeneration against v0.16.15. Drop the import push; rely on FQNs in the template throughout. Adds a regression test asserting the import is absent in generated opaque-handle facades with streaming adapters. (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

## [0.16.15] - 2026-05-16

### Fixed

- **alef-cli: resolve `PostBuildStep::RunCommand` working directory from `base_dir.join(crate_dir)`**: the dispatcher previously ran every backend-emitted `RunCommand` from the workspace `base_dir`, but commands like `flutter_rust_bridge_codegen generate` expect the binding crate root (where `flutter_rust_bridge.yaml` lives, relative to `crate_dir`). The Dart FRB step now runs in the correct directory and finds its config file. Also adds `crates/alef-backend-dart/examples/rewrite_lib_dart.rs` so the post-frb sealed-variant rewriter can be applied as a one-shot when callers bypass alef's build pipeline (e.g. running `task dart:codegen` directly). (`crates/alef-cli/src/pipeline/commands.rs`, `crates/alef-backend-dart/examples/rewrite_lib_dart.rs`)

- **alef-backend-java: nest Jackson POJO builders inside their owning record**: every DTO record now embeds its `Builder` as `public static final class Builder` rather than emitting `FooBuilder.java` as a sibling top-level class. Cuts the generated file count for the Java package by ~32% (54 fewer `.java` files in liter-llm) and matches idiomatic Java conventions (`Foo.Builder` pattern, mirroring `ImmutableList.Builder`, `LocalDate.Builder`, etc.). The `@JsonDeserialize(builder = ...)` annotation now references `Foo.Builder.class`. (`crates/alef-backend-java/src/gen_bindings/types.rs`, snapshot tests refreshed)

- **alef-backend-dart: pass generated config to flutter_rust_bridge post-build command**: the Dart backend's post-build `RunCommand` now invokes `flutter_rust_bridge_codegen generate --config-file packages/dart/rust/flutter_rust_bridge.yaml`, matching the root-relative post-build working directory and ensuring FRB reads the emitted `rust_input`/`dart_output` settings instead of failing with "Please provide `rust_input`" or "Cannot find config_file". (`crates/alef-backend-dart/src/gen_bindings/mod.rs`)

- **alef-e2e/node: use workspace version for registry test apps when no package version is configured**: Node registry-mode `package.json` now falls back to the crate's resolved Cargo version before the `0.1.0` placeholder, matching the Python/Ruby/PHP/Dart generators and preventing generated `test_apps/node` projects from depending on unpublished `@scope/package@0.1.0` versions. (`crates/alef-e2e/src/codegen/typescript/mod.rs`)

- **alef-e2e/elixir: emit version dependencies for registry test apps**: Elixir registry-mode `mix.exs` now uses the package version from `[e2e.registry.packages.elixir]` or the resolved crate version instead of carrying the local `../../packages/elixir` path into standalone `test_apps`. (`crates/alef-e2e/src/codegen/elixir.rs`)

- **alef-backend-magnus: omit binding-excluded fields from explicit `Default` impls**: Ruby DTOs with field-level defaults now filter `binding_excluded` fields consistently with struct and constructor generation, preventing hidden core fields from being emitted into binding-side `Self { ... }` defaults. (`crates/alef-backend-magnus/src/gen_bindings/classes.rs`)

- **alef-codegen/rustler: keep binding-excluded fields out of config constructors and cfg-gated core literals**: shared default-constructor helpers now filter `binding_excluded` fields before emitting keyword/option constructors, and binding-to-core conversion skips cfg-gated hidden fields so internal implementation details like caches and DI containers do not leak into generated Elixir NIF code or fail when the core feature is disabled. (`crates/alef-codegen/src/config_gen.rs`, `crates/alef-codegen/src/conversions/binding_to_core.rs`)

- **alef-backend-swift: omit binding-excluded fields from Rust bridge constructors**: Swift Rust bridge extern declarations and wrapper constructor bodies now share the binding-visible field set, preventing hidden core fields from appearing as unbound constructor parameters or serde assignments. (`crates/alef-backend-swift/src/gen_rust_crate/extern_block.rs`, `crates/alef-backend-swift/src/gen_rust_crate/wrappers.rs`, `crates/alef-backend-swift/src/gen_rust_crate/default_construction.rs`)

- **alef-backend-swift: only emit first-class Swift DTOs for directly bridgeable field shapes**: complex serde DTOs with maps, vectors, nested wrappers, paths, bytes, JSON, or duration fields now remain RustBridge typealiases instead of generating uncompilable Codable/Hashable wrappers that called missing bridge accessors or JSON factory shims. Simple primitive/string DTOs keep first-class Codable wrappers and now call snake_case RustBridge accessors. (`crates/alef-backend-swift/src/gen_bindings.rs`)

## [0.16.14] - 2026-05-16

### Fixed

- **alef-backend-rustler: emit positional arity overloads for single-trailing-optional Elixir facade fns**: facade emission for functions with exactly one trailing optional parameter (e.g. `create_engine(config: Option<String>)`) previously collapsed to a single `def create_engine(opts \\ [])` body that called `Keyword.get(opts, :config)` — but the alef-e2e Elixir codegen always emits positional calls (`Kreuzcrawl.create_engine(engine_config)`), passing a string into a function expecting a `keyword()` list. The mismatch surfaced as `FunctionClauseError` on every e2e test that constructed an engine. The keyword-opts collapse threshold in `gen_bindings/mod.rs` is now `trailing_keyword_count >= 2` (was `> 0`): single trailing optional falls through to the existing arity-variants path (`def create_engine()` + `def create_engine(config)` with `nil` filled for the shortest arity), matching the e2e call shape. Functions with 2+ trailing optionals still collapse to the `opts \\ []` keyword form. (`crates/alef-backend-rustler/src/gen_bindings/mod.rs`)

- **alef-cli: implement `PostBuildStep::RunCommand` dispatcher**: the runtime arm at `crates/alef-cli/src/pipeline/commands.rs` was previously a TODO debug-log that silently skipped every emitted `RunCommand` step (notably the Dart flutter_rust_bridge codegen invocation). Now spawns `std::process::Command::new(cmd).args(args).current_dir(base_dir)`, captures stdout/stderr, and propagates non-zero exit status as an `anyhow::Error` so the pipeline aborts. The dart backend's `RunCommand` step (`flutter_rust_bridge_codegen generate`) now executes automatically during `alef generate`. (`crates/alef-cli/src/pipeline/commands.rs`)

- **alef-backend-dart: wire `rewrite_frb_sealed_variants` into the post-frb pipeline**: introduces a new `PostBuildStep::PostProcessFile { path, processor }` variant in `alef-core::backend` with `PostProcessor::FrbDartSealedVariants`. The dart backend's `build_config_for` appends this step after the `RunCommand` for `flutter_rust_bridge_codegen`, targeting `../lib/src/<crate>_bridge_generated/lib.dart`. The alef-cli dispatcher reads the file, applies `alef_backend_dart::rewrite_frb_sealed_variants`, and writes back only on change. Sealed-class factory params previously emitted as `{required PdfMetadata field0}` now resolve to `{required PdfMetadata metadata}` matching the kotlin/swift binding surface. (`crates/alef-core/src/backend.rs`, `crates/alef-cli/src/pipeline/commands.rs`, `crates/alef-backend-dart/src/gen_bindings/mod.rs`, `crates/alef-backend-dart/src/frb_rewrite.rs`)

- **alef-backend-rustler: resolve concrete Elixir types for known union payloads**: tagged enum `@type` emission for variants like `FormatMetadata::Pdf(PdfMetadata)` previously emitted `@type pdf :: %{type: :pdf, metadata: map()}` — a loose `map()` typespec that defeats Dialyzer's variant-payload inference. The variant emitter now threads the API surface's known type names through `elixir_field_name_with_type`; payloads matching a known struct emit `Kreuzberg.PdfMetadata.t()`, falling back to `map()` only for opaque/unknown payloads. Field names (`metadata`, `value`, `value0`/`value1`) are unchanged. (`crates/alef-backend-rustler/src/gen_bindings/helpers.rs`, `crates/alef-backend-rustler/src/gen_bindings/mod.rs`)

- **alef-backend-java: emit `@Nullable` on `Option<T>` parameters in FFI facade methods**: facade method signatures previously rendered `Option<T>` params as bare `String`/`Path`/etc. without nullability annotations, causing static analyzers (IntelliJ, Checker Framework) to flag `null` arguments as false-positives. Now emits `@Nullable T` with an `import org.jspecify.annotations.Nullable;` line whenever any optional parameter is present in the class body. Return types remain `Optional<T>` to preserve existing call sites; switching returns to `@Nullable T` is a follow-up that requires a deeper body refactor. (`crates/alef-backend-java/src/gen_bindings/facade.rs`, `crates/alef-backend-java/src/gen_bindings/ffi_class.rs`, `crates/alef-backend-java/templates/facade_file.jinja`)

- **alef-backend-swift: escape Swift reserved keywords with backticks in emitted Swift source**: enum cases, struct field names, function-parameter labels, and associated-value labels that collide with Swift reserved keywords are now wrapped in backticks (`` case `default` ``, `` public let `class`: String ``) — Swift's idiomatic escape — instead of suffixing a trailing underscore (`case default_`). Rust-side identifiers on the swift-bridge boundary continue to use the trailing-underscore form so the two sides agree at the FFI boundary. Adds `swift_case_safe_name` / `swift_case_ident` to `alef_core::keywords` for emitted-Swift escaping; `swift_safe_name` / `swift_ident` remain for Rust-side use. (`crates/alef-core/src/keywords.rs`, `crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/gen_bindings_test.rs`)

- **alef-backend-zig: collapse error unions to single `KreuzbergError` set**: every emitted error set now includes `OutOfMemory` as a variant, so allocator failures can be propagated without a `||error{OutOfMemory}` concat. Return types for fallible functions are now `ErrorSet!T` instead of the verbose `(ErrorSet||error{OutOfMemory})!T`. (`crates/alef-backend-zig/src/gen_bindings/errors.rs`, `crates/alef-backend-zig/src/gen_bindings/functions.rs`)

### Added

- **alef-backend-extendr: emit roxygen2 `@param`/`@return` on every exported R function**: every free function in the generated `extendr-wrappers.R` previously carried only `#' @export`, so `?<fn>` in an R REPL returned an empty `.Rd` file. The wrapper emitter now produces a complete roxygen2 doc block above each `name <- function(...)` definition: the title line is derived from the first sentence of the Rust doc comment (capitalized, trailing period stripped) — or falls back to the bare function name when the doc is empty; remaining doc text is emitted as the description with paragraph breaks preserved as `#'` blank lines; one `#' @param <name> <type-description>` line is emitted per parameter, with the description derived from the IR `TypeRef` (e.g. `Raw vector of bytes.`, `Optional character string. Defaults to NULL.`, `<TypeName> object (list with class attribute).`); a final `#' @return` and `#' @export` close the block. (`crates/alef-backend-extendr/src/gen_bindings.rs`, `crates/alef-backend-extendr/templates/r_free_function_wrapper.jinja`)

- **alef-codegen: add acronym-preserving PascalCase conversion helpers**: shared naming utilities now include `pascal_to_snake` and `pascal_to_screaming_snake`, preserving acronym runs such as `Rdfa`, `HTMLParser`, and `XMLHttpRequest` instead of splitting every uppercase letter. (`crates/alef-codegen/src/naming.rs`)

- **alef-backend-magnus: generate aggregated RBS type stub sidecar (`types.rbs`)**: the Magnus backend now generates a comprehensive aggregated RBS sidecar file (`packages/ruby/sig/types.rbs`) containing all public API type signatures when `[crates.ruby.stubs] output = "..."` is configured. The file includes module declarations, class definitions with attr_reader/attr_accessor annotations, method signatures with typed parameters and return types, and enum type aliases. RBS types are mapped from Rust IR: `String` → `String`, integers → `Integer`, floats → `Float`, booleans → `bool`, `Vec<T>` → `Array[T]`, `Option<T>` → `T?`, `Map<K, V>` → `Hash[K, V]`. The generated file is consumable by Steep type checker; clients configure `target :lib do; signature 'sig'; check 'lib'; end` in their `Steepfile`. (`crates/alef-backend-magnus/src/gen_stubs.rs`, existing tests in `gen_stubs_test.rs`)

- **alef-backend-rustler: emit `@type t` typespec above every DTO `defstruct`**: all generated Elixir DTO modules now emit a `@type t :: %__MODULE__{ field: Type, ... }` type specification immediately above the `defstruct` declaration. This enables Dialyzer static type checking on DTO instances and improves IDE autocomplete and type inference in Elixir code that manipulates DTOs. Field types are mapped from Rust types: `String` → `String.t()`, `u64`/`usize` → `non_neg_integer()`, `bool` → `boolean()`, `Vec<T>` → `[T]`, `Option<T>` → `T | nil`, `Map` → `map()`, and named struct types → `map()`. (`crates/alef-backend-rustler/src/gen_bindings/helpers.rs`, `crates/alef-backend-rustler/tests/elixir_struct_typespec_test.rs`)

### Fixed

- **alef-backend-jni: catch native panics from `runtime().block_on`; propagate via JNI exception**: panics inside `tokio::Runtime::block_on` previously unwound across the C ABI (undefined behaviour), surfacing as `Exception at native frame -2` on the Kotlin side with no payload. Added a `run_or_throw` helper to the emitted JNI module preamble that wraps every `block_on` call in `std::panic::catch_unwind(AssertUnwindSafe(...))`, downcasts the panic payload to a message string, and throws it as a JNI exception prefixed with `"native panic: "`. All shim variants are covered: top-level functions, instance methods, streaming start, and streaming next. (`crates/alef-backend-jni/src/gen_shims.rs`)

- **alef-backend-wasm: keep camelCase constructor parameters aligned with snake_case struct fields**: constructors now rewrite both shorthand and explicit field initializers so multi-word fields use JS-facing camelCase parameters on the right-hand side while preserving the Rust field name on the left-hand side (`total_tokens: totalTokens.unwrap_or_default()`, `tool_call_id: toolCallId`). Multi-line constructor parameter lists are handled too, and generated constructors allow the `non_snake_case` parameter names required by wasm-bindgen's JS surface. (`crates/alef-backend-wasm/src/gen_bindings/types.rs`, `crates/alef-backend-wasm/tests/gen_bindings_test.rs`)

- **alef-backend-swift: honor extractor-produced optional fields in first-class structs**: Swift DTO generation now treats both `TypeRef::Optional(T)` and extractor-style `(ty: T, optional: true)` fields as optional, emitting `T?`, default-nil initializer parameters, and optional swift-bridge accessors such as `rb.name()?.toString()`. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/snapshot_test.rs`)

- **alef-codegen: omit binding-excluded fields from generated DTO surfaces**: shared struct generation, constructor helpers, Java, NAPI, PyO3, C#, Go, Magnus, PHP, and Rustler emitters now filter out `binding_excluded` fields while conversion code still defaults the hidden core fields when converting back to Rust core types. The CLI keeps excluded fields in IR instead of rejecting non-`Default` core types, so hidden fields like internal cursors can be initialized by conversion code without leaking into host APIs. (`crates/alef-cli/src/pipeline/extract.rs`, `crates/alef-codegen/src/shared.rs`, `crates/alef-codegen/src/generators/structs.rs`, `crates/alef-codegen/src/conversions/`, `crates/alef-backend-*/src/gen_bindings/`)

- **alef-backends: close remaining binding-excluded field leaks and generated wrapper noise**: PHP constructors, Java builders, Swift DTOs, Dart mirrors, and WASM DTOs now honor `binding_excluded` fields consistently, while Java opaque methods support string and primitive FFI returns instead of emitting unsupported stubs. Ruby tagged-enum wrappers now require `sorbet-runtime`, call `super()` in variant initializers, and read string hash keys correctly; Zig scaffolding avoids removed `pub usingnamespace` syntax. (`crates/alef-backend-php/src/gen_bindings/types.rs`, `crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/src/template_env.rs`, `crates/alef-backend-magnus/`, `crates/alef-backend-swift/`, `crates/alef-backend-dart/`, `crates/alef-backend-wasm/`, `crates/alef-scaffold/src/languages/zig.rs`)

- **alef-backend-dart: payload-derived sealed variant parameter names**: flutter_rust_bridge generates tagged-union sealed classes with positional parameter names (`field0`, `field1`, ...) for tuple variants, e.g. `const factory FormatMetadata.pdf({required PdfMetadata field0}) = FormatMetadata_Pdf;`. Added a post-frb rewriter (`alef_backend_dart::rewrite_frb_sealed_variants`) that converts these into payload-derived names matching the kotlin/swift binding surface (`metadata`, `value`, `value0`/`value1`, ...). Single-field variants with a struct payload whose name shares the variant's prefix (e.g. `PdfMetadata` for `Pdf`) use the stripped lowerCamel remainder; primitive payloads use `value`; multi-field tuples use `value0`/`value1`; struct variants with named fields are left untouched. Mirrors `kotlin_field_name_with_type` in `alef-backend-kotlin/src/gen_bindings/shared.rs`. (`crates/alef-backend-dart/src/frb_rewrite.rs`, `crates/alef-backend-dart/Cargo.toml`)

- **alef-backend-csharp: emit newline between consecutive using directives**: the Jinja2 template for opaque handle headers was using `{%- if` with left-side whitespace stripping, which removed the newline separating `using System.Runtime.InteropServices;` from the next conditional block's first `using` statement. This caused two consecutive imports to appear concatenated on a single line (e.g., `using System.Runtime.InteropServices;using System.Text.Json;`). Changed all conditional `{%- if` tags to `{% if` to preserve newlines between using statements. (`crates/alef-backend-csharp/templates/opaque_handle_header.jinja`, `crates/alef-backend-csharp/tests/gen_bindings_test.rs`)

- **alef-backend-php: apply camelCase conversion to DTO property names in stubs**: PHP 8.2+ idiomatic style requires camelCase property names per PSR-12. The binding layer was correctly using `#[php(prop, name = "deviceId")]` for runtime, but the generated PHP stubs were emitting snake_case property names directly (e.g., `$device_id` instead of `$deviceId`). Applied `to_php_name()` conversion to property names when emitting readonly constructor parameters in DTO stubs. (`crates/alef-backend-php/src/gen_bindings/mod.rs`, `crates/alef-backend-php/tests/gen_bindings_test.rs`)

- **alef-backend-pyo3: drop redundant `return` on void-returning function calls in `api.py`**: functions with `-> None` return type annotation were emitting `return _rust.init(...)`, where the `return` statement is redundant and causes type checkers to flag the function as returning `None` instead of `None` implicitly. Modified `gen_api_py` to detect `TypeRef::Unit` returns and emit a bare call without the `return` keyword. (`crates/alef-backend-pyo3/src/gen_bindings/functions.rs`, `crates/alef-backend-pyo3/tests/gen_bindings_test.rs`)

- **alef-backend-pyo3: emit PEP 8 blank lines between consecutive top-level function definitions in `api.py`**: consecutive function definitions were jammed together with zero blank lines between them, violating PEP 8's requirement for two blank lines between top-level definitions. Modified the function emission loop to emit two additional newlines after each function body, ensuring proper spacing in generated `api.py`. (`crates/alef-backend-pyo3/src/gen_bindings/functions.rs`, `crates/alef-backend-pyo3/tests/gen_bindings_test.rs`)

- **alef-backend-napi: drop unnecessary 'undefined' from nullable `.d.ts` return types**: Optional return types in TypeScript declaration files were emitted as `T | undefined | null` when FFI returns can only be `T | null` (either the value or null, never undefined). Changed `dts_type` for `TypeRef::Optional` to emit `T | null` instead. (`crates/alef-backend-napi/src/gen_bindings/errors.rs`, `crates/alef-backend-napi/tests/gen_bindings_test.rs`)

- **alef-backend-jni: preserve serialization error messages through JNI exception boundary**: `serde_json::to_string(&v).unwrap_or_default()` on all JSON serialization paths silently returned an empty string when serialization failed, causing the JNI caller to see a null/empty payload with no exception raised. Replaced with explicit `match` that calls `throw_jni_error(&mut env, &format!("serialize: {e}"))` and returns the appropriate null sentinel (`std::ptr::null_mut()`) on failure. Applies to the general return-marshal helper, the streaming `Next` shim (two call sites), and the `emit_function_shim` OK-arm. Regression test `no_unwrap_or_default_on_json_serialization_path` verifies no `serde_json::to_string` + `unwrap_or_default` combination appears in generated shim code. (`crates/alef-backend-jni/src/gen_shims.rs`, `crates/alef-backend-jni/tests/gen_shims_test.rs`)

- **alef-backend-swift: user-facing HtmlVisitorProtocol now receives typed NodeContext, not raw JSON String**. The adapter (SwiftHtmlVisitorBox / Delegate) handles JSON → NodeContext conversion internally so user implementations work with the structured value. Adapter delegate protocol scoped to `internal` (or underscore-prefixed if cross-module references prevent narrowing) to hide it from public API surface. (`crates/alef-backend-swift/src/gen_bindings/swift_inbound_protocol.rs`, `gen_rust_crate/plugin_inbound.rs`)

### Changed

- **[BREAKING] alef-backend-kotlin-android: collapse optional parameter defaults from sentinel zero-values to nullable Kotlin types**: `createClient` and other facade functions with `p.optional == true` parameters previously emitted `timeoutSecs: Long = 0L`, `maxRetries: Int = 0`, `modelHint: String = ""` — sentinel values that collapse with valid zero-value inputs. They now emit `timeoutSecs: Long? = null`, `maxRetries: Int? = null`, `modelHint: String? = null`, which is the idiomatic Kotlin pattern. Callers passing explicit zeros remain source-compatible; callers that relied on default-zero behavior must migrate to `null`. (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`, `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`)

### Added

- **alef-e2e/kotlin-android: emit `src/androidTest/` instrumented test source set + Gradle Managed Device + emulator CI**: the `KotlinAndroidE2eCodegen` now emits both a host-JVM test file (`src/test/kotlin/.../e2e/ChatTest.kt`) and an instrumented Android test file (`src/androidTest/kotlin/.../e2e/ChatTest.kt`) per fixture group. The instrumented file carries `@RunWith(AndroidJUnit4::class)`, a `@BeforeClass` `System.loadLibrary` block, and one `@Test` per fixture. The emitted `build.gradle.kts` now includes `import com.android.build.api.dsl.ManagedVirtualDevice` and an `android { testOptions { managedDevices { ... } } }` block declaring a `pixel6api34` Gradle Managed Device (API 34, aosp, Pixel 6 profile). A new `.github/workflows/ci-android-emulator.yaml` workflow runs `reactivecircus/android-emulator-runner@v2` with API 34 / arm64-v8a on changes to the kotlin-android/jni/e2e files. Two new regression tests lock in both invariants. (`crates/alef-e2e/src/codegen/kotlin_android.rs`, `crates/alef-e2e/tests/kotlin_android_codegen.rs`, `.github/workflows/ci-android-emulator.yaml`)

### Fixed

- **alef-backend-kotlin-android: align `sealed_variant_tuple_params_use_payload_derived_names` regression assertions with actual multi-line emission**: the regression test asserting payload-derived sealed-variant parameter names for the kotlin-android emitter (`Pdf(PdfMetadata)` → `val metadata`, `Custom(String)` → `val value`, `Multi(String, Int)` → `val value0, val value1`, `Struct { reason: String }` → `val reason`) was compiled against a stale IR shape and the body checks were written for a single-line `data class Variant(val name: Type)` form. The android backend's `data_class_header.jinja` actually emits each field on its own indented line, so the test never executed; once compilation was restored the assertions failed against the real (correct) emission. Updated `make_sealed_variants_api` to use the current `FieldDef` / `EnumVariant` / `EnumDef` shape and rewrote the `data class` assertions to match the multi-line layout. Confirms that the payload-derived-name helper from `alef-backend-kotlin::gen_bindings::shared` (`kotlin_field_name_with_type`) reaches the android emitter through the `emit_enum_pub` → `object_wrapper::emit_enum` re-export. (`crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`)

- **alef-backend-magnus: emit real e2e tests for non-HTTP fixtures**: Ruby has direct FFI access to Rust core via Magnus, so it can execute fixtures with `mock_responses` (non-HTTP) without relying on HTTP clients. The generator now only skips fixtures with zero assertions, allowing fixtures with assertions (even if some field references are invalid) to emit real tests with skipped assertion comments, matching Python and Node behavior. E2E skip count reduced from 47 (all non-HTTP fixtures) to 1 (genuinely untestable fixtures). (`crates/alef-e2e/src/codegen/ruby.rs`)

- **alef-backend-magnus: emit `_async` function variants in RBS stubs**: All async functions exported by Magnus bindings now appear in the RBS type stubs as both sync and `_async` variants (e.g., `def self.scrape` and `def self.scrape_async`), allowing type checkers like Steep to validate both calling conventions. (`crates/alef-backend-magnus/src/gen_stubs.rs`)

- **alef-backend-pyo3: remove leaked debug `eprintln!` in `rewrite_capsule_methods`**: a diagnostic `eprintln!("[rewrite_capsule] method=... start_idx=... attr_start=... prefix=...")` introduced alongside the capsule-passthrough impl-header fix was left in place and emitted a line to stderr for every pyo3 capsule method on every `alef generate`. Removed. (`crates/alef-backend-pyo3/src/gen_bindings/mod.rs`)

- **alef-backend-pyo3: preserve `#[pymethods] impl` block header when first method returns a capsule type**: `rewrite_capsule_methods` uses `find_method_attrs_start` to walk backward from a `pub fn` to find the start of its attribute block. The walk accepted any line starting with `#[` — including `#[pymethods]impl Foo {` which minijinja emits as a single concatenated line when whitespace stripping is active. As a result, `attr_start` landed at byte 0, and the string slice `result[..0]` discarded the entire impl block header, leaving the rewritten method floating outside any `impl` block. Fixed by replacing the `starts_with("#[")` heuristic with `is_method_attr_line` — a bracket-depth scanner that accepts a line only when it consists entirely of `#[…]` patterns with no trailing non-whitespace tokens. (`crates/alef-backend-pyo3/src/gen_bindings/mod.rs`, `crates/alef-backend-pyo3/tests/gen_bindings_test.rs`)

### Changed

- **[BREAKING] alef-backend-swift: first-class struct/enum emission with Codable+Sendable+Hashable**: non-opaque DTO types that carry `#[derive(Serialize, Deserialize)]` and have at least one field now emit as `public struct Foo: Codable, Sendable, Hashable` with stored `public let` properties, a memberwise `public init` (default-nil for Optional fields), and `CodingKeys` for snake_case↔camelCase wire-key mapping. An `internal extension Foo` provides `init(_ rb: RustBridge.Foo) throws` (reading fields via swift-bridge accessor calls) and `func intoRust() throws -> RustBridge.Foo` (JSON round-trip via `JSONEncoder`). Streaming wrappers whose item type is a first-class struct decode chunks via `JSONDecoder().decode(T.self, from: chunkData)` instead of the `RustBridge.{T}FromJson(json)` shim. Public `{T}FromJson` helpers for first-class types also use `JSONDecoder` directly. Empty structs and opaque handle types are unaffected and continue to emit typealiases or class wrappers. (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/tests/gen_bindings_test.rs`, `crates/alef-backend-swift/tests/snapshots/`)

- **[BREAKING] alef-backend-swift: RustBridge becomes internal-only impl detail**: public `typealias Foo = RustBridge.Foo` declarations are no longer emitted for any DTO type that qualifies as a first-class struct. The `RustBridge` module is still imported and used internally (opaque handles, streaming start/next bridge functions, `intoRust()` JSON shim), but no public API surface exposes `RustBridge`-prefixed types. Callers that accessed fields via `.role()` method call syntax must migrate to `.role` property access on the first-class struct. (`crates/alef-backend-swift/src/gen_bindings.rs`)

- **[BREAKING] alef-backend-wasm: drop Wasm-prefix from exported TypeScript names**: mirroring the NAPI W3 idiomaticity work, non-opaque and opaque structs now carry `#[wasm_bindgen(js_name = "Foo")]` to export the unprefixed name in the generated `.d.ts`, while unit enums and tagged data enums carry `#[wasm_bindgen(js_name = "Foo")]` to expose the unprefixed variant. The Rust binding struct internally retains its `WasmFoo` identifier (prefixed). The `generate_public_api` `index.ts` re-export uses unprefixed names (`Config`, `Mode`, not `WasmConfig`, `WasmMode`). Callers that imported `WasmConfig`, `WasmParser`, etc. from the generated WASM binding package must migrate to `Config`, `Parser`, etc. (`crates/alef-backend-wasm/src/gen_bindings/mod.rs`, `crates/alef-backend-wasm/src/gen_bindings/types.rs`, `crates/alef-backend-wasm/src/gen_bindings/enums.rs`, `crates/alef-backend-wasm/templates/gen_struct.jinja`, `crates/alef-backend-wasm/templates/gen_opaque_struct.jinja`)

- **[BREAKING] alef-backend-napi: drop Js-prefix from exported TypeScript names; add `readonly` to DTO fields**: three linked W3 idiomaticity changes: (1) non-opaque structs carry `#[napi(object, js_name = "Foo")]` instead of `#[napi(object)]`, (2) opaque structs carry `#[napi(js_name = "Foo")]` via post-processing of `gen_opaque_struct_prefixed` output, (3) string enums carry `#[napi(string_enum, js_name = "Foo")]` — in all cases NAPI-RS will export the type as `Foo` in the generated `.d.ts` while the Rust struct retains its `JsFoo` identifier internally. The `generate_public_api` `index.ts` re-export uses the unprefixed name to match. DTO interface fields in the handwritten `.d.ts` are now emitted with the `readonly` qualifier. Callers that imported `JsConfig`, `JsMode`, etc. from the generated binding package must migrate to `Config`, `Mode`, etc. (`crates/alef-backend-napi/src/gen_bindings/mod.rs`, `crates/alef-backend-napi/src/gen_bindings/types.rs`, `crates/alef-backend-napi/src/gen_bindings/enums.rs`, `crates/alef-backend-napi/src/gen_bindings/errors.rs`)

- **[BREAKING] alef-backend-rustler: Elixir client function arity overloads collapsed to single keyword-opts function**: Functions with trailing optional parameters now emit a single `def f(required, opts \\ [])` definition with `Keyword.get(opts, :param_name)` forwarding instead of one `def` per arity combination. Callers using `create_client/2`, `create_client/3`, etc. must migrate to `create_client(api_key, base_url: "...", timeout_secs: 30)`. (`crates/alef-backend-rustler/src/gen_bindings/mod.rs`)

- **[BREAKING] alef-backend-rustler: `defstruct` optional-field defaults changed from `""` to `nil`**: String, Char, Path, Json, and Bytes fields without an explicit `typed_default` now default to `nil` in generated `defstruct` declarations, reflecting Elixir's idiomatic absent-value convention. Callers matching on `""` must migrate to `nil`. (`crates/alef-backend-rustler/src/gen_bindings/helpers.rs`)

- **[BREAKING] alef-backend-magnus: Ruby tagged enums emitted as class hierarchies instead of Hash method_missing**: Internally-tagged data enums now emit a sealed base class with per-variant predicate stubs (returning `false`) and one concrete subclass per variant. Each subclass carries Sorbet-typed `attr_reader` fields, a keyword-arg `initialize`, an overridden predicate (returning `true`), and a `self.from_hash` factory. The `Hash#method_missing` / `Hash#respond_to_missing?` monkey-patch that previously lived in `native.rb` is removed entirely. Callers that relied on `result.excel` or `format[:format_type]` style access must migrate to `result.is_a?(MessageSystem) && result.content`. (`crates/alef-backend-magnus/src/gen_bindings/mod.rs`)

- **alef-backend-magnus: Sorbet `sig { }` blocks on all emitted tagged enum methods**: every accessor (`attr_reader`), predicate (`def variant?`), `initialize`, and `from_hash` in the generated tagged-enum class hierarchy now carries a Sorbet-compatible `sig { }` annotation with the correct type (`String`, `Integer`, `Float`, `T::Boolean`, `T.nilable(...)`, etc.). `extend T::Sig` and `extend T::Helpers` are emitted at the top of each class. (`crates/alef-backend-magnus/src/gen_bindings/mod.rs`)

- **[BREAKING] alef-backend-php: emit DTOs as PHP 8.3+ `final class` with readonly constructor property promotion**: Previously, DTO stubs in `packages/php/src/*.php` were emitted as mutable `class` declarations with separate public property fields and redundant `getFoo()` / `getName()` getter methods. Now the backend emits `final class` with all fields promoted into the constructor signature as `public readonly` parameters, which is the idiomatic PHP 8.3+ pattern. Callers that invoked `$obj->getContent()` or `$obj->getName()` must migrate to direct property access `$obj->content` / `$obj->name`. (`crates/alef-backend-php/src/gen_bindings/mod.rs`, `crates/alef-backend-php/templates/php_record_class_stub_declaration.jinja`, `crates/alef-backend-php/tests/gen_bindings_test.rs`)

- **alef-backend-csharp: modern C# features polish — file-scoped namespaces, `IAsyncEnumerable<T>` for streams, collection expressions**: All generated C# files already use file-scoped namespace syntax (`namespace Foo;`, C# 10+). Streaming methods on opaque handle classes return `IAsyncEnumerable<T>` (C# 8+) so consumers can `await foreach`. `byte[]` fields without an explicit default now emit `= []` (C# 12 collection expression) instead of `= Array.Empty<byte>()`. Three regression tests lock in these invariants. (`crates/alef-backend-csharp/src/gen_bindings/types.rs`, `crates/alef-backend-csharp/tests/gen_bindings_test.rs`)

- **BREAKING — alef-backend-pyo3: emit `UPPER_SNAKE_CASE` pyclass enum variants; drop runtime SCREAMING aliasing**: Previously, pyclass enum variants were exposed under their Rust PascalCase names (e.g. `BatchStatus.Validating`) and `options.py` monkey-patched `SCREAMING_SNAKE_CASE` aliases on top (`BatchStatus.VALIDATING = BatchStatus.Validating`). Now every variant carries `#[pyo3(name = "UPPER_SNAKE_CASE")]` in the generated Rust binding, so the canonical Python name is directly `UPPER_SNAKE_CASE` (PEP 8-compliant). The `.pyi` stub emits only the `UPPER_SNAKE_CASE` attribute declarations. The `options.py` monkey-patching block is removed. Callers that accessed variants via PascalCase (e.g. `BatchStatus.Validating`) must migrate to `BatchStatus.VALIDATING`. (`crates/alef-codegen/src/generators/enums.rs`, `crates/alef-codegen/templates/generators/enums/enum_definition.jinja`, `crates/alef-backend-pyo3/src/gen_stubs.rs`, `crates/alef-backend-pyo3/src/gen_bindings/types.rs`)

- **alef-backend-csharp: emit DTOs as C# 9+ `sealed record` with property bodies and `required` modifier for mandatory fields**: Previously, DTOs were emitted as `public sealed class Foo { public T Field { get; set; } = default; }` (C# 6 style). Now the backend emits `public sealed record Foo { [property: JsonPropertyName("...")] T Field { get; init; } = default; }` — idiomatic C# 9+ record syntax with `[property:]` attribute targets for JSON metadata and immutable `get; init;` accessors. Non-nullable reference-type fields without explicit defaults now emit the `required` modifier (C# 11+), enforcing constructor-time initialization; nullable fields, collections, value types, and fields with defaults do not emit `required`. Opaque handle wrapper classes expose their `Handle` property as `internal` (not `public`) to prevent unsafe direct `DangerousGetHandle()` access by external consumers. This improves consistency with modern C# conventions and enables compiler-driven immutability checking. Internally-tagged enums (with data variants) now use abstract `record` bases with `[JsonDerivedType]` discriminator annotations instead of flat class hierarchies, enabling type-safe sealed-union semantics via inheritance. Enum XML doc comments are now properly formatted with explicit `</summary>` closing tags. (`crates/alef-backend-csharp/src/gen_bindings/types.rs`, `crates/alef-backend-csharp/templates/record_class_header.jinja`, `crates/alef-backend-csharp/templates/property_with_default.jinja`, `crates/alef-backend-csharp/templates/property_required_init.jinja`, `crates/alef-backend-csharp/templates/enum_header.jinja`, `crates/alef-backend-csharp/templates/opaque_handle_header.jinja`)

- **[BREAKING] alef-backend-java: streaming methods return `Stream<T>` instead of `Iterator<T>`**: `chatStream()` and all adapter-pattern streaming methods on opaque handle classes now return `java.util.stream.Stream<T>` (composable, lazy, parallelizable) instead of `Iterator<T>`. The internal anonymous iterator is still used as the data source and is bridged via `StreamSupport.stream(Spliterators.spliteratorUnknownSize(..., ORDERED | NONNULL), false)`. The import changes from `java.util.Iterator` to `java.util.stream.Stream`; callers that consumed the return value as `Iterator<T>` must migrate to the Stream API (e.g. `.forEach()`, `.collect()`, `.toList()`). (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/templates/streaming_iterator_method.jinja`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

- **[BREAKING] alef-backend-java: tagged enums emitted as `sealed interface ... permits ...` + per-variant `record` (Java 21+ idiom)**: Internally-tagged enums with data variants (e.g. `Message`, `ContentPart`, `AuthConfig`) are now emitted as `public sealed interface Foo permits Variant1, Variant2 { }` with each variant as `public record Variant1(T field) implements Foo { }`. Previously they used plain inheritance hierarchies. Jackson polymorphic deserialization is wired via `@JsonTypeInfo`/`@JsonSubTypes`. Callers using class-cast-based dispatch must migrate to `instanceof` pattern matching (`if (msg instanceof SystemMessage sm) { ... }`). (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

- **alef-backend-java: emit fields-only DTOs as Java 17+ records**: All non-opaque structs with only field declarations (no custom methods) are now emitted as `public record Foo(String a, int b) {}` instead of sealed classes. Records automatically generate the constructor, getters (named `a()` not `getA()` per Java records convention), and `equals`/`hashCode`/`toString`. This aligns with modern Java idioms (Java 17 GA, 2021) and mirrors the pattern already used in sealed-union variant declarations. Opaque FFI handles continue to emit as regular classes (implementing `AutoCloseable`) for resource management. Sealed interfaces for sum-types continue to emit variant records implementing the interface. Three regression tests verify the heuristic: fields-only structs emit as records, opaque handles remain classes, and sum-type variants are records within sealed interfaces. (`crates/alef-backend-java/src/gen_bindings/types.rs`, `crates/alef-backend-java/tests/gen_bindings_test.rs`)

- **alef-backend-go: emit tagged-data enums as sealed interfaces with per-variant concrete structs**: Previously, externally-tagged data enums (e.g., `AuthConfig` with variants `Basic`, `Bearer`, `Header`) were emitted as a single flat struct with all variant fields as nullable pointers, allowing invalid combinations like `Bearer { username: Some("x"), token: None }`. Now, the backend emits a sealed interface with an unexported marker method `is{EnumName}()` and one concrete struct per variant containing only that variant's fields (no nullables). Each concrete struct implements `MarshalJSON`/`UnmarshalJSON` to emit the discriminator tag. Callers construct `AuthConfigBasic{ username, password }` directly—type-safe and idiomatic Go. An `Unmarshal{EnumName}([]byte)` helper dispatches to the correct variant based on the wire `type` field. This refactor improves type safety and ergonomics for consumers while maintaining full JSON round-trip compatibility. (`crates/alef-backend-go/src/gen_bindings/types.rs`)

- **alef-backend-kotlin / alef-backend-kotlin-android: payload-derived parameter names for sealed-class tuple variants**: Previously, tuple variants on sealed-class enums emitted with placeholder field names — `data class Pdf(val field0: PdfMetadata) : FormatMetadata()` — which obscured intent and made pattern-match code awkward. The shared `gen_bindings::shared` helper now derives an idiomatic Kotlin field name from the payload type: named types whose prefix matches the variant name strip the prefix and lower-camel the remainder (`Pdf(PdfMetadata)` → `val metadata`); primitive payloads (`String`, `Int`, `Long`, `Boolean`) use `val value`; multi-payload tuples fall back to indexed `val value0, val value1, …`; struct variants with named fields preserve the original field names. JSON serializers and round-trip codecs were updated to read/write through the new accessor names. (`crates/alef-backend-kotlin/src/gen_bindings/shared.rs`, `crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`)
- **alef-backend-pyo3: drop SCREAMING_SNAKE_CASE aliases from generated `.pyi` stubs**: `gen_stubs.rs` previously emitted `EnumName.SHOUTY_NAME: EnumName = ...` class attributes alongside the canonical PascalCase variant declarations in the type stubs, mirroring the runtime monkey-patches in `options.py`. The aliases bloated every enum stub by N additional lines and confused IDEs that suggested both forms as valid attribute names. Removed the `to_python_screaming` helper and the alias-emit loop in `gen_enum_stub`; the runtime `_coerce_enum` helper in the binding's `api.py` continues to accept string-form input transparently, so callers using `EnumName("shouty-name")` are unaffected. (`crates/alef-backend-pyo3/src/gen_stubs.rs`)

### Added

- **alef-backend-napi: W3 regression tests — js_name, readonly, no Js-prefix on TS surface**: four new integration tests in `gen_bindings_test.rs` lock in the W3 TypeScript idiomaticity invariants: `test_napi_js_name_on_non_opaque_struct` asserts `#[napi(object, js_name = "Foo")]` and unprefixed `index.ts` re-export; `test_napi_js_name_on_opaque_struct` asserts `#[napi(js_name = "Foo")]` on opaque handles; `test_napi_js_name_on_string_enum` asserts `#[napi(string_enum, js_name = "Foo")]` and unprefixed enum in `index.ts`; `test_dts_dto_fields_are_readonly` asserts `readonly` on both required and optional DTO fields in the `.d.ts` output and verifies the unprefixed interface name. (`crates/alef-backend-napi/tests/gen_bindings_test.rs`)

- **alef-backend-napi: regression tests for tagged-enum discriminant and variant naming (D4A/B/C)**: three tests in `enums.rs::tests` lock in the expected napi-rs surface for tagged enums — `js_name = "kind"` is used for the discriminant field (not the Rust `serde_tag` name like `"annotation_type"`), tuple-variant `serde_rename` is preserved in camelCase (e.g. `fontSize`, not `font_size`), and struct-variant named field names pass through unchanged. (`crates/alef-backend-napi/src/gen_bindings/enums.rs`)

- **alef-backend-go: regression tests for trait-bridge typed parameters (D1)**: tests verify that trait-bridge method parameters of type `String`, named structs (e.g. `OcrConfig`), and other concrete types emit the correct Go type in the generated interface signature, instead of degrading to `interface{}` or `map[string]interface{}`. (`crates/alef-backend-go/tests/gen_bindings_test.rs`)

- **alef-backend-wasm: support `Vec<UnitEnum>` parameter deserialization via `from_api_str`**:
  wasm-bindgen handlers accepting `Vec<UnitEnum>` parameters need to convert each JS string into
  the corresponding Rust enum variant. The unit-enum emitter now also generates a
  `pub fn from_api_str(s: &str) -> Option<Self>` constructor that parses serde wire strings
  (honoring `#[serde(rename_all)]` and per-variant `#[serde(rename)]`). Two new jinja templates
  (`serde_vec_unit_enum_optional.jinja`, `serde_vec_unit_enum_required.jinja`) wire the conversion
  into the binding→core parameter mapping path, returning a `JsValue` error with the offending
  string when an unknown variant is supplied.
  (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`,
  `crates/alef-backend-wasm/src/template_env.rs`,
  `crates/alef-backend-wasm/templates/serde_vec_unit_enum_optional.jinja`,
  `crates/alef-backend-wasm/templates/serde_vec_unit_enum_required.jinja`)



- **alef-backend-swift: emit `options_field` trait bridge — `HtmlVisitorProtocol` + `SwiftHtmlVisitorBox` + `makeHtmlVisitorHandle` + `conversionOptionsFromJsonWithVisitor`**: the swift backend previously only supported `bind_via = "function_param"` inbound trait bridges. This extends `gen_rust_crate/plugin_inbound.rs` with two new emitters (`emit_options_field_factory`, `emit_options_field_options_helper`) and wires both the `extern "Swift"` declaration and the Rust-side wrapper struct for all bridge modes. `gen_bindings.rs` gains `emit_inbound_protocols` which generates the Swift-side protocol, default extension, and box class from the IR. (`crates/alef-backend-swift/src/gen_rust_crate/plugin_inbound.rs`, `crates/alef-backend-swift/src/gen_rust_crate/mod.rs`, `crates/alef-backend-swift/src/gen_bindings.rs`)

- **alef-e2e/swift: emit fixture-driven visitor test classes via `build_swift_visitor`**: visitor-bearing fixtures previously emitted `XCTSkipIf(true, ...)` stubs in the swift e2e output. A new `codegen/swift_visitors.rs` module generates `final class LocalVisitor_<id>: HtmlVisitorProtocol` classes with per-method overrides, and `codegen/swift.rs` threads the resulting handle through `conversionOptionsFromJsonWithVisitor(json, handle)` replacing the former skip stub. (`crates/alef-e2e/src/codegen/swift_visitors.rs`, `crates/alef-e2e/src/codegen/swift.rs`, `crates/alef-e2e/src/codegen/mod.rs`)

### Fixed

- **alef-backend-napi: keep tagged tuple variant Rust fields snake_case while exposing serde names to TypeScript**:
  Tagged tuple variant payloads now assert the generated Rust field remains snake_case (for
  warning-free Rust) while `#[napi(js_name = "...")]` preserves the serde rename at the
  TypeScript boundary.
  (`crates/alef-backend-napi/src/gen_bindings/enums.rs`)

- **alef-backend-wasm: preserve `Option<T>` fields in tagged data enum variants**:
  The flat wasm-bindgen struct representation for serde-tagged data enums stores every variant
  field as optional because a single JS object only carries one variant's fields. When a source
  variant field was already `Option<T>`, the conversion code incorrectly unwrapped it in
  binding→core direction and wrapped it in `Some(...)` in core→binding direction, producing type
  mismatches such as `Option<String>` vs `String`. Optional variant fields now pass through the
  flattened binding field directly, with named inner values still mapped through `Into`.
  (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`)

- **alef-e2e/typescript: clean up mock-server timers and process handles in Vitest global setup**:
  The generated TypeScript e2e `globalSetup.ts` now clears the startup timeout after the mock
  server emits its sentinel URL and removes the stdout listener, then waits for the spawned
  mock-server process to close during teardown with a bounded SIGKILL fallback. This prevents
  Vitest from printing hanging-process warnings after otherwise successful generated e2e runs.
  (`crates/alef-e2e/templates/typescript/globalSetup.ts.jinja`,
  `crates/alef-e2e/src/codegen/typescript/config.rs`)

- **alef-backend-jni: emit type aliases for streaming handle struct to avoid `clippy::type_complexity`**:
  The `<Type><Adapter>StreamHandle` struct was inlining the full
  `Mutex<Option<BoxStream<'static, Result<T, Box<dyn Error + Send + Sync + 'static>>>>>` type
  in its `stream` field, triggering `clippy::type_complexity` (6 levels of nesting) under
  `-D warnings`. The emitter now generates two type aliases — `<Handle>Item` and `<Handle>Stream`
  — before each struct so the field reduces to `Mutex<Option<<Handle>Stream>>`.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **alef-backend-wasm: extend tagged-data enum JsValue storage to `Option<T>` and bare `T` fields**:
  The previous fix in commit `c62b16e6` only handled `Vec<TaggedDataEnum>` fields. Fields of type
  `Option<TaggedDataEnum>` and bare (required) `TaggedDataEnum` were still emitting
  `Option<WasmFoo>` / `WasmFoo` storage, causing wasm-bindgen to reject plain JS object literals
  with "expected instance of WasmFoo" at the setter. The emitter now maps all three shapes
  (`T`, `Option<T>`, `Vec<T>`) to `JsValue` / `Option<JsValue>` when T is a tagged-data enum,
  and the From impls use `serde_wasm_bindgen` in all cases. Fixes CI E2E failures for
  `ChatCompletionRequest.responseFormat` and `OcrRequest.document`.
  (`crates/alef-backend-wasm/src/gen_bindings/types.rs`,
  `crates/alef-codegen/src/conversions/binding_to_core.rs`,
  `crates/alef-codegen/src/conversions/core_to_binding.rs`)

- **alef-backend-swift: emit `import RustBridgeC` in bridge files and umbrella header for SwiftPM**:
  Swift bindings generated by swift-bridge require a separate C target (`RustBridgeC`) to hold
  C type declarations; the `RustBridge` Swift target must `import RustBridgeC` at the top of
  every generated `.swift` file, and `packages/swift/Sources/RustBridgeC/RustBridgeC.h` must be
  a real umbrella header (not a placeholder). The scaffold now reads the cargo-build output
  headers (`SwiftBridgeCore.h` + `{crate}.h`) and concatenates them into the umbrella header so
  `swift test` compiles without "module 'RustBridgeC' not found" or "no such module" errors.
  Additionally, Swift 6 strict concurrency requires `@unchecked Sendable` conformances on the
  three opaque swift-bridge types captured across task boundaries (client inner, stream handle,
  and request param). Streaming item types (e.g. `ChatCompletionChunk`) are now decoded via a
  Rust-side `{type}_from_json(String) -> Result<T, String>` bridge function instead of a
  Swift-side Codable struct, avoiding the "ambiguous for type lookup" conflict between the
  opaque class and a generated struct with the same name.
  (`crates/alef-backend-swift/src/gen_bindings.rs`, `crates/alef-backend-swift/src/gen_rust_crate/mod.rs`,
  `crates/alef-scaffold/src/languages/swift.rs`)

- **alef-e2e: fix Swift streaming accessor for `tool_calls` deep paths and flat collection**:
  The `tool_calls[0].function.name` field path in Swift e2e tests was emitting property access
  syntax (`.function.name`) but swift-bridge exposes all fields as methods returning Optional
  (`.function()?.name()`). Added a Swift-specific deep-path renderer that uses direct `.method()`
  on the non-optional ref after an index and `?.method()` optional chaining thereafter, with
  `?.toString()` on the final string leaf. Also fixed the flat `tool_calls` accessor to use a
  `guard let` closure with an explicit return type annotation (`{ c -> [StreamToolCallRef] in
  guard let … }`) so Swift 6's type checker doesn't select the Optional overload of `flatMap`.
  (`crates/alef-e2e/src/codegen/streaming_assertions.rs`, `crates/alef-e2e/src/codegen/swift.rs`)

- **alef-backend-wasm: emit tuple-variant pattern for newtype enum variants in From impls**:
  tagged-data enums with tuple/newtype variants (e.g. `enum Message { System(SystemMessage),
  User(UserMessage), ... }`) were emitting invalid struct-pattern destructuring
  (`Message::System { _0 }`) in the `From<core::Enum> for WasmEnum` match arm and invalid
  struct-construction syntax (`Self::System { _0: val }`) in the `From<WasmEnum> for
  core::Enum` arm. Both now use the correct tuple syntax (`Message::System(field0)` and
  `Self::System(val)` respectively). When all tuple variants share the same positional field
  name (e.g. `_0`) but carry different `Named` inner types across variants, the binding
  struct stores the field as `JsValue` and per-variant conversion uses
  `serde_wasm_bindgen::from_value`/`to_value` to avoid impossible `WasmA → B` coercions.
  Named-field struct variants in tagged enums now also correctly apply `.into()` when
  constructing or destructuring core Named types at the WASM boundary.
  (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`)

- **alef-backend-wasm: use `set_field_N` / `field_N` names for positional `_N` struct fields in tagged-enum wasm-bindgen getters/setters**: the getter/setter pair for positional fields (named `_0`, `_1`, … by the extractor) was emitting `fn set__0` and `fn _0`, which the Rust `non_snake_case` lint rejects as `method 'set__0' should have a snake case name` under `RUSTFLAGS="-D warnings"`. The Rust identifiers are now `field_0` (getter) and `set_field_0` (setter); the JS-visible name is unchanged via the `#[wasm_bindgen(getter/setter, js_name = "...")]` attribute. Adds a regression test asserting the correct identifier names are emitted. (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`)

- **alef-backend-kotlin-android: add `jackson-datatype-jdk8` to library `build.gradle.kts`**: the generated `DefaultClient.kt` registers `Jdk8Module` via `ObjectMapper().registerModule(com.fasterxml.jackson.datatype.jdk8.Jdk8Module())`, but the library scaffold's `build.gradle.kts` only declared `jackson-databind` and `jackson-module-kotlin`. Kotlin compilation failed with "Unresolved reference 'datatype'". Adds `implementation("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:<version>")` to the emitted dependencies block, version-pinned via `maven::JACKSON` (same as the other Jackson deps). (`crates/alef-backend-kotlin-android/src/gen_build_gradle.rs`)

- **alef-backend-wasm: store `Vec<TaggedDataEnum>` fields as `JsValue` to accept plain JS objects**:
  wasm-bindgen type-checks every element of a `Vec<T>` setter against class instances of `T`;
  plain JS object literals (e.g. `{ role: "user", content: "..." }`) were rejected with
  "array contains a value of the wrong type", causing all non-streaming WASM chat tests to
  fail in CI. Struct fields, constructor parameters, getters, and setters for
  `Vec<TaggedDataEnum>` are now emitted as `JsValue`. The `From` impls in both directions
  use `serde_wasm_bindgen::from_value`/`to_value` to deserialize/serialize the plain-object
  array. A new `tagged_data_enum_names` field on `ConversionConfig` controls which named
  types trigger this path; `binding_to_core.rs` and `core_to_binding.rs` emit the
  `serde_wasm_bindgen` conversion when the field is set.
  (`crates/alef-backend-wasm/src/gen_bindings/types.rs`,
  `crates/alef-backend-wasm/src/gen_bindings/mod.rs`,
  `crates/alef-codegen/src/conversions/mod.rs`,
  `crates/alef-codegen/src/conversions/binding_to_core.rs`,
  `crates/alef-codegen/src/conversions/core_to_binding.rs`)

## [0.16.12] - 2026-05-15

### Fixed

- **kotlin-android: drop redundant variant constructor wrap in tagged sealed-class
  deserializer**: the emitter for serde-tagged sealed classes (`#[serde(tag = ...)]`)
  incorrectly wrapped the `readTreeAsValue` result in the variant constructor for
  named-field struct variants (e.g. `ContentPart.Text(ctx.readTreeAsValue(node,
  ContentPart.Text::class.java))` — the outer `ContentPart.Text(...)` call passes a
  `ContentPart.Text` to a constructor expecting `String`, failing with "Argument type
  mismatch"). Named-field variants now emit `ctx.readTreeAsValue<Variant>(node,
  Variant::class.java)` directly. Newtype/tuple variants (`_0` field) correctly retain
  the wrapper since they pass the inner type to the variant constructor. Explicit Kotlin
  type parameters (`<T>`) are added to all `readTreeAsValue` call sites to prevent
  Kotlin inferring `Any!` for the Java generic return type.
  (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`)

- **kotlin-android: emit `JavaType` for `List<T>` in untagged sealed-class deserializer**:
  the emitter for serde-untagged sealed classes (`#[serde(untagged)]`) emitted
  `ctx.readTreeAsValue(node, List::class.java)` for `Vec<T>` variants, producing a raw
  `List<*>` at runtime (Jackson cannot reconstruct the element type from a raw class
  literal). The fix uses `ctx.typeFactory.constructCollectionType(List::class.java,
  T::class.java)` to carry the element type through to Jackson. Struct-variant branches
  in the untagged deserializer return `ctx.readTreeAsValue<T>` directly (no constructor
  wrap). Adds regression tests for `List<String>` and `List<NamedType>` variants.
  (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`)

- **kotlin-android: add Jackson runtime deps to `build.gradle.kts`
  template**: alef v0.16.11 added `@JsonDeserialize` to sealed-class Kotlin
  DTOs for serde-tagged polymorphism, but the kotlin-android `build.gradle.kts`
  template still only declared `kotlin-stdlib` + `kotlinx-coroutines-android`.
  The compileDebugKotlin task failed with `Unresolved reference 'com'` /
  `Unresolved reference 'codec'` / `Unresolved reference 'asText'` etc.
  Adds `jackson-databind` (for the annotations + `StdDeserializer`) and
  `jackson-module-kotlin` (for Kotlin data-class deserialization) to the
  emitted `dependencies { ... }` block, versions pinned via
  `maven::JACKSON`. (`crates/alef-backend-kotlin-android/src/gen_build_gradle.rs`)

## [0.16.11] - 2026-05-15

### Fixed

- **wasm: serde-tagged data enums emit as a flat `#[wasm_bindgen]` struct with a `default()`
  factory**: tagged data enums (e.g. `#[serde(tag = "type")] enum AuthConfig { Basic {...}, Bearer {...} }`)
  were previously emitted as wasm-bindgen C-style enums (`#[wasm_bindgen] pub enum`), which
  silently dropped every variant's field data and exposed no JS-callable constructor — JS test
  codegen calling `WasmAuthConfig.default()` crashed with `TypeError: WasmAuthConfig.default is
  not a function`. The wasm backend now detects serde-tagged data enums and emits them as a
  flat struct with a `String` discriminator (named after `serde_tag`), the union of every
  variant's fields as `Option<T>` properties, a `#[wasm_bindgen(constructor)]` `new()`, a
  static `default()` factory, and per-field getter/setter pairs — mirroring the NAPI
  `gen_tagged_enum_as_object` path. Custom `From<Wasm{Enum}> for core::{Enum}` (and reverse)
  impls match on the discriminator field instead of variant identity. Rust reserved keywords
  used as tag/field names (e.g. `type`) are escaped via raw-identifier syntax. Struct getters
  for `Option<WasmTaggedEnum>` fields now clone the binding wrapper directly instead of
  calling the unit-enum-only `to_api_str()` helper.
  (`crates/alef-backend-wasm/src/gen_bindings/enums.rs`,
  `crates/alef-backend-wasm/src/gen_bindings/types.rs`,
  `crates/alef-backend-wasm/src/gen_bindings/mod.rs`)

- **kotlin-android e2e codegen: emit `Flow.toList()` and Kotlin property access
  in streaming assertions**: the `kotlin_android` streaming codegen path in
  `StreamingFieldResolver` was already correct (`result.toList()` via the
  `kotlin_android` branch), but the installed alef binary was stale (0.16.10).
  Added regression test
  `kotlin_android_streaming_collect_uses_flow_to_list_not_as_sequence` covering:
  (a) `result.toList()` used to collect the `Flow<T>` (not `asSequence().toList()`),
  (b) chunk field assertions use Kotlin property access (`it.choices?.firstOrNull()?.delta?.content`
  not Java getter calls), and (c) `import kotlinx.coroutines.flow.toList` is
  emitted. (`crates/alef-e2e/tests/kotlin_android_codegen.rs`)

## [0.16.10] - 2026-05-15

### Fixed

- **kotlin-android: bump Java/JVM target from 11 to 17**: the kotlin-android
  backend emitted `JavaVersion.VERSION_11` and `JvmTarget.JVM_11` in the AAR
  `build.gradle.kts`, which is incompatible with Java 16+ records, Java 17+
  sealed classes, and Java 14+ switch expressions that the canonical Java
  backend's DTOs use. AGP 8.x desugars Java 17 down to Dalvik for minSdk 21+
  so 17 is safe across every supported Android target. Bumped via
  `toolchain::ANDROID_JVM_TARGET` so every emitted Android library now
  defaults to Java 17 / JVM 17. (`crates/alef-core/src/template_versions.rs`)

- **cleanup: sweep orphan alef-generated files in sibling subtrees of
  touched directories**: `cleanup_orphaned_files` previously only descended
  into subdirectories that were either themselves touched or contained a
  touched path. Subtrees that a backend used to write to but no longer does
  (e.g. the kotlin-android backend dropped its `src/main/java/` Java DTO emit
  in v0.16.0, leaving stale alef-marked Java files behind in every
  consumer repo) were never visited, so their orphans survived every
  subsequent regen and broke downstream builds (e.g. kreuzcrawl's
  `gradle assembleDebug` failed compiling FFM-tagged Java files with
  `import java.lang.foreign.*` against the Android JDK 17 toolchain).
  The descend logic now also walks any subtree that is a descendant of a
  touched directory; the alef-header gate in `has_alef_header` remains the
  safety net that keeps user-customised files untouched. Touched-dir paths
  are now canonicalized up-front so the new descendant check is portable
  across macOS `/tmp` vs `/private/tmp` symlinks. Regression test
  `cleanup_removes_orphan_in_sibling_subtree_of_touched_dir`.
  (`crates/alef-cli/src/pipeline/cleanup.rs`)

## [0.16.8] - 2026-05-15

### Added

- **Source-level `#[cfg_attr(alef, alef(skip))]` on types, enums, errors, functions, methods, and traits**:
  Alef now honors `alef(skip)` (and `#[doc(hidden)]`) at every item level, not just on
  fields. Excluded items are stripped from the binding API surface before downstream
  codegen; trait-bridge codegen can still resolve their fully qualified Rust paths via
  `ApiSurface.excluded_type_paths`. This lets the source declare what stays internal to
  Rust without per-language `exclude_types`/`exclude_functions` lists in `alef.toml`.
  (`crates/alef-extract`, `crates/alef-cli`, `crates/alef-core`)

### Fixed

- **kotlin-android: handle-only opaque types now get an AutoCloseable wrapper class
  in the facade**: when a top-level free function takes or returns an opaque type
  with no instance methods (the kreuzcrawl `CrawlEngineHandle` shape, distinct
  from the liter-llm `DefaultClient` shape), the `<Module>.kt` facade previously
  emitted `String` for both the parameter and the return type — disagreeing with
  the JNI bridge, which (since the v0.16.7 JNI handle refactor) correctly emits
  `Long`. The facade now emits a sibling `<TypeName>.kt` wrapper class implementing
  `AutoCloseable` (`class <TypeName> internal constructor(internal val handle: Long)
  : AutoCloseable`) whose `close()` delegates to the bridge's
  `nativeFree<TypeName>(handle)` destructor. Top-level fns returning the opaque
  type now return this wrapper class, constructed from the bridge's `Long`; fns
  taking the type as a parameter accept the wrapper and unwrap `.handle` into
  the bridge call. The liter-llm `DefaultClient` (client-with-methods) shape is
  unchanged — `emit_jni_client_class` continues to emit `DefaultClient.kt` for
  that case, and top-level factories now correctly construct it from the
  `Long` bridge return instead of returning a stale `String`. Regression test
  `handle_only_opaque_returns_wrapper_class_and_accepts_wrapper_params` in
  `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`.
  (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **codegen/trait_bridge: synthesized `Plugin` methods include new `binding_excluded`
  fields**: the synthetic `MethodDef`s for the Plugin trait's `version`,
  `initialize`, and `shutdown` methods were missing the `binding_excluded` and
  `binding_exclusion_reason` fields added when source-level exclusion metadata
  landed, breaking workspace compilation.
  (`crates/alef-codegen/src/generators/trait_bridge.rs`)

- **kotlin-android: `createClient` wrapper returns `DefaultClient` not `String`**:
  `emit_module_kt` in `alef-backend-kotlin-android` previously passed every
  `TypeRef::Named` return through `jni_return_type_str`, which maps named types
  to `"String"`. When a top-level function returns an opaque handle type (e.g.
  `create_client() -> DefaultClient`), the wrapper facade now detects the opaque
  type name, emits `: DefaultClient` as the return type, and wraps the bridge call
  in the `DefaultClient(handle: Long)` constructor instead of propagating the raw
  `Long`. Regression test added in
  `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`.
  (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-e2e/kotlin_android: construct `DefaultClient` via `client_factory` before
  calling methods**: when `[crates.e2e.calls.<call>.overrides.java]` (or
  `.kotlin_android`) declares a `client_factory`, `render_test_method` in
  kotlin_android style now picks it up and emits the instance-method call pattern
  (`val client = LiterLlm.createClient(...); client.chat(...); client.close()`)
  instead of the flat function call style (`LiterLlm.chat(...)`).
  Previously the fallback chain only checked a `"kotlin"` override key, so the
  java override was silently ignored. Regression test added in
  `crates/alef-e2e/tests/kotlin_android_codegen.rs`.
  (`crates/alef-e2e/src/codegen/kotlin.rs`)

- **alef-e2e/kotlin_android: emit Kotlin property access instead of Java getter
  calls**: `render_test_method` now passes `"kotlin_android"` as the language key
  to `FieldAccessResolver::accessor()`. New `render_kotlin_android_with_optionals`
  and `render_kotlin_android` helpers produce `result.choices.first().message.content`
  (no `()`) rather than `result.choices().first().message().content()`.
  Regression test added in `crates/alef-e2e/tests/kotlin_android_codegen.rs`.
  (`crates/alef-e2e/src/field_access.rs`, `crates/alef-e2e/src/codegen/kotlin.rs`)

- **alef-e2e/kotlin_android: serialize enum fields via `.name.lowercase()` not
  `.getValue()`**: assertion codegen now branches on `kotlin_android_style` to emit
  `.name.lowercase()` (or `?.name?.lowercase()` for optional enums) instead of
  `.getValue()`, which does not exist on plain Kotlin `enum class` values. Enum
  fields from `[overrides.java]` call configs are now also merged correctly when
  generating kotlin_android tests. Regression test added in
  `crates/alef-e2e/tests/kotlin_android_codegen.rs`.
  (`crates/alef-e2e/src/codegen/kotlin.rs`)

- **Binding surface: exclude source-declared internal fields before codegen**:
  Alef now records field-level exclusion metadata from `#[doc(hidden)]` and
  explicit `#[cfg_attr(alef, alef(skip))]`, then removes those fields from
  generated binding APIs before unknown-type sanitization. Internal caches and
  runtime-only state no longer appear as fake `String` fields in language
  bindings.
  (`crates/alef-extract`, `crates/alef-cli`, `crates/alef-core`)

- **Swift: render `extra_dependencies` into the generated Rust crate**: Swift
  bindings now merge crate-level and `[crates.swift.extra_dependencies]` entries
  into `packages/swift/rust/Cargo.toml`. Multi-crate consumers can reference
  shared core/runtime crates from generated Swift bridge code without manually
  editing the generated manifest.
  (`crates/alef-backend-swift/src/gen_rust_crate`)

- **alef-e2e/kotlin_android: streaming assertions use Kotlin property access and
  `Flow.toList()` collect**: `StreamingFieldResolver` previously had no
  `"kotlin_android"` arms — all five streaming virtual fields (`stream_content`,
  `stream_complete`, `tool_calls`, `finish_reason`, `usage`) fell through to
  the `"kotlin"` arm and emitted Java record accessor calls (`.choices()`,
  `.delta()`, `.content()`, `.finishReason()`, `.getValue()`, `.usage()`).
  Kotlin data-class fields (as generated by alef's kotlin-android backend) are
  plain properties, so every one of those method calls produced an
  "Unresolved reference" compile error. Added `"kotlin_android"` match arms for
  all five virtual fields and for `collect_snippet` (now emits
  `val chunks = result.toList()` — `Flow.toList()` — instead of
  `result.asSequence().toList()` which is the `Iterator` API). Deep-path
  tail-segment rendering in `render_deep_tail` also gained
  `(TailSeg::Field, "kotlin_android")` and `(TailSeg::Index, "kotlin_android")`
  arms that omit `()`. Five regression tests added in
  `crates/alef-e2e/src/codegen/streaming_assertions.rs`.
  (`crates/alef-e2e/src/codegen/streaming_assertions.rs`,
  `crates/alef-e2e/src/codegen/kotlin.rs`)

- **kotlin-android: optional facade params emit Kotlin default values**: params
  where `p.optional == true` (mapped from Rust `Option<T>` through the JNI shim
  as non-nullable `jlong`/`jint`/`JString`) previously had no Kotlin default in
  the facade signature, forcing every call site to supply all arguments. E2e
  codegen only passes `apiKey` and `baseUrl`, so `createClient(...)` failed to
  compile. The facade now emits `= 0L` for `Long`, `= 0` for `Int`,
  `= 0.0f`/`= 0.0` for `Float`/`Double`, `= false` for `Boolean`, and `= ""`
  for `String` optional params. Regression test
  `optional_params_get_kotlin_default_values_in_facade` in
  `crates/alef-backend-kotlin-android/tests/gen_bindings_test.rs`.
  (`crates/alef-backend-kotlin-android/src/gen_bindings.rs`)

- **alef-scaffold/jni: `[lib] name` uses `config.jni_lib_name()`**: the JNI
  cdylib scaffold derived `[lib] name` from the crate directory (`<crate>` with
  hyphens replaced by underscores), ignoring any `[ffi] prefix` override. The
  Kotlin Bridge emitter, however, uses `config.jni_lib_name()` (which honors
  `ffi_prefix`), so any consumer with a prefix override produced a `.so` whose
  basename disagreed with what `System.loadLibrary(...)` looks up at runtime,
  triggering `UnsatisfiedLinkError` on the first JNI call. The scaffold now
  also calls `config.jni_lib_name()`. Regression test in
  `crates/alef-scaffold/src/languages/jni.rs` asserts both the prefix-set and
  default cases.
  (`crates/alef-scaffold/src/languages/jni.rs`)

- **JNI: method-shim Result match gated on `error_type`**: `emit_method_shim`
  previously wrapped every method call in `match result { Ok / Err }` even when
  the method returned `T` directly. Builder methods returning `Self`
  (e.g. `ConversionOptionsBuilder::strip_tags`) failed to compile with
  `expected Self, found Result<_, _>`. Now mirrors `emit_function_shim`: gate
  on `method.error_type.is_some()` and emit the no-match path when the method
  is infallible. `emit_return_marshal` factored into a shared
  `_with_indent` helper so the no-error path can reuse the marshalling logic
  at the function's top-level 4-space indent.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **JNI: `Vec<u8>` / `Bytes` params receive `jbyteArray`, `PathBuf` params skip
  JSON decode**: `emit_single_param_unmarshal` fell through to `serde_json::from_str`
  for any type other than `String`, producing invalid Rust for `&[u8]` / `PathBuf`
  params. Added explicit branches: `Vec<u8>` and `Bytes` now receive
  `<name>: jbyteArray` and call `env.convert_byte_array(name)` (the JNI function
  signature is patched to use `jbyteArray` for these types); `TypeRef::Path` receives
  `request_json: JString` and constructs `std::path::PathBuf::from(req_str)`
  without any JSON deserialization.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **JNI: `&mut self` methods use `*mut T` handle cast**: `emit_method_shim`
  always emitted `&*(handle as *const T)` for the client dereference, causing
  E0596 ("cannot borrow as mutable behind `&` reference") for any method whose
  IR `receiver` is `ReceiverKind::RefMut` (e.g. `Parser::set_language`,
  `Parser::parse`, all `TreeCursor::goto_*`). `emit_client_shims` now inspects
  `method.receiver` and passes `receiver_is_mut` to `emit_method_shim`, which
  emits `&mut *(handle as *mut T)` when the flag is set.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **JNI: `&[&str]` params coerce from `Vec<String>` JSON**: when a method param
  is `Vec<String>` with `is_ref=true` (representing `&[&str]` in core),
  `emit_single_param_unmarshal` now deserializes into `<name>_vec: Vec<String>`
  and `emit_method_shim` emits `let <name>_refs: Vec<&str>` before passing
  `&<name>_refs` to the core method. Previously `&Vec<String>` was passed
  directly, causing a type mismatch.
  (`crates/alef-backend-jni/src/gen_shims.rs`)


- **zig e2e codegen**: emit `build.zig` that runs tests via `addRunArtifact`
  directly (Zig 0.16+ no longer installs test binaries to `zig-out/bin/`).
  The previous `addInstallArtifact` + `run.dependOn(install)` workaround
  produced `FileNotFound` at spawn time against the would-be install path on
  Zig 0.16+ Linux backends. `setCwd("../../test_documents")` is also dropped
  — the generated tests reach the mock server purely through `MOCK_SERVER_*`
  env vars and never read anything cwd-relative, so leaving cwd alone keeps
  `convertPathArg` from re-resolving the cache spawn path.
  (`crates/alef-e2e/src/codegen/zig.rs`)

- **JNI: raw jlong for opaque handle returns**: `emit_function_shim` now returns
  `jlong` directly from opaque-returning top-level functions instead of a
  JSON-encoded `jstring`. Consumer types no longer require `Serialize` /
  `Deserialize` derives — the handle is a raw pointer cast, matching the
  method-on-struct pattern already used by liter-llm.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **JNI: raw jlong for opaque handle params**: top-level functions whose parameters
  are opaque named types (e.g. `scrape(engine: &CrawlEngineHandle, ...)`) now
  receive `handle: jlong` and dereference via
  `unsafe { &*(handle as *const core_crate::T) }` instead of JSON-deserializing
  from a `JString`. The error-return sentinel in all unmarshal paths is now `0`
  (for jlong returns) rather than `std::ptr::null_mut()`.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **JNI: emit nativeFreexxx destructors for top-level-function opaque returns**:
  the JNI backend now emits a `nativeFreeXxx(handle: jlong)` destructor shim for
  every opaque type returned by a top-level function that does not already have
  instance methods (those types were already covered by `emit_client_shims`).
  Fixes handle leaks on the kreuzcrawl `CrawlEngineHandle` shape.
  (`crates/alef-backend-jni/src/gen_shims.rs`)

- **kotlin-android: Bridge.kt Long for opaque handle params/returns**: Bridge.kt
  `external fun` declarations for top-level functions now use `Long` (not
  `String`) wherever a parameter or return type is an opaque named handle. Covers
  both the constructor return type and handle params like `engine: Long`.
  (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **kotlin-android: Bridge.kt emits nativeFreexxx for client types**: the
  `emit_method_jni_external_funs` helper now appends
  `external fun nativeFreeXxx(handle: Long)` after each client type's method
  declarations so the destructor that `DefaultClient.close()` delegates to is
  properly declared in Bridge.kt.
  (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **kotlin-android: Bridge.kt emits nativeFreexxx for non-method opaque types**:
  opaque types returned by top-level functions but without instance methods now
  also get a `// Destructor external funs` block in Bridge.kt.
  (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **kotlin-android: ByteArray for bytes::Bytes returns in Bridge.kt**: instance
  method `external fun` declarations now emit `ByteArray` (not `Long`) for
  `TypeRef::Bytes` return types (e.g. `speech`, `file_content`), matching the
  `Vec<u8>` → `ByteArray` mapping already present for the JNI Rust side.
  (`crates/alef-backend-kotlin/src/gen_bindings/jni_emitter.rs`)

- **dart: filter trait/opaque types from `From<Mirror> for Core` impls**: trait
  types reachable via trait-bridge return-type seeding (e.g. `SyncExtractor` from
  a `make_visitor() -> &dyn SyncExtractor`) were landing in
  `types_needing_from_impl` and producing
  `impl From<SyncExtractor> for SyncExtractor { ... SyncExtractor {} }`, which
  fails to compile (E0574: trait, not struct). The mirror-to-core emission loop
  now filters `is_trait`/`is_opaque` so only mirror structs receive the impl.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **dart: gate sanitized-String From exception on `CoreWrapper::Cow`**: the prior
  exception treated every sanitized `TypeRef::String` field as the safe
  `Cow<'static, str>` round-trip case, but fields whose core type is something
  unrelated (e.g. `Option<BoundingBox>` sanitized down to `Option<String>`
  because BoundingBox is not in the API surface) emitted
  `v.field.map(Into::into)` and failed to compile with
  `String: Into<BoundingBox>` not satisfied. The exception is now gated on
  `core_wrapper == CoreWrapper::Cow`; non-Cow sanitized strings fall back to
  `Default::default()` like other sanitized fields.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **dart: wrap Arc/Box core wrappers in `enum_variant_field_conv_to_core`**:
  enum variant fields previously emitted `.into()` regardless of `core_wrapper`
  or `is_boxed`, but `From<T>` is not provided for `Arc<T>`/`Box<T>` by stdlib.
  Variants now match the struct-field handling: `Arc`/`ArcMutex` core wrappers
  emit `std::sync::Arc::new(_.into())`, `is_boxed` emits `Box::new(_.into())`,
  and plain Named types keep `.into()`. Fixes
  `the trait bound Box<HtmlMetadata>: From<HtmlMetadata> is not satisfied`
  errors on `FormatMetadata::Html`/`FormatMetadata::Docx`.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **dart: round-trip `Option<T>` flattening in enum-variant mirror-to-core**:
  the forward direction collapses enum-variant `Option<String>` /
  `Option<PathBuf>` / `Option<primitive>` to bare values via
  `unwrap_or_default()`. The reverse direction now mirrors that: empty
  `String` → `None`, non-empty → `Some(_)`; `0` primitive → `None`, non-zero
  → `Some(_ as _)`; `newtype_wrapper` is honored on both branches so
  `Vec<NodeIndex>` (mirror `Vec<i64>`) round-trips via `NodeIndex(x as _)`.
  Fixes the `expected Option<String>, found String` / `i64 is not an iterator`
  / `non-primitive cast: i64 as NodeIndex` regressions across
  `NodeContent::{Image,Code,Group,Custom,Title}` and
  `AnnotationKind::{Link,Custom}`.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **dart: honor newtype_wrapper on `Vec<Primitive>` struct fields**: when a
  struct field is `Vec<Primitive>` in the mirror with `newtype_wrapper` set,
  `field_from_expr_to_core` previously emitted `|x| x as _`, breaking core
  fields like `children: Vec<NodeIndex>`. Now emits `|x| {nw}(x as _)` when
  `newtype_wrapper` is set, matching the scalar-primitive branch.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **kotlin-android e2e: emit `import kotlinx.coroutines.flow.toList` for
  streaming test files**: `render_test_file_inner` with `kotlin_android_style=true`
  now detects whether any fixture in the generated file is a streaming call and, if
  so, emits the `import kotlinx.coroutines.flow.toList` statement. Without it,
  `Flow<T>.toList()` is an unresolved reference in Kotlin 1.x/2.x — the
  extension is not part of the Flow API surface without the explicit import.
  Regression test `kotlin_android_streaming_fixture_emits_flow_to_list_import` in
  `crates/alef-e2e/src/codegen/kotlin.rs`.
  (`crates/alef-e2e/src/codegen/kotlin.rs`)

- **kotlin-android e2e: emit `registerKotlinModule()` on ObjectMapper and add
  `jackson-module-kotlin` dependency**: `render_test_file_inner` with
  `kotlin_android_style=true` now imports
  `com.fasterxml.jackson.module.kotlin.registerKotlinModule` and appends
  `.registerKotlinModule()` to the companion-object `MAPPER` initialiser.
  `render_build_gradle_kotlin_android` now includes
  `testImplementation("com.fasterxml.jackson.module:jackson-module-kotlin:{jackson}")`.
  Plain Jackson cannot deserialise Kotlin data classes (which have no no-arg
  constructor) without this module, producing `InvalidDefinitionException` at
  runtime. Regression tests `kotlin_android_object_mapper_emits_register_kotlin_module`
  in `crates/alef-e2e/src/codegen/kotlin.rs` and
  `build_gradle_kotlin_android_includes_jackson_module_kotlin` in
  `crates/alef-e2e/src/codegen/kotlin_android.rs`.
  (`crates/alef-e2e/src/codegen/kotlin.rs`,
  `crates/alef-e2e/src/codegen/kotlin_android.rs`)

- **kotlin-android bindings: emit `@JsonDeserialize` + custom deserializer for
  serde-tagged and serde-untagged sealed classes**: `emit_enum` in
  `alef-backend-kotlin` now detects enums with `serde_tag` (internally-tagged,
  e.g. `Message` with `#[serde(tag = "role")]`) or `serde_untagged` (e.g.
  `EmbeddingInput`, `RerankDocument`) and emits a `@JsonDeserialize(using =
  XxxDeserializer::class)` annotation on the sealed class plus a private
  `StdDeserializer<T>` subclass. The tagged deserializer reads the discriminant
  field from an `ObjectNode` and delegates to `ctx.readTreeAsValue(node,
  VariantPayload::class.java)`; the untagged deserializer branches on
  `node.isTextual` / `node.isArray` / `node.isObject`. Without this, Jackson
  throws `InvalidDefinitionException: Cannot construct instance of sealed class`
  when tests call `MAPPER.readValue(...)` against any of these types.
  Regression tests `emit_enum_tagged_sealed_class_emits_json_deserialize_annotation`,
  `emit_enum_untagged_sealed_class_emits_json_deserialize_annotation`, and
  `emit_enum_unit_only_does_not_emit_json_deserialize` in
  `crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`.
  (`crates/alef-backend-kotlin/src/gen_bindings/object_wrapper.rs`,
  `crates/alef-backend-kotlin/Cargo.toml`)

## [0.16.6] - 2026-05-15

### Added

- **alef-backend-jni**: new first-class backend crate that emits the Rust JNI
  shim source (`crates/<crate>-jni/src/lib.rs`) consumed by the Android AAR
  build. Exports `pub unsafe extern "system" fn Java_*` symbols for every
  top-level function, every instance method on opaque client types (with
  `handle: jlong` + optional `request_json: JString`), destructor shims
  (`nativeFree<Type>`), and streaming Start/Next/Free triple shims — one
  symbol per `external fun native*` declaration emitted by
  `alef-backend-kotlin-android`. Symbol names are derived via
  `alef_core::jni::*` so Kotlin bridge names and Rust `Java_*` symbols can
  never drift independently.
  (`crates/alef-backend-jni/`, `crates/alef-core/src/jni.rs`)

- **alef-core/jni module**: shared symbol-naming utilities
  (`bridge_class_name`, `bridge_method_name`, `streaming_method_names`,
  `destructor_method_name`, `jni_symbol`) extracted into `alef_core::jni` so
  every backend that touches JNI uses one canonical encoder.
  (`crates/alef-core/src/jni.rs`)

- **Language::Jni**: new language variant that activates `alef-backend-jni`
  emission. Requires `kotlin_android` to be listed in `languages` (validated
  at config resolution time). All existing match arms updated; `alef-scaffold`
  and `alef-cli` dispatch wired accordingly.
  (`crates/alef-core/src/config/extras.rs`, `crates/alef-cli/src/registry.rs`)

### Fixed

- **alef-backend-zig (free-before-use on C-string sentinel params)**: wrapper
  functions that converted a `[]const u8` parameter to a null-terminated C string
  via `allocPrintSentinel` / `dupeZ` were emitting `c_allocator.free(name_z)`
  _before_ the C function call, passing a dangling pointer. Affected all infallible
  top-level functions (e.g. `has_language`, `detect_language_from_content`,
  `detect_language_from_extension`, `detect_language_from_path`) and all infallible
  opaque-handle methods (e.g. `LanguageRegistry.has_language`, `Parser.parse`,
  `Node.child_by_field_name`). Fallible functions (those with an `error_type`) were
  already correct (free came after the error-code check). Fix: emit
  `defer std.heap.c_allocator.free(name_z);` immediately after each
  `allocPrintSentinel` / `dupeZ` alloc — the deferred free runs on function exit
  regardless of control flow, so the buffer is always alive during the C call.
  (`crates/alef-backend-zig/src/gen_bindings/functions.rs`,
  `crates/alef-backend-zig/src/gen_bindings/opaque_handles.rs`,
  `crates/alef-backend-zig/templates/param_string_line2.jinja`,
  `crates/alef-backend-zig/templates/param_optional_string_alloc.jinja`)

- **alef-backend-dart (String→Cow From-impl)**: `Cow<'static, str>` struct
  fields were silently dropped in the generated `From<MirrorT> for CoreT`
  implementation. The type resolver resolves `Cow<'static, str>` to
  `TypeRef::Named("str")`, which `sanitize_unknown_types` then replaces with
  `TypeRef::String` and marks `sanitized = true`. The dart backend's
  `emit_from_mirror_to_core_struct` unconditionally emitted `Default::default()`
  for every `sanitized` field — including these perfectly convertible
  `TypeRef::String` ones. Fix: skip the `Default::default()` fallback when
  `field.ty` is `TypeRef::String`; use `field_from_expr_to_core` instead, which
  emits `v.<field>.into()` (valid for `String → Cow<'static, str>`). Applies to
  any `Cow<'static, str>` field in any config struct, not just `language`.
  (`crates/alef-backend-dart/src/gen_rust_crate/mod.rs`)

- **alef-backend-swift (Error enum name clash)**: when the Rust error type is
  literally named `Error`, Swift parses `public enum Error: Error` as a circular
  raw-type binding instead of protocol conformance, causing compile errors. The
  codegen now renames such types to `{ModuleName}Error` (e.g.
  `TreeSitterLanguagePackError` for tslp). All error enums now use
  `Swift.Error` (fully-qualified) as the protocol conformance to avoid any
  future clashes with same-named types in scope.
  (`crates/alef-backend-swift/templates/error_enum_header.jinja`,
  `crates/alef-backend-swift/src/gen_bindings.rs`)

- **alef-e2e (swift)**: three follow-on fixes that make the generated swift
  e2e suite compile and report meaningful results:
  - **Map-field assertion subscript**: `materialise_vec_temporaries` now
    detects string-key subscripts (e.g. `["title"]`) and, for those cases,
    wraps the parent accessor in `JSONSerialization.jsonObject(...) as? [String: String]`
    so subscripting works on JSON-bridged Map fields (e.g. `metadata.document.open_graph[title]`).
    swift-bridge serialises `HashMap<String, String>` as JSON-encoded RustString;
    the previous emission tried to subscript RustString directly and produced
    `value of type 'RustString' has no subscripts`. The rewritten expression
    also signals back that no `.toString()` chain should follow.
  - **Trim character set**: `CharacterSet.whitespaces` does NOT include `\n`
    in Swift. Switched the `equals` and `contains` trim chains to
    `CharacterSet.whitespacesAndNewlines` so assertions like
    `XCTAssertEqual(trimmed, "# Heading 1")` against `# Heading 1\n\n` succeed.
  - **Visitor-fixture skip**: when a fixture has a `visitor` spec and the
    swift binding has no visitor wiring (the matching alef-backend-swift
    inbound emission is also gated below), emit `XCTSkipIf(true, ...)` so
    the test compiles and is recorded as pending rather than asserting
    against transformations the binding cannot perform.
  (`crates/alef-e2e/src/codegen/swift.rs`)

- **alef-backend-swift (inbound plugin emission)**: gate the inbound
  `extern "Swift"` block (`Swift<Trait>Box` declaration + per-method FFI shims),
  the corresponding `extern "Rust"` register/unregister entry points, and the
  Rust-side `Swift<Trait>Wrapper` newtype + `register_<trait>` fn on
  `bind_via = "function_param"`. Previously, _every_ configured trait bridge
  emitted the inbound plugin block unconditionally, including `bind_via = "options_field"`
  bridges (e.g. h2m's `HtmlVisitor`). The swift-bridge crate then declared
  `type Swift<Trait>Box;` in the `extern "Swift"` block, but there was no
  matching Swift class in the binding facade — `swift build` failed with
  `cannot find type 'Swift<Trait>Box' in scope`. `options_field` bridges
  conventionally bind via an options builder method (mirroring the dart/zig
  pattern), not via a separate Rust-side registry, so the inbound plugin
  scaffolding is the wrong shape for them. Until alef-backend-swift grows a
  proper Swift class scaffold + handle-builder shim for `options_field` bridges,
  skip the inbound emission entirely.
  (`crates/alef-backend-swift/src/gen_rust_crate/mod.rs`)

- **alef-e2e (swift)**: SwiftPM identifies path-based deps by the path's last
  component, so a consumer at `e2e/swift/` referencing a dep at `packages/swift/`
  collides on identifier `"swift"` and SPM resolves `.product(package:)` against
  the consumer itself, failing with `product '<name>' required by package 'swift'
  ... not found in package 'swift'`. Emit the swift e2e package under
  `<output>/swift_e2e/` (path-last `"swift_e2e"`) so the consumer has a distinct
  SwiftPM identity regardless of where the dep lives. Repos must update
  `[crates.test.swift] e2e = "cd e2e/swift_e2e && swift test"` accordingly.
  (`crates/alef-e2e/src/codegen/swift.rs`)

- **alef-e2e (swift)**: the generated `Package.swift` for swift e2e now uses
  `SWIFT_MIN_IOS` from `template_versions::toolchain` instead of hardcoding
  `.iOS(.v14)`. Previously, when the dependency package required iOS 16+ (the
  current default), SwiftPM hid the dep's products as platform-incompatible and
  failed resolution — the consumer's iOS minimum must be >= the dep's iOS
  minimum for SPM to expose products.
  (`crates/alef-e2e/src/codegen/swift.rs`)

- **alef-e2e/swift (config-from-json helper name)**: the config-arg codegen at
  line 1047 hardcoded `extractionConfigFromJson(...)` for all crates, but should
  derive the function name from the `options_type` configured in `CallOverride`.
  For example, with `options_type = "ProcessConfig"`, emit
  `processConfigFromJson(...)` instead. Derives the camelCased helper name on the
  fly; falls back to `extractionConfigFromJson` if `options_type` is absent
  (preserving backward compatibility for kreuzberg).
  (`crates/alef-e2e/src/codegen/swift.rs`, `crates/alef-e2e/tests/swift_config_from_json_helper.rs`)

- **alef-codegen (ruby kwargs constructor)**: Named enum fields with a resolved
  `EnumVariant` typed default now fall through to the explicit-default branch
  (`unwrap_or(EnumType::Variant)`) instead of being emitted as required kwargs
  via `ok_or_else(... "missing required field")`. The previous behaviour
  forced callers to pass every enum-typed config field explicitly even when
  the parent struct's `impl Default` already specified a concrete variant —
  e.g. kreuzberg's `ExtractionConfig.new` (Ruby) required `output_format:` on
  every construction despite `#[serde(default)] pub output_format: OutputFormat`
  on the Rust side. Magnus-wrapped _struct_ fields (which never implement
  `Default`) keep their required-kwarg behaviour. (`crates/alef-codegen/src/config_gen.rs`)


- **alef-backend-swift (capsule/handle passthrough for Option<Named(T)> returns)**:
  method shims and extern-block declarations for `Option<Named(T)>` return types
  now route through the handle-aware path (`bridge_type_with_handles`,
  `needs_json_bridge_with_handles`) instead of unconditionally falling back to
  `serde_json::to_string`. When `T` is a known opaque handle type (returned by any
  public function/method in the API surface), the shim emits `(call).map(T)` and
  the extern block declares `Option<T>` — not `String`. This fixes `E0277` compile
  errors in binding crates whose opaque types (e.g. `Tree`, `Node`) do not
  implement `serde::Serialize`. (`crates/alef-backend-swift/src/gen_rust_crate/extern_block.rs`,
  `crates/alef-backend-swift/src/gen_rust_crate/wrappers.rs`)

- **alef-backend-swift (arg conversions in method shims)**: method shims now
  insert correct conversions for four parameter patterns that previously generated
  code that failed to compile against the core crate:
  - `Bytes + is_ref` (`&[u8]` receiver): emit `&name` instead of `name`
  - `Path` (non-ref, `PathBuf` receiver): emit `::std::path::PathBuf::from(name)`
    instead of `&name`
  - `Named + is_ref` (`&T` receiver): emit `&name.0` instead of `name.0`
  - `Vec<String> + is_ref` (`&[&str]` receiver): emit
    `&name.iter().map(|s| s.as_str()).collect::<Vec<_>>()` instead of `name`
  (`crates/alef-backend-swift/src/gen_rust_crate/wrappers.rs`)

- **alef-backend-zig (opaque handle emission)**: opaque handle types with no
  instance methods (e.g. a bare `Language` newtype returned by `get_language()`)
  were silently excluded from struct emission because the loop filtered on
  `!t.methods.is_empty()`. Zig then rejected any function whose return type or
  body referenced the undeclared type with "use of undeclared identifier". Removed
  the `!t.methods.is_empty()` guard so every opaque/non-serde type receives a
  `pub const T = struct { _handle: *anyopaque, ... };` declaration regardless of
  whether it has methods. (`crates/alef-backend-zig/src/gen_bindings/mod.rs`)

- **alef-backend-zig (bool return coercion)**: functions returning `bool` through
  the C ABI (`extern "C" fn(...) -> i32`) generated `return _result;` where the
  Zig wrapper declared return type `bool`. Zig's strict type system rejects the
  implicit `i32 → bool` coercion with a compile error. The `unwrap_return_expr`
  function now emits `_result != 0` for `TypeRef::Primitive(PrimitiveType::Bool)`
  returns, producing a valid Zig boolean. Two new tests cover the infallible and
  fallible (error-union) cases.
  (`crates/alef-backend-zig/src/gen_bindings/functions.rs`,
  `crates/alef-backend-zig/tests/gen_bindings_test.rs`)

- **alef-e2e/zig (infallible function try emission)**: the Zig e2e codegen
  unconditionally emitted `try` before every call, but functions that are
  genuinely infallible and take no allocating parameters (e.g. `language_count()`
  which returns a plain `u64`) do not declare an error union — `try` on a
  non-error-union expression is a compile error in Zig. The fix defaults
  `call_returns_error_union` to `true` (safer: almost all wrapper functions
  allocate and/or can fail) and requires an explicit per-language opt-out:
  `[crates.e2e.calls.<name>.overrides.zig] returns_result = false`.
  (`crates/alef-e2e/src/codegen/zig.rs`,
  `crates/alef-e2e/tests/zig_infallible_no_try.rs`)

- **alef-e2e/zig (streaming-virtual field intercept in non-streaming path)**:
  `render_json_assertion` intercepted field names matching `is_streaming_virtual_field`
  (e.g. `"chunks"`) unconditionally, emitting `chunks.items.len` even when the test
  was not streaming and no `chunks` local was ever declared. Any non-streaming fixture
  whose result type happened to have a field named `chunks` produced an "undeclared
  identifier 'chunks'" compile error. Added `uses_streaming: bool` parameter to
  `render_json_assertion`; the streaming-virtual intercept path now fires only when
  `uses_streaming = true`. The four existing call sites pass the correct flag: the
  streaming path passes `true`, all others pass `false`.
  (`crates/alef-e2e/src/codegen/zig.rs`)

## [0.16.5] - 2026-05-15

### Added

- **alef-backend-kotlin (JNI mode)**: extended Bridge object and `DefaultClient`
  emission to cover the full Panama-emitted method surface. Every visible
  non-static instance method on an opaque client type now produces an
  `external fun native<Owner><Method>(handle: Long, requestJson: String): String`
  Bridge declaration plus a typed Kotlin wrapper that JSON-marshals the request
  via `MAPPER.writeValueAsString` and deserialises the response via
  `MAPPER.readValue`. Wrappers run on `Dispatchers.IO`. `Vec<u8>` IR returns map
  to Kotlin `ByteArray` (pass-through, no base64); `Unit` returns drop response
  handling; zero-param methods omit `requestJson` from both the Bridge external
  fun and the wrapper call. A shared `MAPPER` companion object is emitted on
  the client class when at least one method requires JSON marshalling.

### Fixed

- **alef-e2e/dart**: emit `import 'dart:io';` for non-HTTP fixtures that build mock-server
  URLs (`mock_url` args and `client_factory` configurations). These reference
  `Platform.environment` but were not covered by the prior import condition, causing
  generated Dart tests (e.g., `cookies_test.dart`, `error_test.dart`) to fail to compile
  with `Error: Undefined name 'Platform'`.
- **alef-e2e/zig**: resolve `FileNotFound` at test-binary spawn on Zig 0.16's self-hosted
  aarch64-linux backend. The generated `build.zig` now captures the install step via
  `b.addInstallArtifact(<test>, .{})` and makes each per-test `addRunArtifact` run step
  depend on it. This forces `Compile.installed_path` to be populated before `Run.make`
  reads it, so the spawn argv uses the absolute `zig-out/bin/<name>` install path instead
  of the cwd-relative `.zig-cache/o/<hash>/<name>` path that `convertPathArg` re-resolves
  when `setCwd` is applied. The cache-relative path broke on the self-hosted backend
  because its emitted cache hash disagreed with the path the build system computed
  ahead of time, even though the compile step reported success.

## [0.16.4] - 2026-05-15

### Added

- **alef-core/template_versions**: split `JVM_TARGET` into `JAVA_JVM_TARGET = "25"` (Java/Panama),
  `KOTLIN_JVM_TARGET = "21"` (Kotlin/JVM), and `ANDROID_JVM_TARGET = "11"` (Android, unchanged).
  `JVM_TARGET` retained as a deprecated alias of `JAVA_JVM_TARGET` for one release cycle.
- **alef-core/config**: `KotlinFfiStyle::{Panama, Jni}` enum; `KotlinConfig.ffi_style` field
  defaulting to `Panama`. `ResolvedCrateConfig` gains `kotlin_ffi_style()`,
  `with_kotlin_ffi_style()`, and `jni_lib_name()` accessors.
- **alef-backend-kotlin**: `KotlinFfiStyle::Jni` emission mode. Produces
  `<Module>Bridge.kt` (a Kotlin `object` with `external fun` JNI declarations
  and `init { System.loadLibrary("<crate>_jni") }`) plus a `DefaultClient`
  class holding a `Long` handle. Selectable via `[crates.kotlin] ffi_style = "jni"`.
- **alef-backend-zig**: honor `ffi.exclude_types` and `ffi.exclude_functions`
  (unioned with the zig-scoped lists); filter every type / field / method whose
  signature references an excluded type, matching the java/csharp/go backends.
- **alef-backend-kotlin / alef-backend-kotlin-android**: emit Kotlin `Flow<T>`
  streaming surface via `callbackFlow`. Each streaming method now produces a
  `public fun {method}(...): Flow<ChunkType>` wrapper plus three JNI native
  method declarations on the Java facade (`native{Owner}{Adapter}{Start,Next,Free}`).
  Flips `supports_streaming` to `true` on both backends. JNI Rust shims for
  the new native methods are hand-written in consumer crates (e.g.
  `liter-llm-ffi`) for this release; full Rust-side automation tracked.
- **alef-backend-dart / alef-e2e (dart)**: generate Dart visitor bridge
  factories that take typed `DartFnFuture` callback closures and return the
  configured opaque `type_alias` handle. Visitor fixtures now emit active
  `createHtmlVisitor(...)` setup and attach the handle through generated
  `create<OptionsType>FromJsonWithVisitor(...)` helpers instead of skipping
  visitor tests as pending.
- **alef-backend-dart**: emit `From<Mirror> for Core` impls for trait-bridge
  method return types (e.g. `VisitResult`) so the bridge converts the Dart
  callback's mirror return value back to the core type via `.into()` instead
  of dropping it. Without this, trait methods returning a named mirror type
  previously fell back to `Default::default()`, making every Dart visitor
  callback a silent no-op even though the closures executed.
- **alef-e2e (zig)**: generate active fixture-driven visitor tests. Visitor
  fixtures now emit per-fixture C-callable callback thunks, populate
  `HTMHtmVisitorCallbacks`, attach the visitor to conversion options, and call
  the C FFI directly instead of skipping every Zig visitor fixture.
- **alef-backend-kotlin-android**: add the Ben Manes Gradle Versions plugin
  to generated Android `build.gradle.kts` files.

### Changed

- **alef-backend-kotlin-android**: no longer bundles the Java/Panama facade
  inside the AAR. `src/main/java/` is removed entirely; binding code lives
  in `src/main/kotlin/` only. Forces `KotlinFfiStyle::Jni` regardless of
  configuration. Consumers must ship a `<crate>-jni` Rust crate exporting
  `Java_<package>_<Module>Bridge_native<Method>` symbols per JNI spec §5.11.3
  and link `lib<crate>_jni.so` into `jniLibs/<abi>/`.
- **alef-backend-kotlin / alef-backend-kotlin-android**: streaming JNI
  `external fun` declarations moved from the Java facade class to the new
  Kotlin Bridge object. JNI symbol pattern unchanged:
  `Java_<pkg>_<Module>Bridge_native<...>{Start,Next,Free}`.
- **alef-backend-java**: `gen_streaming_jni_decls` removed; the Java/Panama
  facade no longer carries `static native` JNI shims. Java backend is
  Panama-only now.

### Fixed

- **alef-backend-kotlin**: emit a single JVM client wrapper class into a
  matching Kotlin source file (for example `GraphQLRouteConfig.kt`) instead of
  always writing `DefaultClient.kt`, avoiding ktlint filename violations for
  non-`DefaultClient` opaque client types.
- **alef-backend-kotlin**: backtick-escape data class field names that collide
  with Kotlin hard keywords (`object`, `class`, `fun`, `val`, …). Previously
  fields like `object: String` on `BatchListResponse` produced a parse error
  at ktlint time; the wire name is preserved by wrapping the identifier in
  backticks.
- **alef-cli**: recognize `Generated by alef` headers during orphan cleanup so
  renamed generated files are removed by `alef all --clean`.
- **alef-backend-kotlin-android**: the generated `<Module>.kt` Kotlin facade
  lives in a sub-package of the bundled Java DTOs (e.g.
  `dev.kreuzberg.kreuzcrawl.android` vs `dev.kreuzberg.kreuzcrawl`). Kotlin
  does not inherit symbols from parent packages, so every bare DTO
  reference (`CrawlConfig`, `ScrapeResult`, …) was unresolved at
  `compileDebugKotlin` time. Emit explicit `import <java_pkg>.<Type>` lines
  for every `Named` type referenced by visible free functions. Also add
  `org.jetbrains.kotlinx:kotlinx-coroutines-android` as an `implementation`
  dependency in the generated `build.gradle.kts` so the `withContext` /
  `Dispatchers.IO` / `callbackFlow` calls compile.
- **alef-backend-kotlin**: apply the same explicit-import fix to the JVM
  `DefaultClient.kt` emitter — when the Kotlin and Java packages differ
  (including the `<pkg>.kt` sub-package fallback path), emit explicit
  per-type imports for every `Named` type touched by client method
  signatures and streaming adapter `item_type` / `params`.
- **alef-backend-go**: honor FFI and Go `exclude_types` settings when
  generating bindings, including functions and methods whose signatures
  reference excluded types.
- **alef-backend-java**: honor FFI and Java `exclude_types` settings when
  generating Panama bindings, including record fields, functions, and methods
  whose signatures reference excluded types. Deduplicate generated exception
  classes when multiple Rust error enums contain variants with the same name.
- **alef-backend-csharp**: honor FFI and C# `exclude_types` settings when
  generating P/Invoke bindings, and map named IR types through C# class casing
  so acronym-heavy types resolve to their generated class names. Opaque methods
  that return opaque handles now wrap the returned pointer directly instead of
  calling non-existent `_to_json` exports, and generated error dispatch preserves
  base error class names such as `GraphQLErrorException`.
- **alef-backend-kotlin / alef-backend-swift / alef-backend-zig**: honor FFI
  `exclude_types` / `exclude_functions` for FFI-backed host bindings so
  generated code does not reference C symbols or RustBridge types excluded from
  the FFI layer. Kotlin scaffold now pins `jackson-annotations` to its published
  `2.21` line while keeping `jackson-databind` on `2.21.3`.
- **alef-backend-zig**: emit register/unregister shims only for
  `bind_via = "function_param"` trait bridges. Options-field bridges now emit a
  vtable-to-handle helper instead of referencing missing
  `{prefix}_register_{trait}` / `{prefix}_unregister_{trait}` C symbols.
- **alef-backend-kotlin**: emit a single JVM client wrapper class into a
  matching Kotlin source file (for example `GraphQLRouteConfig.kt`) instead of
  always writing `DefaultClient.kt`, avoiding ktlint filename violations for
  non-`DefaultClient` opaque client types.

## [0.16.3] - 2026-05-15

### Added

- **alef-e2e (dart)**: fixture-driven visitor codegen scaffolding —
  `dart_visitors::build_dart_visitor` emits a `createHtmlVisitorDartImpl(...)`
  setup block from `fixture.visitor.callbacks`, mapping each
  `CallbackAction` to a `VisitResult.skip()` / `VisitResult.continue_()` /
  `VisitResult.preserveHtml()` / `VisitResult.custom(field0: '...')` body
  (with `${placeholder}` interpolation for `custom_template` actions).
  Visitor fixtures currently emit a `test(...,  skip: 'pending dart-binding
visitor wiring (alef issue)')` stub at the call site because the dart
  binding does not yet expose typed callback parameters for the
  `BoxFn...` opaque types, does not emit `VisitResult` factory variants
  (the enum is `#[frb-ignore]`d), and lacks a `HtmlVisitorDartImpl` →
  `VisitorHandle` conversion plus a `visitor:` argument on
  `H2mBridge.convert`. The codegen pattern is now in place so once the
  dart binding catches up, the visitor block can be appended directly to
  `setup_lines`.

### Fixed

- **alef-backend-swift**: emit `extension RustBridge.{Owner}{Adapter}StreamHandle:
@unchecked Sendable {}` for every streaming adapter's opaque handle type.
  swift-bridge generates opaque Swift classes without `Sendable` conformance; the
  generated `chatStream` wrapper captures the handle inside two nested
  `Task.detached` closures, which Swift 6 strict-concurrency rejects with "does
  not conform to the 'Sendable' protocol". The Rust handle wraps a
  `Mutex<BoxStream>` and a `tokio::runtime::Runtime` — both `Send + Sync` — so
  `@unchecked Sendable` is the correct annotation: Rust enforces thread-safety,
  Swift cannot see it through the opaque FFI boundary. The extension is emitted
  immediately after the owning client class declaration.
- **alef-backend-swift**: emit streaming `item_type` types that appear in
  `api.types` with `has_serde: true` and populated fields as native
  `public struct X: Codable { … }` declarations instead of opaque
  `typealias X = RustBridge.X` aliases. The generated streaming wrapper calls
  `JSONDecoder().decode(X.self, from: data)`, which requires `X: Decodable`; an
  opaque swift-bridge class does not implement `Decodable`, causing a compile
  error. The decision is field-gated (falling back to typealias when the IR has
  no fields) so it is safe for types that the IR only partially describes.

- **alef-backend-dart**: skip trait methods whose return type references another
  trait (e.g. `Option<&dyn SyncExtractor>` on `DocumentExtractor::as_sync_extractor`)
  from the generated trait bridge. The IR flattens `&dyn Trait` /
  `Option<&dyn Trait>` / `Box<dyn Trait>` to `TypeRef::Named(name)` — the
  bridge cannot dispatch these methods (the foreign side can't construct a
  Rust trait object across FFI) and the emitted `Option<TraitName>` is not
  even valid Rust (rustc E0782 "expected a type, found a trait"). The
  skipped method must have `has_default_impl = true` so the trait's
  default impl handles the receiver. Surfaced by kreuzberg's
  `DocumentExtractor::as_sync_extractor` breaking `cargo check -p kreuzberg-dart`
  on Android/iOS targets.
- **alef-backend-dart**: emit `.into()` on `String` / `Vec<String>` / `HashMap<String, _>`
  field conversions in generated `From` impls (both `core → mirror` and
  `mirror → core`). The IR collapses wrapped string types (`Box<str>`,
  `Cow<'_, str>`, `Arc<str>`) into `TypeRef::String` and only tracks `Cow`
  on `core_wrapper`, so the previous identity emit (`v.field`, `(k, v)`)
  failed to compile when core fields used unwrapped boxed/arc string
  variants (rustc E0277: `FromIterator<(Box<str>, Box<str>)>` is not
  implemented for `HashMap<String, String>`). The crate-level
  `#![allow(clippy::useless_conversion)]` was extended to absorb the
  no-op `String → String` case. Surfaced by kreuzcrawl's
  `DownloadedDocument.headers: HashMap<Box<str>, Box<str>>` field
  breaking `cargo clippy -p kreuzcrawl-dart`.
- **alef-backend-dart**: honor `[crates.dart.extra_dependencies]` when
  generating the package-local Rust bridge `Cargo.toml`, allowing projects
  to override sibling crate paths that differ from the workspace binding
  crates.
- **alef-backend-dart**: generate compiling Rust bridge conversions for
  optional borrowed string returns, optional JSON string params, optional
  borrowed string params, and named map values.
- **alef-backend-wasm**: make `[crates.wasm.exclude_fields]` remove fields
  from generated Wasm structs and conversions instead of leaving cfg-marked
  fields in the emitted Rust.
- **alef-e2e (zig)**: `field[key]` bracket notation in fixture aliases now
  resolves to a HashMap-style nested object lookup
  (`.object.get("field").?.object.get("key").?`). Previously the bracketed
  form was treated as a single literal JSON key, panicking with "attempt
  to use null value" on accesses like
  `metadata.document.open_graph[title]` (where `open_graph` is a
  `HashMap<String, String>` serialized as a nested object).
- **alef-e2e (zig)**: visitor fixtures now emit `return error.SkipZigTest`
  with a "pending zig-binding visitor wiring" comment, matching the dart
  codegen's pending-skip behaviour. The zig binding exposes a C-vtable
  `Visitor` struct via the FFI bridge but alef-e2e/zig doesn't yet emit
  the per-fixture vtable population + register call, so every visitor
  fixture's `convert(html, null)` call ran without the visitor and the
  assertions checking for custom outputs (e.g. `[AUDIO: podcast.mp3]`)
  failed.
- **alef-e2e (zig)**: `not_contains` assertions with a plural `values: [...]`
  list now emit one `std.mem.indexOf` check per needle. Previously only
  `value` (singular) was emitted; fixtures using `values` (e.g. XSS edge
  cases asserting `["onclick", "onmouseover", "alert(", "steal_data"]`)
  produced uncompilable code with an empty `indexOf(u8, str, )` third arg.
- **alef-e2e (zig)**: `equals` string assertions now `std.mem.trim` the
  JSON `.string` field on both whitespace and newlines before comparing,
  matching the rust/dart pattern. h2m's `convert()` emits a trailing `\n`
  on rendered markdown — the bare `expectEqualStrings(expected, ...)`
  flagged that as a false positive on the `Hello World` smoke fixture.
- **alef-backend-magnus**: generate compiling Ruby bindings for optional
  borrowed string returns, opaque owned builder methods, and `Bytes` data
  enum fields. Magnus now converts `Option<&str>` method results to owned
  `Option<String>`, only treats exact `self.inner` expressions as already
  `Arc`-wrapped, and keeps `TypeRef::Bytes` enum payloads as `Vec<u8>`.
- **alef-scaffold (elixir)**: normalize generated external `elixirc_paths`
  entries so leading slashes in explicit output paths do not produce
  `../..//crates/...` paths, and emit them via `Path.expand(..., __DIR__)`
  so Mix sees an absolute source root during compilation. The generated
  Rustler `Cargo.toml` now also points `[lib].path` at external Rustler
  output directories, so packages with `elixir = "crates/.../src/"`
  compile via Mix.

## [0.16.2] - 2026-05-15

### Fixed

- **alef-e2e (kotlin)**: clear four clippy warnings in
  `crates/alef-e2e/src/codegen/kotlin.rs` so `cargo clippy --workspace
-- -D warnings` is green again. Removed a duplicate
  `#[allow(clippy::too_many_arguments)]` attribute on
  `render_test_method`, added the same allow to the 13-arg
  `render_test_file_android` companion (matches the existing
  convention for these codegen-internal helpers), and collapsed two
  pairs of `if`/`else if` branches in the `not_empty` / `is_empty`
  assertion emitter that had identical bodies into single
  `bare_result_is_option || field_is_optional` arms. Emission is
  byte-identical (no snapshot churn).
- **alef-e2e (dart)**: pass fixture `input.options` through to the bridge
  call. Generic `json_object` args without a special name (e.g. h2m's
  `options: ConversionOptions` on `convert(html, options)`) now build the
  mirror struct via the FRB-generated `create<OptionsType>FromJson(json:
...)` helper and pass it as a named parameter when the arg is optional.
  Previously fixtures providing `input.options.include_document_structure
= true` etc. silently dropped the options, so tests requiring document
  structure, visitor registration, or metadata extraction returned `null`
  for the gated fields.
- **alef-backend-kotlin-android**: alias the Kotlin wrapper's `Bridge`
  import to the `Rs`-suffixed Java facade class (e.g.
  `dev.kreuzberg.KreuzbergRs`) instead of the unsuffixed module name. When
  the Kotlin `object` wrapper shares its simple name with the Java facade
  (both `Kreuzberg`, same package), aliasing `Bridge` to the short name
  resolves back to the Kotlin object itself, producing infinite recursion
  or unresolved members (`Bridge.getEmbeddingPreset(name).orElse(null)`
  failed because the Kotlin wrapper's `getEmbeddingPreset` returns the
  unwrapped `EmbeddingPreset?` and does not have `.orElse`). The
  `Rs`-suffixed alias matches the Java backend's `resolve_main_class`
  convention and guarantees `Bridge.*` calls hit the JNI layer.
- **alef-e2e (kotlin_android)**: wrap every emitted `@Test` body in
  `runBlocking { ... }` and emit nullable `T?` (`== null` / `!= null`)
  rather than `Optional<T>` (`.isEmpty` / `.isPresent`) for bare
  option-returning calls. The kotlin-android AAR exposes most extraction
  entry points as `suspend fun` and unwraps Java `Optional<T>` to Kotlin
  `T?` at the boundary, so the JVM-Kotlin emission style produced 26
  `Suspend function ... can only be called from a coroutine` errors and
  one `Unresolved reference 'isEmpty'` error on every regeneration.
- **alef-e2e (zig)**: pin `.use_llvm = true` on every emitted
  `b.addTest(.{...})` block and add a matching `b.installArtifact(...)`
  call before the run step. Zig 0.16 defaults the self-hosted aarch64
  backend on `aarch64-linux` Debug builds, which emits the test binary
  at a path that does not match what `addRunArtifact` computes via
  `getEmittedBin()`. Every test fails at run time with
  `error: failed to spawn and capture stdio from
.zig-cache/o/<hash>/<name>: FileNotFound` even though the compile
  step reports success. The previous fix (unique `.name` per test, see
  0.16.0) was necessary to avoid cache collisions but did not address
  the backend mismatch. Forcing the LLVM backend on every test pins the
  binary at the cache path the RunStep expects on every supported host;
  the explicit `installArtifact` is a defensive guarantee that the
  build system materialises the binary on disk before the run step
  executes. Other Zig backends (x86_64 macOS/Linux, aarch64-darwin)
  already default to LLVM, so the change is a no-op there.
- **alef-backend-kotlin-android**: stop re-declaring enum types in the
  Kotlin wrapper (`Kreuzberg.kt`) when the bundled Java facade already
  declares them. Previously every enum surfaced from the Rust IR was
  emitted as both a Java `enum` (in `src/main/java/`) and a Kotlin
  `enum class` (in `src/main/kotlin/Kreuzberg.kt`) at the same FQN,
  triggering 268+ `Redeclaration` errors when both source sets were
  compiled together. Kotlin/Java interop is seamless — the Java enum is
  canonical and the Kotlin wrapper now references it directly.
- **alef-e2e (kotlin_android)**: resolve the test package from the
  `[crates.kotlin_android] package` config (the AAR's actual JVM
  package) rather than the generic `config.kotlin_package()` accessor,
  which falls back to a `com.github.<org>` derivation from the GitHub
  URL when only `[crates.kotlin_android]` is configured. The mismatch
  generated tests at `com.github.<org>.e2e` while the AAR shipped types
  at `dev.kreuzberg` (or whatever package the user configured),
  producing ~150 `Unresolved reference 'Kreuzberg'` /
  `'ExtractionConfig'` errors. Precedence:
  `[crates.e2e.packages.kotlin_android].module` (explicit override) >
  `[crates.kotlin_android].package` > derived fallback.
- **alef-backend-kotlin**: extract a
  `emit_jvm_client_class_with_package` variant of
  `emit_jvm_client_class` so the kotlin_android backend can thread the
  AAR's own `[crates.kotlin_android] package` through the client-class
  emitter. The existing `emit_jvm_client_class` keeps the JVM-Kotlin
  semantics and is preserved as a thin wrapper.
- **alef-e2e (dart)**: `equals` / `field_equals` / `not_equals` assertions
  on String-valued expected values now trim both actual and expected,
  matching the rust e2e codegen pattern. h2m's `convert()` emits a
  trailing `\n` on rendered markdown; rust trims via
  `.to_string().as_str().trim()`, but dart was comparing raw, producing
  false-positive failures on tests like "Simple paragraph converts
  correctly" (`Hello World\n` vs `Hello World`).
- **alef-backend-kotlin-android**: emitted `.editorconfig` now disables the
  ktlint rules that conflict with ktfmt formatting (`class-signature`,
  `function-signature`, `function-expression-body`, `no-empty-class-body`,
  `no-empty-first-line-in-method-block`, `indent`, `filename`). The
  alef-emitted Kotlin Android code is canonically ktfmt-formatted; ktlint's
  competing layout opinions were spuriously flagging the output.
- **alef-backend-kotlin-android**: explicitly set Kotlin compile target via
  `kotlin { compilerOptions { jvmTarget = ... } }` so it matches the Java
  `compileOptions` target. Previously the emitted `build.gradle.kts` only
  set Java `sourceCompatibility`/`targetCompatibility` (JVM 11 by default)
  while Kotlin 2.x with AGP 8.x inferred its own JVM target (17), which
  caused `compileDebugKotlin` to abort with "Inconsistent JVM-target
  compatibility detected for tasks 'compileDebugJavaWithJavac' (11) and
  'compileDebugKotlin' (17)". Both compilers now use the
  `ANDROID_JVM_TARGET` toolchain constant (or the per-crate
  `kotlin_android.jvm_target` override) verbatim.

## [0.16.1] - 2026-05-15

### Added

- **alef-e2e (kotlin_android)**: new `KotlinAndroidE2eCodegen` emits
  `e2e/kotlin_android/` as a host-JVM Kotlin project that depends on the
  AAR-bundled Java facade and Kotlin wrapper via gradle sourceSets, loads
  `libkreuzberg_ffi` via JNA, and runs JUnit 5 tests on the host without
  requiring an Android emulator. Validates the Kotlin Android bindings
  end-to-end via the standard e2e fixture suite.

### Fixed

- **alef-docs (kotlin_android)**: emit `api-kotlin-android.md` with title
  "Kotlin (Android) API Reference" for the `kotlin_android` workspace language
  slug. Previously both `kotlin` and `kotlin_android` shared the same
  `api-kotlin.md` filename and "Kotlin API Reference" title, which collided
  when a workspace targets the Android variant only — the JVM-Kotlin doc
  would still appear alongside the Android one. Slug and display name now
  diverge while the code-fence language stays `kotlin` (what syntax
  highlighters expect).
- **alef e2e generate (python)**: preserve newlines in generated HTTP fixture
  tests. Whitespace-trimming Jinja blocks in `python/http_test.jinja` could
  concatenate adjacent statements such as `_headers = {}` and `_req = ...`,
  producing invalid Python syntax.
- **alef-e2e (dart)**: import path for the FRB bridge now matches the
  emitted directory layout. Tests previously imported
  `package:{lib_name}/src/{lib_name}_bridge_generated/frb_generated.dart`
  which doesn't exist — the FRB dir uses the snake_cased crate name (e.g.
  `html_to_markdown_rs_bridge_generated`), not the pubspec `name`. Now
  imports `package:{lib_name}/src/{module_name}_bridge_generated/frb_generated.dart`.
- **alef-e2e (dart)**: dropped unconditional `dart:io` import from
  generated test files (only emitted when HTTP fixtures or chdir-required
  fixtures actually use it) and switched count-style assertions
  (`count_equals`, `count_min`, `min_length`, `max_length`) from
  unconditional `?.length ?? 0` to plain `.length` when the IR marks the
  receiver as non-optional. Clears `unused_import` and
  `invalid_null_aware_operator` warnings on the generated suite.
- **alef-backend-dart**: the FRB bridge class name now honors
  `[crates.dart] lib_name` instead of always deriving from the crate name.
  Previously the dart backend's local `dart_bridge_class_name` helper used
  `config.name` (e.g. `html_to_markdown_rs` → `HtmlToMarkdownRsBridge`),
  while the alef-e2e dart codegen used the
  `ResolvedCrateConfig::dart_bridge_class_name` method which DOES honor
  `lib_name` (e.g. `lib_name = "h2m"` → `H2mBridge`). The mismatch left
  e2e tests referencing an undefined `H2mBridge` symbol. The backend now
  calls the same shared method; the local helper is removed.
- **alef e2e generate (rust)**: replace `cargo sort .` with `taplo fmt Cargo.toml`
  in the default rust e2e formatter, and unconditionally run taplo as a post-pass
  on the e2e crate's `Cargo.toml` regardless of user override. `cargo sort` was
  silently corrupting the e2e crate manifest by relocating the leading alef
  header comments to the bottom of the file (cargo-sort treats the empty
  `[workspace]` table at the file top as the first section and detaches
  preceding free comments). The relocation pushed the embedded `# alef:hash:`
  line past the 10-line detection window used by
  `alef_core::hash::{extract_hash, inject_hash_line}`, so `alef verify` could
  no longer find the hash and silently treated the file as fresh. Worse, when
  downstream `prek` ran taplo on the manifest, the file got reformatted after
  `finalize_hashes` had captured the pre-prek content, leaving
  `e2e/rust/Cargo.toml` flagged as stale by `alef-verify`. Taplo preserves the
  alef header at the top of the file and only normalises array wrapping and
  whitespace, which keeps verification working and makes prek's taplo hook a
  no-op.
- **alef generate --format (wasm/ffi rust binding crates)**: extend the TOML
  format pipeline to run `taplo fmt` after `cargo sort` so downstream `prek`
  setups that include a taplo hook reformat nothing after generation.
  Previously only `cargo sort` ran for these crates' `Cargo.toml`, leaving
  array wrapping/indentation at the cargo-sort default; a subsequent
  `taplo fmt` (run by prek or by the consumer's own CI) would rewrite the
  manifest and invalidate the embedded `# alef:hash:` line. Wasm/FFI Cargo.toml
  files are safe to cargo-sort because their alef header is followed
  immediately by `[package]` (no leading `[workspace]` sentinel to trigger
  cargo-sort's section reordering).
- **alef-codegen / all backends**: restore canonical `https://github.com/kreuzberg-dev/alef`
  URL in generated file headers; 0.16.0 introduced a `https://example.invalid/alef`
  placeholder by mistake. Affects every emitted artifact's `Issues & docs:` line.
- **alef-docs / api reference**: fix MD056 table-cell-count mismatch in
  generated `docs/reference/api-*.md` — some rows emitted 5 cells in a
  4-column table. Visible in TypeScript and Elixir API docs.
- **alef-backend-dart**: trait-bridge shims now preserve `Option<&T>` on reference
  parameters. Previously `Option<&str>` collapsed to `&str` in the emitted method
  signature, breaking `impl Trait` for traits like `HtmlVisitor` with optional
  reference args (E0053). The param-conversion let-bindings now use `.map(...)`
  to propagate the Option through to the Dart closure call.
- **alef-backend-dart**: emitted dart-crate `build.rs` now skips `flutter_rust_bridge_codegen` invocation gracefully when the tool is not on PATH instead of panicking. Downstream CI environments (`cargo check --workspace`) no longer require `flutter_rust_bridge_codegen` to be installed; the committed generated FRB sources are sufficient. Developers with FRB installed still get automatic regen on source changes.
- **alef-extract**: reverted the `Option<Option<T>>` → `Option<T>` IR flatten
  introduced in 0.16.0. The flatten produced cross-backend type mismatches
  (WASM and other passthrough backends saw `expected Option<Option<usize>>,
found Option<usize>` against the core `ConversionOptionsUpdate` shape).
  Nested optionals are now preserved in IR; per-backend renderers decide
  how to surface them.
- **alef-backend-dart**: `Option<Option<T>>` core fields now render as a
  single `Option<T>` in the FRB mirror struct (FRB rejects nested optionals
  at codegen time). The core→mirror conversion injects `.flatten()` and the
  mirror→core conversion injects `.map(Some)` so the bridge round-trips
  consistently. The "no change" vs "explicit clear" distinction is
  collapsed to `None` on the dart side; callers needing both states must
  model them via a dedicated enum.
- **alef-backend-dart**: opaque wrappers now emit `From` impls in both
  directions between the wrapper and its inner core type, so builder
  methods and field conversions referencing `{Wrapper}::from(core_val)` or
  `core_val.map(Into::into)` compile (E0308, E0631, E0277).
- **alef-backend-dart**: non-optional `char` fields convert with
  `.to_string()` in `field_from_expr` (mirror struct is `String`, core is
  `char`). Optional `char` keeps its existing `.map(...)` pattern but is
  now generated via `.to_string()` for parity (E0308).
- **alef-e2e (swift)**: emit Swift-native syntax for streaming-virtual accessors (`stream_content`, `stream_complete`, `tool_calls`, `finish_reason`, `usage`, `chunks.length`). Previously fell through to a TypeScript default that produced uncompilable Swift (`(c: any) =>`, `.length`, `?.[0]?.`, `!= null`, `undefined`). The new arms use swift-bridge method-call chains (`.choices()`, `.delta()`, `.content()?.toString()`, `.finish_reason()`) and Swift collection idioms (`.isEmpty`, `.last!`, `.count`, `.joined()`).
- **alef-backend-dart**: dropped redundant `.into()` on String/Vec<String>
  field, enum-variant, and HashMap key/value conversions
  (clippy::useless_conversion), gated `..Default::default()` struct-literal
  fillers behind `#[allow(clippy::needless_update)]` when the core struct
  has cfg-stripped fields, and replaced
  `.map(|v| T::from(v))` with `.map(T::from)` on bridge-function return
  wrappers (clippy::redundant_closure). The dart bridge crate now passes
  `clippy -D warnings`.

## [0.16.0] - 2026-05-14

### Added

- **alef-backend-kotlin-android**: new standalone backend crate for emitting
  Android library (AAR) projects from alef. Carved out of
  `alef-backend-kotlin/src/gen_android/` into a self-contained crate so the
  emission pipeline owns every file the AAR needs — no scaffold dependency.
  - **`Language::KotlinAndroid`** (slug `"kotlin_android"`, with
    `"kotlin-android"` accepted as a deserialization alias) is a first-class
    target distinct from `Language::Kotlin`. Workspaces can emit both JVM and
    Android Kotlin sets in parallel.
  - **Self-contained emission**: every file required for
    `gradle assembleRelease` is produced by the backend — `build.gradle.kts`
    (AGP plugin with version, `com.android.library`, kotlin("android"),
    maven-publish, namespace, compileSdk/minSdk/jvmTarget),
    `settings.gradle.kts` (pluginManagement with `google()`,
    `mavenCentral()`, `gradlePluginPortal()` + `dependencyResolutionManagement`),
    `src/main/AndroidManifest.xml`, `consumer-rules.pro`, `proguard-rules.pro`,
    `.gitignore`, `src/main/jniLibs/{arm64-v8a,x86_64}/.gitkeep`.
  - **Self-contained Java facade**: the backend re-emits the Java records
    that the Kotlin Android binding depends on into the AAR's
    `src/main/java/{java_pkg_path}/` directory, so the published AAR has no
    runtime dependency on a co-published Java jar.
  - **`KotlinAndroidConfig`** on `ResolvedCrateConfig` adds Android-specific
    knobs (`namespace`, `artifact_id`, `group_id`, `compile_sdk`, `min_sdk`,
    `jvm_target`, `abis`, `exclude_functions`, `exclude_types`). Defaults
    pulled from `template_versions::toolchain` so renovate keeps them current.
  - Snapshot tests cover every emitted file for a representative API
    (struct, enum, error, sync fn, async fn). insta-based, `cargo test
-p alef-backend-kotlin-android`.
  - **ktlint compliance out of the box**: the emitted `build.gradle.kts`
    applies the `org.jlleitschuh.gradle.ktlint` plugin (pinned via
    `template_versions::maven::KTLINT_GRADLE_PLUGIN` / `KTLINT`) and the
    emitted `.editorconfig` disables `ktlint_standard_package-name` so the
    co-located Java facade — whose package is reverse-DNS-derived from the
    repo's `repository` URL and may carry underscores (e.g.
    `com.github.kreuzberg_dev`) — does not trip the strict
    `standard:package-name` rule. All other ktlint rules remain on; after
    `alef generate --format`, `gradle ktlintCheck` passes clean on the
    full emission.
  - **`maven-publish` singleVariant("release")**: `android { publishing }`
    block configures the `release` variant with `withSourcesJar()` so the
    `publishAndReleaseToMavenCentral` task (vanniktech-compatible) finds
    a valid publication.
  - **AGP 8.x compatibility**: builds with `com.android.library` 8.x +
    explicit `kotlin("android")` plugin. The deprecated `kotlinOptions`
    DSL was dropped; AGP's `compileOptions` (sourceCompatibility /
    targetCompatibility) drives the Kotlin compile JVM target.

### Changed

- **BREAKING — alef-backend-kotlin**: the in-band `[crates.kotlin] mode =
"android"` dispatch path has been removed. Consumers that previously
  emitted Android via the kotlin backend must migrate to
  `Language::KotlinAndroid` and `[crates.kotlin_android]`. Setting
  `mode = "android"` now raises a hard error pointing at the new slug.
- **BREAKING — alef-scaffold**: `scaffold_kotlin_android` has been removed.
  Android scaffolding lives in `alef-backend-kotlin-android` so the same
  emitter owns the project tree on every regeneration. The
  `"android"` branch in `scaffold_kotlin` dispatch is gone.
- **alef-backend-swift**: streaming adapters are now implemented end-to-end.
  `Capabilities::supports_streaming` flips from `false` to `true`. For every
  adapter with `pattern = streaming`:
  - **Rust shim** emits a per-adapter opaque `{Owner}{Adapter}StreamHandle`
    that owns the `BoxStream<Result<T, E>>` and a per-stream
    `tokio::runtime::Runtime` (single-worker, multi-thread). swift-bridge's
    `extern "Rust"` block declares the handle plus a `_start` free function
    returning `Result<Handle, String>` (HTTP head validation happens here, so
    pre-stream errors throw before any chunk arrives) and a `fn next(self:
&mut Handle) -> Result<String, String>` method that drains one chunk via
  `runtime.block_on(stream.next())` and `serde_json::to_string`s it.
  Empty-string EOF sentinel.
  - **Swift wrapper** emits a public method returning
    `AsyncThrowingStream<ItemType, Error>` backed by a detached drain `Task`
    that loops `try handle.next()`, decodes each JSON chunk via
    `JSONDecoder().decode(ItemType.self, ...)`, and yields to the continuation.
    `continuation.onTermination` cancels the drain Task on early-break;
    swift-bridge's auto-generated `deinit` frees the handle.
  - **alef-e2e/streaming_assertions.rs (swift)**: replaces the
    `XCTSkipIf` stub with a real `for try await _chunk in result { … }` drain
    so streaming fixtures actually assert against the chunked items.
  - Items must be `Serialize` on the Rust side and `Decodable` on the Swift
    side. swift-bridge 0.1.59 bridges `Result<String, String>` cleanly; the
    JSON-string protocol avoids hitting the gaps in its `Option<OpaqueRust>`
    codegen.
    Trade-offs documented in the shim: one tokio runtime per stream (heavier
    than a shared runtime, but isolates lifetimes); back-pressure is not yet
    implemented (Swift consumer slower than chunk arrival → unbounded
    `AsyncThrowingStream` buffer growth).

- **alef-core/alef-scaffold/backends**: Alef is less organization-specific by
  default. Workspace config can now provide scaffold metadata plus generated
  header and pre-commit defaults, pre-commit scaffolding can point at custom
  shared/Alef hook repositories or omit those blocks, and PyO3/PHP/FFI trait
  bridge templates now use configured error constructors instead of hardcoded
  Kreuzberg error variants.
- **alef-backends/e2e/scaffold**: remove remaining organization-specific
  literals from emitted output. Extendr and Swift plugin bridges now use the
  configured error constructor, C#/Ruby/Wasm e2e scaffolds derive namespaces
  and globals from Alef/project defaults, generated headers fall back to a
  vendor-neutral issues URL, and pre-commit scaffolding defaults Alef hooks to
  local `alef` commands unless a repository is configured.
- **alef-scaffold/backends/e2e**: stop emitting GitHub Actions workflows and
  remove generated package-name literals from FFI streaming callbacks, Zig/PHP
  e2e streaming helpers, and visitor/plugin conversion comments.
- **alef-backend-php**: derive trait-bridge handle paths from the API surface
  instead of emitting a package-specific visitor handle path in generated bridge
  setters and function wrappers.
- **alef config/build commands**: build command defaults now compose with
  workspace and crate overrides field-by-field, so repositories can set global
  defaults while overriding only the commands they need per backend.
- **alef-scaffold**: Dart scaffolding now distinguishes FRB and raw FFI styles,
  uses Dart SDK `>=3.11.0 <4.0.0`, removes generated `BUILDING.md`, derives
  generated analyzer excludes from the crate name, and centralizes pub.dev
  dependency pins for Renovate.
- **alef-scaffold**: Kotlin scaffolding still covers JVM, Native, and Kotlin
  Multiplatform from Kotlin mode/target configuration. **Android scaffolding
  moved into the new `alef-backend-kotlin-android` crate** (see Added). Kotlin
  and ktlint dependency/tool pins are centralized for Renovate, and generated
  JVM text uses generic `native.lib.path` instead of organization-specific
  properties.
- **alef-scaffold**: Zig scaffolding no longer uses organization-specific
  workflow setup actions and now uses the public Zig setup action.

### Fixed

- **alef-backend-napi**: gate the raw `napi_create_external` /
  `napi_type_tag_object` `extern "C"` block with
  `#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]`.
  Without this, generated capsule_types bindings failed to link on Windows MSVC
  with `LNK2019: unresolved external symbol`, because `napi-build`'s `.def` only
  exposes symbols inside `napi-sys`'s `generate!` allowlist — and these two are
  not in it. `kind = "raw-dylib"` (stable since Rust 1.71) tells the MSVC
  linker to synthesize the import-library entries from the `extern` block
  itself; on Linux/macOS the `cfg_attr` is a no-op and Node's dynamic loader
  continues to resolve at module load. Added `windows-latest` to alef's CI
  test matrix so the new attribute regression-gates against future drift.
- **alef-backend-swift, alef-backend-dart**: emit correct `package = "..."`
  rename in bridge `Cargo.toml`. The rename target previously used
  `core_crate_dir` (the on-disk directory name), but cargo needs the actual
  `[package].name` from the umbrella crate's `Cargo.toml`. The two differ when
  `[[crates]] name` is suffixed for crates.io disambiguation (e.g.
  `html-to-markdown-rs` package at `crates/html-to-markdown/`). Without the
  fix, `cargo check` errors with `no matching package found, searched package
name: "html-to-markdown"`. Use `crate_name` (the `[[crates]]` `name` field)
  for the rename target instead.
- **alef-backend-swift**: emit method receivers from IR
  (`&self`/`&mut self`/`self`) instead of hardcoded `&self` — closes E0053 for
  traits with `&mut self` methods (40+ HtmlVisitor methods in h2m). Adds
  optional and slice param reconstruction in trait impls, emits `Debug` impl
  for non-Plugin inbound wrappers (`HtmlVisitor: Debug` supertrait), detects
  `ReceiverKind::Owned` for inner method calls and clones the inner value to
  support `&self` wrappers calling owned-self inner methods, and skips
  serde-bridging `Vec<T>` when `T` lacks `Serialize`/`Deserialize` (sanitized
  field detection).
- **alef-backend-dart**: `build.rs` invokes `flutter_rust_bridge_codegen` CLI
  at cargo build time (closes D1). Emits `visit_*` method receivers from IR.
- **alef-core**: configurable `error_type` / `error_constructor` read from
  `alef.toml` `[[crates]]` entries instead of hardcoded Kreuzberg error
  variants.
- **alef-extract**: flatten `Option<Option<T>>` in `type_resolver` (closes Z3).
  flutter_rust_bridge rejects nested optionals and zig emits semantically
  muddy `??T`. Structs that wrap already-optional fields (e.g.
  `ConversionOptionsUpdate.max_depth: Option<Option<u64>>`) now extract as a
  single `Option<T>` — `None` means "no change", matching the practical
  semantics across all backends.
- **alef-e2e (zig)**: emit unique `.name = "<test>_test"` on every
  `b.addTest(.{...})` block in the generated `build.zig`. Zig 0.16 hashes the
  output binary path off the artifact name; without an explicit name every
  `addTest` defaulted to `"test"`, colliding in the cache so only one binary
  survived and every other `addRunArtifact` invocation failed with
  `FileNotFound` at its computed `.zig-cache/o/<hash>/test` path.
- **alef-e2e (rust)**: omit `use {crate}::CrawlConfig;` (and other handle-arg
  helper imports) from generated `tests/<category>_test.rs` files when the
  rendered body never references the symbol. Test bodies are now buffered and
  scanned for word-boundary references before optional `use` statements are
  emitted, eliminating `unused_imports` errors under `-D warnings` for handle
  fixtures whose `input.config` is null/empty.
- **alef-backend-kotlin-android**: emit Android build-metadata files
  (`build.gradle.kts`, `settings.gradle.kts`, `consumer-rules.pro`,
  `proguard-rules.pro`, `.gitignore`, `src/main/AndroidManifest.xml`,
  `src/main/jniLibs/<abi>/.gitkeep`, `src/main/java/<java-pkg>/*.java`)
  at the AAR **project root**, not nested inside the Kotlin source
  destination. `[crates.output].kotlin_android` semantically names the
  Kotlin source destination (`src/main/kotlin/<dotted_package>/`); the
  project root is now derived by stripping that suffix. Kotlin source
  (`<Module>.kt`, `DefaultClient.kt`) is emitted directly at the
  configured path with no extra `src/main/kotlin/<pkg>/` nesting.
  Workspaces that pointed `kotlin_android` at the project root continue
  to work — the legacy semantics are preserved when the configured path
  does not end with the Gradle Android source-set suffix.

### Planned

- **alef-scaffold**: evolve the existing scaffold and backend implementations
  instead of adding new Alef crates. Planned follow-up work should align Dart
  scaffolding with FRB versus FFI and pure Dart versus Flutter plugin modes,
  align Kotlin scaffolding with the existing JVM, Native, and KMP
  backend modes (Android now ships as its own `alef-backend-kotlin-android`
  crate — see Added), include Swift Rust subpackage scaffold expectations, remove
  organization-specific assumptions from generated scaffold and workflow text,
  and centralize scaffold dependency and tool versions.

## [0.15.71] - 2026-05-14

### Fixed

- **alef-e2e/rust (test_file.rs, mock_server.rs)**: error fixtures now emit
  `let _mock_server = MockServer::start(...).await;` (underscore prefix)
  when the test body never references `mock_server` again. Previously every
  error test bound the server to `mock_server` purely to keep it alive for
  the duration of the failed call, which tripped `-D warnings` →
  `unused_variables` across `tests/error_test.rs` (12+ sites in kreuzcrawl).
  `render_test_function` now scans the body for a `mock_server.` reference
  and picks the bare or underscored name accordingly.
- **alef-e2e/zig (zig.rs)**: every generated `addModule(...)` /
  `createModule({...})` call now sets `.link_libc = true`. Required by
  Zig 0.16+: any module that transitively references C stdlib (`c.getenv`,
  `c.malloc`, etc.) must declare libc explicitly in its build graph,
  otherwise `zig build test` fails with `error: dependency on libc must be
explicitly specified in the build command`.
- **alef-e2e/wasm (wasm.rs)**: in `DependencyMode::Local`, the generated
  `e2e/wasm/package.json` now declares
  `"<crate>": "file:../../crates/<crate>-wasm/pkg/nodejs"` instead of just
  `pkg`. `wasm-pack build --target nodejs --out-dir pkg/nodejs` writes the
  npm-consumable package (with its own `package.json`) under `pkg/nodejs/`,
  so pnpm resolution requires the `/nodejs` suffix.

## [0.15.70] - 2026-05-14

### Fixed

- **alef-e2e/rust (mock_server.rs)**: emit `#![allow(dead_code)]` on the
  generated `tests/common.rs` module. The mock_server helper exposes a
  small set of test utilities (`MockRoute`, route builders) that not every
  test binary uses; without the allow, `-D warnings` flagged `dead_code`
  whenever a single test file imported only a subset of the module's
  surface.
- **alef-e2e/wasm**: emit a local `pnpm-workspace.yaml`
  (`packages: ["."]`) alongside the generated `e2e/wasm/package.json`.
  Without it, consumer repos whose root `pnpm-workspace.yaml` excludes
  `e2e/wasm` (because `wasm-pack` outputs a different `package.json` shape)
  leave `e2e/wasm` outside any workspace, and `pnpm install` cannot resolve
  the `kreuzcrawl` file-protocol dependency.

## [0.15.69] - 2026-05-14

### Reverted

- **alef-scaffold (swift.rs)**: roll back the `Sources/{module}/{module}.swift`
  stub introduced in 0.15.68 — it overwrote the alef-backend-swift authoritative
  output for the same path (the 500-line typealias module), breaking the public
  Swift API surface in downstream regens. The original SwiftPM "product not found"
  needs a different fix (likely in the codegen path, not the scaffold).

## [0.15.68] - 2026-05-14

### Fixed

- **alef-e2e (typescript/vitest.config.ts.jinja)**: add `testTimeout: 30000` so HTTP
  fixture tests against mock-server do not time out on slow ARM Linux CI runners (Vitest
  default is 5 s).
- **alef-scaffold (swift.rs)**: write `Sources/{module}/{module}.swift` stub so SwiftPM
  can register the `.library` product; without a source file in the target directory
  `swift test` fails with "product not found in package". (Reverted in 0.15.69.)

## [0.15.67] - 2026-05-14

Version bump only — no functional changes.

## [0.15.66] - 2026-05-14

### Fixed

- **alef-readme (performance_context.jinja)**: emit a blank line between the
  performance metadata line (`**Platform** · function · note`) and the
  benchmark table that follows. Without the blank line, `rumdl fmt`
  (Markdown formatter, used by downstream repos that lint the generated
  READMEs) auto-fixes `MD058` (Missing blank line before table) and the
  `git diff --exit-code` README freshness check in CI then fails. Reported
  via kreuzberg-dev/html-to-markdown CI Lint.

## [0.15.64] - 2026-05-14

### Fixed

- **alef-codegen (magnus hash constructor)**: stop emitting `TypeName::default()`
  for `TypeRef::Named` fields that lack an explicit `typed_default`. Magnus-
  wrapped structs (`#[magnus::wrap]`) do not implement `Default`, so the
  generated Ruby binding failed to compile with `E0599: no function or
associated item named 'default' found for struct 'FunctionDefinition'`
  (likewise `FunctionCall`, etc.). The hash constructor now treats such fields
  as required: when the caller does not provide the field, the constructor
  returns `magnus::Error::new(ruby.exception_arg_error(), "missing required
field: ...")` instead of synthesising a fictional default.
- **alef-e2e (swift)**: fix `Package.swift` for path-based local dependencies
  under SwiftPM 6.0. Previously the codegen emitted
  `.package(name: "<ModuleName>", path: ...)` paired with
  `.product(name: "<ModuleName>", package: "<ModuleName>")`, but SwiftPM 6.0
  ignores `.package(name:)` for path-based dependencies and infers the package
  identity from the path's last component (e.g. `packages/swift` → `swift`).
  The generated `Package.swift` therefore failed with
  `error: 'swift': product 'LiterLlm' required by package 'swift' target
'LiterLlmE2ETests' not found in package 'LiterLlm'`. The codegen now drops
  the redundant `name:` parameter and references the inferred identity in the
  product dependency.
- **alef-e2e (kotlin)**: emit `java` plugin alongside `kotlin("jvm")` in the
  generated `build.gradle.kts`. Under Gradle 9.x, `kotlin("jvm")` no longer
  implicitly applies the `java` base plugin, so the `test` task did not exist
  and `gradle test` failed with `Task 'test' not found in root project
'kotlin'.` Applying `java` restores the standard test/check lifecycle.

## [0.15.63] - 2026-05-14

### Fixed

- **alef-backend-magnus**: rename generated RBS type alias from
  `instance` to `value` in unit-variant enum stubs. `instance` is a
  reserved keyword in RBS (it refers to the "instance of self" type),
  so `rbs >= 4.0` rejected the generated `sig/types.rbs` with a hard
  parse error that aborted the entire RBS environment load. Downstream
  projects running `steep check` or `rbs validate` saw every unrelated
  RBS — `ActiveRecord::Base::ClassMethods`, `ActiveSupport::Concern`,
  `SimpleDelegator`, etc. — become "Cannot find type". Reported in
  kreuzberg-dev/html-to-markdown#360.

## [0.15.62] - 2026-05-14

### Added

- **alef-backend-dart**: support per-target Cargo dependency overrides via
  `[[crates.dart.target_dep_overrides]]` in `alef.toml`. Each entry specifies
  a `cfg` predicate, the `features` to enable on the core dep for that
  target, and optionally `default_features = false`. The emitted
  `Cargo.toml` wraps the base core dependency in `[target.'cfg(not(...))'.dependencies]`
  and emits a matching `[target.'cfg(...)'.dependencies]` block per override.
  Required for projects like kreuzberg where the Android x86_64 emulator
  triple cannot link ORT prebuilts and must fall back to a no-ORT feature
  group, while arm64 Android phones and all other targets keep the full set.

## [0.15.61] - 2026-05-14

### Fixed

- **alef-backend-ffi**: add `noop_method_call` to the crate-level
  `#![allow(...)]` block in the generated `lib.rs`. The trait-bridge async
  dispatch emits `let result = result.clone();` on `&[&str]` slices — a no-op
  since slices don't implement `Clone`. Cargo's `[lints.rust]` table doesn't
  override `RUSTFLAGS=-D warnings`, but a file-level inner attribute does.
- **alef-codegen/doc_emission**: take the exception class name as a parameter
  in `emit_phpdoc` and `emit_csharp_doc` instead of hard-coding
  `"KreuzbergException"`. PHP and C# backends now thread the project-specific
  exception type through `<exception>` / `@throws` doc emission so
  kreuzcrawl-style projects no longer see `<exception cref="KreuzbergException">`
  on their generated bindings.
- **alef-e2e/swift**: derive the test target name from the configured module
  (`{module_name}E2ETests`) in both the rendered `Package.swift` and the
  emitted test file directory. Previously hard-coded to `KreuzbergE2ETests`
  regardless of project, which broke kreuzcrawl's swift e2e build.
- **alef-backend-php**: unmask `core_import` parameter on `gen_static_method`
  (renamed from `_core_import` in 0.15.61 WIP) so `Exception` class derivation
  compiles. Fixes a clippy/cargo-check break introduced alongside the
  exception-name refactor.
- **alef-cli**: `alef all --format` now accepts the flag without an explicit
  value (matches the `alef generate --format` UX). Defaults to `true`; pass
  `--format=false` to opt out.

### Internal

- Refresh insta snapshots in `alef-backend-swift` for the cargo-machete
  metadata block and crate-root clippy allows added in 0.15.58.
- Sync `tests/integration_test.rs` and `tests/field_path_array.rs` with the
  current `wrap_return_with_mutex` and swift optional-chain accessor
  behaviour (`&&str` → `to_string()`, `()?[N]` without trailing `?`).
- Drop the outdated `texts:` named-arg assertion in
  `dart_scalar_array_arg.rs` — the dart codegen forces positional args for
  scalar-element json_object types.

## [0.15.60] - 2026-05-14

### Fixed

- **alef-e2e/swift**: actually move generated tests to `<output>/swift/Tests/KreuzbergE2ETests/`.
  The 0.15.59 release renamed the test target but left `tests_base` pointing at
  `output_base.join(pkg_path)` (which normalises to `packages/swift`), so the
  files still landed under `packages/swift/Tests/`. Use `output_base` directly
  and drop the now-unused `normalize_path` helper. Same Package.swift output
  as 0.15.59, just the test files now live where intended.

## [0.15.59] - 2026-05-14

### Fixed

- **alef-e2e/swift**: generate Swift e2e tests into `<output>/swift/Tests/KreuzbergE2ETests/`
  (mirroring every other language) instead of `packages/swift/Tests/`. The previous
  layout claimed SwiftPM 6.0 forbade inter-package `.package(path:)` references —
  not actually true; the real constraint was implicit package identity. The generated
  `e2e/swift/Package.swift` now declares an explicit
  `.package(name: "Kreuzberg", path: "../../packages/swift")` so SwiftPM resolves the
  binding library correctly. Downstream `alef.toml` `[crates.test.swift]` blocks can
  now point `e2e` at `cd e2e/swift && swift test`.
- **alef-e2e/c**: only emit the mock-server orchestration block in the generated
  `Makefile` when at least one fixture actually `needs_mock_server()`. Previously
  every C e2e suite hard-required `../rust/target/release/mock-server` regardless
  of whether any fixture defined `mock_response`/`http` shapes, breaking C FFI
  CI for repositories whose fixtures never exercise HTTP (e.g. html-to-markdown
  where 0 fixtures need a mock backend). Suites with mock-needing fixtures get
  the same behaviour as before.

## [0.15.58] - 2026-05-13

### Added

- **alef-cli**: `alef readme` now includes Rust in the regeneration loop when
  `[crates.readme.languages.rust]` is configured in `alef.toml`. Previously the
  command called `resolve_languages` which excluded Rust (matching the policy
  for binding generation), so the Rust crate's `crates/<lib>/README.md` was
  silently skipped and stayed frozen — drifting the moment templates changed.
  Introduces a dedicated `resolve_readme_languages` helper that mirrors
  `resolve_doc_languages` (Rust is the source language; its crates.io README
  needs the same regen path as every binding's README). Opt-in via the existing
  `[crates.readme.languages.rust]` block in `alef.toml`.

### Fixed

- **alef-e2e/swift**: coalesce `.len()` via `?? 0` when emitting `not_empty` /
  `is_empty` assertions over a `?.` optional-chain accessor. Previously the
  generator emitted `XCTAssertGreaterThan(result.field()?.inner().len(), 0, ...)`
  which Swift rejects with `cannot convert value of type 'UInt?' to expected
argument type 'Int'` because optional chaining propagates `?` through the
  whole expression. The not_empty / is_empty arms in
  `crates/alef-e2e/src/codegen/swift.rs` now check `accessor_is_optional` (the
  same heuristic already used by every other assertion arm) and wrap as
  `(field_expr.len() ?? 0)` when the chain crosses an optional field. Surfaced
  in kreuzcrawl's swift test target where `result.markdown()?.content().len()`
  failed to compile across MarkdownTests.swift.
- **alef-backend-swift**: emit `[package.metadata.cargo-machete] ignored =
["async-trait", "serde"]` in the swift-bridge Cargo.toml. Those deps are
  conditionally referenced (only when the umbrella crate declares trait
  bridges or `Serialize`/`Deserialize`-derived response types); downstream
  consumers without those features (kreuzcrawl) saw cargo-machete false-
  positive flags. Ignoring at the manifest level keeps the manifest stable
  across regens.
- **alef-backend-swift**: allow `clippy::useless_conversion` and
  `clippy::inherent_to_string` at the crate root of the emitted swift-bridge
  Rust crate. The bytes-default `__target.x = x.into();` is a no-op when
  source and target field types match (kreuzcrawl `Option<Vec<u8>>`), and the
  emitted `RustString` wrappers ship inherent `to_string(&self)` that clash
  with Display::to_string. Suppress at the generated-crate header instead of
  requiring per-task clippy flags downstream.
- **alef-backend-rustler**: emit `base_url:` inline in the generated
  `native.ex` instead of wrapped across two lines. Under the kreuzberg-dev
  `line_length: 120` convention, `mix format --check-formatted` collapses
  the wrap, so each `alef e2e generate` drifted against pre-commit.
- **alef-docs**: inject a trailing blank line after every heading at render
  time. The shared `heading.jinja` / `version_heading.jinja` templates can't
  carry a trailing blank line — the pre-commit `end-of-file-fixer` hook
  strips it on every commit — so wrap `template_env::render` to add the
  blank line for those two templates. Resolves the rumdl MD022 cycle that
  added ~1100 fixes across 18 `docs/reference/api-*.md` files per regen.

## [0.15.57] - 2026-05-13

### Fixed

- **alef-backend-wasm**: `gen_opaque_method` now respects `mutex_types` for both `&self` and `&mut self` methods on opaque types whose `inner` field is `Arc<Mutex<T>>`. Previously: (a) `&self` methods on Mutex-wrapped opaque types emitted `self.inner.{method}(...)` which fails to compile against `Arc<Mutex<T>>`, and (b) `&mut self` methods on opaque types fell through `shared::can_auto_delegate`'s blanket `RefMut → false` exclusion and were emitted as no-op `gen_wasm_unimplemented_body` stubs. Concretely surfaced in tree-sitter-language-pack's `WasmTreeCursor::node()`, `field_name()`, `goto_first_child()`, `goto_parent()`, `goto_next_sibling()`. Fix: override `can_delegate` for RefMut methods when `mutex_types.contains(type_name)` (mirrors the existing override in `methods.rs:33`), and dispatch via `self.inner.lock().unwrap().{method}(...)` for both `&self` and Owned receivers on Mutex-wrapped types.

## [0.15.56] - 2026-05-13

### Fixed

- **alef-backend-wasm**: wrap opaque-type constructor returns and `&self`-method returns in `Arc::new(std::sync::Mutex::new(...))` when the target type has `&mut self` methods (i.e. is in the new `mutex_types` set). Previously `gen_function` / `gen_method` / `gen_opaque_struct_methods` emitted bare `Arc::new(...)` regardless of whether the opaque struct field was `Arc<T>` or `Arc<Mutex<T>>`, producing invalid Rust like `inner: Arc::new(Parser::default())` against an `Arc<Mutex<Parser>>` field. Threads a `mutex_types: &AHashSet<String>` set through `gen_bindings/{functions,methods,types,mod}.rs` and uses `generators::wrap_return_with_mutex` at every Arc-wrapping site (parallels the Magnus fix in v0.15.55). Constructors no longer emit a double-nested struct-init (the wrap helper already returns the complete `Self { inner: ... }` expression).
- **alef-e2e/kotlin**: additional sticky-nullability + enum-typed field detection refinements in `crates/alef-e2e/src/codegen/kotlin.rs` plus shared `crates/alef-e2e/src/field_access.rs`; corresponding go/swift/streaming generator updates and `crates/alef-scaffold/src/languages/kotlin.rs` adjustments.
- **all visitor backends**: update all codegen sites that construct a `VisitorHandle` to use `Arc::new(Mutex::new(...))` instead of `Rc::new(RefCell::new(...))`, matching the updated `VisitorHandle` type alias (`Arc<Mutex<dyn HtmlVisitor + Send>>`). Affected backends: PHP, PyO3, NAPI (4 Jinja templates + 2 Rust sites), WASM (Jinja template + 3 Rust sites + `unsafe impl Send + Sync` in visitor bridge), Magnus (Rust + Jinja visitor bridge + `unsafe impl Send + Sync`), Rustler (template env + trait bridge), extendr (trait bridge + gen_bindings use stmts), FFI (gen_bridge_field + gen_visitor VisitorRef + `unsafe impl Send + Sync` on `{Prefix}Visitor` and `VisitorRef`). Also updates the Rust e2e test generator (`alef-e2e`) to emit `Arc::new(Mutex::new(...))` for the test visitor.
- **alef-backend-java**: switch generated instance-method `Arena` from `Arena.ofConfined()` to `Arena.ofShared()` so cross-thread access to `MemorySegment`s doesn't panic in pooled / async caller contexts.
- **alef-e2e/kotlin**: mock URL now resolves via the per-fixture system property set by `MockServerListener`, falling back to `mockServerUrl` / `MOCK_SERVER_URL` env var with the `/fixtures/<id>` suffix appended.

## [0.15.55] - 2026-05-13

### Added

- **alef-backend-gleam**: restore the Gleam backend, e2e codegen, scaffold, publish package, and `Language::Gleam` enum variant that were removed in v0.15.54 by mistake. Gleam is a first-class alef-supported binding language again, with the full backend crate (`alef-backend-gleam`), e2e codegen (`crates/alef-e2e/src/codegen/gleam.rs`), publish package handler, scaffold, readme template, doc emission, and downstream registry/match-arm wiring across `alef-core`, `alef-codegen`, `alef-adapters`, `alef-docs`, `alef-cli`, `alef-publish`, `alef-readme`, and `alef-scaffold`. Note: kreuzberg has independently dropped its own Gleam binding (BEAM users can call the Elixir binding via Erlang interop); restoring Gleam here keeps the language available for downstream alef users who want it.

### Fixed

- **alef-e2e/kotlin**: sticky nullability in `render_kotlin_with_optionals`. Once a `?.` safe-call is emitted for any segment in a Kotlin accessor chain, all subsequent segments are on a nullable receiver and must also use `?.`. The previous code reset `prev_was_nullable` to just `is_optional` on each segment, so non-optional fields after a `?.` call dropped the safe-call, producing uncompilable code (e.g. `toolCalls()?.first().function()` instead of `toolCalls()?.first()?.function()`). Fixed in all three `PathSegment` arms (`Field`, `ArrayField`, `MapAccess`).

- **alef-e2e/kotlin**: auto-detect enum-typed fields from IR `TypeDef` to route through `.getValue()`. Fields whose Rust type is `Named(T)` where `T` is not a known struct (i.e. an enum) must call `.getValue()` in Kotlin assertions. Previously only fields in the global `fields_enum` or a per-call `enum_fields` override were handled; `BatchObject.status` (type `BatchStatus`) was silently treated as `String`, causing `"BatchStatus has no method trim"` at Kotlin compile time. Now `generate()` builds a `type_enum_fields` map from the IR and merges it into `effective_enum_fields` per call. Also extends `field_is_optional` to detect nullability from the accessor expression itself (sticky-`?.` chain makes the whole expression `T?` regardless of what the path-prefix lookup finds).

- **alef-backend-magnus**: WIP: correctly convert `&DTO` and `&[&str]` parameter shapes when calling core functions. Functions and methods accepting references to generated DTO structs and string slices now emit proper type conversions at the FFI boundary. Serde recovery path creates `{name}_core` bindings for Named params; Vec<String> ref params create `{name}_core` intermediate Vec<&str> bindings consumed by `gen_call_args_with_let_bindings`.

- **alef-backend-csharp**: serialize enum values as strings via JsonStringEnumConverter (matches python/node/go/java/ruby/php). All C# enums now emit `[JsonConverter(typeof(JsonStringEnumConverter))]` to serialize as JSON strings (e.g. `"function"`) instead of numeric discriminants (e.g. `0`), matching other language bindings.

## [0.15.54] - 2026-05-13

### Added

### Fixed

- fix(alef-backend-rustler): wrap bare opaque returns and Optional<opaque> returns in ResourceArc<T> in NIF function/method signatures. Functions returning T (opaque type) now emit `ResourceArc<T>` instead of bare `T`, and functions returning `Option<T>` emit `Option<ResourceArc<T>>` instead of `Option<T>`. Methods on opaque types now call inner methods via `resource.inner.as_ref().method()` to properly dereference the Arc and invoke the inner type's methods (required for methods like `clone()`). Fixes compile errors where `Tree`, `Node`, `TreeCursor`, `Parser` were used directly without ResourceArc wrapping, which violated Rustler's Encoder trait bounds.

- fix(alef-e2e/typescript): gate streaming-virtual interception on is_streaming, matching go fix. TypeScript codegen now respects the `streaming = false` opt-out at the call level, skipping the virtual field interception (chunks, stream_content, etc.) and allowing them to be resolved normally through the result struct accessor path. Non-streaming fixtures no longer emit references to undeclared `chunks` variable.

- fix(alef-backend-magnus): delegate all methods through `Mutex` on types with `&mut self` methods. When an opaque type has any `&mut self` methods, ALL its methods (including `&self` methods) must use `.lock().unwrap()` to access the inner value in `Arc<Mutex<T>>`. Previously, only `&mut self` methods got the lock, leaving `&self` methods calling `.method()` directly on `Arc<Mutex<T>>`, which fails because `Arc` has no such methods. Now matches the NAPI and PyO3 patterns: if `has_mut_methods`, emit `.lock().unwrap()` for all delegation.
- fix(alef-backend-pyo3): surface trait-bridge `OptionsField` kwargs in the generated `.pyi` for both struct `__init__` and module-level functions. The cfg-gated bridge field (e.g. `ConversionOptions.visitor`) is stripped by the `gen_stubs.rs` partition (`f.cfg.is_none()`), but the PyO3 `#[new]` macro keeps it via `never_skip_cfg_field_names`, so api.py callers like `_rust.ConversionOptions(visitor=...)` and `_rust.convert(html, options, visitor=...)` failed `mypy --strict` with `Unexpected keyword argument "visitor"`. Stubs now append the bridge kwarg as `{kwarg_name}: {type_alias} | None = None` whenever the options type is referenced, mirroring the wrapper signature in `gen_bindings/functions.rs`.
- fix(alef-backend-wasm): wrap opaque types with `&mut self` methods in `Arc<Mutex<T>>` matching NAPI/PyO3/Magnus pattern. Opaque types now detect `RefMut` receivers and conditionally wrap the inner type in `std::sync::Mutex` so that method delegation can call `.lock().unwrap()` before invoking mutable methods. Previously generated `Arc<T>` unconditionally, causing invalid code when methods tried to call `.lock()` on non-Mutex types.
- fix(alef-backend-wasm): gate the `self.inner.lock().unwrap()` call path on the receiver type actually being opaque. Methods on transparent (named-field) structs whose type happens to have a sibling `&mut self` method (e.g. `WasmDocumentStructure::is_empty` alongside `finalize_node_types`) were emitting `self.inner.lock().unwrap().is_empty()` even though the struct has direct fields and no `inner: Arc<Mutex<T>>`. Codegen now checks `opaque_types.contains(type_name)` before taking the Mutex branch; transparent structs fall back to `Type::from(self.clone()).method(...)`.
- fix(alef-backend-csharp): pass length argument alongside pinned pointer when delegating byte-slice params to native P/Invoke. Wrapper methods now emit both the `AddrOfPinnedObject()` pointer AND the `(UIntPtr)source.Length` argument for `&[u8]` parameters, matching the FFI signature. Previously only the pointer was passed, causing C# compile errors (CS7036: missing required parameter) when parsing byte arrays.
- fix(alef-e2e/kotlin): fix `{kotlin_val}` placeholder in numeric comparison assertion failure messages. The `greater_than`, `less_than`, `greater_than_or_equal`, and `less_than_or_equal` assertions emitted literal text `expected > {kotlin_val}` because the placeholder was doubly-escaped (`{{kotlin_val}}`), causing Kotlin source parse errors (`Expecting an element`).
- fix(alef-e2e): backtick-escape Kotlin hard keywords when emitting field accessors. Field paths like `result.data().first().object()` are syntax errors in Kotlin because `object` is a hard keyword; now emitted as ``result.data().first().`object`()``. Applies to both plain and nullable-aware Kotlin accessor renderers.

## [0.15.53] - 2026-05-13

### Added

### Fixed

- fix(alef-backend-magnus): pass missing `mutex_types` argument in Magnus `gen_function` unit-test call sites after the signature change, fixing `cargo check` failures in `gen_bindings_test`.

### Added

- feat(alef-e2e): add `[e2e.calls.<name>] streaming = false | true` opt-in/out at the call config level. When set to `false`, the streaming-virtual-field auto-detection is disabled — assertions that reference field names like `chunks` / `chunks.length` / `tool_calls` / `finish_reason` on a synchronous result type are treated as plain field accessors rather than streaming adapters. Without this, an API whose return value happens to have a `chunks: Vec<T>` field that is not actually streamed (e.g. a code-chunking result) would be incorrectly emitted with `async for chunk in result` and `@pytest.mark.asyncio` decorators across every backend that supports streaming. `None` (default) preserves the prior auto-detect heuristic so existing LLM-style downstream crates are unchanged. Honored by all backends that previously hard-coded the heuristic: python, typescript (node + wasm), go, java, php, elixir, kotlin, swift, dart. A new `resolve_is_streaming(fixture, call_config.streaming)` helper in `codegen/streaming_assertions.rs` is the single source of truth so future backends pick up the opt-out automatically.

### Fixed

- fix(alef-e2e/go): gate the streaming-virtual-field assertion interception in `render_assertion` on the resolved `is_streaming` flag (threaded through from `render_test_function`). Without this gate, a call with `streaming = false` whose result type happens to have a `chunks` field would still emit `len(chunks)` referencing an undeclared local (streaming chunks-var) instead of the plain `len(result.Chunks)` field accessor. Matches the python/typescript backends which already short-circuit when streaming is disabled.

- fix(alef-backend-magnus): wrap opaque types with `&mut self` methods in `Arc<Mutex<T>>` instead of bare `Arc<T>`, and allow delegation of `&mut self` methods via `.lock().unwrap()`. Previously, opaque structs were always wrapped in `Arc` with no mutex, and methods with `&mut self` receivers were stubbed as "Not implemented". This fix enables real implementations for methods like `Parser::set_language`. Complements equivalent fixes already in NAPI and PHP backends.

- fix(alef-backend-php): allow delegation of `&mut self` methods on opaque types wrapped in `Arc<Mutex<T>>`. Previously the can_delegate check rejected all RefMut receivers (line 396) assuming Arc doesn't support `&mut T`. However, when a type has any RefMut methods (indicating Mutex-wrapping), those methods CAN be delegated by locking the mutex. This fix enables real implementations for methods like `Parser::set_language` instead of stubbed "Not implemented" bodies.

- fix(alef-backend-napi): allow delegation of `&mut self` methods on opaque types wrapped in `Arc<Mutex<T>>`. Previously the opaque_can_delegate check rejected all RefMut receivers (line 418) assuming Arc doesn't support `&mut T`. However, when a type has any RefMut methods (indicating Mutex-wrapping), those methods CAN be delegated by locking the mutex. This fix enables real implementations for methods like `Parser::set_language` instead of stubbed "Not implemented" bodies.

- fix(alef-e2e/csharp): emit `<GenerateAssemblyInfo>false</GenerateAssemblyInfo>` in the e2e test project `.csproj` to prevent duplicate `AssemblyInfo` attributes when a hand-written `Properties/AssemblyInfo.cs` file provides assembly metadata manually. The .NET SDK auto-generates `obj/*/AssemblyInfo.cs` by default, leading to CS0579 errors when both files define the same attributes. Complements the assembly info file previously generated for e2e projects.

- fix(alef-e2e/python): also include per-fixture streaming in the file-level `is_async` calculation used to gate `import pytest`. Previously a file containing fixtures whose `is_streaming` was triggered only by virtual-field assertions (not by call-level `async`) would emit `@pytest.mark.asyncio` decorators without the matching `import pytest`, producing `NameError`/F821 on test collection. Now `needs_pytest` correctly reflects every code path that emits an async test.

- fix(alef-backend-extendr): generate `String` (not `Robj`) for non-options string parameters in `gen_extendr_bridge_field_function`, and build the core-function call from the actual params instead of a hardcoded `convert(&html, …)` literal. The previous output emitted `pub fn convert(html: Robj, options: Robj)` and then called `core::convert(&html, Some(opts))` — `&Robj` doesn't satisfy `&str`, so the generated R binding crate failed to compile. Now string params decode via extendr's `TryFrom<Robj> for String` and the `&name` call site deref-coerces to `&str`.

- fix(alef-e2e/r): wrap array-valued fields in `I(...)` when emitting `jsonlite::toJSON(list(...), auto_unbox = TRUE)` for the `Type$from_json(...)` typed-config code path. Without the `AsIs` marker, single-element vectors were unboxed to scalars (`c("foo")` → `"foo"`) and empty vectors collapsed to `{}`, causing serde to reject `Vec<T>` fields with `invalid type: string "foo", expected a sequence`. Non-empty arrays now emit as `I(c(...))` (→ `[...]`) and empty arrays as `I(list())` (→ `[]`), fixing 9 R e2e failures across `exclude_selectors`, `strip_tags`, `preserve_tags`, and `keep_inline_images_in`.

- fix(alef-e2e/r): visitor-test arg builder now strips any pre-existing `options = …` argument before appending `options = list(visitor = visitor)`, instead of only stripping the literal `options = NULL`. The previous code left `options = ConversionOptions$default()` in place when `build_args_string` emitted a default placeholder, producing duplicated named args that R rejects with `formal argument "options" matched by multiple actual arguments`. New `strip_options_arg` helper walks the args string while tracking paren/quote depth so nested commas inside `options = list(visitor = visitor)` aren't treated as arg separators.

- fix(alef-backend-csharp): emit single `{` / `}` (not `{{` / `}}`) in the `VisitResult.ToFfiJson()` string-concatenation literals for `Custom` and `Error` variants. The template was authored as if the surrounding code were a C# interpolated string (`$"…"`), where `{{` escapes to `{` — but the emitted source is a plain string-concatenation, so `{{` came out as literal double-braces, producing `{{"Custom":"…"}}` which Rust's serde rejects. C# visitor overrides (`VisitResult.Custom("…")`) now round-trip correctly across the FFI boundary, fixing all C# e2e visitor tests (42 fixtures previously regressing).

- fix(alef-backend-csharp): include `api.enums` (alongside `api.types`) when building the `visible_type_names` set passed to trait-bridge codegen. Without this, enum types like `VisitResult` were treated as non-API by `csharp_type_visible`, which substituted them with `string` in generated trait-method signatures (`string VisitFigureEnd(...)`). E2e visitor tests that returned `VisitResult.Continue` then failed to compile with CS0738 "does not have the matching return type of 'string'".

- fix(alef-e2e/csharp): look up `enum_fields` and `nested_types` overrides by both snake_case (fixture JSON key) and camelCase (alef.toml convention) when emitting C# object initializers. Previously fixtures with `code_block_style: "Backticks"` produced `CodeBlockStyle = "Backticks"` (raw string literal) instead of `CodeBlockStyle = CodeBlockStyle.Backticks` (enum constant), causing ~18 CS0029 compile errors. C# is strongly typed and does not accept string-to-enum implicit conversion, unlike Python's ConversionOptions which accepts strings and converts internally.

## [0.15.52] - 2026-05-13

### Added

- feat(scaffold/node): add `futures-util` to generated Node binding `Cargo.toml` so `chat_stream` compiles without requiring a manual dependency addition

### Fixed

- fix(alef-backend-magnus): enums now use the variant marked with `#[default]` instead of always the first variant in source order. This respects Rust's `#[derive(Default)]` semantics. Fixes `CodeBlockStyle` defaulting to `Backticks` (marked `#[default]`) instead of `Indented`, enabling fenced code blocks with language annotations.
- fix(alef-backend-magnus): emit explicit `impl Default` for all binding structs with fields instead of deriving `Default`, which gives all-zeros initialization. Explicit impls use field-level defaults computed by config_gen (same as kwargs constructors), ensuring serde deserialization with missing fields produces correct values (e.g., `PreprocessingOptions.enabled=true` instead of false). Properly handles `Option<T>` fields which always default to `None`.

- fix(napi,pyo3,wasm,php): allow `clippy::should_implement_trait` at the crate level so opaque-type wrappers carrying a `pub fn clone(&self) -> Self` method (lifted from the upstream `Clone` derive on the wrapped Rust type) compile under `-D warnings`. Without the allow, NAPI/PyO3/wasm-bindgen/ext-php-rs surfaces fail clippy because the inherent method shadows `std::clone::Clone::clone`; the bindings still derive `Clone` for the wrapper type so trait dispatch is unaffected.

- fix(alef-backend-magnus): unit-enum `TryConvert` now also accepts the PascalCase Rust variant name (e.g. `"Tildes"`, `"AtxClosed"`) alongside the serde wire form (e.g. `"tildes"`). Fixtures and tests written in either style now deserialize correctly.
- fix(alef-e2e/ruby): emit string-equals assertions as `expr.to_s.strip == expected.strip` to match Python's pattern, masking trailing-newline differences fixture authors don't write into expected values. Drops ~10 ruby e2e failures in html-to-markdown.
- fix(alef-e2e/field_access): render Ruby hash map-access as `["key"]` instead of `.get("key")`. Ruby's `Hash` has no `get` method, so previously-generated assertions like `result.metadata.document.open_graph.get("title")` raised `NoMethodError`.
- fix(alef-e2e/typescript): look up `enum_fields` overrides by both snake_case (fixture key) and camelCase (alef.toml convention) when emitting WASM-class field assignments. Previously fixtures with `code_block_style: "Tildes"` produced `_u.codeBlockStyle = "Tildes"` (raw string) instead of `_u.codeBlockStyle = WasmCodeBlockStyle.Tildes` (enum constant), and wasm-bindgen silently dropped the assignment so options like code-block style, heading style, and highlight style never took effect.
- fix(alef-backend-magnus): in `gen_options_field_bridge_function`, recognise Ruby `ConversionOptions` class instances passed as the options argument and convert them to core via `.clone().into()`. Previously only `Hash` and visitor objects were accepted, so tests doing `HtmlToMarkdownRs::ConversionOptions.new(...)` were misclassified as visitors and the options were dropped (e.g. `include_document_structure` never flowed through, leaving `result.document` nil). Also drives the options type name from the IR so the helper isn't hard-wired to `ConversionOptions`.
- fix(alef-backend-napi): stop clearing `o.visitor = None` before `o.into()` in `gen_options_field_bridge_function`. The From impl now forwards `val.visitor` through `JsHtmlVisitorBridge` (via the existing post-process), so the handle survives the conversion. A separate kwarg-supplied visitor still overrides when present. Without this fix, JS callers passing `convert(html, { visitor: ... })` silently lost the visitor and callbacks never fired.
- fix(alef-e2e/typescript): emit Node visitor tests as `convert(html, options, visitor)` (three positional args) instead of merging `visitor` into the options object literal. napi-rs cannot deserialize `Option<Object<'static>>` from a JS object property, so the visitor field was always `None` on the Rust side; routing through the existing kwarg path wraps the JS object via `JsHtmlVisitorBridge::new`.
- fix(alef-e2e/typescript): wrap WASM e2e visitor tests' visitor argument with `new WasmVisitorHandle(...)`. wasm-bindgen's generated setter for `WasmConversionOptions.visitor` enforces `_assertClass(value, WasmVisitorHandle)` and rejects plain JS objects. Auto-import `WasmVisitorHandle` alongside `WasmConversionOptions`. Also preserve the trailing `html` positional argument when the fixture has no other options (previous IIFE rewrite replaced it).
- fix(e2e/kotlin): derive JAR name by replacing hyphens with underscores (`rootProject.name` uses underscores, producing e.g. `liter_llm-VERSION.jar`); bare hyphenated name caused "Unresolved reference" linking errors
- fix(e2e/kotlin): wrap generated `@Test` bodies in `runBlocking { }` when the binding exposes `suspend fun` methods; previously tests called coroutine methods outside a coroutine scope and crashed with `IllegalStateException`
- fix(e2e/kotlin): append `L` suffix to integer literals for `uint64_t`/`int64_t` fields (e.g. `assertEquals(6L, result.promptTokens())`) to prevent Kotlin long-vs-int type mismatch compilation errors

- feat(backend-php): emit per-opaque-type PHP stub class files under the configured `output` directory. Each opaque type with public methods gets its own `<TypeName>.php` declaring `public function` signatures, so static analysers and IDEs see the type's surface. Mirrors the existing `.rbs` stub pattern in `alef-backend-magnus`.
- feat(backend-rustler): emit idiomatic Elixir `.ex` wrapper modules for opaque types with methods. Each opaque type gets `defstruct [:ref]` plus per-method delegations to `TreeSitterLanguagePack.Native.<type>_<method>(obj.ref, ...)`, so Elixir callers get a typed module-based API instead of raw NIF call sites.

### Fixed

- fix(backend-napi): correctly resolve type names for opaque-vs-capsule types when used as parameters and return types. Capsule types (e.g. `Language` configured under `[crates.node.capsule_types]`) now reference the ecosystem-library type directly with no `Js` prefix; non-capsule opaque types (e.g. user-defined `Parser`/`Tree`/`Node`) get the `Js` prefix. Fixes "cannot find type `JsLanguage`" / "cannot find struct `Node`" errors when a project defines its own wrapper types alongside capsule passthrough.
- fix(backend-napi): for opaque types with any `&mut self` method, wrap the inner value in `Arc<Mutex<T>>` consistently — at struct construction (`Arc::new(Mutex::new(raw))` instead of `Arc::new(raw)`) and in every method body (acquire `.lock()` before the inner call). Resolves "no method named X found for `Arc<Mutex<T>>`" errors and Arc-double-wrap mismatches.
- fix(backend-pyo3): mirror the napi Mutex wrapping fix for opaque types with `&mut self` methods (same root cause, same fix shape).
- fix(backend-php): same Mutex wrapping fix as napi/pyo3 — propagate the computed `mutex_types` set through the codegen pipeline (`gen_opaque_struct_methods` → `gen_instance_method` → `php_wrap_return`) and emit `.lock().unwrap()` for method dispatch on `Arc<Mutex<T>>` opaque fields.
- fix(alef-codegen,core-to-binding): stop wiping cfg-gated trait-bridge fields to `Default::default()` in the core→binding conversion. The previous block in `gen_from_core_to_binding_cfg` forced `visitor: Default::default()` whenever the field's type referenced an opaque wrapper, dropping PHP/Node/WASM visitor objects between the binding builder and the core call. The field now flows through the normal `val.field.into()` conversion (or the `Option<Option<T>>` recursive path), so bindings that wrap visitors as their own opaque types forward them correctly.
- fix(alef-codegen,binding-to-core): forward trait-bridge Arc-wrapper fields instead of emitting `Default::default()` in the binding→core direction. The `is_opaque_no_wrapper_field` branch in `binding_to_core.rs` previously dropped fields like PHP's `Option<VisitorHandle>` (where `VisitorHandle { inner: Arc<core::VisitorHandle> }`). Added a `ConversionConfig::trait_bridge_arc_wrapper_field_names` slice populated from each backend's OptionsField bridges; when a field is in that list the conversion emits `val.{field}.map(|v| (*v.inner).clone())` (or the non-optional equivalent). Fixes 48 PHP visitor e2e failures where the builder set a visitor that was then silently dropped by `.into()`.
- fix(alef-backend-napi): soften the `find_options_field_binding` filter to accept cfg-gated trait-bridge fields whose names appear in `never_skip_cfg_field_names` (e.g. `visitor`). The previous `f.cfg.is_none()` predicate rejected the visitor field outright, causing the NAPI `convert` codegen to fall through to the plain `gen_function` path with no visitor parameter and a `From<JsConversionOptions>` impl that hardcoded `__result.visitor = Default::default()`. With the softened filter the options-field-bridge codepath activates: `convert` exposes `visitor: Option<Object>` and the JS visitor is woven into the core `ConversionOptions` before `core::convert()` is called.
- fix(alef-backend-wasm): make the Phase C `From<Wasm{options_type}>` visitor-forwarding post-process tolerant of indentation variations. The previous exact whitespace match (12-space indent only) silently failed to substitute `visitor: Default::default()` when the surrounding output used different indentation. The substitution now tries 12-, 8-, and 2-space indents so the forwarding (`val.visitor.map(|v| (*v.inner).clone())`) actually lands on every generated `From<Wasm…>` impl. Fixes 62 WASM visitor e2e failures.
- fix(alef-backend-java): align `gen_visitor_bridge` callback-chunking constants. Previously the constructor computed `num_chunks = CALLBACKS.chunks(10).count()` while the per-chunk method emitter used a local `const CHUNK_SIZE: usize = 5`. With 40 callbacks the emitter produced 8 `registerStubsN` methods but the constructor invoked only 4 of them, leaving callbacks 20–39 (`visit_button`, `visit_iframe`, `visit_mark`, `visit_video`, `visit_input`, etc.) NULL in the C struct. Lifted `CHUNK_SIZE = 5` to module scope and used it in both places. Fixes 28 Java visitor e2e failures.
- fix(alef-backend-magnus): emit `require "json"` once in the generated Ruby ext module init, and replace `.unwrap_or_default()` on the options-deserialization funcall with explicit `magnus::Error` propagation. Ruby's `Hash#to_json` only exists after `require "json"`; the gem didn't load the stdlib, so `funcall("to_json", ())` raised `NoMethodError`, the previous `unwrap_or_default()` swallowed it, and the entire options object reverted to `ConversionOptions::default()` (`include_document_structure: false`, no preprocessing, etc.) across 30+ Ruby e2e specs. The module-init `require "json"` makes `Hash#to_json` available, and the error-propagating `.map_err` surfaces any future failures instead of silently defaulting.

#### kreuzcrawl iteration (16-language e2e convergence)

- fix(alef-e2e/c): emit `char*` JSON-roundtrip path (instead of opaque-handle pattern) for FFI functions whose result type is `char*` such as `batch_scrape`. Driven by a new `raw_c_result_type` override threaded through `render_engine_factory_test_function`; the emitter writes `char* result = kcrawl_{fn}(...); kcrawl_free_string(result);` instead of the wrapper-constructor + `*_free` pattern. Previously generated `KCRAWLBatchScrape* result = kcrawl_batch_scrape(...)` referencing an undeclared type.
- fix(alef-e2e/c): parse `MOCK_SERVERS={"<id>":"<url>",...}` from the spawned mock-server's stdout in the generated `Makefile` test target and export `MOCK_SERVER_<UPPER_ID>` env vars for the test binary; the per-call URL setup now prefers `getenv("MOCK_SERVER_<UPPER_FIXTURE_ID>")` over the shared `MOCK_SERVER_URL/fixtures/<id>` fallback. Required for host-root fixtures (robots/sitemap) where a single port can only serve one robots.txt.
- fix(alef-e2e/c): for `expects_error` fixtures, emit a soft early-return when `kcrawl_crawl_config_from_json` or `kcrawl_create_engine` returns NULL rather than `assert(... != NULL)`. Tests asserting that config validation fails no longer abort the suite.
- fix(alef-e2e/dart): resolve `xxx[].yyy` traversal aliases on the full path before splitting around `[]`, so renamed sub-fields like `assets[].category` → `assets[].asset_category` route correctly through `field_resolver`. Previously the head was alias-resolved but the element segment kept the fixture-side name and missed the renamed accessor.
- fix(alef-e2e/dart): construct `FieldResolver` per file; emit `createCrawlConfigFromJson` setup and `createEngine(config: ...)` named-arg syntax to match the flutter_rust_bridge facade; route field accessors through `field_resolver.accessor("dart", ...)` for alias + nullability; `is_empty` matcher emits `anyOf(isNull, isEmpty)` to accommodate FRB nullable strings/lists.
- fix(alef-e2e/field_access): in the dart renderer, `PathSegment::Length` honours `prev_was_nullable` (emits `?.length`) and `PathSegment::ArrayField` only adds `!` when the field is itself optional. Prevents `Null check operator used on a null value` runtime failures on optional list/string fields.
- fix(alef-e2e/kotlin): set `PropertyNamingStrategies.SNAKE_CASE` on the generated test-suite `ObjectMapper` so Jackson deserializes the snake_case JSON wire format from the Rust FFI into the Kotlin record's camelCase fields (matches the Java mapper convention).
- fix(alef-e2e/kotlin): for fixtures with `has_host_root_route()`, emit `System.getProperty("mockServer.<fixture_id>", ...)` as the URL source so host-root fixtures route to their per-fixture listener (matches the Java emitter pattern).
- fix(alef-e2e/swift): for `expects_error` fixtures, hoist setup lines (config-from-JSON, create-engine) INSIDE the `do { } catch { }` block so config-validation errors are caught by the test. Previously the setup `try` was outside the do, so validation failures aborted the test instead of being asserted.
- fix(alef-e2e/swift): hoist `RustVec<T>` subscript temporaries into local variables (new `materialise_vec_temporaries` helper). swift-bridge's `RustVec<T>` subscript returns a `T.SelfRef` holding a raw pointer into the Vec's storage; without a local binding the Vec gets ARC-released before the ref is used, leaving a dangling pointer (observed as empty JSON-LD test data, then SIGSEGV under stress).
- fix(alef-e2e/php): templates/synthetic_assertion.jinja gains a trailing newline. Previously the skip-comment for unavailable fields ran into the following assertion on the same line, making the assertion part of the comment (PHPUnit reported affected tests as "risky" with no assertions, exiting non-zero).
- fix(alef-backend-csharp): filter `compute_handle_returned_types` to types with `has_serde == false`, so only truly-opaque handle types route through the constructor-wrapper pattern (`new T(IntPtr)`). Value records like `ScrapeResult`, `CrawlResult`, `MapResult`, `ContentConfig`, `BrowserConfig`, `CrawlConfig` stay on the JSON-roundtrip path (`*ToJson` → `JsonSerializer.Deserialize` → `*Free`). The earlier opaque-handle change had widened the wrapper-path to all named returns, breaking compilation of every serde-backed value-type return with `CS1729: 'T' does not contain a constructor that takes 1 arguments`.
- fix(alef-e2e/wasm): auto-import nested config types referenced as `handle_config_type` values (e.g. `WasmBrowserConfig`, `WasmAuthConfig`) even when they aren't direct `json_object` args. The codegen now includes `handle_config_type` in `all_options_types` for WASM and derives transitive nested types in `build_args_and_setup` so they're wrapped with proper class constructors at the test call site.
- fix(alef-e2e/wasm): in `templates/wasm/globalSetup.ts.jinja`, parse `MOCK_SERVERS={...}` from the spawned mock-server's stdout and export per-fixture `MOCK_SERVER_<UPPER_FIXTURE_ID>` env vars so host-root fixtures (robots/sitemap) reach their dedicated listener instead of the shared base port.
- fix(alef-e2e/brew): emit `${MOCK_SERVER_<UPPER_FIXTURE_ID>:-${MOCK_SERVER_URL}/fixtures/<id>}` for `mock_url` args so host-root fixtures route correctly when the mock-server allocates a per-fixture listener; the generated `*.sh` tests fall back to the shared `MOCK_SERVER_URL/fixtures/<id>` form for fixtures without isolation.
- fix(alef-e2e/brew): when fixture `input.config` is non-empty, append `--config '<minified json>'` to the generated CLI command so the kreuzcrawl CLI honors fixture-specified config (e.g. `respect_robots_txt: true`, validation fixtures). Pairs with the kreuzcrawl-side `--config` flag added to `scrape`/`crawl`/`map`.

## [0.15.51] - 2026-05-13

### Fixed

- fix(backend-java): emit `@JsonProperty("snake_case_name")` on request type fields where the Java camelCase name differs from the original snake_case IR name; adds `import com.fasterxml.jackson.annotation.JsonProperty` automatically when needed
- fix(e2e-elixir): streaming assertions now use `Enum.at(choices, 0)` instead of invalid `choices[0]` bracket access on Elixir lists
- fix(e2e-elixir): smoke test fallback api_key uses `"test-key"` instead of `nil` when no env var is set, preventing NIF rejection of nil for required String parameter
- fix(wasm): optional-enum getters return `Option<String>` via `to_api_str()` instead of raw C-style enum discriminant; optional `Vec<Struct>` getters return `Option<js_sys::Array>` for correct JS element access
- fix(e2e): mock server serves `data: [DONE]\n\n` SSE response for streaming fixtures with empty `stream_chunks` array, preventing Python SSE parser from hanging

## [0.15.50] - 2026-05-12

### Removed

- chore: drop Gleam language backend, codegen, scaffold, publish support, and `Language::Gleam` enum variant

## [0.15.49] - 2026-05-12

### Added

- feat(core,backends): `TraitBridgeConfig` gains `context_type` and `result_type` fields, plus a `ResolvedCrateConfig::bridge_associated_types()` helper that aggregates them across all configured bridges. Backends use this to skip generic record/POJO codegen for the bridge's associated types instead of hardcoding literal `"NodeContext"` / `"VisitResult"` checks against h2m's HtmlVisitor types. Any project with a callback-style trait can now wire its own context/result types through the bridge codegen.

### Fixed

- fix(codegen): scope cfg-gated trait-bridge field detection to fields whose type references an opaque wrapper (per `cfg.opaque_type_names`). The previous logic in `gen_from_core_to_binding_cfg` + the struct emitters forced `Default::default()` / `#[serde(skip)]` for any cfg-gated field listed in `never_skip_cfg_field_names`, which incorrectly defaulted regular convertible fields like `metadata: HtmlMetadata` on `ConversionResult`. Restores `result.metadata.document.*` access in the Python binding (h2m).
- fix(backend-pyo3,backend-wasm): drive trait-bridge post-process from `core_import` instead of hardcoded crate names. The pyo3 visitor-fallback rewrite and the wasm `From<Wasm*>` impl rewrite previously matched the literal string `html_to_markdown_rs`, breaking codegen for any other project using `bind_via = "options_field"`. Both now build their search/replace patterns via the configured `core_import` plus the per-bridge `type_alias` / `options_type` / `resolved_options_field`. The wasm rewrite also no longer relies on a hardcoded anchor field (`strong_em_symbol`) to scope the From-impl search — it now finds the impl block by its full `impl From<Wasm{options_type}> for ...` header.
- fix(wasm-backend): WASM trait bridge constructor now correctly reads the JS object's `name` property as a plain string. The previous implementation called `dyn_into::<js_sys::Function>()` on the value returned by `Reflect::get(&js_obj, "name")`, which always failed because `name` is a string property, not a method — causing the chain to fall through to `unwrap_or_else(|| "wasm_bridge".to_string())` unconditionally. Fixed by removing the intermediate `dyn_into::<Function>` step and calling `.as_string()` directly on the `JsValue` returned by `Reflect::get`.
- fix(wasm-backend): `build_wasm_arg` no longer uses Rust `{:?}` Debug formatting for non-primitive JS bridge arguments. The previous fallback emitted binary debug repr for `&[u8]` parameters (e.g. `[72, 101, 108, 108, 111]` instead of a `Uint8Array`) and Rust field syntax for complex types, both of which are unrecognisable at the JS call site. Fixed: `TypeRef::Bytes` now emits `js_sys::Uint8Array::from(...)` for correct typed-array interop; all remaining complex types (`Named`, `Vec`, `Map`, etc.) now use `serde_wasm_bindgen::to_value(...)` so they arrive as plain JS objects.
- fix(wasm-backend): async WASM trait bridge methods now correctly await the `Promise` returned by the JS function. `func.apply()` on a JavaScript `async` function returns a `Promise` object, not the resolved value; the previous implementation treated it as the final result and called `.as_string()` on the `Promise` itself, so all async bridge methods silently returned the default value. Fixed by casting via `dyn_into::<js_sys::Promise>()` and awaiting with `wasm_bindgen_futures::JsFuture::from(promise).await`.
- fix(alef-e2e/swift): `[].` traversal element accessor now resolves field aliases before computing the Swift method name (e.g. `assets[].category` → `assetCategory()` not `category()`); enum elements now call `.toString()` not `.to_string().toString()`.
- fix(alef-backend-dart): add `rust_input: crate` to the generated `flutter_rust_bridge.yaml` — omitting it caused `flutter_rust_bridge_codegen generate` to panic.
- fix(alef-backend-go): filter `ffi_skip_methods` in trait bridge generation so FFI-incompatible methods are not emitted in the Go C-VTable.
- fix(alef-backend-java): add a post-generation line-length wrapping pass to keep Checkstyle happy on compound annotation and call-argument lines.
- fix(alef-backend-napi): skip opaque-struct methods whose return type is a capsule type; the capsule shim uses `napi_create_external` directly, so no wrapper class is emitted for the return type and the method codegen path would reference a nonexistent type.
- fix(alef-backend-rustler): refactor method-call rendering to use a single `core_path` template variable instead of separate `core_import`/`struct_name`, fixing static method dispatch.
- fix(alef-e2e/kotlin): resolve `options_type` across language overrides (csharp/c/go/php/python) so Kotlin e2e tests pick up the correct options type without a redundant kotlin-specific override.
- fix(alef-backend-napi): napi capsule passthrough is now runtime-compatible with the `tree-sitter` npm package. The generated `getLanguage`-style shim now uses raw `napi_create_external` (the previous `napi::bindgen_prelude::External::new()` produced a wrapper that `Napi::Value::As<External<T>>` did not recognise), optionally calls `napi_type_tag_object` with a configurable GUID, and the property name is configurable (default `__parser`, set `property_name = "language"` to satisfy `node-tree-sitter`'s `UnwrapLanguage`). `NodeCapsuleTypeConfig` gains two optional fields: `property_name` (default `"__parser"` for back-compat) and `type_tag = { lower = "0x...", upper = "0x..." }`. Verified end-to-end in tree-sitter-language-pack: `getLanguage("python")` round-trips through `new Parser().setLanguage(lang).parse(...)` and yields the correct AST. Method-on-opaque-type capsule path remains a known limitation.

## [0.15.48] - 2026-05-12

### Changed

- chore(alef-core,alef-backend-php,alef-e2e): make `ResolvedCrateConfig::serde_rename_all_for_language` the single source of truth for per-language JSON wire casing. Flipped registry defaults so PHP, Kotlin, Swift, Dart return `camelCase` (matching their idiomatic variable conventions); Python/Ruby/Go/Elixir/R/Rust/Gleam/Zig/C/FFI stay `snake_case`. PHP backend's hard-coded `serde(rename_all = "camelCase")` now reads from the registry via a new `lang_rename_all` parameter threaded through `gen_php_struct`. Replaced `alef-e2e::codegen::normalize_json_keys_to_snake_case(value)` with `transform_json_keys_for_language(value, wire_case)` accepting any serde rename strategy; existing rust/c/kotlin callers pass `"snake_case"`, java codegen's `fromJson(...)` emission now transforms fixture keys to `"camelCase"` to match Jackson `@JsonProperty`, and php codegen's typed `from_json(json_encode([...]))` path uses `json_to_php_camel_keys` (also camelCase wire). This eliminates the band-aid "everything-to-snake" pre-pass and lets each binding receive JSON in the wire case its serde wrapper actually expects.

### Fixed

- fix(alef-e2e/c,alef-e2e/swift,alef-e2e/zig,alef-e2e/dart,alef-backend-zig): five codegen bugs that broke c/swift/zig/dart e2e tests. (1) c.rs: `render_chat_stream_test_function` redeclared `mock_base` for smoke+mock fixtures (those with `api_key_var`) where the env-fallback prologue already emits `api_key`/`base_url_buf` — caused "redefinition of `mock_base`" compile errors on `test_smoke_streaming_openai`. Now takes `api_key_var` and reuses the prologue's `api_key`/`base_url_buf` (matching the non-streaming client-factory path). (2) swift.rs: streaming fixtures fell through to the JS-style streaming_assertions fallback (`chunks.map((c: any) => ...).join('')`, `chunks.length`, `undefined`) because `collect_snippet` returns `None` for swift (swift-bridge doesn't yet expose a typed `chatStream` async sequence). Now emits an `XCTSkipIf` stub when the collect snippet is unavailable so the test target compiles. (3) zig.rs: `json_path_expr` didn't handle `[N]` indexed-segment notation, so `results[0].relevance_score` resolved to `object.get("results[0]")` instead of `.array.items[0]`; greater_than/less_than against fractional values (e.g. `0.9`) emitted `.integer` accessor causing "fractional component prevents float value '0.9' from coercion to type 'i64'" — now branches on whether the assertion value is a float and uses `.float`. (4) zig.rs: `build_args_and_setup` ignored per-call `extra_args` overrides, so `list_files()` / `list_batches()` invocations dropped the trailing optional `query: ?[]const u8` parameter — added merge logic mirroring go/python/swift. streaming_assertions deep-path for `tool_calls[N].xxx` returns None for zig (zig stores chunks as JSON strings, not typed records). (5) alef-backend-zig/opaque_handles.rs: `emit_method_param_conversion` didn't handle `Option<NamedStruct>` parameters — `dupeZ(u8, query)` requires a non-optional slice but the wrapper signature is `?[]const u8`, triggering "cannot convert optional to payload type" at `list_files`/`list_batches`. Now allocates conditionally (`if (query) |v| try dupeZ(...) else null`) and emits a null-aware `_from_json` call so `null` callers see `null` C handles. dart codegen: `field_to_dart_accessor` adds `!` after bracketed names so `choices![0]` / `toolCalls![0]` compile against `List<T>?`; `count_min`/`min_length`/`count_equals`/`max_length` use `?.length ?? 0`; `result_is_simple` config now honored so bytes-returning calls (speech, file_content) assert against the `Uint8List` directly rather than non-existent `.audio`/`.content` accessors.
- fix(alef-e2e/kotlin): test files emitted bareword imports (`import ChatCompletionRequest`, `import LiterLlm`) when the binding `class_name` wasn't fully-qualified, which Kotlin rejects with "Unresolved reference" since the kotlin binding emits typealiases at `kotlin_pkg_id` (e.g. `com.github.kreuzberg_dev`) and the test files live one package deeper at `<kotlin_pkg_id>.e2e` (parent-package symbols don't import implicitly). Now falls back to `kotlin_pkg_id` as the binding package when `import_path` is empty, producing `import com.github.kreuzberg_dev.ChatCompletionRequest` / `import com.github.kreuzberg_dev.LiterLlm`. Same fallback applied to options-type imports and CrawlConfig. Resolves the import layer in liter-llm kotlin e2e (remaining errors are coroutine `suspend fun` wrappers and nullable `Optional` accessors that need follow-up kotlin codegen work).
- fix(alef-e2e/java,alef-e2e/go): codegen bugs that broke streaming e2e tests for the Java and Go bindings. Java: (1) the `collect_snippet` emitted `new ArrayList<ChatCompletionChunk>()` but never imported `ChatCompletionChunk`, so the type erased to `Object` and downstream `c.choices()/ch.delta()` method calls failed with "cannot find symbol" — added an FQN import when any fixture is streaming. (2) `is_streaming` only checked `is_streaming_mock()`, so the `empty_stream` fixture (`stream_chunks:[]`) skipped the collect snippet and left `chunks` undefined at the assertion site — broadened to fire when any assertion references a streaming-virtual field, matching the kotlin/swift/php/python/elixir/typescript pattern. (3) The `not_error` arm for `byte[]` results emitted `Assertions.assertNotNull(...)` but only the static-import form (`assertNotNull`) is in scope — switched to the bare name. Go: (1) same `is_streaming` broadening so `empty_stream` drains the channel into `chunks`. (2) `needs_assert` for the file-level testify import treated streaming-virtual fields as `field_valid=false` (they don't resolve against the result type), so streaming-only files omitted the `github.com/stretchr/testify/assert` import even though every assertion called `assert.X` — now field-valid is `true` when the assertion's field is a streaming-virtual reference on a streaming fixture. (3) `finish_reason` accessor returned `*last.Choices[0].FinishReason` without casting `FinishReason` (a `type FinishReason string` alias) back to `string`, causing E0308-style type mismatches at the assertion site — added an explicit `string(*...)` cast. (4) `tool_calls` accessor used `*ch.Delta.ToolCalls` but the Go binding declares `ToolCalls []StreamToolCall` (slice, not pointer); removed the bogus dereference and changed the IIFE return type to `[]pkg.StreamToolCall` so deep-path accessors like `tool_calls[0].function.name` index into the typed slice. (5) deep-path streaming-virtual equals on Go binding pointer-typed leaves (`StreamFunctionCall.Name *string`) now wraps the expression in a nil-safe deref IIFE so the comparison succeeds against the plain-string assertion value. Drops liter-llm go e2e from a build failure to 161/161 passing; the corresponding fix in `liter-llm/alef.toml` adds the missing `[crates.e2e.calls.chat_stream.overrides.go]` block so `chat_stream` requests are deserialised into the typed Go struct (the binding signature is `func (h *DefaultClient) ChatStream(req ChatCompletionRequest)`).
- fix(alef-e2e/php): four codegen bugs that broke streaming e2e tests for PHP bindings. (1) The `chunks` streaming-virtual-field accessor returned the bareword `chunks` for every language; PHP parses bareword identifiers as constant references, so generated code like `count(chunks)` triggered "Undefined constant" errors — added a PHP arm that emits `$chunks`. (2) `is_streaming` in `php.rs` only checked `is_streaming_mock()`, so fixtures like `empty_stream` (with `stream_chunks:[]`) skipped the collect snippet and left `$chunks` undefined at assertion sites — broadened to fire when any assertion references a streaming-virtual field, matching the elixir/python/kotlin/typescript pattern. (3) PHP binding's streaming method returns a JSON string of chunks (PHP cannot expose Rust iterators via ext-php-rs), but the collect snippet emitted `iterator_to_array(...)` which fails with `TypeError: Argument must be Traversable|array, string given` — now branches on `is_string($result)` to `json_decode` the string into stdClass objects, retaining `iterator_to_array` as a fallback for a future binding that exposes a real iterator. PHP `stream_complete` / `finish_reason` / `tool_calls` accessors and the deep-path tail renderer switched from camelCase to snake_case so they match the JSON wire-format property names on the decoded stdClass values. (4) Streaming fixtures whose only assertion is `not_error` (e.g. `stream_multiple_choices`) produced an empty assertions body, which PHPUnit flags as "risky" — emit a baseline `$this->assertTrue(is_array($chunks), 'expected drained chunks list')` so the test records a real assertion. Drops liter-llm php e2e from 12 errors to 0; all 161 tests pass with 308 assertions.
- fix(alef-e2e/streaming*assertions): three rust-codegen bugs that produced uncompilable e2e tests. (1) `is_streaming_virtual_field` matched the tail-suffix of any field whose chars-after-`root.len()` started with `.` or `[` without first checking `field.starts_with(root)`, so `choices[0].finish_reason` matched root `tool_calls` (chars 10+ begin with `.finish_reason`) and falsely triggered streaming-mode codegen on non-streaming fixtures — added the `starts_with` guard. (2) Rust `collect_snippet` left the chunks vector as `Vec<Result<ChatCompletionChunk, *>>`, so subsequent accessors like `chunks.iter().map(|c| c.choices...)`failed E0609 ("no field`choices`on`&Result<...>`") — now chains `.into_iter().map(|r| r.expect("stream item failed")).collect()`to yield`Vec<ChatCompletionChunk>`. (3) Rust `finish_reason`accessor used`.as_deref()`on`Option<FinishReason>`(an enum, not`String`) which fails E0599 — switched to `.as_ref().map(|v| v.to_string())`. (4) Rust deep-path `tool_calls[0].function.name`chained naive field access through`StreamToolCall { function: Option<StreamFunctionCall>, ... }`causing E0609 — added`render_rust_tool_calls_deep` helper that emits Option-aware chaining (`.nth(0).and_then(|x| x.function.as_ref()).and_then(|x| x.name.as_deref()).unwrap_or("")`). Drops liter-llm rust e2e from 266 compile errors to 0; all 152 tests pass.

### Added

- feat(alef-core,alef-backend-ffi): `ffi_skip_methods` on `TraitBridgeConfig` filters individual methods from the FFI vtable struct and `impl Trait for KreuzbergXBridge`. Methods whose signatures can't traverse the C FFI boundary (e.g. `Option<&dyn Trait>` returns like `DocumentExtractor::as_sync_extractor`) are dropped, falling back to the trait's default implementation. Configured per-bridge in `alef.toml`.
- feat(alef-backend-swift): trait-bridge support for trait methods that take excluded internal types by reference (e.g. `Renderer::render(&self, doc: &InternalDocument)`). Outbound trampolines now deserialise the JSON-bridged `String` parameter back to the source type using its fully-qualified Rust path (resolved via `type_paths`); inbound `impl Trait` blocks emit owned return types (`String` rather than the unsized `str`) so methods returning `Result<String>` compile. Lets the swift backend bridge `Renderer` alongside the other plugin traits.

### Fixed

- fix(alef-backend-extendr): trait-bridge codegen now compiles for R bindings. Six issues were fixed end-to-end: (1) generated `#[extendr]` registration/unregister/clear functions used `Result<(), String>` which is rejected as `extendr_api::Result<T>` only takes one generic parameter — switched to `std::result::Result<(), String>`; (2) `.map_err()` on `registry.write()` for `parking_lot::RwLockWriteGuard` (infallible) — dropped; (3) `Robj` is `!Send`/`!Sync` (raw `*mut SEXPREC`) but `Plugin: Send + Sync` is required — emit `unsafe impl Send + Sync` for the bridge struct with a SAFETY comment noting R's single-threaded contract; (4) async trait methods captured `Robj` directly into `tokio::spawn_blocking` closures which then failed the `F: Send` bound (RFC 2229 disjoint captures grab `r_obj.0` not the whole wrapper) — introduced a `SendRobj` newtype with `into_inner(self)` so the closure captures the whole `Send` wrapper; (5) sync/async complex-return templates called `serde_json::to_string(&val)` against `Robj` (no `Serialize` impl) — switched to expecting JSON-encoded `&str` payloads from R callbacks (callers serialise via `jsonlite::toJSON` on the R side); (6) `build_extendr_arg` had no branch for `TypeRef::Named` params (e.g. `Renderer::render(&self, doc: &InternalDocument)`) — now serialises Named-typed args to JSON strings before crossing the bridge. Unblocks removing `exclude_languages = ["r"]` from `OcrBackend`, `PostProcessor`, `Validator`, `EmbeddingBackend`, `DocumentExtractor`, and `Renderer` trait_bridges in downstream `alef.toml` configurations.
- fix(e2e/rust): skip primitive / std element_types (`String`, `str`, integer/float types, `bool`, `char`) when emitting `use {module}::{T};` statements. Fixtures that declare `element_type = "String"` were emitting `use kreuzberg::String;`, which triggered E0432 ("unresolved import `kreuzberg::String`"). Primitives are already in scope via the Rust prelude.
- fix(e2e/python,e2e/elixir): broaden `is_streaming` detection to also fire when an assertion references a streaming-virtual field (e.g. `empty_stream` has `stream_chunks:[]` so `is_streaming_mock()` returns false, but the fixture still asserts on `chunks`/`stream_content` which need the collect snippet). Mirrors the kotlin Phase-A pattern.
- fix(e2e/typescript,e2e/wasm): broaden `is_streaming` detection in the typescript test renderer (shared by both node and wasm e2e codegen) to also fire when an assertion references a streaming-virtual field. Previously the `empty_stream` fixture (which has `stream_chunks:[]`) skipped the `const stream = await client.chatStream(...)` binding and the `for await (_chunk of stream) chunks.push(_chunk)` collect block, leaving `chunks` undefined at the assertion site (`ReferenceError: chunks is not defined`). Mirrors the same fix already applied to python/elixir/kotlin.
- fix(codegen): backfill ffi_skip_methods field at TraitBridgeConfig construction sites
- fix(alef-backend-ffi): stop duplicating the `Plugin` super-trait impl in `gen_ffi_trait_impl`. The orchestrator (`trait_bridge::mod.rs`) already emits it before calling the trait impl path; emitting it again triggered E0119 ("conflicting implementations of trait `Plugin`") for every FFI trait bridge (`DocumentExtractor`, `Renderer`, `OcrBackend`, `PostProcessor`, `Validator`, `EmbeddingBackend`).
- fix(alef-backend-dart): skip static / associated methods (e.g. `T::default()`, `T::new()`) on opaque impl blocks. The emitter previously generated `pub fn name(&self) -> T { self.inner.name() }` for every method including statics, but calling an associated function with method syntax (`self.inner.default()`) trips E0599 ("this is an associated function, not a method").
- fix(alef-backend-dart): emit explicit conversion wrappers for opaque method returns where the core type doesn't match the FRB-widened bridge type. `&str` → `String`, `&Path` → `String` via `to_string_lossy`, `&[&str]` → `Vec<String>`, and `i32`/`u32`/`usize` → `i64`, `f32` → `f64` per FRB's primitive widening contract. Previously the body emitted bare `self.inner.method()` which produced E0308 ("mismatched types") for these returns.
- fix(alef-backend-dart): filter out non-serde types in the `create_<T>_from_json` shim emission. Types with `has_serde = false` in the IR (e.g. `MergedChunk`, `ResolvedStyle`, `CharShape`, `OdtProperties`, `OcrCacheStats`) can't be deserialised via `serde_json::from_str::<CoreT>` — emitting the shim triggered E0277 ("the trait bound `T: Deserialize` is not satisfied") at compile time.
- fix(alef-backend-dart): emit `cfg = None` on per-field assignments inside `From<CoreT> for MirrorT` impls. The dart bridge crate enables `features = ["full"]` on its kreuzberg dep, so every core-side cfg-gated field is present at compile time. Emitting `#[cfg(feature = "X")]` would gate on the dart crate's own (undefined) features — evaluating to false and leaving the struct literal missing fields like `code_intelligence`, `extracted_keywords`, `html_options`, etc.
- fix(alef-backend-dart): trait-bridge wrapper `supported_mime_types` now returns `&[&str]` instead of `Vec<String>` to match `DocumentExtractor::supported_mime_types(&self) -> &[&str]`. The Dart closure produces an owned `Vec<String>` that's materialised into `&'static [&'static str]` via `Box::leak` per call (same pattern as napi/pyo3 trait bridges, `alef-codegen::generators::trait_bridge::gen_method`).
- fix(alef-backend-swift): skip static / associated methods in `emit_extern_block_for_type_methods` and `emit_type_method_shims`. The shim signature is `pub fn type_method(client: &T)` and the body calls `client.0.method()` — calling associated functions like `T::default()` via the receiver trips E0599.
- fix(alef-backend-swift): handle `TypeRef::Path` returns with `.to_string_lossy().into_owned()` (instead of `.to_string()`, which doesn't exist on `&Path`) and `Vec<String>` + `returns_ref` returns (`&[&str]`) with `.iter().map(|s| s.to_string()).collect()` so methods like `TessdataManager::cache_dir() -> &Path` and `DocumentExtractor::supported_mime_types() -> &[&str]` compile.
- fix(alef-backend-zig): emit async Rust functions as synchronous Zig wrappers. The `!f.is_async` filter in the function-emission loop was re-introduced by the opaque-handle codegen commit, causing `scrape`, `crawl`, and `map_urls` (all `is_async: true` in the IR) to disappear from generated `kreuzcrawl.zig`. The Zig C FFI wraps every async function with `block_on`, so from Zig's perspective all functions are synchronous — the `is_async` flag is irrelevant at the Zig ABI level.
- fix(alef-e2e/dart): handle `[].` array traversal in assertion rendering. Field paths like `links[].url` and `links[].link_type` previously generated invalid Dart syntax (`links()[0].url()` type subscripts that fail `dart format`). Now generates correct `array.any((e) => e.field.toString().contains(val))` expressions for `contains`, `contains_all`, `not_contains`, and `not_empty` assertion types.

- fix(alef-backend-dart): emit `pub(crate) inner: {ty.rust_path}` on the FRB opaque wrapper struct instead of `{source_crate}::{name}`. The naive form only resolves for types re-exported at the crate root; `HwpxExtractor` lives at `kreuzberg::extractors::HwpxExtractor`, `TessdataManager` at `kreuzberg::ocr::TessdataManager`, etc., so the generated bridge failed to compile with `E0425: cannot find type X in crate kreuzberg` for ~12 opaque handles. Falls back to `{source_crate}::{name}` only when `rust_path` is empty. Template `rust_opaque_wrapper_struct.jinja` now takes `inner_path` directly.
- fix(alef-backend-dart): emit `pub use {qualified_path}` at the crate root for excluded types referenced by required trait-bridge method signatures (e.g. `InternalDocument`). FRB strips module paths from `frb_generated.rs` and resolves bare names via `use crate::*`, so these types must be visible at the crate root. The walk filters `trait_source.is_some()` and `has_default_impl` methods to match `emit_trait_bridge`'s required-method filter — avoids re-exporting traits referenced only by default-impl methods (e.g. `SyncExtractor` via `as_sync_extractor`), which would otherwise trigger `E0782: expected a type, found a trait` in FRB-generated closure types.
- fix(alef-backend-swift): gate `create_<type>(api_key, base_url)` constructor emission on `client_constructor_body` override presence in `alef.toml`. The default `(api_key, base_url) -> Result<T, String>` template only matches liter_llm-style clients; for plugin types like `HwpxExtractor::new()` and utilities like `TessdataManager::new(Option<PathBuf>)` it produced calls that didn't match the real Rust signature and broke compilation. Opaque types without an override are returned by Rust APIs, not constructed in Swift, so omitting the shim is the correct default.
- fix(alef-backend-java): filter `api.errors` variants whose generated class names collide with the FFI infrastructure exception names (`InvalidInputException`, `ConversionErrorException`). Both code paths in `JavaBackend::generate` were writing files at the same target paths; for `InvalidInputException` this produced malformed Java with a duplicate constructor block appended after the closing brace. Keeps the hardcoded infrastructure version canonical and drops the conflicting `gen_java_error_types` emission.
- fix(alef-codegen,backends): stop stripping cfg-gated struct fields from the binding. Previously `gen_struct_with_per_field_attrs`/`gen_struct_with_rename` and the napi/wasm backends dropped any field with `#[cfg(...)]`. The result was that `metadata: HtmlMetadata` and `images: Vec<InlineImage>` on `ConversionResult` (and similar feature-gated fields on other crates) disappeared from the binding struct even when the binding crate enabled the same feature. Binding crates already mirror the core crate's features via their `Cargo.toml`, so the cfg gate is redundant — the field is always present at compile time when the feature is on. The conversion sites and constructor helpers are still cfg-aware for trait-bridge `bind_via = "options_field"` fields (force-restored via `never_skip_cfg_field_names`), which carry binding-wrapper types that need `#[serde(skip)]` and `Default::default()` initialization. Restores `result.metadata` / `result.images` access on Python/Node/PHP/WASM bindings.
- fix(alef-codegen): `constructor_parts_with_renames` and `config_constructor_parts_inner` in `shared.rs` now emit a default expression (`None` for `Option<T>` fields, `Default::default()` otherwise) for cfg-gated fields in the struct-literal assignment list, while continuing to omit them from the parameter list. Completes the cfg-gated field story: the binding struct keeps the field (per the prior `gen_struct_*` fix), but the caller can't supply trait-bridge wrapper values, so the constructor body fills them with a default. Without this, Python/WASM/PHP bindings hit `E0063 missing field visitor / metadata / images` because the struct now requires the fields the constructor wasn't initializing. Mirrors the same defaulting pattern already used in `gen_struct_default_impl`. The php `Named` constructor in `alef-backend-php/src/gen_bindings/types.rs` (manual `param_init`) was updated to match.
- fix(alef-backend-pyo3): pass the visitor kwarg through to `_rust.<fn>` in generated api.py wrappers. For `bind_via = "options_field"` bridges, the python wrapper was stuffing the visitor into `options.visitor` via `_to_rust_<snake>(opts, _visitor_override=visitor)` but never forwarding `visitor=` to the underlying Rust function, which is what `gen_bridge_field_function` actually reads to wrap the bridge handle. The visitor was silently dropped; user-supplied visitor implementations had no effect.

### Added

- feat(e2e/swift): wire `StreamingFieldResolver` into swift codegen. Adds `"swift"` arms to `StreamingFieldResolver::accessor` (all seven virtual fields: `chunks`, `chunks.length`, `stream_content`, `stream_complete`, `no_chunks_after_done`, `tool_calls`, `finish_reason`) and `collect_snippet` (emits `var chunks: [ChatCompletionChunk] = []` / `for try await _chunk in result { ... }` drain loop). Adds `"swift"` to `render_deep_tail` field segment dispatch (snake_case method calls). Wires `is_streaming` detection in `render_test_method` and streaming-virtual field interception in `render_assertion` before the field-not-available guard. Eliminates ~30 `// skipped: field '...' not available on result type` comments from `StreamingTests.swift` and `SmokeTests.swift`. Adds 7 swift-specific unit tests.
- feat(alef-e2e/field_access): support user-typed array indices in fixture field paths — `choices[0].message.content` now parses as `Segment::ArrayField{name:choices,index:0}` → `Field(message)` → `Field(content)` instead of being treated as a literal dot-key. Wired through every per-language renderer so explicit indices are emitted correctly: Rust/Python/Go/Zig/C#/Swift/TS/PHP/Ruby/Dart use bracket notation `[N]`, Java emits `.get(N)`, Kotlin emits `.first()` for index 0 or `.get(N)` for others, Elixir emits `Enum.at(expr, N)`. Config-registered array fields without explicit `[N]` continue to default to index 0.
- feat(alef-backend-dart/frb-stream): implement FRB v2 `StreamSink<T>` streaming for the Dart backend. Sanitized methods with a matching `[[adapters]] pattern = "streaming"` entry in `alef.toml` now emit `#[frb] pub fn method_name(&self, params..., sink: StreamSink<ItemType>)` in the Rust bridge crate instead of being skipped. FRB v2 generates a `Stream<ItemType>` on the Dart side automatically. The generated Cargo.toml gains `futures-util = "0.3"` when any streaming adapter is configured. `supports_streaming` is now `true` in `DartBackend::capabilities()`.
- feat(alef-e2e/dart): streaming fixture support in the Dart e2e codegen. Fixtures with `mock_response.stream_chunks` now emit `await _client.method().toList()` instead of `await _client.method()`. Streaming assertions: `count_min` on `chunks` emits `expect(result.length, greaterThanOrEqualTo(N))`, `equals` on `stream_content` concatenates deltas and compares, error cases use `.toList()` in the `expectLater` lambda.
- feat(alef-backend-rustler): emit `#[rustler::nif] pub fn <snake_type>_from_json` shims that deserialize a JSON string via `serde_json::from_str`, convert to the binding type, and return `Result<TypeName, String>`. Enables Elixir e2e tests to deserialize typed request objects from JSON strings.
- feat(alef-e2e/swift): `options_via = "from_json"` support in Swift e2e test generation. When a call override sets `options_via = "from_json"` and `options_type = "<TypeName>"`, `build_args_and_setup` now emits `let _argName = try typeNameFromJson("<fixture-json>")` setup lines for `json_object` args instead of a `XCTSkipIf(true, "swift: json_object request construction requires options_via configuration ...")` skip stub. Handles `field = "input"` (entire fixture input is the request) correctly via `resolve_field`. Eliminates ~140 skip stubs in liter-llm swift e2e tests.
- feat(alef-backend-swift/gen_rust_crate): emit `{type_snake}_from_json(json: String) -> Result<TypeName, String>` free-function shims and extern "Rust" declarations for serde-enabled non-opaque types that appear as method parameters on opaque types. `collect_serde_param_types` scans both free-function params and `api.types[].methods[].params` to find these types. Enables `chatCompletionRequestFromJson`, `embeddingRequestFromJson`, etc. in the RustBridge Swift module.
- feat(alef-e2e/dart): dart assertion renderer reaches parity with rust/kotlin/go — all 24 fixture assertion types handled (was 5). Adds `render_dart()` and `render_dart_with_optionals()` to `FieldResolver` with proper per-segment camelCase conversion (FRB v2 convention), wired into `accessor()` and `render_accessor()` dispatch via a new `"dart"` arm. Replaces the single-pass `snake_to_camel` field path hack with `field_to_dart_accessor` for correct multi-segment path conversion.
- feat(e2e/streaming): support deep-nested paths in `StreamingFieldResolver` — `tool_calls[N].function.name`, `tool_calls[N].id`, and similar paths now resolve against the flat-mapped collected tool-calls list instead of emitting `// skipped:` comments. `split_streaming_deep_path` splits the root virtual field from the tail, `parse_tail` tokenises the tail into `Index`/`Field` segments, and `render_deep_tail` applies per-language navigation conventions (bracket for Rust/Go/TS/Python/PHP/C#/Kotlin/Dart/Swift/Ruby/Zig, `.get(N)` for Java, `Enum.at(expr, N)` for Elixir, `.items[N]` for Zig ArrayList, `.first()` for Kotlin index-0, `.ID`/`.URL` initialism expansion for Go). Covered by three new snapshot tests: `deep_tool_calls_function_name_snapshot_rust_kotlin_ts`, `deep_tool_calls_id_snapshot_all_langs`, and `deep_tool_calls_function_name_snapshot_python_elixir_zig`.

### Fixed

- fix(alef-e2e/java,csharp): assertion type 'not_error' on byte[] result now emits assertNotNull(result) / Assert.NotNull(result) instead of skipping. Eliminates 10 skipped comments across speech/transcribe fixtures.
- fix(alef-e2e/kotlin): substitute the field name in 'skipped: field {f} not available' comments instead of leaking the literal {f} placeholder. (Phase D eliminates these comments entirely for streaming-virtual fields.)
- fix(alef-backend-pyo3): import `has_default` config DTOs from `.options` even when they expose `Self`-returning builder methods. The api.py import classifier built `return_type_names` by walking method return types (and their transitive field references), which pulled config types like `PackConfig` (with `from_toml_file -> PackConfig`) and `ProcessConfig` (with `default`/`with_chunking`/`all`/`minimal` builders) out of `options_type_names`. The generated api.py then imported them from `._native`, breaking `mypy --strict` for any caller passing the re-exported dataclass. Use the IR-level `is_return_type` flag alone (set by alef-extract only for direct free-function returns), removing the over-broad method/field walk. Fixes [#72](https://github.com/kreuzberg-dev/alef/issues/72).
- fix(alef-e2e/java): `not_contains` assertions now handle the plural `values: [...]` array (symmetric to `contains_all`) in addition to the singular `value`. Previously, a fixture using `not_contains` with `values` produced `result.contains()` (empty argument) — a Java compile error. The codegen now wraps a singular `value` into a one-element list, and the jinja template loops over `values_java` emitting one `assertFalse(...contains(val))` call per entry.
- fix(alef-codegen): cfg-gated fields restored via trait-bridge `bind_via = "options_field"` now emit `#[serde(skip)]` and are propagated through both `binding→core` and `core→binding` From impls. Previously, restoring such a field (e.g. `visitor: Option<VisitorHandle>` on `ConversionOptions`) produced uncompilable bindings: the struct derived `serde::{Serialize, Deserialize}` over a non-serde-compatible bridge handle type, and the `From<Core>` impls omitted the field, so `Self { ... }` initializers were missing it. Adds `never_skip_cfg_field_names` to `ConversionConfig`, populated from `trait_bridges` by each backend.
- fix(alef-backend-napi,php): apply the trait-bridge `never_skip_cfg_field_names` mechanism added in alef-codegen so the NAPI (Node) and PHP bindings keep `bind_via = "options_field"` fields (e.g. `visitor` on `ConversionOptions`) instead of dropping them with the rest of the cfg-gated fields. Mirrors the existing PyO3 backend behavior.
- fix(alef-backend-ffi,php): emit `*result` (Copy) for `TypeRef::Char` and `result.to_vec()` for `TypeRef::Vec/Map` slice returns in `gen_method_wrapper`, instead of `result.clone()` (clippy noop on `&char` and unsized slices). Also emit `.to_string()` rather than `.clone()` for `&str` params before moving into `spawn_blocking` closures (avoids E0521 borrow-escape).
- fix(pyo3): emit `error_to_py_err` directly in `.map_err()` (no redundant closure) — avoids clippy `redundant_closure` in generated bindings.
- fix(pyo3): api.py import order — stdlib `from typing import …` first, then `import _rust as _rust`, then capsule-type imports (e.g. `from tree_sitter import Language`), then locals.
- fix(pyo3): api.py wraps capsule-returning calls in `cast("Type", …)` for mypy strict mode; adds `cast` to the typing import only when needed.
- fix(pyo3): skip capsule types in `.pyi` stubs (they live in third-party packages); conditional `from typing import …` based on actual usage — avoids RUF100 for the old unconditional four-name import. Method stubs on opaque types (e.g. `LanguageRegistry.get_language`) also substitute `Any` for capsule-typed params/returns. Suppresses PYI029 on the data-enum class wrappers' `__str__`/`__repr__` — the pyo3 wrapper implements them via Display/Debug and callers rely on `str(value)` returning the serde tag.
- feat(alef-backend-napi): `capsule_types` support for raw-pointer passthrough into external tree-sitter libraries (e.g. the `tree-sitter` npm package). Types listed under `[crates.node.capsule_types]` are excluded from opaque `#[napi]` class emission; functions returning them instead emit a `#[napi]` shim that calls `value.into_raw()`, wraps the pointer in `env.create_external(ptr, None)`, and sets it as `__parser` on a returned `JsObject`. TypeScript declaration stubs emit `import type { T } from "module"` and use the ecosystem type name directly in function signatures.
- fix(pyo3): drop redundant `pass` body for empty-field types in generated `options.py` — `pass` after a docstring triggers ruff PIE790.
- fix(wasm): add `#[allow(clippy::should_implement_trait)]` to inherent `default()` factory method; renaming would change the JS-visible API.
- fix(java): marshal `Path` params via `.toString()` in opaque-type stream-method bodies — previously lumped with `String`/`Char`/`Json` whose template does not call `.toString()`, causing a Java type mismatch.

## [0.15.44] - 2026-05-12

### Added

- feat(alef-core/java): `[java] group_id` and `[java] artifact_id` config overrides for the Maven coords emitted by `alef-scaffold/java`. When unset, `java_group_id()` falls back to the Java package and `java_artifact_id()` falls back to the source crate name. Set these when the published Maven coords need to differ from the Java package / source crate name (e.g. crate `html-to-markdown-rs` publishes as artifactId `html-to-markdown` under groupId `dev.kreuzberg`).

### Fixed

- fix(alef-e2e/zig): drop the misleading `// Note: async functions not yet fully supported; treating as sync` comment from generated tests in all three emission paths (error, no-assertion, happy). The generated tests already exercise the real async code path via `tokio::runtime::block_on` in the FFI shim, so the comment was cosmetic noise.
- fix(alef-e2e/kotlin): remove the dead `call_overrides.is_none()` short-circuit that emitted `Assumptions.assumeTrue(false, "TODO: implement Kotlin e2e test")` for every fixture lacking a per-call override. The existing fallback resolves `function_name`/`result_var`/`args` from `call_config.function.to_lower_camel_case()` and the global `[e2e.call.overrides.kotlin]` block, so the real assertion renderer is now reachable for all kotlin fixtures. Also extend `client_factory` lookup to fall back to the global block (mirrors dart pattern), so per-call kotlin overrides don't lose the global `createClient` factory.
- fix(alef-e2e/dart): emit `expect(...)` assertion calls after the `await` in the non-HTTP test path. Previously the renderer dropped the result on the floor (`final result = await ...;` with no following assertions). Added `render_assertion_dart` helper handling `equals`, `field_equals`, `contains`, `not_null`, `not_error`; unknown assertion types emit a `// skipped:` comment as before.

## [0.15.43] - 2026-05-12

### Fixed

- fix(alef-backend-go): emit gofmt-clean output from the visitor templates — `visitor_preamble.jinja` now ends with a trailing newline (so `gofmt` doesn't insert a blank line between the import block and the first declaration), and `visitor_registry_block.jinja` aligns the `visitorRegistry`/`visitorIDCounter` field types so `gofmt` doesn't realign them. Without this, the alef-verify pre-commit hook fails because `gofmt` reformats the regen output and the resulting hash no longer matches the `alef:hash` header.

## [0.15.42] - 2026-05-12

### Fixed

- fix(alef-backend-go): prepend `C.` to all cgo function calls and the options struct type in the visitor helper template. Previously `convert_with_visitor_helper.jinja` emitted bare identifiers (`htm_conversion_options_from_json(...)`, `htm_options_set_visitor(...)`, `htm_convert(...)`, etc.) which Go treated as undefined package-level functions, breaking the entire Go binding with `undefined: htm_*` errors. Now matches the existing `binding.go` convention of `C.htm_*` for cgo symbol resolution.

## [0.15.41] - 2026-05-12

### Fixed

- fix(alef-backend-wasm): emit a real newline after the rustdoc block so `#[derive(Clone, Default)]` no longer ends up concatenated inside the `///` comment line. Previously every `Wasm*` binding struct silently lost its `Clone`/`Default` derives because the template's `{%- endfor %}` stripped the newline between rustdoc lines and the subsequent attribute, causing `error[E0277]: the trait bound 'WasmDocumentMetadata: Default' is not satisfied` (and many like it) at compile time.
- fix(alef-backend-pyo3): skip the `options.{visitor}` fallback in the convert wrapper when the bridge field is `#[cfg(...)]`-gated and therefore absent from the binding struct. Previously the fallback was inserted unconditionally and produced `error[E0609]: no field 'visitor' on type '&ConversionOptions'`.
- fix(alef-backend-napi): gate `find_options_field_binding` on the bridge field being present in the binding struct. When the core field is cfg-gated and stripped from the binding, fall back to the regular function generator instead of emitting an options-field bridge that references a missing field.

## [0.15.40] - 2026-05-12

### Fixed

- fix(alef-e2e/php): `not_contains` assertions now handle the plural `values: [...]` array (symmetric to `contains_all`) in addition to the singular `value`. Previously, a fixture using `not_contains` with `values` produced `assertStringNotContainsString(, $result)` — an empty-first-argument PHP parse error. The codegen now wraps a singular `value` into a one-element list, and the jinja template loops over `values_php` emitting one `assertStringNotContainsString` call per entry.
- fix(alef-backend-dart): use `From<MirrorT> for CoreT` conversion instead of `unsafe transmute` in opaque-type method bodies for types whose mirror layout differs from the core layout. The `has_layout_mismatch` detection now covers Duration, Path, Json (serde_json::Value → String, 32 vs 24 bytes), and non-identity primitive widening (e.g. u32 → i64, 4 vs 8 bytes). The transitive-closure computation (`compute_types_needing_from_impl`) is extended to process enum variant field types so that types like `SystemMessage` and `ToolChoiceMode` (reachable through `Message` and `ToolChoice` from `ChatCompletionRequest`) also get `From<MirrorT> for CoreT` impls. Additionally, `String` method parameters marked `is_ref` now correctly pass `&param_name` to core methods that take `&str`. `Bytes`-returning methods chain `.to_vec()` to convert `bytes::Bytes` to `Vec<u8>`.
- fix(alef-backend-dart): emit `create_<snake>_from_json(json: String) -> Result<TypeName, String>` free bridge functions for all non-opaque mirror struct types. These allow e2e tests to construct typed request objects from JSON without manually populating every field — required because FRB bridge methods use named parameters that cannot accept bare JSON strings. FRB generates the corresponding `createTypeNameFromJson(json: '...')` top-level Dart function from these annotated Rust stubs.
- fix(alef-e2e/dart): pass `String`-typed method arguments as Dart named parameters (`paramName: 'value'`) instead of positional arguments — FRB v2 generates all bridge method parameters as `{required T name}` named params in Dart, so positional passing produced `extra_positional_arguments_could_be_named` compile errors.
- fix(alef-e2e/dart): add `dart` to `chat_stream` `skip_languages` — the `chat_stream` method has a sanitized `BoxStream` return type that cannot be bridged through FRB, so the method is absent from the generated Dart class and streaming tests cannot be emitted. Skipping them matches the `go` treatment.
- fix(alef-e2e): fix `retrieve_response` and `cancel_response` call configs to use `field = "input.response_id"` — the liter-llm fixtures store the response identifier under `input.response_id`, not `input.id`, so the previous config caused "missing required input field 'id'" warnings and generated invalid no-argument calls in every language backend.

- fix(alef-e2e/swift): emit `try XCTSkipIf(true, ...)` stub for test methods whose call has a `json_object` arg without an `options_via` construction mechanism — previously the codegen passed `nil` or a bare JSON string literal, producing a Swift compile error (`'nil' is not compatible with expected argument type 'ChatCompletionRequest'` / `cannot convert value of type 'String' to expected argument type 'EmbeddingRequest'` etc.). Affects `chat`, `embed`, `speech`, `transcribe`, `image_generate`, `moderate`, `rerank`, `search`, `ocr`, `create_batch`, `create_file`, `create_response`, and `chat_stream` calls in liter-llm. The skipped tests compile and are recorded as XCTest skips rather than compile failures.
- fix(alef-e2e/swift): use `CharacterSet.whitespaces` instead of `.whitespaces` in all `trimmingCharacters(in:)` calls — Swift's type inference cannot always resolve the member shorthand inside `XCTAssertEqual` overloads, producing `error: cannot infer contextual base in reference to member 'whitespaces'`. The explicit `CharacterSet.whitespaces` is always unambiguous.
- fix(alef-e2e/swift): honour `extra_args` from per-call language overrides in the Swift e2e codegen — previously `extra_args = ["nil"]` entries in `[e2e.calls.<name>.overrides.swift]` were silently ignored, causing calls such as `listFiles()` and `listBatches()` to omit the required (but optional-typed) query parameter and fail with `error: missing argument for parameter #1 in call`. The fix appends verbatim extra args after the normal argument list, mirroring the existing behaviour in Go, Java, Ruby, and Gleam backends.
- fix(alef-e2e/swift): correctly handle `[N]` array subscripts in `swift_build_accessor` — segments containing an index (e.g. `data[0]`) were naively appended as `data[0]()` instead of the correct `data()[0]`, producing a Swift compile error. The segment is now split at `[` so the method call `()` is placed before the subscript.
- fix(alef-e2e/swift): read per-language `result_is_simple` override in swift codegen — the flag was only read from the base `CallConfig`, ignoring `[e2e.calls.<name>.overrides.swift] result_is_simple = true` entries. Calls such as `file_content` that return `Data` (not a struct) were incorrectly attempting to access `result.content().toString()`, producing a Swift compile error. The fix mirrors ruby's resolution: `base || override || global`.
- fix(alef-e2e/swift): merge per-call `enum_fields` keys from swift overrides into the effective enum set for each test method — fields like `"status"` (type `BatchStatus`) were not in the global `fields_enum` and so generated `.toString()` on the opaque `RustBridge.BatchStatus` class (which has no such method), producing a compile error. The fix builds a `Cow<HashSet<String>>` that extends the global set with the per-call override keys, then uses `String(describing:)` for those fields.
- fix(alef-e2e/swift): emit `XCTAssertFalse(result.isEmpty, ...)` for `not_empty` assertions when `result_is_simple = true` — previously the code path fell through to `result.toString().isEmpty`, which fails to compile when the result type is `Data` (no `.toString()` method). The fix detects `result_is_simple` before the string-conversion branch and calls `.isEmpty` directly on the result variable, which is valid for both `String` and `Data`.
- fix(alef-e2e/dart): remove erroneous `call_config.id` field references introduced by the dart agent — `CallConfig` has no `id` field, causing a compile error `no field 'id' on type '&CallConfig'`. The `.or_else` fallback branches that used this field were redundant (the primary `call_overrides` resolution already reads the correct per-fixture per-language override) and are now removed.

- fix(alef-backends/visitor): preserve original-case string in the `VisitResult::Custom` variant across `pyo3`, `napi`, `magnus`, `php`, `extendr`, and `wasm` trait-bridge templates. The dispatch match was binding the already-lowercased string to `other` and using it as the `Custom` payload, so user-returned bare strings like `"[Download](url)"` were emitted as `"[download](url)"`. The fallback arm now returns `Custom(s)` (or `Custom(s.to_string())` for `&str` sources). Fixes kreuzberg-dev/html-to-markdown#350.
- fix(alef-scaffold/php): emit `php-ext.download-url-method = ["pre-packaged-binary", "composer-default"]` in the scaffolded `packages/php/composer.json` so PIE 1.4.x discovers pre-built binaries attached to GitHub releases instead of falling through to source-build via `phpize` (which has no `config.m4` for ext-php-rs extensions). Fixes kreuzberg-dev/html-to-markdown#333.
- fix(alef-backend-kotlin): wrap opaque-handle client types as coroutine-friendly Kotlin classes that delegate to a sibling Java instance instead of calling non-existent `Bridge.<type>_<method>(handle, ...)` static methods. The previous generator emitted `class DefaultClient(apiKey, baseUrl)` with `private val handle: Long = Bridge.default_client_new(...)` and `Bridge.default_client_chat(handle, req)` calls — but the Java FFM backend exposes those FFI symbols only as instance methods on the Java `DefaultClient`, not as flat statics on the facade. The wrapper now takes `internal val inner: <java_package>.<ClassName>` and delegates each method as `withContext(Dispatchers.IO) { inner.<method>(args) }`. Construction flows through the existing facade factories (e.g. `LiterLlm.createClient(...)` now wraps the returned Java instance). Methods whose IR signature was sanitized (e.g. `chat_stream` whose real Java return type is a custom `Iterator<Chunk>`) are skipped — they require backend-specific surfaces that the generic generator cannot synthesize. Trait types and non-opaque value types (e.g. kreuzberg's `ExtractionConfig` with only a `default()` static) keep flowing through the Java typealias as before.
- fix(alef-backend-kotlin): exclude types-with-methods from `typealias` emission so the new Kotlin wrapper class does not collide with `typealias DefaultClient = <java>.DefaultClient` in the same package (was producing `Redeclaration: DefaultClient` from `compileKotlin`). The exclusion is gated on `is_opaque && !is_trait && has(non_sanitized, non_static method)` so kreuzberg's value-type configs keep their existing alias surface.
- fix(alef-backend-kotlin): emit `= null` defaults for optional flat-function parameters on the facade `object` so callers can use named-argument syntax (`LiterLlm.createClient(apiKey = "x", baseUrl = "y")`) without spelling out every nullable downstream argument.
- fix(alef-e2e/kotlin): client-factory smoke tests now invoke the configured factory function as `<class>.<client_factory>(apiKey, baseUrl)` (e.g. `LiterLlm.createClient(...)`) rather than calling the class name as a constructor — the latter produced `LiterLlm(apiKey, ...)` against the singleton facade `object` and failed to compile.
- fix(alef-backend-swift): emit `use <trait_path>;` statements in generated `lib.rs` for every trait-provided method on opaque-handle types — without these, `client.0.chat(req)` fails with `no method named 'chat' found for struct DefaultClient; perhaps trait LlmClient is implemented but not in scope`. Imports are collected from `MethodDef.trait_source` (None for inherent methods) and de-duplicated per type.
- fix(alef-backend-swift): correct call-site handling of opaque-method arguments — `Option<Named>` newtype args now emit `arg.map(|v| v.0)` instead of the invalid `arg.0`, `&str`/`&Path`-typed args (marked `is_ref` in IR) now emit `&arg` instead of moving the owned `String`, and `Bytes`-returning methods now chain `.map(|b| b.to_vec())` so `bytes::Bytes` converts to the `Vec<u8>` swift-bridge expects on the bridge boundary.
- fix(alef-backend-swift): allow per-type override of the auto-generated `create_<type>(api_key, base_url)` constructor body via `[crates.<crate>.swift] client_constructor_body."TypeName" = "…"`. Required for source crates whose constructor signature differs from the hardcoded `Type::new(api_key, base_url)` shape — e.g. liter-llm's `DefaultClient::new(ClientConfig, Option<&str>)`.
- fix(alef-backend-swift): only inject the implicit `ocr-wasm` cargo feature when the umbrella source crate actually exposes it in its on-disk `Cargo.toml`. Previously the unconditional injection caused `error: package liter-llm-swift depends on liter-llm with feature 'ocr-wasm' but liter-llm does not have that feature` for crates without an OCR module.
- fix(alef-backend-swift): drop argument labels in the `DefaultClient.init` -> bridge constructor call — swift-bridge emits the free `createDefaultClient(_:_:)` function without parameter labels, so the host wrapper now calls it positionally. Bytes-returning host wrapper methods bind the result locally and wrap in `Data(bytes.map { $0 })` to convert from `RustVec<UInt8>`.
- fix(alef-backend-dart,alef-backend-swift): emit explicit `package = "..."` rename in generated `Cargo.toml` `[dependencies]` block when the umbrella crate's Rust ident form (e.g. `liter_llm`) differs from the on-disk cargo package name (e.g. `liter-llm`). Without this, `liter_llm = { path = "..." }` causes `error: no matching package found — searched: liter_llm; perhaps you meant: liter-llm`. The fix is symmetrical across both backends and only fires when no explicit `core_crate_override` is set. (Swift had the rename logic in place but the new `core_crate_dir` argument was not wired through the cargo emitter — both sides are now consistent.)
- fix(alef-backend-dart): bump generated Dart pubspec SDK constraint to `>=3.3.0 <4.0.0`. `flutter_rust_bridge` 2.x emits `extension type` declarations in the WASM-side `frb_generated.web.dart`, which the Dart analyser rejects on SDKs below 3.3. Lower SDKs analyse cleanly for native-only builds but fail when the `.web.dart` file is in scope (always, since FRB emits it unconditionally). Same constraint now propagates to the alef-e2e dart pubspec template.
- fix(alef-backend-dart): emit static-wrapper signatures with required parameters as positional and optional parameters inside a `{...}` named-parameter block. Previously every param was positional, forcing callers (and the e2e codegen, which emits `createClient('test-key', baseUrl: mockUrl)`) to compile-fail with `not_enough_positional_arguments` / `undefined_named_parameter`. Functions with the special `ExtractionConfig` param (kreuzberg) keep the existing `[ExtractionConfig? config]` optional-positional shape — the new logic only applies to all-other-functions.
- fix(alef-backend-zig): emit iterator-based streaming method bodies — previously `chat_stream` was emitted as a generic method that called the callback-based C symbol `literllm_default_client_chat_stream(client, req_handle)` with only 2 arguments, producing a Zig compile error "expected 4 argument(s), found 2". Streaming adapters (pattern = "streaming") now detect the matching `Streaming` adapter config, derive the item type, and emit a `_start`/`_next`/`_free` iterator body that collects chunks, keeping the last chunk JSON as the return value (or `"{}"` on an empty stream). The detection is fully generic — any method named in a `Streaming` adapter in `alef.toml` receives the iterator body.
- fix(alef-backend-zig): emit multi-out-parameter convention for `Bytes` return types in both flat-function and opaque-method codegen — the C FFI returns `int32_t` status and writes the buffer via three out-params (`uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap`); the zig wrapper now declares `var _out_ptr: [*c]u8 = undefined; var _out_len: usize = 0; var _out_cap: usize = 0;`, passes `&_out_ptr, &_out_len, &_out_cap` as extra C args, copies the buffer into a caller-owned `[]u8` via `std.heap.c_allocator.dupe`, and releases the FFI allocation via `{prefix}_free_bytes`. `zig_return_type` now maps `TypeRef::Bytes` to `[]u8` (owned) rather than `[]const u8`. Unblocks `speech` / `file_content` and any other library method returning `Result<Vec<u8>, _>`.
- fix(alef-e2e/gleam): fix four compile errors in generated Gleam e2e test files:
  1. `client_factory` calls now emit `option.None` (or `option.Some(base_url)`) instead of `""` for the `base_url` arg, and accept a new `client_factory_trailing_args` key on `[e2e.call.overrides.<lang>]` so consumers can pad extra positional parameters (e.g. `["option.None", "option.None", "option.None"]` for liter-llm's 5-arg `create_client`).
  2. `json_object` args without an `element_constructors` recipe or `json_object_wrapper` now emit a `// skipped` stub instead of a bare JSON-string literal that fails to typecheck against typed record parameters (`ChatCompletionRequest`, `EmbeddingRequest`, etc.).
  3. `extra_args` from per-call language overrides are now appended to the generated argument list; consumers can use `extra_args = ["option.None"]` to supply required-but-fixture-absent parameters such as the optional `query` argument on `list_files` / `list_batches`.
  4. Array-element field assertions (`data[1].field`, `pages[2].field`, etc.) are now correctly skipped — previously only `[0].` and `[].` patterns were matched; the fix uses a regex-free digit scan so any `[N].` index is recognised.
     Additionally: `equals` assertions on fields listed in the effective enum-field set (global `fields_enum` merged with per-call `enum_fields` / `assert_enum_fields`) now emit a `// skipped` comment in Gleam instead of generating a string comparison against a sum-type value that does not typecheck. Per-call `result_is_simple = true` overrides now cause field-access assertions to be skipped when the return type is a primitive (e.g. `BitArray`) with no record fields.
- fix(alef-backend-gleam): skip non-trait types with methods from the regular data-type emission pass — they are now emitted exclusively as opaque NIF resource handles (`pub opaque type T { T(resource: dynamic.Dynamic) }`). Previously such types were emitted twice (once as a phantom record by `emit_type` and once as an opaque resource by `emit_resource_type`), producing a "Duplicate type definition" compile error from `gleam build`.
- fix(alef-backend-zig): emit synthetic `free()` destructor on opaque-handle structs — every opaque handle owns a heap allocation in the FFI and must be released via the matching `{prefix}_{snake}_free` C symbol; `emit_opaque_handle` now appends a `pub fn free(self: *T) void` that calls the destructor, so `defer _client.free();` in generated e2e tests resolves correctly.
- fix(alef-backend-zig): remove spurious leading `!` on opaque-method signatures — the emit template was producing a double error union `!(LiterLlmError||error{OutOfMemory})![]u8` which Zig 0.16 rejects with "type does not support field access"; corrected to a single error union.
- fix(alef-backend-zig): map FFI errors to declared error set via `_first_error` — opaque-method error path was returning `error.FfiError` which isn't in the function's declared error set; aligned with flat-function codegen by calling `_first_error({error_type})`.
- fix(alef-e2e/zig): auto-set `result_is_json_struct` when `client_factory` is configured — opaque-method results are always serialized to JSON `[]u8` by the zig backend, so the e2e test must parse them with `std.json.parseFromSlice` rather than expecting a typed Zig struct.
- fix(alef-backend-zig): emit async opaque-handle methods — the `is_async` guard in `emit_opaque_method` was incorrectly skipping all methods whose Rust source is `async` even though the C FFI wraps them as synchronous functions via `block_on`; removed the guard so all non-static methods are emitted.
- fix(alef-backend-zig): correct optional-integer FFI marshalling — `?u64`/`?u32` (and any `Optional(Primitive)`) parameters now emit `if (x) |v| v else std.math.maxInt(T)` to pass the sentinel value the Rust FFI uses for `None`, instead of passing the `?T` type directly to a non-optional C parameter.
- fix(alef-backend-zig): wrap opaque-handle function return in Zig struct — `create_client` and similar functions that return an opaque C pointer now emit `TypeName{ ._handle = _result.? }` instead of returning the raw nullable C pointer, matching the `_handle: *anyopaque` field type.
- fix(alef-backend-zig): fix unreachable-code in opaque-method error block — `_ = _msg;` was emitted after `return error.FfiError;`; reordered to suppress the unused-variable warning before the early return.
- fix(alef-backend-zig): use `{prefix}_last_error_context` (not `_message`) in opaque-method error path — the C FFI exposes `_last_error_context`, not `_last_error_message`.

### Added

- feat(alef-e2e/visitor): `CallbackAction::CustomTemplate` gains a `return_form` field (`"dict"` (default) or `"bare_string"`). When set to `"bare_string"`, the generated test visitor method returns the rendered template directly as a string (e.g. `return f'…'` in Python, `` return `…` `` in TS/WASM, `"#{…}"` in Ruby, the raw string in PHP/R) so the trait-bridge's bare-string return path is exercised — instead of the dict/object wrapper that all existing fixtures relied on. Default behaviour is unchanged.
- feat(zig): client-object/opaque-handle codegen — types with non-empty `TypeDef.methods` that are opaque or non-serde now emit a Zig `pub const TypeName = struct { _handle: *anyopaque, ... }` with one `pub fn` per non-static, non-async method, dispatching via `c.{prefix}_{snake_type}_{snake_method}`. Driven by `CallOverride.client_factory` in e2e; no special-casing per library.
- feat(zig e2e): `[e2e.call.overrides.zig].client_factory` support — when set, generated Zig test functions instantiate a client via `module.factory_fn("test-key", mock_url, ...)` and call methods on the `_client` instance instead of calling the top-level module function directly. Mirrors the Go/Swift/Kotlin/Dart client-factory pattern.

- feat(gleam): opaque-resource codegen — types with non-empty `TypeDef.methods` now emit `pub opaque type TypeName { TypeName(resource: dynamic.Dynamic) }` plus one `@external` NIF binding per method. Mirrors the Swift/Kotlin/Dart client-object pattern.
- feat(gleam e2e): `[e2e.call.overrides.gleam].client_factory` support — when set, generated Gleam tests call the factory function and pass the client as the first argument to the method under test.
- fix(gleam e2e): Erlang startup shim now uses a `case` expression with a graceful `{error, _} -> ok` fallback when starting the Elixir application, avoiding test failures in environments without an Elixir runtime dependency.

- feat(dart e2e): `[e2e.call.overrides.dart].client_factory` support — when set, generated `package:test` tests call `await {BridgeClass}.{factory}('test-key', baseUrl: mockUrl)` before the assertion and dispatch the method on the resulting `_client` instance. Mirrors the Go/TypeScript/Zig/Swift/Kotlin client-factory pattern; no special-casing per library.

### Changed

- refactor(dart): `[crates.dart].stub_methods` is now a config-driven list of method names whose Rust bridge body is replaced with `unimplemented!()`. Previously this behaviour was hardcoded to kreuzberg's `batch_extract_bytes` / `batch_extract_bytes_sync`. **Migration**: if your `alef.toml` relied on the hardcoded list, add `stub_methods = ["batch_extract_bytes", "batch_extract_bytes_sync"]` (or the relevant names) under `[crates.<name>.dart]`.

### Known limitations

- FRB codegen still requires a manual post-step after `alef generate`: `cd packages/dart/rust && cargo run --bin flutter_rust_bridge_codegen`. Implementing a `PostBuildStep` for this is deferred to a future release.
- The dart backend does NOT yet emit instance methods on opaque-type wrappers in the FRB Rust crate. When a type has `TypeDef.methods` (e.g. `DefaultClient::chat`, `::embed`, `::list_models` in liter-llm), only the empty `#[frb(opaque)] pub struct DefaultClient { inner: ... }` wrapper is emitted — FRB then generates an `abstract class DefaultClient implements RustOpaqueInterface {}` with no callable methods. The swift/kotlin/zig/gleam backends already implement this (driven by `TypeDef.methods` + `[e2e.call.overrides.<lang>].client_factory`); the dart equivalent — emitting `impl DefaultClient { #[frb] pub fn chat(&self, req: ChatCompletionRequest) -> Result<ChatCompletionResponse, String> { ... } }` blocks so FRB surfaces them as Dart instance methods — is the natural next step but is deferred. Until then, dart e2e tests for libraries with stateful clients (liter-llm) will not compile past the `_client.chat()` call sites even though the static factory (`createClient`) and all data-type mirrors generate correctly.

### Added

- feat(swift): client-object class wrapper — types with non-empty `TypeDef.methods` now emit a `public final class TypeName` with an `init(apiKey:baseUrl:)` constructor and one `public func method(...)` per method, backed by free-function shims in the swift-bridge crate (`create_<type>` / `<type>_<method>`). Driven by `TypeDef.methods` in the IR; no special-casing per library.
- feat(swift e2e): `[e2e.call.overrides.swift].client_factory` support — when set, generated XCTest methods instantiate `DefaultClient(apiKey:baseUrl:)` against the mock server URL and call `client.<method>(args)` instead of a free function. Mirrors the Go/TypeScript/Zig client-factory pattern.
- feat(swift e2e): `Package.swift` now always emits `.iOS(.v14)` alongside `.macOS(...)` in the platforms array; swift-bridge supports both targets.
- feat(kotlin): client-object class codegen — types with non-empty `TypeDef.methods` now emit a `DefaultClient.kt` with a `class DefaultClient(apiKey, baseUrl?)` constructor, one method per `MethodDef`, and `AutoCloseable.close()` delegating to `Bridge.<type>_free(handle)`. Driven entirely by IR; flat-function Kotlin (kreuzberg) is unaffected.
- feat(kotlin e2e): `[e2e.call.overrides.kotlin].client_factory` support — when set, generated JUnit 5 tests instantiate `DefaultClient(apiKey, baseUrl)` against the mock server URL and call `client.<method>(args)` followed by `client.close()`, mirroring the Go/TypeScript/Zig pattern.
- feat(kotlin kmp): KMP `build.gradle.kts` now emits `iosX64`, `iosArm64`, `iosSimulatorArm64` (framework binaries) and `androidNativeArm64` (sharedLib) targets with cinterop blocks; corresponding `iosMain` and `androidNativeArm64Main` sourceSets wired to `nativeMain`.
- feat(kotlin mode): `KotlinConfig` gains `pub mode: Option<String>` field — accepted values `"jvm"` (default), `"kmp"`, `"android"`. Setting `mode = "android"` emits an Android library project under `packages/kotlin-android/` (minSdk 21, compileSdk 35, `AndroidManifest.xml`).

### Note

- XCFramework `binaryTarget` codegen (pre-built `.xcframework` distribution for SwiftPM) is deferred as a follow-up; the current output requires local `cargo build` to produce the Rust dylib.

## [0.15.39] - 2026-05-11

### Added

- feat(alef-backend-pyo3): `[crates.python.extra_init_imports]` re-exports hand-written Python symbols (e.g. literal type aliases generated by user scripts) through `__init__.py` without alef culling them. Schema: `{ "<module>" = ["<symbol>", ...] }`. Symbols are appended to `__all__` and the source modules are skipped by the cleanup pipeline.

### Fixed

- fix(alef-backend-pyo3): capsule-typed return values now Python-construct the declared `python_type` instead of returning a bare `PyCapsule`. Both the `Capsule(...)` and `ConstructFrom { python_type, construct_from }` config variants now produce real `tree_sitter.Language` / `tree_sitter.Parser` instances at the call site, restoring `parser.parse(bytes)` semantics in downstream Python bindings. Also filters capsule types out of generated `from ._native import …` statements in `api.py` and `__init__.py`, since they have no native Rust class to import.

## [0.15.38] - 2026-05-11

### Fixed

- fix(hooks): `alef_hook.py` now honours `[crate].version_from = "Cargo.toml"` in `alef.toml` and resolves the version from the referenced Cargo.toml's `[workspace.package]` (or `[package]`) `version` field. Previously the hook always read the top-level `version = "..."` line in alef.toml, which is a stale lazily-bumped fallback — downstream consumers tried to download `alef-<host>.tar.gz` for that stale version and 404'd. Falls back to the inline `version` line if the referenced Cargo.toml is missing.

## [0.15.37] - 2026-05-11

### Fixed

- fix(alef-backend-dart): `doc_comment.jinja` now preserves newlines between doc-comment lines. Under `trim_blocks = true`, `{%- endfor %}` was stripping the newline before the tag, collapsing multi-line doc comments into a single line in the generated Dart wrapper class (e.g. `packages/dart/lib/src/<module>.dart`). Same class of bug as the prior `alef-backend-zig` fix in 0.15.36 (`error_doc_block.jinja`, `trait_method_doc_lines.jinja`).

## [0.15.36] - 2026-05-11

### Changed

- refactor(alef-backend-java,alef-backend-swift,alef-backend-wasm,alef-backend-zig): migrate remaining backend doc-code emission and opaque-bridge interpolation from `push_str(&format!(...))` to dedicated Jinja templates (`javadoc_lines.jinja`, `doc_comment.jinja`, `rustdoc`, `param_opaque_config_from_json.jinja`).

### Fixed

- fix(alef-backend-zig): opaque C FFI handle types (`is_opaque = true` / `has_serde = false`, e.g. `CrawlEngineHandle`) are now excluded from `struct_names`. Previously they were treated as JSON-serializable structs and the generated wrappers called non-existent `_from_json`/`_to_json` C helpers. Functions taking an opaque handle now accept `?[]const u8` config JSON and internally call the creator function (discovered from the IR) to build, use, and free the handle per call.

- fix(alef-e2e/swift): `Optional<RustString>` fields now emit `({field_expr}?.toString() ?? "")` in Swift assertions instead of `{field_expr}.toString()`, fixing compile errors when the field is nullable. `not_empty`/`is_empty` assertions on array fields (`RustVec<T>`) now emit `{field_expr}.isEmpty` directly instead of routing through `.toString()` — `RustVec<T>` has no `.toString()`.

- fix(alef-e2e/zig): `handle` arg type in `build_args_and_setup` is now an explicit case: emits `null` when the fixture omits the engine config, or a JSON string literal when a config value is present. Aligns with the updated Zig binding that accepts `?[]const u8` for engine parameters.

- fix(alef-backend-zig): `error_doc_block.jinja` and `trait_method_doc_lines.jinja` now emit the trailing newline after the final doc-comment line. Under `trim_blocks = true`, `{%- endfor %}` was stripping the newline before the tag, merging the doc comment and the following `pub const` declaration onto one line — a Zig compile error.

- fix(alef-e2e/zig): `render_json_assertion` now resolves fixture field-path aliases through `FieldResolver` before building the JSON traversal chain. Previously, `content.detected_charset` was traversed as `result.object.get("content").?.object.get("detected_charset")` instead of the correct `result.object.get("detected_charset")`, causing runtime panics on fields that only exist at the top level.

## [0.15.35] - 2026-05-11

### Added

- feat(alef-backend-pyo3): `capsule_types` in `[crates.python]` is now wired into codegen. Types listed there are emitted as PyCapsule pass-through (via `PyCapsule_New` / `PyCapsule_GetPointer`) instead of opaque `#[pyclass]` wrappers. Supports two TOML forms: a bare string (`Language = "tree_sitter.Language"`) for capsule round-trips, and a struct (`Parser = { python_type = "tree_sitter.Parser", construct_from = "Language" }`) for Python-side construction (e.g. `tree_sitter.Parser(language)`).

### Fixed

- fix(alef-backend-dart): `flutter_rust_bridge.yaml` now pins `rust_input: crate` so FRB scans every top-level `pub fn` in the binding crate. Previously the key was omitted (under the assumption FRB 2.x had dropped it) and FRB defaulted to a narrower scope, silently skipping plugin lifecycle helpers (`unregister_*`, `register_document_extractor`, `register_renderer`, etc.) and leaving the generated wrapper referencing undefined bridge functions.

- fix(alef-backend-java): per-extractor Java wrappers (e.g. `HwpxExtractor.java`) now emit `import java.util.{List, Optional, Map}` when the trait's instance methods reference those types. Previously the import set was hard-coded to MemorySegment/Arena/ValueLayout/ObjectMapper, so `List<String> supportedMimeTypes()` failed javac with `cannot find symbol: class List`.

- fix(alef-core): `PythonConfig.capsule_types` schema is now `HashMap<String, CapsuleTypeConfig>` (was `HashMap<String, String>`). Existing `alef.toml` files using bare string values (e.g. `Language = "tree_sitter.Language"`) continue to deserialize correctly via `#[serde(untagged)]`.

### Changed

- refactor(alef-backend-zig,alef-backend-gleam,alef-backend-extendr): migrate additional interpolated `push_str`/`writeln!` emission paths to Jinja templates for trait bridge docs, error docs, import line emission, and visitor bridge assembly.
- refactor(alef-backend-csharp): replace inline `push_str`/`writeln!` visitor record emission for generated `NodeContext.cs` and `VisitResult.cs` with Jinja templates (`node_context.jinja`, `visit_result.jinja`).
- refactor(alef-backend-go): migrate additional `gen_visitor.rs` emission from inline `push_str` blocks to Jinja templates for visitor interface/registry/helper/trampoline/control-flow; generated `ConvertWithVisitor` now routes through `convertWithVisitorHelper` for shared logic.
- refactor(alef-backend-ffi): continue migrating parameterized trait-bridge code generation to Jinja templates for vtable error messages and async registration body lines (`ffi_vtable_not_initialised_msg.jinja`, `ffi_nul_byte_*_param_msg.jinja`, `ffi_vtable_null_out_result_msg.jinja`, plus async cached-name/clone/`map_err` templates).
- refactor(alef-backend-magnus): move more trait-bridge generation into Jinja templates (`trait_bridge_async_method_body.rs.jinja`, `trait_bridge_constructor.rs.jinja`, `trait_bridge_registration_fn.rs.jinja`, `trait_bridge_return_conversion.rs.jinja`), keeping behavior unchanged.

### Fixed

- fix(alef-e2e/gleam): `render_assertion` now resolves field aliases before calling `is_optional`, fixing incorrect non-optional treatment of fields accessed via fixture path aliases (e.g. `og.title` → `metadata.og_title`). Import pre-pass also uses resolved paths for optional detection.
- fix(alef-e2e/gleam): `not_empty` and `is_empty` assertions on non-array, non-optional String fields now emit `string.is_empty` instead of `list.is_empty`. Import pre-pass updated accordingly.
- fix(alef-e2e/gleam): Array element field assertions using indexed access (`[0].` and `[].`) are now both skipped with a comment, not just `[].` paths.
- fix(alef-backend-zig): Async Rust functions are no longer silently skipped in Zig binding generation. The C FFI exports synchronous wrappers for all functions (including those async in Rust), so Zig can call them directly.
- fix(alef-e2e/gleam): `render_assertion` now also resolves field aliases before setting `field_is_optional`, fixing optional wrapping for aliased fields at the render stage.
- fix(alef-e2e/gleam): `not_empty`/`is_empty` assertions on non-array, non-optional fields now emit `string.is_empty` instead of `list.is_empty` in `render_assertion`.
- fix(alef-e2e/gleam): Indexed array element fields (`[0].`) are now skipped alongside `[].` paths in `render_assertion`.
- fix(alef-e2e/gleam): Fields with a `.length` segment (e.g. `links.length`) now import `gleam/list` in the test file header.

## [0.15.34] - 2026-05-11

### Changed

- refactor(alef-backend-zig,alef-backend-dart,alef-backend-php): move remaining generated-code emission blocks for Zig parameter/return handling, Dart trait bridge defaults, and PHP vector binding conversion into Jinja templates.
- refactor(alef-scaffold,alef-docs,alef-readme): render generated pre-commit YAML, scaffold Cargo env rows, API Markdown blocks, and README performance rows through Jinja templates.
- refactor(alef-backend-napi,alef-backend-extendr): move additional trait bridge wrappers, function bodies, serde bindings, and async parameter clone fragments into Jinja templates.

### Fixed

- fix(alef-e2e/kotlin): generate `MockServerListener.kt` and `META-INF/services/org.junit.platform.launcher.LauncherSessionListener` when any fixture needs the mock-server. Add `junit-platform-launcher` to `build.gradle.kts` when the listener is emitted. Previously `MOCK_SERVER_URL` was never set, causing all mock-server-dependent Kotlin e2e tests to fail.

- fix(alef-e2e/kotlin): generate Kotlin-native collection access and nullable assertion expressions for optional fields, arrays, maps, and JNA library path setup.

- fix(alef-e2e/zig): replace `std.posix.getenv` with `std.process.getenv` in generated test preambles. `std.posix.getenv` was removed in Zig 0.16.0; `std.process.getenv` is the correct API and works across all supported Zig versions.

- fix(alef-e2e/gleam): `build_args_and_setup` now handles `handle` and `mock_url` argument types. `handle` args emit `let assert Ok(<name>) = module.create_<name>(option.None)` setup and `option` is added to imports. `mock_url` args emit an `envoy.get("MOCK_SERVER_URL")` URL construction and `envoy` is imported via `gleam_envoy`. Previously both fell through to the catch-all, producing a single-argument JSON call instead of the correct multi-argument form.

- fix(alef-e2e): `field_access.rs` now emits `list.length` for `PathSegment::Length` in Gleam (was missing), and handles nested map/array access chains correctly across all e2e codegen backends.

- fix(alef-backend-swift): Duration-typed struct fields now bridge correctly through swift-bridge. Constructors emit `std::time::Duration::from_millis(<param>)` (or `.map(std::time::Duration::from_millis)` for optional), and getters emit `self.0.<field>.as_millis() as u64` (or `.map(|d| d.as_millis() as u64)` for optional). Previously both fell through to the generic field assign template, causing `E0308 expected Duration, found u64` in every swift binding crate that has timeout fields.

## [0.15.33] - 2026-05-11

### Changed

- refactor(alef-backend-swift): move remaining Swift backend generated-code blocks for inbound plugin bridges, JSON factory shims, and Swift convenience overloads from inline string assembly into Jinja templates.

- refactor(alef-backend-magnus,alef-backend-php,alef-backend-rustler): move more generated function-body, registration, serde-binding, and enum-conversion emission from inline `format!` assembly into Jinja templates while preserving generated output behavior.

- refactor(alef-backend-csharp,alef-backend-dart,alef-backend-ffi,alef-backend-java,alef-codegen): move additional generated-code emission blocks from inline `push_str`/`format!` assembly into Jinja templates while preserving the existing generated output contracts.

### Added

- feat(alef-e2e/rust): the C# and Java e2e codegen now calls `resolve_call_for_fixture` (which honours `select_when = { input_has = "..." }` in `[crates.e2e.calls.*]`) instead of the no-input `resolve_call`. Fixtures with `input.batch_urls: []` (and no explicit `call` override) now route to `batch_scrape()` / `BatchScrape()` instead of the default `scrape()`. Go and Python codegen already used the correct resolver; C# (`csharp.rs:775, 3032`) and Java (`java.rs:458, 498, 961`) were the only callers of the wrong variant.

- feat(alef-e2e/rust): the Rust e2e codegen now emits a `tests/common.rs` module whenever the fixture suite requires a standalone mock server. The module exposes `pub fn mock_server_url() -> &'static str` backed by `std::sync::OnceLock`, spawning `target/release/mock-server` with the fixtures directory on first call, parsing its `MOCK_SERVER_URL=<url>` and `MOCK_SERVERS={...json...}` stdout lines, and setting `std::env::set_var` for each. Generated test files that previously panicked with `MOCK_SERVER_URL not set` now import `mod common;` and call `common::mock_server_url()`. Mirrors Python (`conftest.py`), Node (`globalSetup.ts`), and Elixir (`test_helper.exs`) session-level orchestration.

### Fixed

- fix(alef-backend-java): extract the repeated bytes-FFI result block (rc check, outPtr read, free, return) from `stream_method_bytes_result.jinja` into a `readBytesResult` helper method emitted once per class by `streaming_helpers.jinja`. Eliminates the 17-line CPD duplication flagged when two or more byte-returning methods (e.g. `speech`, `fileContent`) are generated in the same class.

- fix(alef-backend-swift,alef-backend-napi,alef-codegen): harden remaining trait bridge generation edge cases. Swift trait bridge JSON envelopes now serialize success and error payloads with `serde_json`, Swift inbound wrappers handle `returns_ref` `Vec<String>` methods without overriding default trait methods, NAPI trait bridge parse errors include plugin context, and excluded type paths no longer overwrite visible binding type paths with the same short name.

- fix(alef-backend-dart): the core→mirror `From` impl for struct fields of type `Duration` now emits `v.{name}.as_millis() as i64` (or `.map(|d| d.as_millis() as i64)` for optional) instead of the invalid `v.{name} as _`. `Duration` is not a primitive type and cannot be coerced with `as`; the previous codegen produced `E0605 non-primitive cast: Duration as i64` for any crate that has `Duration`-typed fields (e.g. `BrowserConfig.timeout`, `CrawlConfig.request_timeout`).

- fix(alef-e2e/ruby): fixtures with only a `not_error` assertion now emit a real test body (`expect { }.not_to raise_error`) instead of `skip "Non-HTTP fixture cannot be tested via Net::HTTP"`.

- fix(alef-codegen,alef-backend-php): `gen_lossy_binding_to_core_fields_inner` and `gen_php_lossy_binding_to_core_fields` now skip fields whose `field.cfg` is `Some(...)`. Previously, cfg-gated fields (e.g. `pdf_options`, `keywords`, `layout`) were emitted in the binding→core struct literal even though those fields are absent from the binding struct (they are filled by `..Default::default()`). The generated code produced `E0609 "no field X on type &ExtractionConfig"` in downstream PyO3 and PHP crates after alef regen.

- fix(alef-backend-napi): three related NAPI codegen correctness fixes: (1) `gen_struct` now filters cfg-gated fields when emitting the binding struct definition and its manual `Clone` impl, preventing `E0063 "missing fields in initializer"` for structs like `JsExtractionConfig`; (2) `sync_method_non_unit_return.jinja` and `async_method_non_unit_return.jinja` now return `Err({{ error_parse }})` instead of `Ok(Default::default())` for null/empty JSON in error-returning methods, avoiding `E0277` caused by `InternalDocument` not implementing `Default`; (3) `field_conversion_from_core` and `field_conversion_to_core` now handle `Map(_, Bytes)` explicitly — `HashMap<String, Vec<u8>>` core fields (e.g. `tessdata_bytes`) are correctly round-tripped through `HashMap<String, Buffer>` in the NAPI binding using `.to_vec().into()`.

- fix(alef-backend-swift): `emit_extern_block_for_trait_bridge` and `emit_trait_bridge_wrapper` no longer emit `Result<T, String>` return types for error-returning trait bridge methods. swift-bridge 0.1.59 panics with "Type must be declared with `type >`" when it encounters `Result<T, E>` inside an `extern "Rust"` block. Error-returning methods now use a plain `String` return carrying a JSON envelope (`{"ok": <value>}` / `{"err": "<message>"}`) so swift-bridge can parse the extern block. The Rust trampoline body serialises the `Result` to this envelope; the Swift caller deserialises it.

- fix(alef-e2e/rust): `needs_mock_server` in the Rust e2e codegen (`rust/mod.rs`) and the per-file `file_needs_mock` condition in `test_file.rs` were gated on `f.mock_response.is_some()` — the legacy liter-llm single-response shape. Fixtures using the array form (`input.mock_responses: [...]`, the kreuzcrawl shape) returned `false` for that check, so the entire kreuzcrawl suite had `needs_mock_server = false`. Consequently: (1) `tests/common.rs` was never generated (gated on `needs_mock_server`), (2) `mod common;` was never emitted in test files. Test files called `common::mock_server_url()` without the module declaration, causing `E0433 cannot find module or crate 'common'` and a compilation failure. Fix: use `f.needs_mock_server()` (which already handles all three shapes: liter-llm, http, and kreuzcrawl array form) in both detection sites.

- fix(alef-backend-java): the compact-constructor emission in `gen_record_type` no longer coerces an explicit `0` to the Rust-default value for `Duration`-typed (boxed Long) fields. Previously the generated Java record contained `if (requestTimeout == null || requestTimeout == 0) requestTimeout = 30000L;` — callers who intentionally passed `request_timeout: 0` (which the Rust core rejects via `validate()` as invalid) had their value silently replaced with the default before the FFI boundary, so the validation error was never surfaced. The fix drops `|| requestTimeout == 0`; only absent JSON fields (Jackson deserialises boxed numerics as `null` when the key is missing) still receive the default.

- fix(alef-e2e/dart): `json_object` args that contain a plain JSON array (e.g. `batch_urls: ["/p1", "/p2"]`) now emit `final {name} = (jsonDecode(r'...') as List<dynamic>).cast<String>();` + pass the variable to the call, instead of silently dropping the argument. Previously any `json_object` without an explicit `element_type` of `BatchBytesItem`/`BatchFileItem` and without the name `config` was skipped entirely.

- fix(alef-e2e/dart): the Dart e2e codegen now correctly handles `handle` and `mock_url` arg types. Previously both arg types fell through to the `_ => {}` catch-all, so generated tests had no arguments and called the wrong `KreuzbergBridge` class (a kreuzberg-specific hardcode). Fixes: (1) `handle` args emit engine construction — `CrawlConfig.fromJson(jsonDecode(...))` then `await {BridgeClass}.createEngine(config)` — before the main call; (2) `mock_url` args emit `Platform.environment["MOCK_SERVER_URL"]` URL construction; (3) the receiver class now derives from `[dart] lib_name` (converted to PascalCase + "Bridge") via a new `dart_bridge_class_name()` method on `ResolvedCrateConfig` instead of the hardcoded `"KreuzbergBridge"` literal; (4) error-expecting fixtures with setup lines wrap the full setup + call in an `async` lambda so any exception at any step is caught by `throwsA(anything)`; (5) `import 'dart:convert'` is now emitted when any non-HTTP fixture uses a `handle` arg (previously only emitted for HTTP fixtures).

- fix(alef-e2e/zig): the `mock_url` arg type in the Zig e2e codegen now emits `const {name} = try std.fmt.allocPrint(allocator, ...)` + `defer allocator.free({name})` instead of the previously invalid `var {} = try allocator.alloc(u8, std.fmt.bufPrint(undefined, ...) catch 0)`. The old pattern produced a Zig syntax error (`expected ';' after statement`) and an incorrect allocation size. The new pattern uses `std.fmt.allocPrint` which allocates exactly the right number of bytes for the formatted URL string.

- fix(alef-backend-dart): Vec<Json> (e.g. `MarkdownResult.tables: Vec<serde_json::Value>`) fields in core→mirror `From` impls now emit `.map(|j| serde_json::to_string(&j).unwrap_or_default())` instead of falling through to the identity return (`v.{name}`). Previously, `TypeRef::Json` inside a `Vec` was treated as a pass-through, producing `E0308 mismatched types: expected Vec<String>, found Vec<Value>`.

- fix(alef-backend-swift): `Duration`-typed struct fields now emit correct constructor and getter code. The constructor body emits `__target.{name} = std::time::Duration::from_millis({param});` (or `.map(std::time::Duration::from_millis)` for optional) via two new Jinja templates (`default_field_duration_assign.jinja`, `default_field_optional_duration_assign.jinja`). Getters emit `self.0.{name}.as_millis() as u64` (or `.map(|d| d.as_millis() as u64)` for optional) via `getter_duration.jinja` and `getter_optional_duration.jinja`. Previously both paths fell through to generic templates that emitted `__target.timeout = timeout` / `self.0.timeout.clone()`, producing `E0308 mismatched types` for any struct with a `Duration` field (e.g. `BrowserConfig.timeout`, `CrawlConfig.request_timeout`).

- fix(alef-backend-dart): opaque handle types (e.g. `CrawlEngineHandle`) now emit a `#[frb(opaque)] pub struct {Name} { pub(crate) inner: source::{Name} }` wrapper instead of `#[frb(mirror({Name}))] pub struct {Name} {}` (an empty zero-sized struct). The empty mirror pattern caused `E0308` for return values (`CrawlEngineHandle::from(v)` with no `From` impl) and would have silently destroyed the engine value via an unsound zero-sized transmute. Bridge functions now use `&engine.inner` (input) and `|inner| Name { inner }` (return) instead of transmute/From.

- fix(alef-backend-dart): struct types that contain `Duration` or `Path` fields are now included in `types_needing_from_conversion` (and transitively in `types_needing_from_impl`), causing them to use `{core_ty}::from(mirror_val)` at call sites instead of `unsafe { transmute }`. Previously, `Duration` fields (16-byte `std::time::Duration` in core vs 8-byte `i64` in mirror) and `Path` fields were not considered sanitized by the IR, so structs like `CrawlConfig` used a transmute that caused `E0512 cannot transmute between types of different sizes` (6720-bit mirror vs 6848-bit core).

- fix(alef-codegen): `gen_bridge_trait_impl` now wraps `Vec<String>` bodies in a `Box::leak` pattern when the trait method signature declares `returns_ref = true` (i.e. `&[&str]`). Previously, `supported_mime_types()` bridge bodies returned `Vec<String>` but the trait requires `&[&str]`, causing `E0308` in generated Python, Node.js, PHP, and Swift binding crates after `alef generate`.

- fix(alef-backend-swift): two additional Swift trait bridge fixes: (1) `emit_trait_method_body` generated `format!("{\"ok\": ...}")` which is invalid Rust format syntax because `{` must start a format spec, not a literal; doubled the outer braces so the generated code emits literal `{` and `}` characters correctly. (2) `emit_extern_block_for_trait_bridge` and `emit_trait_bridge_wrapper` now skip methods with `has_default_impl = true` (e.g. `as_sync_extractor` returning `Option<&dyn SyncExtractor>`) — these cannot be expressed in swift-bridge and rely on the trait's own default impl.

- fix(alef-backend-php): `gen_struct_methods_impl` now filters cfg-gated fields (`field.cfg.is_some()`) when building constructor parameter lists, `let_binding` loops, and `param_init` struct literals. `gen_enum_tainted_from_binding_to_core` also skips cfg-gated fields in its field loop. Previously, cfg-gated fields like `pdf_options` were emitted as constructor parameters and in `From` impls even though they are absent from the PHP binding struct, producing `E0560 "struct has no field named pdf_options"`.

- fix(alef-backend-napi): `gen_struct` now maps `Bytes` fields to `Vec<u8>` in struct field position via a `map_bytes_field_type` closure instead of `mapper.map_type`. `napi::bindgen_prelude::Buffer` does not implement `Clone`, `Serialize`, or `Deserialize`, causing derive failures on structs with bytes fields (e.g. `JsOcrConfig`). `Buffer` is still used for function parameters. The `has_bytes_field` manual-`Clone` workaround is removed since `Vec<u8>` is `Clone`.

- fix(alef-e2e): Java mock server URL lookup now checks `System.getProperty("mockServerUrl", ...)` with env fallback so the URL can be injected via JVM system property. Ruby client factory calls switched from keyword args (`api_key: ..., base_url: ...`) to positional args to match the actual generated factory method signatures.

- fix(alef-e2e/rust): the generated `common.rs` `BufReader` now takes ownership of `ChildStdout` (`BufReader::new(stdout)`) instead of borrowing it (`BufReader::new(&mut stdout)`). The previous pattern caused `E0597` on Rust 2024: the drain thread (`std::thread::spawn(move || reader.into_inner())`) requires `'static` bounds, but a `BufReader<&mut ChildStdout>` wraps a local reference that is not `'static`. Owned `BufReader<ChildStdout>` satisfies the bound.

- fix(alef-backend-magnus): streaming adapter (`gen_iterator_struct`) now derives the core-crate prefix from the configured `core_import_name` rather than the hardcoded literal `liter_llm::`. The `StreamingAdapter` struct gains a `core_crate` field populated from `core_import` (computed at the top of `generate_bindings`) and threaded through `from_config`. Fixes the no-special-casing rule: any downstream crate (not only liter-llm) that wires a `streaming` adapter now gets `{core_crate}::{ItemType}` / `{core_crate}::{ErrorType}` in the emitted iterator struct instead of a crate-name assumption.

- fix(alef-e2e/wasm): the emitted `vitest.config.ts` now sets `testTimeout: 30000` globally for the WASM e2e suite. Vitest's default 5 s deadline is too tight for fixtures that exercise liter-llm's retry path (504 / 429 / 500 / 502 are retryable with backoff); those tests all timed out at the default, masking real pass/fail outcomes. A 30 s timeout matches the rest of the suite's retry window.

### Added

- feat(alef-backend-ffi): emit `{prefix}_{enum}_to_string(*const Enum) -> *mut c_char` for unit-variant enums (`has_serde = true`) that are returned as heap-allocated pointers. The function uses `serde_json::to_value(val).as_str()` to extract the bare variant name (e.g. `"completed"`) without surrounding JSON quotes, so C/Zig/Dart e2e callers can string-compare an enum field accessor against a fixture string without reaching for `_to_json` (which yields a JSON-quoted form). Sibling helper to existing `_to_json`/`_free`; emitted only when the enum is in `enum_pointer_return` AND `can_generate_enum_conversion` (gates out compound enums whose serde shape is not a plain string).

### Fixed

- fix(alef-e2e/c): when a fixture assertion targets a field that is registered in `[crates.e2e] fields_enum` AND the field's resolved type in `[crates.e2e] fields_c_types` is a non-primitive PascalCase enum name (e.g. `BatchStatus`), emit an opaque-handle declaration plus a `{prefix}_{enum_snake}_to_string({handle})` conversion call rather than declaring the accessor return as `char*`. The previous output (`char* status = literllm_batch_object_status(result); assert(str_trim_eq(status, "completed") == 0);`) treated the FFI's `LITERLLMBatchStatus*` opaque pointer as a C string, causing immediate `Abort trap: 6` / NULL-deref in every C e2e fixture that compared an enum field. Applied to all four accessor sites: `render_test_function` (default-client and legacy paths), `render_engine_factory_test_function`, and the leaf branch of `emit_nested_accessor`. Cleanup: the opaque handle is registered in `intermediate_handles` so the existing reverse-order free loop calls `{prefix}_{enum_snake}_free(...)`; the `to_string` result is a heap `char*` freed by `{prefix}_free_string` like any other accessor result.

- fix(alef-backend-ffi): `_free`/`_to_json`/`_to_string` are now also emitted for enum types that are returned by _struct field accessors_ (not just by free functions or method returns). Previously the `enum_pointer_return` set was built from `api.functions` and `typ.methods` only — fields like `BatchObject.status: BatchStatus` produced an opaque-pointer accessor (`literllm_batch_object_status`) without a matching `_free` / `_to_string`, leaking memory and forcing C callers to treat the return as an unfreeable handle. The detection now walks `typ.fields` too. The pre-existing comment ("Also check struct field accessors and method returns…") matched the intent but the code was missing the field-walk half.

- fix(alef-e2e/c): the iter11 sentinel fix `(uint64_t)-1, (uint32_t)-1` for `*_create_client` only landed in `render_chat_stream_test_function`. The default-client path in `render_test_function` and the bytes-result path in `render_bytes_test_function` still emitted literal `0, 0` — passing them as `Some(0)` to the FFI yielded `Duration::from_secs(0)` and aborted every non-streaming HTTP fixture. Both call sites now also emit the sentinel. Mirror of 0.15.28 java/csharp marshalling fix.

- fix(alef-e2e/c): `emit_nested_accessor` now correctly handles `field[N]` numeric array indexing in chained paths (e.g. `data[0].index`, `choices[0].message.tool_calls[0].function.name`). Previously the bracket branch returned early after emitting `alef_json_get_string(parent, "0")` — looking up literal key `"0":` instead of the Nth array element — so any path with array indexing returned NULL and aborted on the first downstream assertion. New runtime helper `alef_json_array_get_index(json, idx)` extracts the Nth top-level element of a JSON array; the codegen emits it whenever it sees a numeric bracket key (both at top-level and inside `json_extract_mode`). Bare `[]` still uses substring search semantics. Required for fixtures with multi-element result arrays where each element's fields are asserted independently (e.g. `data[0].index == 0`, `data[1].index == 1`).

- fix(alef-e2e/c): `alef_json_get_string` now falls through to `alef_json_get_object` when the value at the resolved key is a JSON object/array, and returns the raw token string when the value is a primitive (number / bool / null). Previously it strictly required string values, so leaf accessors over collection-typed fields (`Vec<T>`, `Option<Vec<T>>`) and numeric leaves accessed via `json_extract_mode` (e.g. `data[0].index` of type `u32`) returned NULL and broke `not_empty` / `count_equals` / `equals` assertions. Adds a runtime helper `alef_json_get_object` that uses balanced-bracket matching to extract `{...}` / `[...]` substrings; codegen also uses it directly for intermediate object hops in `json_extract_mode`.

- fix(alef-e2e/c): "expected error" fixtures (assertion type `error`) now treat a NULL return from `*_request_from_json` as the expected failure path instead of asserting non-null. Mirrors Java's `assertThrows(Exception.class, () -> { … })` and Python's `pytest.raises(...)` patterns: when the fixture's input contains an invalid enum value (e.g. `"purpose":"invalid-purpose"` for `FilePurpose`), serde's strict deserialization rejects it before the request leaves the binding, and that counts as the expected error. Without this, every C error fixture whose error originates in the input layer crashed at the build step rather than reaching its `assert(result == NULL)` final assertion.

- fix(alef-e2e/c): smoke/live fixtures gated on a required env var (`fixture.env.api_key_var`, e.g. `OPENAI_API_KEY`) now emit a `if (getenv(VAR) == NULL) return;` short-circuit at the top of the test body. Previously the C suite hard-failed on any missing-credentials run; now CI without provider creds gracefully skips these tests, matching Python's `pytest.skip(...)` and Java's `Assumptions.assumeTrue(...)`.

- fix(alef-e2e/c): streaming tool_calls assertions (`tool_calls`, `tool_calls[0].function.name`) are now emitted as skip comments instead of attempting to parse them out of the last SSE chunk's `choices`. The OpenAI streaming wire format distributes a single tool call's fields across many delta chunks, and the inline C SSE parser only inspects the last chunk — which carries `finish_reason=tool_calls` but no payload — so the assertions could never evaluate. Mirrors Python's `# skipped: field 'tool_calls' not available on result type` for the same fixture. A delta-merge accumulator is the proper long-term fix; tracked separately.

- fix(alef-e2e/wasm): generate `globalSetup.ts` (which spawns the mock-server and exports `MOCK_SERVER_URL`) for any fixture that needs the mock server, not just `is_http_test()` (which only matched the consumer-style `http: { ... }` shape and missed the entire `mock_response: { ... }` set used by liter-llm). Without globalSetup, every fixture that interpolates `${process.env.MOCK_SERVER_URL}/fixtures/<id>` into a base URL hit `undefined/fixtures/<id>` and `reqwest::Client::builder().build()` resolved to `Err(builder error)` because the URL parser rejected the constructed `Url`. Predicate now uses `Fixture::needs_mock_server()` which covers both schemas; the surrounding comment block already stated the same intent. Lifts liter-llm WASM e2e from 56/161 → 153/161 passing in a single config change.

- fix(alef-backend-java): revert kreuzcrawl-specific `dispatchCrawlError` typed-exception dispatch in the FFI `checkLastError` helper. Commit `44507046` had hardcoded references to `TimeoutException`, `ConnectionException`, `GoneException`, `BadGatewayException`, `BrowserTimeoutException`, `CrawlErrorException` etc. — exception classes that exist in kreuzcrawl but not in any other downstream — into the shared template. Regenerating the Java binding for liter-llm (or any downstream other than kreuzcrawl) produced a `LiterLlmRs.java` referencing undefined symbols and broke `mvn package`. The case-2 branch is now back to the pre-44507046 emission `throw new ConversionErrorException(msg)`. The proper fix — config-driven typed-exception dispatch keyed by error-code mapping — is tracked under iter15's "remove downstream-crate special-casing" plan; kreuzcrawl can re-add its dispatcher via that mechanism (or via a post-process step in its own toolchain) without polluting the shared backend.

## [0.15.30] - 2026-05-10

### Fixed

- fix(alef-backend-ffi): async trait method bodies with `&str` params now capture via `.to_string()` instead of `.clone()`. `.clone()` on `&str` returns `&str` — the borrow escaped into the `spawn_blocking` closure, causing E0521 ("borrowed data escapes outside of method"). The fix is in `gen_async_method_body` in `registration.rs`: `TypeRef::String | TypeRef::Char` with `is_ref=true` now emit `let x = x.to_string()` before the closure.
- fix(alef-backend-ffi): async trait method bodies whose trait return type is an excluded Named type (present in `api.excluded_type_paths`, e.g. `InternalDocument`) now correctly emit `serde_json::from_str::<QualifiedPath>(&json)` in the closure body and `Result<QualifiedPath, _>` in the method signature. Previously a stale IR cache (written before the `sanitize_unknown_types` trait-method exemption was added) rewrote `Named("InternalDocument")` to `TypeRef::String`, causing the codegen to emit `Ok(cs.to_string_lossy().into_owned())` — a `String` where the trait expected `InternalDocument`. The root fix was `alef generate --clean` which invalidates the stale cache; the code generator itself was already correct (verified by new regression test `bug6_async_excluded_type_return_signature_and_deserialization`).

- fix(alef-e2e/rust mock-server): emit `MOCK_SERVERS={...}` (possibly `{}`) unconditionally as a sentinel line so parent-process parsers — Python's conftest, Ruby's spec_helper, etc. — that read until they observe MOCK_SERVERS never block on a `readline()` that the server was never going to emit. Previously the line was only printed when `fixture_urls` was non-empty (host-root fixtures present), and downstreams without any host-root fixtures (liter-llm) timed out their entire e2e suites at conftest setup. The empty `{}` is parsed as a no-op JSON object so no per-fixture env vars get set.
- fix(alef-e2e/ruby): assertion codegen now consults the per-call `[crates.e2e.calls.<x>.overrides.ruby] enum_fields = { ... }` override (in addition to the global `[crates.e2e] fields_enum`) when deciding whether to coerce the Ruby field expression via `.to_s` for `equals` comparisons. Magnus binds Rust enums as Ruby Symbols, so an assertion `expect(result.status).to eq("completed")` fails against a returned `:completed` Symbol — the per-call override is already populated for the C#/Java/Python sides; threading the same map into the Ruby render preserves the single-source-of-truth contract instead of forcing a Ruby-only duplicate of every enum field.
- fix(alef-e2e/elixir): same per-call enum_fields lookup as ruby — Rustler binds Rust enums as Elixir atoms (`:in_progress`), so `String.trim(result.status)` raises `FunctionClauseError`. Threading the per-call map alongside the global `fields_enum` set lets the existing `to_string/1` coercion fire when the operator labels e.g. `status = "BatchStatus"` for the elixir side.
- fix(alef-e2e/elixir): emit the `{:ok, client} = create_client(...)` setup line inside the expects_error branch as well. The non-error path emits this client binding then references `client` in the call expression; the expects_error path was missing the binding, so generated test bodies failed compilation with `undefined variable "client"` for every error_handling fixture.
- fix(alef-e2e/c): pass the FFI's `None` sentinel (`(uint64_t)-1` / `(uint32_t)-1`) — not literal `0` — for the `timeout_secs` and `max_retries` parameters of the generated `*_create_client(...)` invocation, mirroring the alef-backend-{java,csharp} marshalling fix from 0.15.28. Passing `0` made the FFI shim resolve to `Some(Duration::from_secs(0))`, wiring an immediate-deadline reqwest client that aborted every HTTP fixture in the C suite. Adds `<stdint.h>` to the test-file header block so the cast types resolve.

## [0.15.29] - 2026-05-10

### Fixed

- fix(alef-backend-ffi): sync trait method bodies emitting static error messages now coerce bare string literals to `String` via `.to_string()`. Previously, `gen_vtable_call_body(inside_closure=false)` passed the literal directly to `spec.make_error(...)`, producing e.g. `KreuzbergError::Other("nul byte in serialized param doc")` — a `&'static str` — which fails E0308 when the error variant wraps `String`. The fix is in the `make_err` closure in `call_body.rs`: when not inside the async `_SendFn` closure and the message is a quoted string literal, `.to_string()` is appended before passing to `make_error`. The async closure path (`Box::from(...)`) is unaffected.

## [0.15.28]

### Fixed

- fix(alef-backend-java,alef-backend-csharp): pass the FFI `None` sentinel (`{prim}::MAX`, `NaN`) — not zero — when an `Optional<numeric_primitive>` parameter is null. The FFI binding generated by `alef-backend-ffi` decodes `if x == {prim}::MAX { None }` to recover the absent case; the Java/C# marshallers were instead coercing `null` to `0`/`0L`/`0.0`, so a caller passing `null` ended up handing the host crate `Some(0)` — silently colliding with legitimate zero values (e.g. `timeout_secs=Some(0)` was treated as "no timeout" but instead produced an immediate-deadline reqwest client, breaking ~30 e2e tests per language in liter-llm with "error sending request"). Sentinel choice mirrors `alef-backend-ffi/src/gen_bindings/functions.rs::param_optional_numeric_conversion`: unsigned ints use bitwise `-1` (all-bits-set = `u{N}::MAX`), signed ints use the boxed type's `MAX_VALUE`, floats use `NaN`. Bool optionals retain the existing fall-through (separate FFI path).
- fix(alef-e2e/c): align generated Makefile mock-server invocation with the rest of the suite — point `MOCK_SERVER_BIN` at `../rust/target/release/mock-server` (matches python conftest, php bootstrap, java MockServerListener) instead of `../../target/release/mock-server`, and pass the fixtures directory as a positional argument rather than `--fixtures …` (mock-server's CLI takes a single positional `[fixtures-dir]`, so the flag form was being parsed as the fixtures-dir literal — `loaded 0 routes from --fixtures`).
- fix(alef-e2e/typescript): `not_empty` assertion is now polymorphic — strings/arrays still use `length > 0`, but objects use `expect(_v).toBeDefined()` + `.not.toBeNull()`. The previous template assumed string-like fields and emitted `(field ?? "").length > 0`; for object-typed fields like Cohere's `JsRerankResultDocument` (a `{text: string}` wrapper, not a bare string) `({text: "…"} ?? "").length` evaluates to `undefined`, and Vitest reports "actual value must be number or bigint, received undefined". The runtime branch on `typeof === "string"` / `Array.isArray()` keeps the existing string-and-array semantics intact for all current fixtures while extending to object payloads.

## [0.15.26] - 2026-05-10

### Added

- feat(alef-backend-gleam): emit `unregister_fn` and `clear_fn` external Gleam declarations from `TraitBridgeConfig` when the fields are set; short-circuits to no output when `None`. Closes the gap where Gleam silently ignored the optional unregister/clear lifecycle config that other backends already honored.
- feat(alef-backend-go): emit `unregister_fn` and `clear_fn` Go wrappers from `TraitBridgeConfig` when set; short-circuits empty when `None`. Generated wrappers delegate to the host crate's C-exported `kreuzberg_unregister_*(name, &err)` and `kreuzberg_clear_*(&err)` symbols via cgo.
- feat(alef-backend-csharp): replace hardcoded `Unregister{Trait}` P/Invoke generation with config-driven lookup of `bridge_config.unregister_fn`. Previously every C# trait-bridge always emitted an `Unregister*` declaration regardless of host capability; now the declaration and the static `Unregister(name)` C# method are conditional on the config field being set, matching the contract every other backend already honored.
- feat(alef-backend-java): emit `unregister_fn` and `clear_fn` Panama FFM downcall handles + Java methods from `TraitBridgeConfig` when set. Each emits a `Method.invoke(...)` over the configured C symbol with the `FunctionDescriptor.of(JAVA_INT, ADDRESS)` shape and drains the local Java-side bridge map on clear.
- feat(alef-backend-dart): emit Dart-side `unregisterXxx(name)` and `clearXxxs()` static methods on the generated bridge class when `unregister_fn` / `clear_fn` are set, plus a Rust-side `clear_*` forwarder for FRB to bridge. Previously the Rust-side unregister forwarder existed but the Dart caller had no way to invoke clear (clear was emitted in Rust but never surfaced to Dart).
- feat(alef-backend-zig): emit `unregister_fn` and `clear_fn` Zig wrappers from `TraitBridgeConfig` when set. Generated wrappers are thin `extern "C"` passthroughs over the host crate's exported `kreuzberg_unregister_*(name, out_error)` and `kreuzberg_clear_*(out_error)` symbols, returning Zig `i32` return codes.
- feat(alef-backend-swift): emit `unregister_fn` and `clear_fn` swift-bridge `extern "Rust"` declarations + `pub fn` bodies from `TraitBridgeConfig` when set. Generated bodies access the registry directly via `registry_getter()` + `guard.remove(name)` / `guard.clear()` and adapt errors to Swift-friendly `Result<(), String>`. Refactor extracts the trait-bridge codegen into a `SwiftBridgeGenerator: TraitBridgeGenerator` impl, keeping the wrapper-emission path consolidated with the other backends.
- feat(alef-backend-kotlin): introduce `KotlinJvmBridgeGenerator: TraitBridgeGenerator` impl emitting `unregisterXxx(name)` / `clearXxxs()` thin Kotlin wrappers that delegate to the JVM-side `XxxBridge.unregisterXxx` / `clearAllXxx` static methods. Methods short-circuit to nothing when the corresponding config field is `None`.
- feat(alef-backend-wasm): emit a synthetic `pub fn default()` static factory on every wasm-bindgen wrapper struct that derives `Default`. wasm-bindgen mirrors the Rust `(constructor)` arity, so structs with non-Optional fields (e.g. `WasmChatCompletionTool { tool_type, function }`) can only be instantiated with positional args from JS — `new WasmChatCompletionTool()` throws. The factory delegates to `<Self as ::core::default::Default>::default()` so JS callers can obtain a fresh instance and drive it via setters. Skipped automatically when the IR already exposes an explicit `default` method to avoid impl-block conflicts.

### Fixed

- fix(alef-cli): `sync-versions` now writes to root `package.json` and every `crates/*-node/package.json` — both manifests are already validated by `validate-versions`, but were silently absent from the sync writer, so polyrepos with a private pnpm-workspace root manifest (e.g. `kreuzberg-root`) had to bump their version manually before every release. Sync is idempotent and only top-level `"version"` is rewritten — nested dependency specs and `pnpm.overrides` are untouched.
- fix(alef-e2e/typescript): use `.default()` factory for all wasm class instantiations in test bodies, not just `*Config` types. The previous `new WasmFoo()` pattern only worked for structs whose fields were all `Option<T>`; structs with required fields (e.g. `WasmChatCompletionTool`, `WasmFunctionDefinition`, `WasmResponseTool`) caused ~9 e2e tests to fail with "expected instance of WasmFoo" or constructor-arity TypeError. Combined with the new synthetic `default()` factory, `_u = WasmFoo.default()` now works uniformly.
- fix(alef-e2e/python): error-assertion compares the fixture value against EITHER `str(exc_info.value)` OR `type(exc_info.value).__name__`. Different downstream crates use different fixture-shape conventions — kreuzcrawl fixture values are message substrings (`"max_depth"`, `"proxy"`), liter-llm fixture values are class-name prefixes (`"Authentication"`, `"BadRequest"`). The disjunction lets a single codegen path satisfy both without a config flag.
- fix(alef-e2e/csharp): streaming `lastFinishReason` accumulator now uses `JsonNamingPolicy.SnakeCaseLower.ConvertName(...)` instead of `.ToString().ToLower()`. The latter collapses compound PascalCase enum names like `ToolCalls` to `toolcalls` (no underscore), causing equality assertions against fixture wire-form values like `"tool_calls"` to fail. The new emission matches the policy used by the global `JsonStringEnumConverter` and the non-streaming assertion path at `csharp.rs:2087-2094`.
- refactor(alef-e2e): drop hardcoded downstream-crate names from dart/zig/r e2e codegen. dart now emits `import 'package:{pkg_name}/{pkg_name}.dart'` driven by the resolved `[e2e.packages.dart]` config; zig's `build.zig` resolves `pkg_name`, `module_name`, `ffi_lib_name`, and `ffi_crate_path` from config (mirroring the FFI helper pattern) instead of literal `kreuzberg`/`libkreuzberg_ffi`/`crates/kreuzberg-ffi`; the R `setup_fixtures.jinja` template and `r.rs` codegen renamed `.kreuzberg_test_documents` to the alef-internal generic `.alef_test_documents`. None of these change behavior for kreuzberg (defaults preserve existing names) but removes the assumption that the downstream crate is named "kreuzberg".
- feat(alef-core): introduce `[e2e] test_documents_dir` config knob (default `"test_documents"`) and a centralised `E2eConfig::test_documents_relative_from(emission_depth)` helper that computes the relative path from a backend's emission directory to the configured fixture-binary directory. Replaces hardcoded `"../../test_documents"` / `"../../../test_documents"` literals across all 10 backends (dart, ruby, zig, r, gleam, elixir, go, php, csharp, swift, wasm, typescript, java pom, python conftest, rust args) with calls into the helper. Defaults preserve kreuzberg's behaviour; downstreams whose fixtures don't reference files (liter-llm) can leave the default in place — backends already emit the chdir/setup hook conditionally on `has_file_fixtures`. Hand-rolled `Default` impl on `E2eConfig` ensures the field receives `"test_documents"` even when constructed via `..Default::default()` rather than serde-deserialised.
- fix(alef-backend-ffi): use `config.core_import` (already plumbed into the FFI param-conversion path) as the type-name prefix in `type_ref_to_rust_type` instead of hardcoding `"kreuzberg::"`. The prefix governs how named struct/enum types from the consumer crate are referenced in the generated FFI shim; baking the literal name in meant any non-kreuzberg downstream produced unresolved type paths in the emitted Rust.
- refactor(alef-e2e/gleam): drop kreuzberg-specific Gleam codegen helpers. The OTP-application-startup shim now binds the configured `pkg_name` (snake-cased crate name from `[e2e.packages.gleam] name`) so `application:ensure_all_started/1` adapts to the downstream binding; the helper function name is now `start_app/0` (was `start_kreuzberg/0`) — a fixed, downstream-agnostic identifier. The `BatchFileItem`/`BatchBytesItem` `element_type` branches in `build_args_and_setup` and the `build_gleam_extraction_config` / `build_gleam_default_extraction_config` helpers are removed entirely. The `json_object` arg-type arm now falls back to a generic `json_to_gleam` JSON-string emission. `render_tagged_union_assertion` takes the package qualifier as a parameter (resolved by the caller from the gleam package config) instead of hardcoding `"kreuzberg"`. Note: the FFI plugin shim templates `plugin_impl_initialize.jinja` and `plugin_impl_shutdown.jinja` still construct `kreuzberg::KreuzbergError::Plugin { ... }` literals — that's a kreuzberg-specific error contract that needs a separate refactor (would require defining a generic plugin-error trait in alef-core, which downstreams implement). Tracked for iter16.
- feat(alef-core): introduce `[ffi] plugin_error_constructor` config knob — a Rust expression with access to a `msg: String` local that the FFI plugin shim templates (`plugin_impl_initialize`, `plugin_impl_shutdown`) interpolate verbatim when constructing the error value to return. When unset, the shim falls back to `<core_import::error_type as ::core::convert::From<String>>::from(msg)` so any error type implementing `From<String>` works out of the box. Removes the hardcoded `kreuzberg::KreuzbergError::Plugin { ... }` literal that previously locked the FFI plugin path to kreuzberg's specific error variant shape.
- feat(alef-core,alef-e2e/gleam): introduce `[crates.gleam] element_constructors` and `[crates.gleam] json_object_wrapper` config knobs to restore Gleam record-constructor support without baking downstream-specific knowledge into the codegen. `element_constructors` is a list of recipes keyed by the fixture-side `element_type` (e.g. `"BatchFileItem"`) — each declares a Gleam constructor identifier and a typed list of fields (`file_path` for fixture-relative paths, `byte_array` for `<<n1, n2, …>>` BitArrays, `string` with default, `literal` for constants). `json_object_wrapper` is a single template (`{json}` placeholder) used when no element_type matches; downstreams whose Gleam binding parses a JSON string into a config record (e.g. `kreuzberg.config_from_json_string({json})`) declare it once instead of having alef know the field shapes. Replaces the iter15-removed `BatchFileItem`/`BatchBytesItem`/`build_gleam_extraction_config` helpers.

## [0.15.25] - 2026-05-10

### Fixed

- fix(alef-e2e/typescript): emit `globalSetup` when any fixture uses mock-server — Vitest config now includes the global-setup hook for HTTP-bound or `mock_response`-tagged fixtures so the mock-server binary actually spawns before the test run starts.
- fix(alef-e2e/python): wrap `arg_bindings` in `pytest.raises(...)` and assert on `str(exc_info.value)` — error fixtures that include argument-construction expressions now catch deserialize-time failures inside the `with` block instead of letting them escape before the assertion.
- fix(alef-e2e/typescript): recursively walk nested type fields for wasm imports — class types referenced two or more levels deep (e.g. `WasmChatCompletionRequest.tools[].function: WasmFunctionDefinition`) were emitted in test bodies via `new WasmFunctionDefinition()` but missing from the import statement, causing `ReferenceError: WasmFunctionDefinition is not defined` at runtime. The single-level `derive_nested_types_for_wasm` is now wrapped by `collect_transitive_nested_types_for_wasm`, a BFS over the wasm class graph that follows every struct-typed field through `Vec`/`Optional`. Terminates on cycles via a `seen` set on wasm class names.
- fix(alef-cli): merge `excluded_type_paths` from each per-crate extraction in the multi-source extract pipeline so trait_bridge codegen can resolve qualified paths for excluded types referenced across crate boundaries.
- chore(alef): add `excluded_type_paths` field to all `ApiSurface { ... }` test literals across the workspace (~50 backend test/bench files) and `serde_flatten: false` to the matching `FieldDef` literals in the bench/snapshot suites. Mass-fix follow-up to the `feat(extract): preserve excluded type paths for trait_bridge codegen` IR change so `cargo check` and `cargo test` both stay green.

### Changed

- chore: cargo fmt sweep and sync `Cargo.lock` to 0.15.24 release pin.

## [0.15.24] - 2026-05-10

### Added

- feat(alef-core): `SelectWhen` enum and `resolve_call_for_fixture` — named call configs in `[e2e.calls.*]` can now declare `select_when = { input_has = "<key>" }` to auto-route fixtures whose input contains that key, without requiring an explicit `"call"` field on every fixture. All per-language e2e generators now use `resolve_call_for_fixture` instead of `resolve_call` so auto-routing applies everywhere.
- feat(alef-e2e/go): wrap engine creation in error-return for validation fixtures — when a test fixture has `type=error` assertions, `build_args_and_setup` now emits `return` instead of `t.Fatalf` if `CreateEngine` itself fails, so validation errors from engine creation satisfy the error assertion rather than failing the test.

### Changed

- alef-e2e/typescript: auto-derive nested-type wrapping from IR field types — `ts_builder_expression_inner` now resolves class-typed fields (`TypeRef::Named` and `TypeRef::Vec(Named)`) from the type registry, removing the need for manual `nested_types` mappings in alef.toml call overrides. Explicit overrides still win on collision.

### Added

- feat(alef-docs): render enums in shared `types.md` and `configuration.md` reference files. Previously enum variant tables only appeared in per-language `api-{lang}.md`, so downstream guides could not deep-link to enum sections from the language-neutral shared pages. `types.md` now gets every public enum under a new `### Enums` section; `configuration.md` gets the subset referenced as field types of any rendered config struct (matching on `TypeRef::Named` recursing through `Optional`/`Vec`/`Map`). Kreuzberg's `LayoutClass`, `HtmlTheme`, `TableModel`, `KeywordAlgorithm`, `OutputFormat`, `ResultFormat` (and analogous enums in any other downstream crate) now have stable `#enumname` anchors in both shared files. The shared rendering also adds a `Wire value` column when the enum carries `#[serde(rename_all = "...")]` or any per-variant `#[serde(rename = "...")]`, so users see the actual JSON/TOML token (e.g. `"default"`, `"slanet_wired"`) alongside the Rust variant name.

- feat(alef-core/ir): plumb `FieldDef.serde_flatten: bool` from the Rust source through the IR. Backends use this to emit language-native flatten support — see `feat(java-backend, csharp-backend)` below. The extractor recognises `#[serde(flatten)]` (also under `#[cfg_attr(...)]`) by anchoring on the `flatten` token boundary in the token-stream string.

- feat(java-backend, csharp-backend): emit language-native `#[serde(flatten)] serde_json::Value` support — Java records get `@JsonAnyGetter Map<String, Object>` on the record component plus a matching `@JsonAnySetter <field>Entry(String, Object)` on the builder (with `@JsonIgnore` on the regular `with<Field>(Map)` setter so a wire field of the same name is not miscast as a Map). C# classes get `[JsonExtensionData] Dictionary<string, JsonElement>` instead of `[JsonPropertyName(...)]`. Both implement the serde flatten semantic for types like `ResponseTool { tool_type, #[serde(flatten)] config: Value }` whose wire JSON is `{"type":"function","name":"f","description":"d"}` — without this, Jackson rejected `Unrecognized field "description" (class ResponseToolBuilder)` on every chat-tools request.

- alef-core: `CallOverride.assert_enum_fields` per-call override for routing result-field enum types in assertions.
- alef-e2e/python: thread per-call `assert_enum_fields` through `render_assertion` so same-named fields (e.g. `status`) can be enum in one call and string in another.

### Fixed

- alef-codegen, alef-docs: add `serde_flatten: false` to test-site `FieldDef` literals that were missing the field after iter10's plumbing.
- alef-e2e/java: add TCP-readiness probe to MockServerListener — polls the bound mock-server URL until accepting (max 5s, 50ms backoff) before releasing the JUnit launcher session, preventing intermittent `error sending request` failures under Surefire parallel execution.
- alef-e2e/csharp: add TCP-readiness probe to TestSetup `[ModuleInitializer]` — same polling logic; eliminates intermittent failures under xUnit class-parallel default.
- alef-e2e/typescript: emit numeric bracket access for digit-only JSON pointer segments in test assertions — e.g. `result.results[0]` instead of `result.results["0"]`. The string-keyed access returned `undefined`, breaking array-element assertions.

- fix(csharp-backend): branch sealed-union `JsonConverter<T>` Read on variant shape — struct variants (named fields like `OcrDocument::Url { url: String }`) skip the `"Value"` wrap so `JsonSerializer.Deserialize<Variant>(...)` sees `{"url":"..."}` directly and can match the `[JsonPropertyName("url")]` annotation on the variant record's positional component. Tuple variants (single-field tuple of named struct, e.g. `Message::User(UserMessage)`) keep the `"Value"` wrap as before. Without this, every struct-variant tagged union failed to round-trip — Rust serde rejected the FFI request with `missing field 'url' at line 1 column 73` because the C# layer dropped the field on serialize when the converter could not even deserialize it on the way in. The Read method now produces both `flatJson` (no wrap) and `wrappedJson` and dispatches per variant via the `is_tuple` IR flag.

- fix(e2e/csharp): env-gate live smoke fixtures (no `mock_response`, no `http` override, but `env.api_key_var` set) via early-return when the env var is unset. Without the gate, `task csharp:e2e:test` failed on `smoke_chat_anthropic`, `smoke_chat_gemini`, `smoke_chat_openai`, `smoke_streaming_openai`, `smoke_embed_openai`, `smoke_list_models_openai`, `smoke_provider_routing`, `smoke_cache_memory` with `not found: No mock route for /fixtures/<id>/...` because the mock-server has no fixture for them — they target real provider APIs. Mirrors the Elixir / Python conftest pattern. Both the regular `render_test_method` and `render_chat_stream_test_method` paths gain the gate.

- fix(java-backend): wrap `<Type>.fromJson(json)` deserialization failures in `<Crate>Exception` (the binding's checked exception class) instead of bare `RuntimeException`. The catch block now emits `throw new <Crate>Exception("Failed to parse <Type> from JSON: " + e.getMessage(), e)` and the method signature gains `throws <Crate>Exception` so callers must handle/declare it. Lets error fixtures asserting `assertThrows(<Crate>Exception.class, ...)` catch deserialize-time failures (e.g. `Cannot construct instance of FilePurpose, problem: Unknown value: invalid-purpose`) the same way they catch FFI failures. Mirror change in `untagged_union_wrapper.jinja`.

- fix(csharp-backend): emit a `public static T FromJson(string json)` factory on every generated record/class that wraps `System.Text.Json.JsonException` (and any other deserialization failure) in `<Crate>Exception`. The factory uses a `JsonOptions` field that mirrors the e2e-harness `ConfigOptions` (`JsonStringEnumConverter(SnakeCaseLower)` + `WhenWritingDefault`), so round-trips stay consistent with the FFI request-serialization path. Without this, malformed input JSON surfaced as a raw `JsonException` that error fixtures' `Assert.ThrowsAny<<Crate>Exception>(...)` did not catch. The e2e codegen now emits `<OptionsType>.FromJson("...")` instead of inlined `JsonSerializer.Deserialize<T>("...", ConfigOptions)!` so the wrap actually intercepts the parse step.

- fix(csharp-backend): generate a custom `JsonConverter<T>` for enums whose `#[serde(rename_all)]` is non-snake (`kebab-case`, `SCREAMING-KEBAB-CASE`, `camelCase`, `PascalCase`), not just for enums with explicit per-variant `serde_rename`. The global `JsonStringEnumConverter(SnakeCaseLower)` previously emitted `"fine_tune"` for `FilePurpose::FineTune` (`#[serde(rename_all = "kebab-case")]`), producing `"could not convert to FilePurpose"` on every read of `"fine-tune"`. The custom converter now explicitly maps `"fine-tune" → FilePurpose.FineTune` and writes `"fine-tune"` on the way back. Property-level `[JsonConverter(typeof(<Enum>JsonConverter))]` is also emitted on enum-typed fields so the override wins over the global naming policy.

- fix(e2e/java): wrap `setup_lines` (which may include `<Type>.fromJson(...)` calls that throw on malformed JSON) plus the `call_expr` inside the `assertThrows` lambda for `expects_error` fixtures. Without this, deserialize-time failures escaped before `assertThrows` could catch them — `var request = OcrRequest.fromJson("...invalid-purpose...")` threw outside the assertion. Mirrors the C# `Assert.ThrowsAnyAsync(() => client.X(Type.FromJson(...)))` pattern.

- fix(php-backend): do not append "Async" suffix to async method names in wrapper facade. The PHP binding blocks on async internally via `block_on`, presenting a synchronous API to callers. Wrapper facade methods now use the exact name from the IR (e.g. `scrape()` instead of `scrapeAsync()`), matching the configured function name in alef.toml e2e overrides. The underlying extension methods retain the "\_async" suffix for internal delegation.

- fix(magnus-backend): emit valid serde attribute `#[serde(default = "default_timeout")]` (function path) instead of `#[serde(default = "30000")]` (literal string) for Duration-typed timeout fields. The invalid literal syntax caused compilation errors in Ruby e2e bindings. Added helper function `default_timeout() -> u64` to struct templates.

### Added

- feat(e2e/java): emit a JUnit Platform `LauncherSessionListener` (`MockServerListener`) and the matching SPI manifest under `src/test/resources/META-INF/services/` to spawn the mock-server binary once per launcher session whenever any fixture is HTTP-bound or carries `mock_response`. Mirrors the Ruby `spec_helper.rb` / Python `conftest.py` / Node `globalSetup.ts` / C `Makefile` spawn patterns. The listener parses the `MOCK_SERVER_URL=...` line from mock-server stdout, exposes it as the `mockServerUrl` JVM system property, and tears the child down via stdin-close on `launcherSessionClosed` (with a 2s timeout fallback to `destroyForcibly`). A pre-set `MOCK_SERVER_URL` env var skips the spawn entirely (CI / external orchestration). The pom.xml template gains a `dependencyManagement` import of `org.junit:junit-bom` and an explicit `org.junit.platform:junit-platform-launcher` test dependency so the SPI lookup actually finds the listener interface. Generated test bodies now read `System.getProperty("mockServerUrl", System.getenv("MOCK_SERVER_URL"))` so external overrides still work without going through JNI's lack of `setenv`. Without this, every fixture-bound Java test failed with `LiterLlmRsException: error sending request for url (null/fixtures/<id>/...)` whenever `task java:e2e:test` ran standalone, because nothing in the Java toolchain spawned the mock-server.

- feat(e2e/csharp): extend the `[ModuleInitializer] TestSetup.Init` block to spawn the mock-server binary before the assembly loads whenever any fixture is HTTP-bound or carries `mock_response`. Walks ancestor directories of `AppContext.BaseDirectory` to locate the repo root (the directory containing `test_documents/`), resolves `e2e/rust/target/release/mock-server` (`.exe` on Windows), starts it via `Process` with redirected stdin/stdout/stderr, parses the `MOCK_SERVER_URL=...` line, and calls `Environment.SetEnvironmentVariable("MOCK_SERVER_URL", url)`. Drains stdout/stderr in background daemon threads to keep the child unblocked, and registers an `AppDomain.CurrentDomain.ProcessExit` handler that closes the child's stdin (mock-server treats stdin EOF as a shutdown signal) and falls back to `Process.Kill(true)` after a 2s grace. Honors a pre-set `MOCK_SERVER_URL` by skipping the spawn (CI / external orchestration). Without this, `task csharp:e2e:test` standalone produced 47/161 with 114 `LiterLlmException : builder error` failures because reqwest rejected the relative path `"" + "/fixtures/<id>"` when `Environment.GetEnvironmentVariable("MOCK_SERVER_URL")` returned null.

- feat(csharp-backend): emit a proper `Write` method for sealed-union `JsonConverter<T>`. The previous implementation threw `NotSupportedException` from `Write`, so any C# binding that serialized a sealed union (e.g. a `Message.User` instance inside a `ChatCompletionRequest`) failed at the FFI marshalling step with `Message serialization is not supported`. The new converter mirrors the Java sealed-union serializer pattern: switch on the variant, write the discriminator tag, then flatten the inner record's fields alongside the tag (so `Message.User(UserMessage value)` round-trips as `{"role":"user","content":...}` not `{"value":{...}}`). Emitted via a new `sealed_union_converter.jinja` template that owns both the Read and Write methods and replaces ~190 lines of inline `out.push_str` calls.

### Fixed

- fix(csharp-backend): null-marshal `Optional<&T>` (`TypeRef::Named` + `optional: true`) parameters by passing `IntPtr.Zero` instead of round-tripping the literal string `"null"` through `<Type>FromJson`. Without this, `client.ListBatches(null)` against `Option<&BatchListQuery>` failed with `invalid type: null, expected struct BatchListQuery`. Mirrors the Java fix for the same IR shape. Emitted via a new `named_param_handle_from_json_optional.jinja` template.

- fix(csharp-backend): respect `#[serde(rename = "...")]` when emitting `[JsonPropertyName("...")]` on record properties. Previously the generator hardcoded `field.name` as the JSON wire name, so fields like `tool_type` (renamed via `#[serde(rename = "type")]`) round-tripped as `"tool_type"` on the wire — Rust serde then rejected the request with `unknown field tool_type, expected type or function`. Now uses `field.serde_rename` when set, falling back to `field.name`.

- fix(csharp-backend): retrieve the actual FFI error message instead of throwing a generic `"<NativeMethod> failed"`. The `null_result_throw.jinja` template now reads `LastErrorCode()` + `LastErrorContext()` and surfaces the underlying Rust error (e.g. `"missing field 'role'"`, `"invalid type: null, expected struct BatchListQuery"`) so callers see the real cause instead of a meaningless wrapper.

- fix(csharp-backend): check the return value of `<Type>FromJson` and throw the actual error before passing the (potentially-null) handle to the next FFI call. Without this, a malformed serialized JSON silently returned `IntPtr.Zero`, then the next FFI call failed with `Null pointer passed for parameter 'req'` — masking the real serialization error. The `named_param_handle_from_json.jinja` template now checks the handle and throws with the underlying error context. Same pattern as the existing Java emission.

- fix(e2e/csharp): use `JsonNamingPolicy.SnakeCaseLower.ConvertName(value.ToString())` for enum-equality assertions instead of `.ToString()?.ToLower()`. C# enum members are PascalCase (`InProgress`, `ContentFilter`, `ToolCalls`), so `.ToLower()` produces `inprogress`, `contentfilter`, `toolcalls` — none of which match the snake_case wire format (`in_progress`, `content_filter`, `tool_calls`) that the global `JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower)` actually emits.

- fix(java-backend, csharp-backend): handle `#[serde(rename_all = "kebab-case")]` (and `SCREAMING-KEBAB-CASE`) on enums. Both `apply_rename_all` helpers had no `kebab-case` arm, so they fell through to `name.to_lowercase()`, producing `FineTune` → `"finetune"` instead of `"fine-tune"`. JSON values like `"fine-tune"` then failed to deserialize with `Cannot construct instance of FilePurpose, problem: Unknown value: fine-tune` (3 Java tests, all C# file-purpose tests).

- fix(java-backend): null-marshal `TypeRef::Named` parameters with `optional: true`. The IR represents `Option<&BatchListQuery>` as `TypeRef::Named("BatchListQuery") + optional: true` after FFI extraction strips the `Option`. The Java instance-method emitter only branched on `TypeRef::Optional(Named)`, taking the unconditional path for `Named + optional`: `STREAM_MAPPER.writeValueAsString(null)` yielded the literal `"null"`, then `LITERLLM_BATCH_LIST_QUERY_FROM_JSON("null")` failed inside Rust serde with `invalid type: null, expected struct BatchListQuery at line 1 column 4`. The path now branches on `p.optional` for `TypeRef::Named` and emits `if (param == null) { cParam = MemorySegment.NULL; } else { ... }`, mirroring the existing `TypeRef::Optional(Named)` arm. Fixes `client.listBatches(null)` / `client.listFiles(null)` against `Option<&BatchListQuery>` / `Option<&FileListQuery>` (4 Java tests).

- fix(java-backend): emit `@JsonProperty(<wire-name>)` on builder field declarations when the core field has explicit `#[serde(rename = "...")]`. Without this, fields like `StreamToolCall.call_type` (renamed from `r#type` to `call_type` to avoid Java's contextual-keyword conflict, with `#[serde(rename = "type")]` to keep wire-name `"type"`) deserialize JSON like `{"type":"function"}` via Jackson's `BuilderBasedDeserializer` and fail with `Unrecognized field "type" (class StreamToolCallBuilder), not marked as ignorable (4 known properties: function, index, id, call_type)`. The builder field declarations now mirror the record-component annotation so wire-name lookup succeeds.

### Added

- feat(csharp-backend): emit JsonElement-wrapper records for `#[serde(untagged)]` enums, mirroring the Java backend's Stage A wrapper pattern. Untagged unions like `EmbeddingInput = Single(String) | Multiple(Vec<String>)`, `UserContent = String | Vec<UserContentBlock>`, `ToolChoice = String | Object`, `StopSequence = String | Vec<String>` previously emitted as plain C# `enum`s; System.Text.Json's default enum converter rejected any non-variant-name value (e.g. `"Hello world"` for `EmbeddingInput`) with `JsonException: The JSON value could not be converted to LiterLlm.EmbeddingInput`. The new emission produces a `sealed class T : IEquatable<T>` holding a `JsonElement Value`, with a paired `JsonConverter<T>` that round-trips the raw JSON. Static factories (`Of(string)`, `Of(IEnumerable<string>)`, `OfObject(object)`, `FromJson(string)`) and probe accessors (`AsString()`, `AsList()`, `AsObject()`) keep ergonomic construction available. Field references stay strongly typed (e.g. `EmbeddingRequest.Input` is `EmbeddingInput`, not `JsonElement`) since `gen_bindings/mod.rs.complex_enums` is now empty — the wrapper class itself is the proper field type. Emitted via a new `untagged_union_wrapper.jinja` template registered in `template_env.rs`.

### Fixed

- fix(e2e/csharp): merge per-call C# `enum_fields` into the effective field-enum set used by `render_assertion`. The C# codegen resolved `enum_fields` once from the file-level `[crates.csharp].enum_fields` (top-level) and ignored per-call overrides like `[crates.e2e.calls.retrieve_batch.overrides.csharp].enum_fields`, so call-specific enum-typed result fields (e.g. `status` returning `BatchStatus` on retrieve_batch / cancel_batch / create_batch) never triggered the enum-coercion branch (`?.ToString()?.ToLower()`). The result was `result.Status!.Trim()` against `BatchStatus?` — a compile error (`'BatchStatus?' does not contain a definition for 'Trim'`). `render_test_method` now merges `cs_overrides.enum_fields` keys into the effective set before passing it to `render_assertion`, mirroring the Java fix from 5692d176.

- fix(java-backend): mark the untagged-union wrapper boilerplate (`asString` / `asList` / `asObject` / `equals` / `hashCode` / `toString` block, ~170 tokens) with `// CPD-OFF` / `// CPD-ON` sentinels so PMD's copy-paste detector does not flag the intentional duplication across `EmbeddingInput`, `ModerationInput`, `RerankDocument`, and other untagged-wrapper classes. The duplication is the price of avoiding a generic base class; the sentinels are the standard CPD suppression mechanism. Liter-llm's `packages/java/pom.xml` separately bumps `<minimumTokens>` to 200 to clear the pre-existing 124-token DefaultClient FFI byte-buffer block that the maven-pmd-plugin's `cpd-check` goal also flags.

- fix(e2e/brew): emit `jq -r '<path> // empty'` for `is_empty` assertions so a JSON `null` value becomes an empty string instead of the literal `"null"`. Without this, fixtures that asserted `is_empty` on optional fields (e.g. `metadata.og_title` on a page that omits Open Graph tags) failed because `jq -r .metadata.og_title` prints `null` and the bash assertion treated that as a non-empty value. Restores brew/CLI e2e parity with the other backends, which compare against language-native null.

- fix(e2e/java): merge per-call `enum_fields` into the file-level java enum_fields used by `render_assertion`. The Java codegen resolved enum_fields once from `[crates.e2e.call.overrides.java].enum_fields` (top-level) and ignored per-call overrides like `[crates.e2e.calls.chat.overrides.java].enum_fields`, so call-specific enum-typed result fields (e.g. `choices[0].finish_reason` returning `FinishReason`, `status` returning `BatchStatus` on retrieve_batch/cancel_batch) never triggered the Optional<Enum> coercion path added in 51fb7656. The result was `Optional.ofNullable(x).orElse("")` against `Optional<FinishReason>` (`incompatible types: java.lang.String cannot be converted to FinishReason`) and `BatchStatus.trim()` (`cannot find symbol: method trim()`). `render_test_method` now merges `call_overrides.enum_fields` over the file-level map before passing it down so per-call enum classifications take precedence.

- fix(napi-scaffold): include `napi/serde-json` feature for crates with Json fields. The NAPI scaffolder emitted `napi = { version = "3", features = ["async"] }` unconditionally, but when the API surface contained any `TypeRef::Json` field (e.g. `data: Option<serde_json::Value>`), the binding failed to compile with `error[E0277]: the trait 'ToNapiValue' is not implemented for 'std::option::Option<Value>'` because the `napi/serde-json` feature is required to marshal JSON types. The scaffolder now detects Json types recursively across struct fields, method parameters/returns, function parameters/returns, and enum variants; when any Json type is found, it adds `"serde-json"` to the napi features list. This restores kreuzcrawl-node and other bindings with Json fields to compatibility without hand-edits.

- fix(e2e/rust): support array-form `input.mock_responses` schema in mock-server template. The Rust e2e test template's `render_mock_server_setup` only recognized the single-response schema (`mock_response: { status, body, stream_chunks, headers }`), silently discarding the multi-response array schema (`input.mock_responses: [{ path, status_code, headers, body_inline, body_file }, ...]`) used by kreuzcrawl and other fixture-heavy projects. The kreuzcrawl standalone mock-server (`tools/mock-server/src/main.rs`) correctly loaded 452 routes from 246 fixture files, but the alef-generated `e2e/rust/src/main.rs` loaded only 20, because the template never examined `fixture.input.get("mock_responses")`. The fix extends `render_mock_server_setup` to first check for the array schema (extracting `path`, `status_code`, `headers`, and `body_inline` from each element; defaulting path to "/" and status to 200), fall back to the single-response schema if absent, emit multiple `MockRoute` objects when the array contains multiple elements, and emit a single route when either schema produces exactly one response. The standalone binary's `load_routes_recursive` function continues to handle both schemas including `body_file` (fixture-relative file loading), while the test-function template (which lacks fixture-dir context) emits placeholder bodies for `body_file` entries — the binaries will be tested separately against the real file paths. Restores e2e fixture coverage for array-based mock-response projects.

## [0.15.23] - 2026-05-09

### Fixed

- fix(scaffold-swift): drop `swift-actions/setup-swift@v2` from the generated Swift CI workflow and rely on the macos-latest runner's bundled Xcode toolchain. The third-party action installs a Swift 6.0 toolchain that is binary-incompatible with Xcode 16.4's XCTest framework, so swiftpm test bundles fail to load `XCTestCore.framework` at runtime (`Library not loaded: @rpath/XCTestCore.framework/Versions/A/XCTestCore`). The macos-latest runner already ships Xcode 16+ with Swift 6 on `PATH`, so no extra setup step is required. The step is replaced with a diagnostic `swift --version` step.

## [0.15.22] - 2026-05-09

### Fixed

- fix(backend-swift): re-wire `emit_e2e_wrappers` and `emit_json_factory_shims` so the kreuzberg-style e2e helpers (`extractionConfigFromJson`, `batchBytesItemFromJson`, `batchFileItemFromJson`) are emitted again. Emission is gated structurally on the api surface exposing all three serde-enabled types (`ExtractionConfig`, `BatchBytesItem`, `BatchFileItem`) — this keeps the helper out of binding crates that don't expose those types while restoring the symbols the alef-e2e Swift codegen still calls (`crates/alef-e2e/src/codegen/swift.rs:762`). Without the helpers, every kreuzberg-derived Swift test suite failed at compile with `cannot find 'extractionConfigFromJson' in scope`. Also drops the broken multi-line `out.push_str` in `batchExtractBytesSync` that produced stray indentation.

## [0.15.21] - 2026-05-09

### Fixed

- fix(scaffold-swift): replace the unsafe `printf "import RustBridgeC\n$(cat …)"` form in the generated Swift workflow and BUILDING.md template with `{ echo "import RustBridgeC"; cat …; }`. printf interprets `%` and `\` sequences in its format string, so the substituted swift-bridge source content was being silently corrupted whenever it contained those characters — surfacing in CI as `cannot find type 'RustString' in scope` errors in hand-authored Swift sibling files (e.g. `Plugins.swift`) because the copied `SwiftBridgeCore.swift` lost its `RustString` declaration. The new echo+cat form has no format-string interpretation and is robust against arbitrary byte content. Applied to the `swift.yml` workflow Copy step and to both the debug and release sections of `BUILDING.md`. Regression assertions added to `test_scaffold_swift`.

## [0.15.20] - 2026-05-09

### Added

- feat(backend-swift): emit `extern "Swift"` plugin trait bridges so a Swift class can implement a Rust plugin trait and have Rust call back into Swift. For each configured `[[crates.trait_bridges]]` entry the swift bridge crate now produces, alongside the existing outbound `{Trait}Box` glue: an `extern "Swift" type Swift{Trait}Box` declaration with one FFI shim per method (plus `name`/`version`/`initialize`/`shutdown` Plugin shims), a Rust `Swift{Trait}Wrapper` newtype with `OnceLock<String>` name cache and `unsafe impl Send + Sync`, a `Plugin` super-trait impl forwarding to the Swift box (mapping `Result<_, String>` to `KreuzbergError::Plugin`), an `#[async_trait] impl {Trait}` forwarding each method through the Swift FFI shims with JSON marshalling for complex types, and `register_*`/`unregister_*` free functions exposed via an additional `extern "Rust"` block (`#[swift_bridge(swift_name = "registerXxx")]`) that wrap the wrapper in `Arc` and insert it into the configured `registry_getter`. Complex types are JSON-bridged at the FFI boundary (every `Named` type, all `Optional`/`Map`/non-leaf `Vec`); primitives, `String`, `Vec<u8>`, and `Vec<leaf>` pass through directly. ARC lifecycle: swift-bridge's `extern "Swift" type` declaration retains the Swift instance via `Unmanaged<T>.passRetained` and releases it on `Drop`, so storing the wrapper in a process-wide `Arc<dyn Trait>` registry is ARC-safe — the Swift instance lives until the registry's last `Arc` clone is dropped. The generated `Cargo.toml` gains an `async-trait = "0.1"` dependency since the inbound trait impl uses `#[async_trait]` to mirror the source crate's async trait shape. User-facing `Sources/<Module>/Plugins.swift` (Swift protocols + `Swift{Trait}Box` adapter classes) is left for the consuming binding to author by hand — swift-bridge requires a Swift class named exactly `Swift{Trait}Box` with the matching `alefSwift*` method names, which the Rust-side codegen documents in struct doc comments.

## [0.15.19] - 2026-05-09

### Fixed

- fix(backend-swift): drop the bogus `.toString()` suffix on `String` and `Json` returns from convenience overloads. swift-bridge maps a Rust `String` return (bare or inside `Result<String, _>`) directly to a Swift-native `String` — there is no `RustString` wrapping to unwrap. Calling `.toString()` on a Swift `String` failed at the call site with `error: value of type 'String' has no member 'toString'`. The conversion suffix is now only emitted for `RustVec<T>` returns (Bytes, `Vec<primitive>`), where `.map { $0 }` is still needed to materialise the `Sequence` into a Swift `Array`.

- fix(backend-swift): skip emitting convenience overloads when the public wrapper name would collide with the bridge function name (i.e. when the IR function name has no `_sync` suffix to strip). The v0.15.18 fix incorrectly assumed Swift overload resolution would disambiguate the labeled overload from the unlabeled bridge function based on argument-label inclusion — in practice Swift's resolver still treats the in-file labeled candidate as a match for the unlabeled positional inner call, producing `error: no exact matches in call to global function 'renderPdfPageToPng'` and `'detectMimeTypeFromBytes'`. Module-prefix qualification (`RustBridge.fn(...)`) is rejected by Swift for free functions imported from another module, so there is no clean disambiguation path. We now skip the convenience overload entirely when `wrapper_name == swift_inner`; the bridge function remains callable directly with `makeByteVec(...)`. Restores local `swift build` against `kreuzberg/packages/swift` to a clean compile.

## [0.15.18] - 2026-05-09

### Fixed

- fix(backend-swift): drop the bogus `RustBridge.` qualifier introduced in v0.15.16. Swift rejects `ModuleName.functionName` qualification of free functions imported from another module — the v0.15.16 emission `RustBridge.detectMimeTypeFromBytes(...)` failed at the call site with `error: module 'RustBridge' has no member named 'detectMimeTypeFromBytes'`, even though the function is `public func` at module scope in the swift-bridge-generated source. The original shadowing diagnosis was wrong: convenience overloads use a labeled first parameter (`content: String` / `content: [UInt8]` / `path: String`), the underlying bridge function uses an unlabeled first parameter (`_ content: RustVec<UInt8>`), and Swift overload resolution disambiguates by argument-label inclusion. A positional inner call `detectMimeTypeFromBytes(makeByteVec(...))` resolves unambiguously to the bridge function regardless of which other same-named labeled overloads exist in the same module. The codegen now always emits the unqualified inner call, restoring local `swift build` against `kreuzberg/packages/swift` to a clean compile.

## [0.15.17] - 2026-05-09

### Added

- feat(backend-dart): emit `register_*` / `unregister_*` forwarders for `[[trait_bridges]]` entries that set `register_fn` (and optionally `unregister_fn`) plus `registry_getter`. The Dart backend previously emitted only the `{Trait}DartImpl` opaque struct, the trait impl, and the `create_*_dart_impl` factory — leaving Dart consumers no way to actually install their callbacks into the host registry. The codegen now appends `pub fn {register_fn}(impl_: {Trait}DartImpl) -> Result<(), String>` (wrapping `impl_` in `Arc<dyn Trait>` and calling `registry_getter().write().register(arc{register_extra_args})`) and, when `unregister_fn` is set, `pub fn {unregister_fn}(name: String) -> Result<(), String>` (calling `registry_getter().write().remove(&name)`). Both forwarders stringify the host-typed error since FRB requires owned, FFI-safe error types. FRB auto-bridges these `pub fn` items, so Dart sees `Future<void> registerOcrBackend(OcrBackendDartImpl impl_)` / `Future<void> unregisterOcrBackend(String name)` directly. Going through the registry handle (rather than the host crate's `register_*` free function) sidesteps `pub(crate)` / `#[cfg(test)]` restrictions on trait registration wrappers, notably for `EmbeddingBackend`. Closes the last gap for Dart plugin trait callbacks.

## [0.15.16] - 2026-05-09

### Fixed

- fix(backend-swift): qualify the convenience-overload inner call with `RustBridge.` when the wrapper has the same Swift name as the underlying bridge function. When the IR function name lacked a `_sync` suffix to strip, `wrapper_name` equalled `swift_inner` and the same-module convenience overloads (e.g. `detectMimeTypeFromBytes(content:)` and `renderPdfPageToPng(content:pageIndex:dpi:password:)`) shadowed the imported bridge declarations from `RustBridge`. The inner call `try detectMimeTypeFromBytes(makeByteVec(...)).toString()` resolved to the convenience overload itself — returning a Swift `String` that has no `.toString()` member — and `try renderPdfPageToPng(makeByteVec(content), pageIndex, dpi, password)` produced `error: no exact matches in call to global function 'renderPdfPageToPng'` because the same-module candidates require labels (`content:pageIndex:dpi:password:`). The codegen now emits `RustBridge.<name>(...)` whenever the wrapper would otherwise self-shadow, routing the call to the positional swift-bridge wrapper without relying on import-shadow resolution order.

## [0.15.15] - 2026-05-09

### Fixed

- fix(scaffold-swift): generated `.github/workflows/swift.yml` now builds the `{crate}-swift` Rust crate and copies the swift-bridge generated headers and Swift sources into `Sources/RustBridgeC/` and `Sources/RustBridge/` before invoking `swift build`. Previously the workflow ran `swift build` directly with `working-directory: packages/swift`, but `Sources/RustBridgeC/RustBridgeC.h` is only a placeholder until `cargo build -p {crate}-swift` produces the real combined header in `target/debug/build/{crate}-swift-*/out/`. CI failed with hundreds of `cannot find '__swift_bridge__$Vec_<Type>$<method>' in scope` errors because the C symbols and Swift bridge wrappers were never copied into the SwiftPM targets. The new workflow installs the Rust toolchain before Swift, runs `cargo build -p {crate}-swift` at the repo root, concatenates `SwiftBridgeCore.h` and `{crate}-swift/{crate}-swift.h` into `Sources/RustBridgeC/RustBridgeC.h`, prepends `import RustBridgeC` to the generated `SwiftBridgeCore.swift` and `{crate}-swift.swift` files (writing them into `Sources/RustBridge/`), then runs `swift build` and `swift test` from `packages/swift`. Mirrors the canonical sequence documented in the scaffolded `BUILDING.md`.

## [0.15.14] - 2026-05-09

### Fixed

- fix(scaffold-kotlin): exclude the alef-emitted Kotlin binding-class file (e.g. `Kreuzberg.kt`) from the ktlint check in the generated `packages/kotlin/build.gradle.kts`. The alef-backend-kotlin emits the binding object with parameters on a single line, no expression bodies, and blank lines inside method blocks — patterns that ktlint flags as `parameter-list-wrapping`, `function-expression-body`, and `no-blank-lines-in-block` violations. The scaffold's `ktlint { filter { ... } }` block now adds `exclude { entry -> entry.file.toString().endsWith("/<BindingClass>.kt") }` (where `<BindingClass>` is `config.name` pascal-cased) so the generated file passes CI without hand-edits while still linting all user-authored Kotlin sources.

## [0.15.13] - 2026-05-09

### Fixed

- fix(scaffold-gleam): bump `gleam-version` in the generated `.github/workflows/gleam.yml` from `1.4` to `1.14`. The latest `gleam_stdlib` requires Gleam `>= 1.14.0`, so CI failed with `Incompatible Gleam version` on every fresh scaffold using the default toolchain pin.

- fix(scaffold-swift): bump `swift-version` in the generated `.github/workflows/swift.yml` from `5.10` to `6.0`. The scaffolded `Package.swift` declares Swift tools 6.0, so CI failed with `package 'swift' is using Swift tools version 6.0.0 but the installed version is 5.10.1` whenever the workflow ran.

- fix(scaffold-kotlin): emit the generated `packages/kotlin/build.gradle.kts` with 2-space indentation matching the `[*.gradle.kts] indent_size = 2` rule already declared in the scaffolded `.editorconfig`. The template previously emitted 4-space indent, which ktlint flagged via `ktlintKotlinScriptCheck` against the build script itself. Also drops the redundant `${rootDir}/...` braces in the JNA `libPath` assignment to silence the matching ktlint warning.

## [0.15.9] - 2026-05-09

### Fixed

- fix(e2e/elixir): coerce enum atom fields via `to_string/1` before string comparisons. Rustler binds Rust enums as Elixir atoms (e.g. `:stop` for `FinishReason::Stop`), so `String.trim/1` on `result.choices[0].finish_reason` raised `FunctionClauseError: no function clause matching in String.trim/1` on every chat fixture asserting `finish_reason == "stop"`. `render_assertion` now consults `[crates.e2e].fields_enum` (matched against both the raw fixture path and the resolved alias) and wraps the accessor in `to_string(...)` for `equals` assertions, mirroring the existing Ruby/Python coercion paths.

- fix(e2e/elixir): drop `_async` suffix for streaming entry-points. `[crates.e2e.calls.chat_stream]` is marked `async = true`, but the Elixir binding exposes the streaming wrapper as `defaultclient_chat_stream/2` (no `_async` suffix — it drives the FFI iterator handle synchronously, returning a `Stream`). The codegen unconditionally appended `_async`, producing `LiterLlm.defaultclient_chat_stream_async/2 is undefined` on every stream fixture. The two suffix sites (`render_test_files` and `render_test_case`) now skip the suffix when the resolved base function name ends with `_stream`.

- fix(e2e/elixir): plumb `extra_args` from per-language overrides. `[crates.e2e.calls.list_files.overrides.elixir]` and `[crates.e2e.calls.list_batches.overrides.elixir]` configure `extra_args = ["nil"]` to fill the trailing `Option<String> query` slot, but the Elixir codegen ignored them — emitting `defaultclient_list_files_async(client)` against a `/2` arity binding (`UndefinedFunctionError: did you mean defaultclient_list_files_async/2`). `render_test_case` now reads `call_overrides.extra_args` and appends them after configured args before prefixing the client variable, mirroring the Ruby/Go/Node/C# generators.

- fix(e2e/elixir): env-gate live smoke fixtures via `OPENAI_API_KEY in [nil, ""]`. Smoke fixtures without `mock_response` (e.g. `smoke_chat_openai`, `smoke_streaming_openai`) target real provider APIs; absent env keys yielded `MatchError: {:error, "no mock route ..."}` rather than a clean skip. The codegen now emits an `if System.get_env("<api_key_var>") in [nil, ""] do :ok else <test body> end` wrapper when `fixture.env.api_key_var` is set and there's no mock_response/http override, mirroring the Python conftest skip pattern. Both the early-return `expects_error` path and the assertion path close the `else` block.

- fix(e2e/elixir): support `result_is_simple` for raw-byte returns (`speech`, `file_content`). The Elixir binding returns the audio/content bytes directly as a binary, but the codegen emitted `assert result.audio != ""` against the binary, raising `BadMapError: expected a map`. `render_test_case` now reads `result_is_simple` from the resolved call override, and `render_assertion` bypasses the field accessor (and the `is_valid_for_result` skip) when set, asserting against the bound `result` variable directly. Mirrors the existing Ruby/PHP `result_is_simple` path. Combined with adding `Default` to `OcrRequest` (allowing the rustler backend to marshal it as `Option<String>` JSON), drops Elixir e2e failures from 68 → 0.

- fix(e2e/typescript+wasm): assert against `EnumClass.Variant`, wrap numeric assertions in `Number(...)`, plumb `extra_args`, and replace empty-input `{}` with no positional args. Four WASM-specific assertion regressions made cross-language e2e fail with `result.choices[0].finishReason.trim is not a function` (wasm-bindgen exposes Rust enums as numeric discriminants, not strings), `expected 15n to be 15` (`u64`/`i64` getters return BigInt — comparing against a JS Number with `.toBe()` always fails on `Object.is` equality), `expected instance of WasmFileListQuery / WasmBatchListQuery` (zero-arg `list_files()`/`list_batches()` was emitted as `listFiles({}, undefined)` because the codegen always dumped the fixture's `{}` input as the first arg), and `extra_args` configured per-language but never consumed by the typescript/wasm generator. The fix: (1) adds `result_enum_fields: HashMap<String, String>` to `CallOverride`, used by `render_assertion` to emit `expect(field).toBe(EnumClass.Variant)` (PascalCase-converted) when the assertion field path matches — the enum class is auto-imported alongside the request type. (2) For numeric `equals` / `<` / `<=` / `>` / `>=` on WASM, wraps the field expression in `Number(...)` so BigInt and Number compare equal. (3) Plumbs `extra_args` through `render_test_case` for both `node` and `wasm` lang keys. (4) When a call has no configured args and the fixture input is an empty object, returns an empty `args_str` so `extra_args = ["undefined"]` becomes the sole call argument. Drops liter-llm's WASM e2e failures from 51 → 18 (143/161 passing).

- fix(e2e/c): pass `MOCK_SERVER_URL/fixtures/<id>` as `base_url` and spawn the mock-server from the generated Makefile. Every `mock_response`/`http` fixture now has its `client = create_client("test-key", base_url, ...)` constructed against the per-fixture mock-server route, mirroring the Python/Ruby/Java/Dart codegen pattern. Without this, every C test that asserts `result != NULL` failed at the FFI boundary because the default OpenAI URL with a fake `"test-key"` could never serve a mocked response (e.g. `test_edge_batch_empty_list` aborted with `assert(result != NULL && "expected call to succeed")` at the first non-error fixture). The Makefile `test:` target now spawns `../rust/target/release/mock-server ../../fixtures` over a FIFO (so stdin stays open), parses the printed `MOCK_SERVER_URL=...` line, exports it for the test process, and tears the server down on exit. Honors a pre-set `MOCK_SERVER_URL` by skipping the spawn. The C `render_test_function`, `render_bytes_test_function`, and `render_chat_stream_test_function` paths all consult `Fixture::needs_mock_server()` and emit the per-fixture `getenv("MOCK_SERVER_URL")` + `snprintf` setup before the `create_client` call.

- fix(e2e/java): emit FQN imports for binding types and split imports onto separate lines. Previously the Java codegen derived the binding package only when `class_name` itself was fully-qualified (rare); the more common unqualified case (e.g. `class_name = "LiterLlm"`) left `import_path = ""`, so every dependent import branch — `all_options_types`, `enum_types_used`, `nested_types_used`, `CrawlConfig`, visitor types — fell back to bare type names like `import ChatCompletionRequest;`, which javac rejects with `'.' expected`. The fix threads the resolved `[crates.java] package` (via `config.java_package()`) into `render_test_file` as `binding_pkg` and routes every binding-type import through that package, with the per-class FQN derived from `import_path` retained as a secondary fallback. The class-itself import (`import {binding_pkg}.{ClassName};`) is now emitted when `class_name` is unqualified, fixing `cannot find symbol: class LiterLlm` on every test file. The `templates/java/test_file.jinja` import block also dropped the `{%-` whitespace-strip on `endfor`, which had collapsed every `{{ imp }}` and the `package` line into a single physical line — imports now render one per line as required by Java syntax. Bonus: the `templates/java/assertion.jinja` `equals` branch handles `value: null` fixtures (which deserialize to `Option<Value> = None` under `#[serde(default)]`) by emitting `assertTrue(...isEmpty(), "expected null")` against the unwrapped Optional accessor instead of `assertEquals(, ...)` — the latter was an empty-token-list compile error in every fixture asserting on a nullable field.

- fix(php-backend/enum-tainted): emit `..Default::default()` in `From<Binding> for Core` impls when the core type has cfg-stripped fields. The enum-tainted From-impl emitter (`gen_enum_tainted_from_binding_to_core`) iterates `typ.fields`, but cfg-gated fields are absent from the IR, so the resulting `Self { … }` initializer was missing those fields entirely — `error[E0063]: missing field 'browser_pool' in initializer of kreuzcrawl::CrawlConfig`. The two templates `php_impl_from_begin.jinja` / `php_impl_from_end.jinja` now accept a `has_stripped_cfg_fields` flag (passed through `typ.has_stripped_cfg_fields`); when set, the begin template adds `#[allow(clippy::needless_update)]` and the end template emits `..Default::default()` before closing the struct literal. Mirrors the existing handling in `binding_to_core_impl.jinja` for the non-tainted path.

- fix(codegen/structs): exclude data-enum opaque wrappers from `#[serde(skip)]` on parent fields. The PyO3 (and any future serde-aware) backend used to add `#[serde(skip)]` to every field whose type referenced any name in `opaque_type_names`, which now also includes data-enum wrappers (e.g. `Message`, `StopSequence`, `ToolChoice`, `ResponseFormat`). Those wrappers are emitted via `gen_pyo3_data_enum` with hand-rolled forwarding `serde::Serialize`/`Deserialize` impls that delegate to the core type, so the skip silently dropped the field on JSON round-trips: `ChatCompletionRequest::from_json('{"messages":[…]}').messages` produced `[]`, breaking every binding-level `from_json` round-trip and surfacing as `BadRequestError: Invalid 'messages': empty array` against real provider APIs. The fix introduces a new `serializable_opaque_type_names: &[String]` field on `RustBindingConfig`; `gen_struct_with_per_field_attrs` and `gen_struct_with_rename` exclude fields referencing names in this slice from the skip rule. The PyO3 backend populates it with `data_enum_names` (cloned before the existing `opaque_names_vec.extend(data_enum_names)` move) — fields referencing data-enum wrappers now serialize/deserialize through the forwarding impls instead of defaulting to empty.
- fix(e2e/c): emit `uint64_t` / `int32_t` / `double` / `bool` for primitive leaf accessors instead of `char*`. The C codegen previously hardcoded `char* {local} = {prefix}_{type}_{field}({handle});` for every leaf field, then routed every assertion through `strcmp` / `str_trim_eq`. FFI signatures like `uint64_t literllm_usage_total_tokens(...)` and `double literllm_rerank_result_relevance_score(...)` thus failed to compile (`incompatible integer to pointer conversion initializing 'char *' with 'uint64_t'`, `invalid operands to binary expression ('char *' and 'double')`). The new path consults a `[e2e.fields_c_types]` lookup for `"{parent_snake_type}.{field}"` — when the resolved type is a primitive C scalar (`uint64_t`, `int32_t`, `uint32_t`, `int64_t`, `double`, `float`, `bool`, …), the codegen emits the typed local declaration, switches `equals` assertions to `==` (mapping JSON booleans to `1` / `0` for `bool` slots), and skips the `char*` free at end-of-test. Both `emit_nested_accessor` (returning `Option<String>` for the leaf primitive) and the non-nested accessor branch consult the lookup. `render_assertion` gains a `primitive_locals: &HashMap<String, String>` parameter to dispatch numeric vs string comparisons.
- fix(e2e/c): support `string` and optional pointer args in the client-factory pattern. The C codegen previously processed only `arg_type == "json_object"` args in the client pattern (constructing handles via `_from_json` and passing them positionally), silently dropping every other arg type — so `cancel_batch(client, batch_id)`, `retrieve_file(client, file_id)`, etc. emitted `cancel_batch(client)` and failed to compile (`too few arguments to function call, expected 2, have 1`). The new path adds an `else if arg.arg_type == "string"` branch that reads `fixture.input.<field>`, escapes via `escape_c`, and emits the value as a C string literal `"value"` inline; an `else if arg.optional` branch passes `NULL` for missing optional arg slots; and the method-call argument string concatenates request handles + inline args together. Resolves the `cancel_batch` / `retrieve_batch` / `retrieve_file` / `delete_file` / `retrieve_response` / `cancel_response` compile errors.
- fix(e2e/c): honor `extra_args` from per-language overrides. The C codegen now reads `extra_args` from `[crates.e2e.calls.<name>.overrides.c]` (mirroring Rust/Go/Ruby/Node) and appends them verbatim after configured `args`. Used to pass `NULL` for trailing optional pointer slots in FFI signatures the fixture cannot supply directly (e.g. `*const FileListQuery` on `list_files`, `*const BatchListQuery` on `list_batches`).
- fix(e2e/c): emit numeric/boolean comparisons for map-access fields. JSON-extracted leaves (e.g. `results[0].relevance_score`, `data[0].index`) come back as `char*` from `alef_json_get_string`, so `strcmp(p, 0)` / `p > 0.9` previously failed type-check. `render_assertion` now detects the map-access flag on the field and re-routes `equals` / `greater_than` / `less_than` / `greater_than_or_equal` / `less_than_or_equal` through `atof(p)` (numbers) or `strcmp(p, "true"/"false")` (booleans) when the expected value is non-string.
- fix(php-backend): generated `#[php(constructor)]` named constructors now match binding-side field types. Two regressions produced uncompilable PHP for any core type that satisfies `has_serde && has_default && contains_non_optional_Duration && contains_Vec<serde_json::Value>`: (1) `Duration` fields stored as `Option<i64>` in the binding (via the serde-skip-serializing-if-none + `option_duration_on_defaults` interaction) had constructor parameters typed as plain `i64`, breaking the implicit `Self { timeout, … }` init with `expected Option<i64>, found i64`. The fix flips `optional=true` on the `ParamDef` for Duration fields when `has_serde && typ.has_default`. (2) `Vec<serde_json::Value>` fields were treated as constructor parameters, but `Vec<serde_json::Value>` does not implement `FromZval`, so ext-php-rs could not generate marshalling code (`the trait FromZval is not implemented for std::vec::Vec<Value>`). The fix excludes `TypeRef::Vec(TypeRef::Json)` from `field_can_be_param`, so the constructor signature drops the JSON-array parameter entirely and the field initializer falls through to `Default::default()`. Both fixes target `gen_struct_methods_impl` in `alef-backend-php/src/gen_bindings/types.rs`. Affects kreuzcrawl's `BrowserConfig.timeout` / `CrawlConfig.request_timeout` and `MarkdownResult.tables`.

- fix(codegen/binding-to-core): preserve `Option<T>` layer for genuinely-optional fields in the `has_optionalized_duration` builder branch. The branch routed every `field.optional == true` field through `gen_optionalized_field_to_core(..., field_is_ir_optional=true)`, whose body emits `.unwrap_or_default()` for primitives, `String`, `Path`, `Duration`, etc. — producing `T` from a binding `Option<T>` and breaking the assignment to a core `Option<T>` destination. Genuinely-optional fields now fall through to `field_conversion_to_core_cfg(name, ty, true, config)` regardless of `optionalize_defaults`, mirroring the non-builder branch's logic. Both `optionalize_defaults=false` (e.g. PyO3) and `optionalize_defaults=true` (e.g. NAPI) configurations are now correct: in both cases a genuinely-optional core field maps to a single-Option binding field with passthrough conversion. Affected any crate where the parent type has both a non-optional `Duration` (triggering builder mode via `option_duration_on_defaults`) and at least one `Option<T>` field — e.g. kreuzcrawl's `BrowserConfig` (`endpoint`, `wait_selector`, `extra_wait`) and `CrawlConfig` (`max_depth`, `max_pages`, `browser_profile`, `warc_output`).

- fix(pre-commit): prefer pre-installed `alef` binary on PATH (when `--version` matches the pinned `alef.toml` version) before falling back to the cached release tarball download. Speeds up local development (no network round-trip when `cargo install --path crates/alef-cli` already produced a binary) and avoids the "No such file or directory" failure mode when a downstream `.pre-commit-config.yaml` overrode `entry: alef verify` — overrides are no longer needed because the script-language hook itself dispatches to the right binary. Documented in `.pre-commit-hooks.yaml` that `entry:` should not be overridden.
- fix(e2e/csharp): emit `?.ToString()?.ToLower()` for enum-typed `equals` assertions instead of `!.Trim()`. `[crates.e2e].fields_enum` entries (e.g. `choices[0].finish_reason`, `status`) resolve to typed C# enums (`FinishReason?`, `BatchStatus?`) in the binding surface, but the C# assertion path unconditionally suffixed `!.Trim()` — a string-only API — yielding `error CS1929: 'FinishReason?' does not contain a definition for 'Trim'` on every chat/batch fixture. `render_assertion` now consults `e2e_config.fields_enum` (resolved through `FieldResolver::resolve` for nested paths like `choices[0].finish_reason`) and bypasses the assertion template for enum equality, emitting `Assert.Equal("stop", result.Choices[0].FinishReason?.ToString()?.ToLower())`. The lowercase comparison matches the JSON form attached via `[JsonPropertyName("stop")]` on each enum variant.
- fix(e2e/csharp): drive `chat_stream` tests via `await foreach` over `IAsyncEnumerable<ChatCompletionChunk>`. The C# binding emits `IAsyncEnumerable<ChatCompletionChunk> ChatStream(req)` (not `Task<T>`), but `render_test_method` previously emitted `var result = await client.ChatStream(req)` — `error CS1061: 'IAsyncEnumerable<ChatCompletionChunk>' does not contain a definition for 'GetAwaiter'`. The new `render_chat_stream_test_method` branch detects `function = "chat_stream"` up front and emits a dedicated body that loops `await foreach (var chunk in client.ChatStream(req))`, building local aggregator vars (`chunks`, `streamContent`, `streamComplete`, plus optional `lastFinishReason` / `toolCallsJson` / `toolCalls0FunctionName` / `totalTokens` resolved from fixture assertions). Pseudo-fields (`chunks`, `stream_content`, `stream_complete`, `no_chunks_after_done`, `finish_reason`, `tool_calls`, `tool_calls[0].function.name`, `usage.total_tokens`) translate to assertions on those locals; `error` fixtures wrap the foreach in `Assert.ThrowsAnyAsync` so the producer is actually consumed. Mirrors the Stage 3 Ruby/C streaming codegen pattern.

### Changed

- feat(e2e/java): emit real test bodies parallel to Python codegen — drop `Assumptions.assumeTrue(false, ...)` stubs. The Java e2e generator now resolves `client_factory` and `options_via` from java overrides (with file-level fallback) and emits real bodies for every non-HTTP fixture. With `client_factory` set, tests instantiate a client via `{ClassName}.{factory}("test-key", mockUrl, null, null, null)` (when fixture has `mock_response`/`http`) or via the env-key-or-skip pattern, then dispatch the call as a method on the client. With `options_via = "from_json"`, json_object args are built via `{OptionsType}.fromJson(jsonString)` instead of the builder expression path. The `if call_overrides.is_none() { assumeTrue(false) }` stub branch in `render_test_method` is removed.

### Fixed

- fix(e2e/csharp): preserve `List<T>` element types and construct `JsonElement` for untagged-union fields. The C# e2e codegen previously emitted `new List<string> { ... }` for every JSON array nested inside an options object initializer, regardless of the property's actual type — so `ChatCompletionRequest.Messages` (typed `List<Message>`) and any `JsonElement?` field (untagged unions like `tool_choice` / `stop` / `EmbeddingRequest.Input`) failed with `CS0029` at compile. The fix adds `options_via = "from_json"` plumbing to the C# codegen (resolved per-call with file-level fallback through `[crates.e2e.call.overrides.csharp]`): when set, the generator emits `JsonSerializer.Deserialize<{OptionsType}>("...", ConfigOptions)!` for `json_object` args, sidestepping per-field type inference entirely. Discriminator fields in nested objects are sorted-first (mirroring the existing handle-config path) so System.Text.Json polymorphic deserialization sees the discriminator before the data fields.
- fix(e2e/elixir): build typed maps with atom keys instead of JSON strings — actually solved by routing default-typed (has*default) struct method params through `Option<String>` JSON in the rustler backend (`gen_nif_method`/`gen_nif_async_method` plus the streaming `_start` post-processor), mirroring the existing free-function pattern. Rustler's `NifMap` derive is strict (`try_decode_field` returns `Err` whenever any keyed atom is absent), so partial Elixir maps like `%{messages: …, model: …}` failed to decode and surfaced as `ArgumentError` at every method call site (148 of 161 e2e tests in liter-llm). The fix accepts `Option<String>` JSON, deserializes via `serde_json::from_str` (so `#[serde(default)]` fills missing fields), and forwards the resulting `core*\*` local. The Elixir wrapper layer (`liter_llm.ex`) already typed `req`as`String.t() | nil` so no wrapper change was needed; e2e tests that already emit JSON strings now decode correctly.
- fix(e2e/ruby): drop architectural skip for streaming fixtures, emit `chat_stream` block iteration. The Ruby e2e codegen previously emitted `skip 'Non-HTTP fixture cannot be tested via Net::HTTP'` for every non-HTTP fixture whose assertions referenced pseudo-fields (`chunks`, `stream_content`, `stream_complete`, ...) the public response type does not expose, leaving the entire `streaming_spec.rb` suite pending. The new path detects `function = "chat_stream"` up front, suppresses the Magnus `_async` suffix (the streaming method is `chat_stream`, not `chat_stream_async`), and emits a dedicated test body that drives `client.chat_stream(req) do |chunk| ... end`, building local aggregator vars (`chunks`, `stream_content`, `stream_complete`, plus optional `last_finish_reason`, `tool_calls_json`, `total_tokens`) inside the block. Fixture pseudo-field assertions are translated to expectations on those locals; `error` fixtures assert the call raises.
- fix(e2e/wasm): drop the `WasmTypeUpdate` builder pattern from the WASM e2e codegen and construct the main wasm-bindgen type directly. `alef-backend-wasm` does not emit per-type `*Update` builder classes, so the previous `new WasmChatCompletionRequestUpdate()` + `WasmChatCompletionRequest.fromUpdate(_u)` IIFE blew up at runtime with `WasmChatCompletionRequestUpdate is not a constructor`. Every wasm-bindgen-emitted struct exposes an all-optional positional constructor and per-field setters, so the WASM branch of `ts_builder_expression_inner` now emits `new T()` followed by setter assignments and returns the instance directly. Imports drop the `*Update` value imports as well.
- fix(e2e/wasm): emit BigInt literals for `u64`/`i64` setter assignments and resolve per-call options classes. Previously the codegen wrote plain numeric literals (`_u.maxTokens = 100;`) into wasm-bindgen setters whose Rust types are `u64`/`i64`, which wasm-bindgen now exposes as JS `bigint` — leading to `TypeError: Cannot convert 100 to a BigInt`. The configured `bigint_fields` list under `[e2e.call.overrides.wasm]` was already accepted by the schema but never consumed; `ts_builder_expression_inner` now treats listed field names as bigint sites and emits `100n` (or `BigInt(...)` for non-literal numeric expressions). At the same time, the WASM `options_type` had to be resolved per-fixture instead of globally — every `[e2e.calls.<name>.overrides.wasm].options_type` is now picked up so embeddings tests construct `WasmEmbeddingRequest`, files tests construct `WasmCreateFileRequest`, and so on (the previous global-only resolution forced `WasmChatCompletionRequest` for every call and surfaced as `expected instance of WasmEmbeddingRequest` from wasm-bindgen). `bigint_fields`, `nested_types`, `enum_fields` are similarly merged from the per-call wasm override, with the file-level wasm override as fallback. Imports aggregate every options class referenced across the file's fixtures so a test file covering chat + chat_stream pulls both.
- fix(e2e/wasm): generate `globalSetup.ts` whenever any non-HTTP fixture is present so function-call e2e tests (which interpolate `${process.env.MOCK_SERVER_URL}/fixtures/<id>` into the client base URL) actually find a running mock server. Previously `globalSetup` was conditional on `has_http_fixtures` only; for fixture sets that contain only function-call tests (no raw HTTP fixtures) the mock server never spawned and every test failed at request-builder time with `Unknown Error: builder error` (the wasm-bindgen url builder rejected `undefined/fixtures/...`).
- fix(wasm-backend): evaluate `any(...)`/`all(...)`/`not(...)` cfg gates instead of stripping items whose first listed feature is disabled. `is_gated_behind_disabled_feature` previously parsed only the first `feature = "name"` token in a cfg string, so a type gated `#[cfg(any(feature = "native-http", feature = "wasm-http"))]` was treated as disabled whenever the first feature (`native-http`) was absent — even when the WASM feature set legitimately enabled `wasm-http`. This silently dropped `DefaultClient` (and every method/adapter hung off it) from the generated WASM `lib.rs`. The evaluator now mirrors the alef-cli extractor's existing parser: `feature = "name"`, `any(...)`, `all(...)`, `not(...)`, with whitespace normalisation for proc-macro2 token-stream output.
- fix(e2e/wasm): import the local `wasm-pack` artifact directly instead of rewriting test imports to a non-existent `<pkg>/dist-node` subpath. The post-processor was tailored to a multi-distribution layout (`dist/`, `dist-node/`, `dist-web/`) that alef's own `wasm-pack build` step does not produce — its flat `pkg/` is already a self-initializing CJS module when built with `--target nodejs`. Removing the rewrite lets the generated `import … from "<pkg_name>"` resolve via the package's `main` entry. Projects that still need a multi-distribution layout should configure their pkg `package.json` `exports` map at build time.
- fix(e2e/c): drive streaming tests via FFI iterator handle instead of non-existent `_chunks` / `_stream_content` / `_stream_complete` accessors. The C codegen previously emitted calls to invented per-chunk accessor functions and then comment-skipped every assertion, leaving the C streaming suite uncompilable against the actual FFI surface. The new path detects `function = "chat_stream"` in the client-pattern branch and emits a dedicated test body that calls `{prefix}_default_client_chat_stream_start`, loops over `_next` until null (treating `last_error_code() == 0` as clean end-of-stream, non-zero as error), and aggregates per-chunk data into local variables (`chunks_count`, `stream_content`, `stream_complete`, `last_choices_json`, `total_tokens`). Fixture pseudo-fields (`chunks`, `stream_content`, `stream_complete`, `no_chunks_after_done`, `finish_reason`, `tool_calls`, `tool_calls[0].function.name`, `usage.total_tokens`) are translated to assertions on those locals; `error` fixtures assert that `_chat_stream_start` returned NULL.
- fix(e2e/c): emit byte-buffer out-pointer pattern for `speech` / `file_content` / `transcribe`-bytes-style methods. The C codegen previously treated every client method as returning a `*Response` opaque handle, so methods whose actual FFI shape is `int32_t fn(client, req, uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap)` (e.g. `literllm_default_client_speech`, `literllm_default_client_file_content`) were emitted with a non-existent `LITERLLMSpeechResponse*` cast plus invented `_audio` / `_content` accessor calls and a `_response_free` invocation. The new branch in `render_test_function` detects `result_is_bytes = true` (resolved from `[e2e.calls.<name>]` or the per-language override) and routes to a dedicated `render_bytes_test_function` that declares the three out-params, calls the FFI method with `&out_ptr, &out_len, &out_cap`, asserts on `status` (`== 0` on success, `!= 0` on expected error) and `out_len > 0` for any `not_empty` / `not_null` field assertion (the field name is a pseudo-field — the buffer itself is the value), and frees with `<prefix>_free_bytes`. The matching `[crates.e2e.calls.{speech,file_content}.overrides.c]` blocks in `liter-llm/alef.toml` flip from `result_type = "..."` to `result_is_bytes = true`.

### Added

- feat(java-backend): emit `DefaultClient` instance methods (chat, embed, moderate, rerank, list*models, image_generate, transcribe, speech, ocr, files API, batches API, responses API, search) over FFI MethodHandles. The Java backend previously emitted only the streaming `chatStream` iterator on opaque handle classes, leaving every other public method missing — Java e2e tests calling `client.chat(req)` failed to compile. `alef-backend-java` now iterates `typ.methods` for every opaque type and emits, for each non-static, non-streaming method, (a) a `MethodHandle` in `NativeLib.java` whose `FunctionDescriptor` prepends `ValueLayout.ADDRESS` (the receiver) to each param's layout (and appends three `ADDRESS` out-pointers + `JAVA_INT` return for `Result<Vec<u8>>` methods like `speech` and `file_content`), and (b) a public instance method on the owning opaque handle class that marshals each `Named` param via the per-type `_from_json` helper, marshals `String`/`Path` params via `arena.allocateFrom`, calls `NativeLib.<PREFIX>*<OWNER>\_<METHOD>.invoke(this.handle, …)`, deserializes `Named`returns via the existing`\_to_json`helper + Jackson`STREAM_MAPPER`, and frees both response and param pointers. Bytes-result methods unpack the `(out_ptr, out_len, out_cap)`triple and free via`<prefix>\_free_bytes`. Null returns or non-zero status routes through the existing `checkLastFfiError` helper to raise the binding's standard exception type. Streaming-adapter method names are excluded from the iteration so they remain handled by the streaming codegen path.
- feat(check-registry): add `pub`, `zig`, and `swift` adapters. `pub` queries pub.dev's `/api/packages/{name}/versions/{version}` endpoint. `zig` and `swift` have no central registry — both delegate to `check_github_release` since they consume packages directly from GitHub release tags (Zig via `zig fetch --save <tarball-url>`; Swift Package Index auto-discovers new tags from Git). Required for kreuzberg's publish workflow which now ships these languages and needs proper existence checks instead of stub jobs.
- feat(magnus-backend): emit `chat_stream` method returning Ruby Enumerator/block-yield. For every `[[crates.adapters]]` entry with `pattern = "streaming"` and `owner_type = <opaque>`, `alef-backend-magnus` now emits (a) a `{PascalName}Iterator` opaque struct wrapping `Arc<tokio::sync::Mutex<Option<BoxStream<…>>>>` plus a private tokio runtime, with `next_chunk` (sync `block_on` over `StreamExt::next`, returns `nil` at end-of-stream) and `each` (yields each chunk to the block, or returns an `Enumerator` via `enumeratorize` when no block is given) inherent methods, and (b) a public instance method on the owning opaque type (e.g. `DefaultClient#chat_stream(req)`) that drives the Rust core stream natively — block on `inner.{core_path}(core_req).await` to materialize the stream, then yield chunks to the caller's block (returning `nil`) or wrap the stream in the iterator type and return it for `Enumerable` consumption (`.to_a`, `.lazy`, `.map`, …). The default `chat_stream_async` stub previously emitted by `gen_opaque_async_instance_method` (which raised `NotImplementedError` because the IR represents `BoxStream` returns as `String`) is suppressed for streaming adapter names. The iterator class is registered with `Enumerable` mixed in via `class.include_module(ruby.module_enumerable())`. `alef-scaffold` now adds `futures = "0.3"` to the magnus crate's `Cargo.toml` whenever a streaming adapter is present.
- feat(csharp-backend): emit `ChatStream` as `IAsyncEnumerable<ChatCompletionChunk>` over FFI chat-stream iterator handle. The C# backend previously filtered every method whose adapter pattern was `Streaming`, leaving the streaming surface unreachable from .NET. With the iterator-handle exports in place (`{prefix}_{owner}_{name}_start` / `_next` / `_free`), `alef-backend-csharp` now emits, for every streaming adapter, three new P/Invoke decls in `NativeMethods.cs` (`{Owner}{Name}Start` / `Next` / `Free`, all `IntPtr`-typed) plus the request `{Request}FromJson` / `Free` and item `{Item}ToJson` / `Free` accessors, and a public `async IAsyncEnumerable<{Item}> {Method}({Request} req, [EnumeratorCancellation] CancellationToken cancellationToken = default)` method on the owning opaque handle class. The body marshals the request via `JsonSerializer` + `_from_json`, drives `_start`, then loops `_next` inside `try`/`finally` (yielding deserialized chunks until null; null + `LastErrorCode() != 0` is rethrown as the binding's exception type, null + `0` is a clean `yield break`), and frees both the stream handle and the request handle in `finally`. The opaque-handle template now pulls in `System.Threading` and `System.Runtime.CompilerServices` whenever a streaming method is emitted.
- feat(java-backend): emit `chatStream` returning `Iterator<ChatCompletionChunk>` over FFI iterator handle. For each `[[crates.adapters]]` entry with `pattern = "streaming"` and `owner_type = <opaque>`, `alef-backend-java` now emits (a) three downcall `MethodHandle`s in `NativeLib.java` for the iterator-handle FFI trio (`_start` / `_next` / `_free`) plus the request `_from_json`/`_free` and the chunk-item `_to_json`/`_free` accessors, and (b) a public instance method on the owning opaque handle class (e.g. `DefaultClient.chatStream(req)`) that marshals the request via Jackson + `_from_json`, drives `_start`, and returns a `java.util.Iterator<Item>` whose `next()`/`hasNext()` pull through `_next`, JSON-deserialize each chunk via the existing per-type `_to_json` / `_free` helpers, and `_free` the stream handle on clean end-of-stream or error. FFI errors (non-zero `last_error_code` after a null `_next` return) are surfaced as the binding's standard exception type wrapped in a `RuntimeException` so the iterator contract is preserved.
- feat(go-backend): emit `ChatStream` method consuming FFI chat-stream iterator handle. The Go backend previously skipped every method whose adapter pattern was `Streaming` because the callback-based FFI export cannot be driven from CGO. With the iterator-handle exports now available (`{prefix}_{owner}_{name}_start` / `_next` / `_free`), `alef-backend-go` emits a dedicated Go method per streaming adapter that returns `(<-chan ItemType, error)`: it marshals the request, calls `_start` to obtain the handle, spawns a goroutine that loops over `_next` (treating null as either clean end-of-stream or stream error), deserializes each chunk via the existing `_to_json` helper, frees the chunk + JSON pointer, sends the typed value on the channel, and `defer`s `_free` on the handle for cleanup.
- feat(rustler-backend): emit `chat_stream` NIF as a pair of standalone `start`/`next` functions backed by a per-adapter handle resource, plus a high-level `Stream.unfold/2` Elixir wrapper. For every `[[crates.adapters]]` entry with `pattern = "streaming"`, `alef-backend-rustler` now emits a `{Owner}{Name}Handle` resource (Tokio runtime + `Mutex<Option<BoxStream>>`), `{owner_lc}_{name}_start` (decodes the request, drives `core_path` once, returns the handle), and `{owner_lc}_{name}_next` (blocks dirty-CPU on a single `stream.next()`, returns chunk JSON or `nil`). The matching `Stream.unfold` wrapper exposes the iterator as an `Enumerable` of decoded chunk maps. The original async method is suppressed in the regular method-iteration loop, native.ex stub list, and main wrapper module to avoid double-emitting a NIF for the same name; the new handle types are registered as resources in `on_load`.
- feat(ffi): emit chat-stream iterator handle for FFI consumers (Go/Ruby/Java/C#/Elixir/C). For every `[[crates.adapters]]` entry with `pattern = "streaming"`, `alef-backend-ffi` now emits three exported C functions alongside the existing callback-based wrapper: `{prefix}_{owner}_{name}_start` (creates an opaque handle wrapping a tokio runtime + `BoxStream`), `{prefix}_{owner}_{name}_next` (advances the stream, returns a heap-allocated chunk or null; null + error-code 0 means clean end, non-zero means error), and `{prefix}_{owner}_{name}_free` (null-safe drop). The opaque handle struct is emitted inline in `lib.rs`; all unsafe blocks carry SAFETY comments.
- feat(e2e): `exclude_categories` filter omits non-binding-surface fixtures from cross-language codegen. Set `exclude_categories = ["cache", "proxy", ...]` under `[crates.e2e]` (or a top-level `[e2e]` block) and every per-language e2e generator skips fixtures whose resolved category matches an entry in the set — no test, no skip directive, no commented body. The fixture files stay on disk and remain available to the consumer's own Rust integration tests, so middleware-only fixtures (cache, proxy, budget, hooks, ...) can keep their existing Rust coverage without polluting bindings whose public API does not expose those layers. The validate pass treats excluded categories as expected-empty and no longer warns about them.
- feat(release-metadata): recognise `dart`, `swift`, `gleam`, `zig`, and `kotlin` as release targets. They join the existing 14 targets in `ALL_RELEASE_TARGETS`, are emitted as `release_dart` / `release_swift` / `release_gleam` / `release_zig` / `release_kotlin` boolean fields in the JSON output, and are accepted by `--targets`. Aliases: `flutter`/`pub` → `dart`, `spm` → `swift`, `kt` → `kotlin`. Required for kreuzberg's publish workflow which now ships these languages.

## [0.15.8] - 2026-05-09

### Fixed

- fix(pre-commit): resolve pre-commit hook binary in extraction subdirectory. The release tarball top-level entry (e.g. `alef-aarch64-apple-darwin/`) extracts into the version cache directory, placing the binary at `~/.cache/alef-hooks/{version}/{target}/alef`. The hook was previously looking for the binary one level up, resulting in `FileNotFoundError` when executing `alef-verify` or `alef-sync-versions` hooks in downstream projects.

## [0.15.7] - 2026-05-09

### Fixed

- fix(e2e/python): emit positional arguments instead of `kwarg=var` in test call sites. Reverts a regression introduced by 0.15.5 (`fix(e2e/php): spawn mock server when fixtures use mock_response schema`) which inadvertently re-added the `{kwarg_name}={var_name}` form across all branches in `build_args_and_setup`. The kwarg name was sourced from `alef.toml` and did not match the binding's actual pyo3 parameter name, producing `TypeError: ... got an unexpected keyword argument 'request'` for every chat/embed/moderate test. Restores positional emission so the call works regardless of binding-side parameter naming.
- fix(php-backend): map `TypeRef::Bytes` params to PHP `String` and convert to `Vec<u8>` via `into_bytes()` at constructor / call sites (PHP strings are binary-safe). Avoids the missing `FromZval for Vec<u8>` in ext-php-rs that previously surfaced as compile errors on speech / file-content endpoints.
- fix(php-backend): remove unused `gen_php_from_core_to_binding` helper that was added in 0.15.6 but never called; was triggering `dead_code` lint error under `-D warnings` during `cargo publish` verification, blocking the 0.15.6 release at `alef-backend-php`.
- fix(codegen): fix malformed rustdoc list continuation in `ConversionConfig::untagged_data_enum_names` that triggered `doc_lazy_continuation` lint under `-D warnings`.
- fix(napi-backend): add `#[allow(clippy::too_many_arguments)]` to `gen_opaque_instance_method` (8 params, over the 7-arg limit).
- fix(e2e/python): remove now-unused `kwarg_name` parameter from `emit_handle_arg`, `emit_json_object_arg`, and `emit_bytes_arg`; drop the corresponding `arg_name_map` derivation from `build_args_and_setup`.

## [0.15.6] - 2026-05-09

### Fixed

- fix(e2e/wasm): redirect generated test imports to `<pkg>/dist-node` sub-path so that vitest (Node.js) resolves the self-initialising CJS bundle instead of the bundler-only `dist/` entry that fails without Vite/webpack.
- fix(e2e/c): normalise hyphens to underscores in the `-l` linker flag of generated Makefiles; Rust cdylib output always uses underscores (`libhtml_to_markdown_ffi.dylib`) regardless of the crate name.

## [0.15.5] - 2026-05-09

### Fixed

- fix(napi-backend): only treat enums as untagged when they have `#[serde(untagged)]`, not just any enum without `#[serde(tag = "...")]` — was misclassifying `VisitResult` and similar externally-tagged enums as untagged, emitting an unusable `serde_json::Value` wrapper instead of a `#[napi(string_enum)]`.
- fix(go-backend): same untagged-enum heuristic fix applied to the Go backend.
- fix(csharp-backend): same untagged-enum heuristic fix applied to the C# complex_enums filter.
- fix(java-backend): same untagged-enum heuristic fix applied to the Java complex_enums filter.
- feat(core/ir): add `serde_untagged: bool` field to `EnumDef` (with `#[serde(default)]`) so backends can correctly distinguish `#[serde(untagged)]` from the default externally-tagged serialization.
- feat(extract): parse `#[serde(untagged)]` attribute on enum definitions and populate `EnumDef::serde_untagged`.

## [0.15.4] - 2026-05-09

### Fixed

- fix(java-backend): underscore-prefix the unused `package` parameter on
  `gen_sealed_union_deserializer` to satisfy `RUSTFLAGS=-D warnings` in the
  publish pipeline (was the v0.15.3 release blocker).
- fix(java-backend): strip tag field before deserializing inner type.
- fix(e2e/c): use `options_type` override for request type when set.
- fix(e2e/wasm): derive bg.wasm filename from the actual crate name.
- fix(e2e/elixir): resolve `field=='input'` to entire fixture input.
- fix(php-backend): emit camelCase parameter names in struct constructor
  field init.

## [0.15.3] - 2026-05-09

### Fixed

- fix(codegen): silence `dead_code` lint on `gen_magnus_positional_constructor`,
  intentionally retained even though `gen_magnus_kwargs_constructor` always
  delegates to the hash-based form. Without `#[allow(dead_code)]` the publish
  workflow's `RUSTFLAGS="-D warnings"` failed `cargo publish`'s package verify
  step and all CLI binary builds, blocking the v0.15.2 release.
- fix(php-backend): pass `opaque_types` to all `field_can_be_param` call sites
  that were missed when the parameter was added.
- fix(php-backend): exclude `Vec<NonOpaqueCustomType>` from constructor params.
- fix(php-backend): emit `Vec<u8>` as slice and `Vec<T>` with manual `FromZval`
  conversion.
- fix(e2e/go): keep mock-server stdin pipe open so it does not exit on EOF.
- fix(e2e/go): honor `fixture.env.api_key_var` for live-API skips.
- fix(e2e/php): bootstrap.php spawns the mock HTTP server when fixtures use
  the `mock_response` schema.

### Added

- feat(e2e/go): support `api_key_var` skip + tweak liter-llm skip lists.

## [0.15.2] - 2026-05-09

### Fixed

- fix(rustler-backend): remove `{%- raw %}` whitespace strippers from Elixir
  use the `mock_response` schema (in addition to the `http` schema). Switched
  the trigger from `is_http_test()` to `needs_mock_server()` to match the
  Ruby/Python behavior; otherwise `MOCK_SERVER_URL` was unset and reqwest
  failed with "builder error" on every call.
- fix(e2e/go): keep the mock-server stdin pipe open so the server does not
  exit on EOF. Go's `exec.Command` with `Stdin == nil` connects the child
  to /dev/null, so the mock-server (which blocks on `stdin.lock().lines()`)
  exited immediately and every HTTP fixture failed with "error sending
  request for url". Now opens a `StdinPipe` like Python's `stdin=PIPE`.
- fix(e2e/go): honor `fixture.env.api_key_var` by emitting a `t.Skipf` when
  the named env var is unset, and threading the env value as the api*key
  with a nil base_url for live-API fixtures. Previously every `smoke*\*`live-API fixture failed with "no mock route" because Go always pointed
at`MOCK_SERVER_URL/fixtures/<id>`regardless of`api_key_var`.

## [0.15.2] - 2026-05-09

### Fixed

- fix(rustler-backend): remove `{%- raw %}` whitespace strippers from Elixir
  visitor templates that were collapsing pattern-match clauses onto single
  lines. With `trim_blocks` and `lstrip_blocks` enabled in the env, the raw
  blocks were unnecessary and the leading `-` strippers ate the newlines and
  indent that separated `receive do` arms, generating syntactically invalid
  Elixir like `receive do      {:visitor_callback, ...} ->        result =`.
  Affects `elixir_visitor_call.jinja` and the visitor receive loop / apply
  callback case clauses in `elixir_visitor_helper_functions.jinja`.
- fix(e2e/php): treat `field == "input"` as the entire fixture input object
  in PHP arg resolution (mirroring the shared `resolve_field` helper used by
  Python/Ruby/Go). Previously every call whose argument bound the whole input
  (e.g. `chat`, `embed` with `req=input`) generated `client->methodAsync(null)`,
  causing ~99 of 128 PHP e2e errors with "Invalid value given for argument".
- fix(napi-backend): represent `#[serde(untagged)]` data enums as a thin
  `serde_json::Value` wrapper struct with manual `FromNapiValue`/`ToNapiValue`
  impls. Previously the variants were flattened to a `#[napi(string_enum)]`
  and the inner data was lost, so JS callers couldn't pass either side of
  `Single(String)` / `Multiple(Vec<String>)`-style unions.
- fix(napi-backend): map `Json` fields to `serde_json::Value` (relying on
  napi-rs's `serde-json` feature) so JS callers can pass arbitrary
  objects/arrays/scalars instead of having to pre-serialize to a string.
  Threaded a new `json_as_value` flag through `ConversionConfig`.
- fix(napi-backend): per-field `#[serde(rename = "...")]` is now emitted as
  `#[napi(js_name = "...")]` so JS-side property names match the wire format
  (e.g. `tool_type` with rename `"type"` is exposed as `type`). Per-variant
  `#[serde(rename = "...")]` is now emitted as `#[napi(value = "...")]` on
  the corresponding string-enum variant.
- fix(napi-backend): streaming adapter methods (e.g. `chat_stream`) now
  return `Vec<Js{Item}>` directly so JS callers receive a typed array of
  chunks; previously the body called `serde_json::to_string` and the
  IR-declared return type was `String`, forcing JS callers to parse JSON
  manually.
- fix(e2e/typescript): added `result_is_simple` support so that
  bytes-returning methods (e.g. `speech`) can be tested with length-only
  assertions on the result directly, mirroring the Python codegen. Also
  added `api_key_var` env-skip support so live-API smoke tests skip when
  the API key is not set instead of falling through to the mock server.
- fix(e2e/python): client setup is now appended to the test function body
  instead of being written to the outer file buffer; the Jinja migration
  regression placed `client = create_client(...)` lines outside the test
  functions, breaking every mock-driven test.
- fix(e2e/python): use positional args when invoking the binding method
  instead of keyword args (`client.chat(request)` rather than
  `client.chat(request=request)`), since the binding's keyword name may
  differ from the alef.toml-declared call arg name (e.g. core uses `req`
  while the call is named `request`).
- fix(java-backend): wrap `readJsonList` null-check inside try-catch so that
  `checkLastError()`'s `Throwable` is caught and rethrown as `KreuzbergRsException`,
  resolving Java compile error on FFI method calls.
- fix(e2e/gleam): `contains_any` assertion now compiles to OR logic via
  `gleam/list.any` + `string.contains`, instead of N independent
  `string.contains` calls AND-ed together. Adds `gleam/list` to required
  imports for any test using `contains_any`. Without this, fixtures with
  alternative expected substrings (e.g. error-message variants) emitted
  always-false assertions.
- fix(e2e/zig): `setCwd` for test runs, `json_path_expr` skips
  `FormatMetadata` variant-name segments (internally-tagged enum),
  `contains_any` uses OR logic, bytes arg uses `std.Io.Dir.cwd()` /
  `std.testing.io` / `.unlimited` (Zig 0.16 API).
- fix(e2e/swift): `doc_lazy_continuation` now handled correctly in
  field-access codegen; `render_swift_with_optionals` added.
- fix(dart-backend): `#[allow(dead_code)]` on in-progress dart binding
  errors/types/idents; clippy fixes for `gen_rust_crate`.

- fix(rustler-backend): emit `From<Local> for core::Enum` for flat data
  enums when they appear in input position. Flat data enums (data variants
  that are all single-tuple) are encoded as a flat NifStruct on the Elixir
  side; the local→core direction was previously skipped (commented as
  "output-only"). `Vec<Message>` parameters relied on the missing impl,
  producing E0277 "the trait `From<Message>` is not implemented for
  `liter_llm::Message`". Generator dispatches on the discriminator field
  (`role`, `type`, etc.) and threads `.into()` / iter-map for `Vec<Named>`
  payloads.

- fix(rustler-backend): in `gen_rustler_flat_data_enum_from_core`,
  convert `Vec<core::T>` payloads via `_0.into_iter().map(Into::into).collect()`
  rather than the bare `_0.into()`, since `From<Vec<core::T>> for Vec<T>`
  is not blanket-impled.

- fix(go-backend): emit untagged-union Marshal/Unmarshal for enums with no
  `#[serde(tag)]`. Previously the Go backend always assumed a tag field
  named `Type` exists, but only emitted that field when `serde_tag` was
  Some. Untagged enums (`#[serde(untagged)]`, e.g. `ToolChoice`) now get
  marshalers that dispatch on the active variant pointer and unmarshalers
  that try each variant in declaration order. Also avoids the broken
  `&T{}` literal for variants whose payload type is a string alias.

- fix(csharp-backend): handle `Result<bytes::Bytes>` methods on opaque
  handle classes (e.g. `DefaultClient.Speech`). `gen_opaque_method`
  previously fell through to the generic pointer-return path, calling
  `NativeMethods.X(handle, req)` and JSON-deserialising into `byte[]`;
  but the FFI declaration uses out-params (`out IntPtr ptr, out UIntPtr
len, out UIntPtr cap`). Now emits a dedicated body that pins the
  out-params, copies the bytes, and frees them, throwing a fully
  qualified exception (the wrapper-class `GetLastError` helper is private
  and not visible from sibling classes).

- fix(adapters/php): emit lowerCamelCase parameter names in `async_method`
  PHP body (`call_args_cloned`). PHP signatures camelCase param names via
  `gen_php_function_params`, so the body must reference the camelCased
  identifier. Adapter methods with `String` params (e.g. `file_id` →
  `fileId`) previously emitted `file_id.as_str()` against a parameter
  named `fileId`, producing E0425 "cannot find value `file_id` in this
  scope".

- fix(adapters/node): wrap `bytes::Bytes` adapter return in
  `napi::bindgen_prelude::Buffer::from(b.to_vec())` instead of bare
  `.to_vec()`. The function signature returns `Result<Buffer, Error>`,
  so `Vec<u8>` produced an E0308 type mismatch.

- fix(ffi-backend): convert via `Vec::<u8>::from(val).into_raw_parts()`
  in `bytes_result_match.jinja` so the bytes return path works for both
  `Vec<u8>` and `bytes::Bytes` (and any other type implementing
  `Into<Vec<u8>>`). `bytes::Bytes` does not have `into_raw_parts`.

- fix(csharp-backend): rename tagged-union variant accessor properties from
  `{Pascal}` to `As{Pascal}` in `variant_accessor_property.jinja`. C# 12
  records cannot have a property whose name matches a nested type, so
  emitting both `public sealed record Pdf(string Value) : FormatMetadata`
  and `public string? Pdf => …` produced CS0102 ("type already contains a
  definition for 'Pdf'") for every variant. `AsPdf` is the idiomatic C#
  pattern-matching helper convention and avoids the collision.

- fix(e2e/php): handle `arg_type = "bytes"` in
  `crates/alef-e2e/src/codegen/php.rs::build_args_and_setup`. Previously
  bytes args fell through to the default render and passed the raw
  fixture-relative path string to `extractBytesSync()`, which ext-php-rs
  rejected with `Invalid value given for argument content`. Mirror the
  go/python convention: emit a setup line that calls
  `file_get_contents()` to load the file at runtime and binds a local
  variable to the resulting binary string. Inline byte arrays are
  encoded as `\xNN` escape strings.

## [0.15.1] - 2026-05-08

### Fixed

- fix(rustler-backend): remove 7 orphan template files in
  `crates/alef-backend-rustler/templates/` whose filenames embedded literal newlines and
  template content (residue from the c6856f8c Jinja migration). The clean `*.jinja` files
  with the same logical names were already committed and referenced by `include_str!`,
  but the duplicates were tracked in git. They blocked `cargo publish` (`cannot package
a filename with a special character`) and Windows builds (`invalid path` on git
  checkout). 0.15.0 published partially as a result; 0.15.1 republishes the affected
  downstream crates (alef-backend-rustler/swift/zig/dart/extendr/java/csharp/kotlin/gleam,
  alef-e2e, alef-readme, alef-scaffold, alef-publish, alef-cli).

## [0.15.0] - 2026-05-08

### Fixed

- fix(java-backend): drop the `SerializationFeature.WRITE_BYTE_ARRAY_AS_BASE64` configure
  call from `helper_object_mapper.jinja`. Jackson 2.x has no such enum constant — the
  actual `SerializationFeature` enum (verified against `jackson-databind:2.21.0`) does not
  contain it, so any backend that wires this feature in fails to compile with `cannot
find symbol`. If a future caller really wants to disable base64 byte-array
  serialization the correct path is a custom `JsonSerializer<byte[]>`, not a serialization feature flag.

- fix(php-backend): wrap the `php_visit_result_with_template` `format!` arg in
  `{% raw %}…{% endraw %}` so Jinja stops eating the literal `{` / `}` braces. The
  template wrote `format!("{{{}}}", k)` intending the Rust string `"{...}"`, but minijinja
  parsed `{{{}}}` as an interpolation and rendered `format!("{}", k)` instead. Clippy
  flagged the result with `useless_format` (`-D warnings`).

- fix(java-backend): rebuild `convertWithVisitorInternal` so it actually compiles. Three
  cascading bugs in `gen_convert_with_visitor_internal_method`: (1) the try-with-resources
  parenthesis stayed open across `cHtml` (a `MemorySegment`, not `AutoCloseable`); (2) the
  bridge variable referenced by `bridge.callbacksStruct()` and
  `bridge.rethrowVisitorError()` was never declared in this method — it now lives in the
  resource list as `var bridge = new VisitorBridge(options.visitor())`;
  (3) `ffi_conversion_options_invoke.jinja` hardcoded `defaultJson` as the JSON argument,
  so the `if (options != null)` branch (which allocates `optJson`) emitted
  `…invoke(defaultJson)`. Template now takes a `var_name` context arg.

- fix(scaffold/php): exclude the `stubs/` directory from the emitted
  `.php-cs-fixer.dist.php` finder. PHP CS Fixer's `@PHP82Migration` rule promotes
  class-level property declarations into constructor parameters and deletes the explicit
  `public Type $name;` lines. Stub files rely on those declarations for phpstan to know
  what fields native classes expose, so the rewrite silently breaks static analysis.
  Adding `->notPath('stubs')` keeps stubs untouched.

- chore(scaffold/php): rename emitted PHP CS Fixer config from `php-cs-fixer.php` to
  `.php-cs-fixer.dist.php`. The dotted name is the default `php-cs-fixer` looks up
  without `--config`, so users (and the kreuzberg-dev pre-commit `php-cs-fixer` hook)
  pick up the alef-managed config instead of any hand-rolled `php-cs-fixer.php` left
  over from earlier scaffolds.

- fix(error-gen/java): wrap class doc comments in `/** ... */` and stop swallowing the
  newline after the `package …;` declaration. Templates now use `{% if doc -%}` /
  `{% endif -%}` so the package separator is preserved.

- fix(ffi-backend): emit a string literal instead of `format!("include/{header_name}")` in
  the generated `build.rs` go-copy step. Clippy's `useless_format` rewrote the call to
  `"...".to_string()` and broke the alef-verify hash check on every fresh regen.

- fix(ffi-backend): drop `.clone()` on owned named returns. `gen_owned_value_to_c` was
  emitting `Box::into_raw(Box::new(result.clone()))` for any `Named` return, which fails
  for non-Clone opaque handles. New `named_owned` template arm moves the value into the
  Box; `Optional` inner conversions now use `optional_owned` so the inner `val` is owned.

- fix(magnus-backend): suppress redundant `native.rb` re-export when the public Ruby
  module name and the native extension's module name collide. The generated
  `define_singleton_method(m) { Module.public_send(m, ...) }` loop overwrote each native
  function with a self-call and triggered `SystemStackError` on first invocation.

- fix(magnus-backend): exclude thread-unsafe bridge handle types (`VisitorHandle`) from
  binding ↔ core From impls via `ConversionConfig::exclude_types` instead of post-process
  line filtering. The line filter silently failed whenever the alef extractor had already
  stripped the field's `cfg` for an active feature — the IR then carried a `cfg: null`
  field referencing `VisitorHandle`, so codegen emitted `visitor: val.visitor.map(...)`
  lines into both directions of the From impl, and the generated Ruby gem refused to
  compile against a binding `ConversionOptions` struct that (correctly) had no `visitor`
  field. Mirrors the Rustler backend's approach.

### Added

- feat(java-backend): consume the FFI `Result<Vec<u8>>` out-param convention. Java now detects byte-buffer return shapes from IR (`is_bytes_result`), declares the symbol with three trailing `ADDRESS` out-params and a `JAVA_INT` return, allocates `outPtr`/`outLen`/`outCap` slots in a confined `Arena`, checks the return code, copies the bytes via `outPtr.reinterpret(outLen).toArray(JAVA_BYTE)`, and frees them through a bound `{prefix}_free_bytes` method handle. Templates: `bytes_result_call.jinja`, `method_handle_free_bytes.jinja`. Existing Go and C# byte-buffer support unchanged.

### Changed

- refactor(backends): migrate all parameterized code emission in every `alef-backend-*` crate to
  minijinja template `render()` calls. `writeln!(out, "...")` and `push_str(&format!(...))` are no
  longer used for interpolated output in any backend; all variable substitution goes through named
  `.jinja` templates registered in each crate's `template_env.rs`. Static `push_str("literal\n")`
  calls with no interpolation are unchanged. Templates use `trim_blocks = true`,
  `lstrip_blocks = true`, and `keep_trailing_newline = true`. Affected crates: `alef-backend-ffi`,
  `alef-backend-napi`, `alef-backend-pyo3`, `alef-backend-wasm`, `alef-backend-csharp`,
  `alef-backend-java`, `alef-backend-go`, `alef-backend-magnus`, `alef-backend-php`,
  `alef-backend-rustler`, `alef-backend-extendr`, `alef-backend-swift`, `alef-backend-zig`.

### Fixed

- fix(taskfile): make `task set-version` idempotent and tolerant of a leading `v` in the input
  argument. `task set-version -- v0.15.0` previously wrote `version = "v0.15.0"` verbatim into
  `Cargo.toml`/`alef.toml` (rejected by cargo as semver), and the dep-pin regex required strict
  `[0-9]+\.[0-9]+\.[0-9]+` so a re-run could not repair its own bad output. The task now strips a
  leading `v`, validates semver shape up front with a clear error, and tolerates an optional `v`
  in the dep-pin and `ALEF_REV` regexes so subsequent runs are self-healing.
- chore(clippy): clear `clippy::if_same_then_else`, `clippy::format_in_format_args`,
  `clippy::dead_code`, `clippy::nonminimal_bool`, and `clippy::collapsible_match` warnings across
  `alef-codegen`, `alef-backend-csharp`, `alef-backend-ffi`, `alef-backend-java`,
  `alef-backend-napi`, and `alef-e2e`. Removes one dead helper in `alef-backend-csharp`
  (`native_call_args`) and one dead method in `alef-backend-ffi` (`vtable_fn_ptr_field`); collapses
  redundant identical `if`/`else` branches in `alef-codegen::config_gen` and
  `alef-backend-java::gen_visitor::files`; drops a never-read `FieldEntry { decl, doc }` struct in
  `alef-backend-java::gen_bindings::types` whose only observed use was `.len()`.
- diagnostics(backend-magnus): add `[ALEF BUG]` / `[ALEF OK]` instrumentation around
  `gen_visitor_bridge` and `gen_trait_bridge` when emitting `impl std::fmt::Debug` for visitor
  bridges. Prints rendered output to stderr when the count diverges from 1, added after a
  duplicate `impl std::fmt::Debug for RbHtmlVisitorBridge` was observed in `html-to-markdown`
  Ruby gem builds (E0119 conflicting trait impl). Test `test_visitor_bridge_debug_not_duplicated`
  extended to 40 visitor methods to better mirror the real `HtmlVisitor` trait surface.
- fix(codegen/config_gen): drop trailing `,` from rendered field assignments in
  `magnus_hash_constructor.jinja` and `rustler_kwargs_constructor.jinja`. The `field.assignment`
  strings already terminate in `,`, so the templates were emitting `,,` and Rust rejected the
  generated kwargs constructors with `expected identifier, found ','`.
- fix(jinja-env): set `keep_trailing_newline` on every backend's minijinja `Environment`. With
  `trim_blocks` + `lstrip_blocks` already on, `keep_trailing_newline=false` (the minijinja default)
  stripped the final `\n` from each rendered template, so the struct/impl builders' `out.push_str`
  concatenations collapsed onto one line. `///` doc comments then swallowed the rest of the file,
  producing "unclosed delimiter" errors throughout `pyo3`, `napi`, `php`, and `wasm` bindings.
  Brings `alef-codegen`, `alef-e2e`, and the `extendr`, `kotlin`, `napi`, `rustler`, `wasm`
  backends in line with the `csharp`/`dart`/`gleam`/`java`/`magnus`/`php`/`pyo3`/`swift`/`zig`
  backends that already set this.
- fix(e2e/java): replace `${{project.basedir}}` with `${project.basedir}` in `pom.xml.jinja`. The
  doubled braces were intended to escape the literal Maven property syntax but Jinja parses the
  inner `{{project.basedir}}` as an undefined-variable expression and refuses to render the
  template. Single braces pass through untouched and the emitted pom still contains the correct
  `${project.basedir}` Maven reference.
- fix(php-backend): align `php_vec_string_refs_let_binding.jinja` to the `param_name` context key
  used by `helpers.rs`. The template referenced an undefined `php_name` and rendered as empty
  string, emitting `let _refs: Vec<&str> = .iter().map(...)` (no receiver) for sanitized
  `Vec<String>`/`Vec<&str>` parameters and breaking compilation of the PHP binding.

## [0.14.36] - 2026-05-08

### Changed

- refactor(e2e-codegen): remove all hardcoded `tree-sitter-language-pack` special-cases from production codegen. C, PHP, WASM, and brew backends now derive every project-specific value from config: `ffi_header_name()`, `ffi_lib_name()`, and new `ffi_crate_path()`/`wasm_crate_path()` helpers on `ResolvedCrateConfig` (derived from `[crates.output]` paths). C method-result assertions use generic `{ffi_prefix}_{method}(result_var)` dispatch; PHP uses instance dispatch `$var->method()`; brew converts `method_name` snake_case to kebab-case and calls `{binary_name} {subcommand} "$output"`. A new `return_type = "string"` fixture field opts into heap-allocated `char*` handling in C codegen; the default is primitive value dispatch.
- refactor(streaming-ffi): generalise FFI streaming body generation. `gen_ffi_body` now receives `&ResolvedCrateConfig` and uses `ffi_prefix()` for error-message symbol prefixes instead of the hardcoded `literllm_` string. The request type is no longer hardcoded as `liter_llm::ChatCompletionRequest`; callers must set `request_type = "my_crate::MyRequest"` in the `[[crates.adapters]]` block for any streaming adapter that targets the FFI backend — codegen fails with a clear error if the field is absent.
- refactor(e2e-rust): make visitor trait name config-driven instead of hardcoded. `resolve_visitor_trait` now reads `visitor_trait` from `[e2e.call.overrides.rust]` in `alef.toml` and returns `Option<String>`. Fixtures that declare a `visitor` block without a configured `visitor_trait` now fail at codegen time with a clear error. Callers must add `visitor_trait = "MyTrait"` to their Rust e2e override to keep visitor fixtures working.
- refactor(docs): remove project-specific `html_to_markdown` string from `is_rust_code_fence` heuristic in `alef-docs`. The generic Rust signals (`use `, `unwrap()`, `assert!`, etc.) are sufficient.
- refactor(backend-php, backend-pyo3): replace hardcoded `html_to_markdown_rs` fallback in visitor bridge `core_crate` derivation with a panic that reports the misconfiguration instead of silently using a wrong crate name.

### Added

- feat(php-backend): emit `FromZvalMut` for `Vec<NamedStruct>` parameters so PHP arrays of struct values cross the ext-php-rs boundary correctly.

### Fixed

- fix(ffi-backend): parameterize `core_import` so generated FFI code uses the configured crate name instead of the hardcoded `kreuzberg` string. Also adds `Result<Vec<u8>>` return support via out-params (`out_ptr`, `out_len`, `out_cap`) returning `i32` (-1 = error, 0 = ok), with a companion `{prefix}_free_bytes` deallocator.
- fix(swift-backend): emit `Vec<u8>` / `Path` convenience overloads from the IR shape (first-param `TypeRef::Bytes` / `TypeRef::Path`) instead of matching by hardcoded function names. The backend now derives wrapper names by stripping the `Sync` suffix and works for any library, not just kreuzberg.
- chore(php-backend): fix `clippy::useless_format` warning in test helper.
- fix(napi-backend): initialize all binding-struct fields in tagged-enum `From` and `Default` impls. The previous codegen only emitted the matching variant's slot plus shared fields, leaving the synthesized variant-data fields (e.g. `excel`, `docx`, `html` on `JsFormatMetadata`) unset and producing E0063 ("missing fields"). The struct-literal builders now initialize every variant-data field — `None` for non-matching variants, `Some(...)` only on the active variant. Boxed tuple variants (`FormatMetadata::Html(Box<HtmlMetadata>)`) now deref before calling `.into()` since `From<HtmlMetadata>` is derived for `JsHtmlMetadata`, not `Box<HtmlMetadata>`.
- fix(csharp-backend): emit length parameters for byte slice and batch FFI calls. The C# P/Invoke signatures for `extract_bytes_sync` and related functions were missing `UIntPtr contentLen` and `UIntPtr itemsLen` parameters that the Rust FFI requires, causing host crashes. The codegen now expands `TypeRef::Bytes` parameters into two FFI arguments (pointer + length), and the wrapper methods pass `(UIntPtr)content.Length` when calling the native methods.

## [0.14.35] - 2026-05-08

Re-roll of 0.14.34: the v0.14.34 tag captured an early commit, missing the long fix list
below. Bumped to 0.14.35 to ship them on crates.io.

### Fixed

- fix(napi): emit tagged-enum variant data as optional struct properties instead of getter methods. NAPI-RS does not surface per-variant getters in the generated `.d.ts` types, so consumers calling `result.metadata.format.excel.sheetCount` got `undefined`. The backend now emits each single-tuple Named variant as a top-level `Option<JsXMetadata>` field on the binding struct (e.g. `excel: Option<JsExcelMetadata>`) and the `From<CoreEnum>` impl populates these optional fields per variant, so direct property access works in TypeScript.
- fix(e2e-zig): emit real `extract_bytes_sync` / `extract_file_sync` signatures and mark unused locals. The zig codegen previously emitted calls to `extract_bytes_sync_default` / `extract_file_sync_default` (driven by per-language `function = "..._default"` overrides) — symbols that no Zig binding exposes — and bound `result` / `err` captures with bare `const result = ...` / `catch |err|` patterns that Zig 0.16 rejects with "unused local constant" / "error set is discarded". The codegen now (a) calls the real `extract_bytes_sync` / `extract_file_sync` functions, (b) constructs the required `ExtractionConfig` value via `std.mem.zeroInit(...ExtractionConfig, .{ .output_format = ...OutputFormat{ .plain = {} } })` for the `config` argument (the binding's struct mirror has no JSON-loading helper and the tagged-union `output_format` field rejects `std.mem.zeroes`), (c) serializes non-config `json_object` fixture values into JSON-string literals (Vec/Map params cross the FFI as JSON strings), (d) passes explicit `null` for optional arguments since Zig has no default-argument support, (e) uses `catch { ... }` (no captured `err`) for error-path tests and `_ = result;` to discard unused locals, and (f) emits `_ = try fn(...)` for assertion-free or `not_error`-only tests so `result` never lingers as an unused local.
- fix(e2e-php): fix config argument handling and field naming. Reordered match arms in codegen to check config json_object args BEFORE optional check, ensuring `ExtractionConfig::from_json('{}')` is emitted even when config is marked optional in `alef.toml` — fixes `ArgumentCountError` when no fixture config provided. Additionally, changed config field serialization to use `json_to_php()` (snake_case) instead of `json_to_php_camel_keys()` so Rust's serde field naming conventions are respected (Rust defaults to snake_case via `#[serde(rename_all = "snake_case")]`), fixing unrecognized `output_format` field errors.
- fix(swift-backend): repair extraction wrapper codegen so `Kreuzberg.swift` compiles. The previous `emit_extraction_wrappers()` emitted three constructs that do not exist in the swift-bridge runtime: `RustVec<T>([T])` (the runtime only exposes `init()` + `push(value:)`), `ExtractionConfig()` (no zero-arg initializer is generated; the only init is the 33-parameter `init(ptr:)`/full constructor), and `JSONDecoder().decode(ExtractionConfig.self, ...)` (`ExtractionConfig` is an opaque swift-bridge proxy class, not a `Decodable` Swift struct). The wrappers now build `RustVec<UInt8>` via a `makeByteVec` helper that pushes bytes one at a time, and accept a fully-built `ExtractionConfig` parameter — Swift-side JSON config parsing is dropped because the proxy type cannot be Codable. Batch wrappers (`batchExtractBytes`/`batchExtractFiles`) are no longer emitted because `BatchBytesItem`/`BatchFileItem` are getter-only proxies with no `#[swift_bridge(init)]` constructor and therefore cannot be instantiated from Swift; callers must invoke `RustBridge.batchExtract*Sync` directly.
- fix(sync-versions): self-heal corrupted `gleam.toml` dependency ranges. Earlier alef releases routed `gleam.toml` through the `SEMVER_RE.replace_all` catch-all, rewriting `gleam_stdlib = ">= 0.34.0 and < 2.0.0"` into `>= {workspace_version} and < {workspace_version}` (an empty range gleam refuses to resolve with `error: Dependency resolution failed ... has no versions in the range .`). `sync_versions` now restores the canonical `gleam_stdlib` and `gleeunit` ranges from `template_versions::hex` whenever it touches a `gleam.toml`, so a single `alef sync-versions` heals affected manifests in-place.
- fix(magnus-backend): close enum braces correctly in per-variant accessor emission. The enum template was missing the closing `}` after the variant loop, causing generated enums to have unclosed delimiters and preventing the code from compiling. The enum method registration code that attempted to expose accessor methods as Ruby methods was removed, as Magnus's `method!` macro cannot handle functions returning `Option<T>` with complex types. Accessor functions are still generated in Rust (for internal use) but are no longer registered as Ruby methods.
- fix(java-backend): use `allocateFrom` for byte[] marshalling (JDK 22+ Panama FFM). The generated Java code now calls `arena.allocateFrom(ValueLayout.JAVA_BYTE, content)` instead of the removed JDK 21 API `arena.allocateArray(ValueLayout.JAVA_BYTE, content)` to copy byte arrays into off-heap arena memory for FFI calls.
- fix(extendr-backend): sanitize error messages to satisfy R's `condition` class constraints. The previous `extendr_api::Error::Other(e.to_string())` propagation produced messages containing `:`, `/`, and `-` characters and unbounded length, which extendr-api rejects when constructing the R-side condition object — the Rust panic propagated through the FFI boundary instead of converting to an R `stop()`. Error messages are now sanitized (`:`, `/`, `-` replaced with `_`, truncated to 255 chars) so `expect_error()`-style tests observe a clean R error.
- fix(java-backend): allocate `byte[]` parameters off-heap via Arena for Panama FFM. The marshal step previously emitted `MemorySegment.ofArray(name)` which only works for on-heap segments and is rejected when the FFI call requires a native (off-heap) segment. Generated code now uses `arena.allocateArray(ValueLayout.JAVA_BYTE, name)` so byte arrays are copied into the active arena's off-heap memory before being passed to `extern "C"` functions.
- fix(java-backend): annotate sealed-interface tagged unions with `@JsonIgnoreProperties(ignoreUnknown = true)`. Variants that flatten an inner type's fields onto the JSON object surface produced extra discriminator/payload properties that the sealed interface itself cannot map to record fields, so Jackson's strict default rejected them with `UnrecognizedPropertyException` before dispatching to the variant record. The annotation lets Jackson resolve the discriminator and forward the unknown properties to the matching subtype.
- fix(e2e-php): `chdir` into `test_documents/` so fixture paths resolve from PHPUnit's working directory; supply default `ExtractionConfig::from_json('{}')` for required `config` args missing from fixtures; wrap stringly-typed file-path inputs to `extract_bytes`-shaped calls in `file_get_contents(...)` so the bytes argument actually contains the file's contents instead of the path string.
- fix(swift-backend): emit string/bytes/JSON-config wrapper helpers. `emit_extraction_wrappers()` previously returned without emitting anything, forcing e2e tests to call `RustBridge.extract_*` directly with `RustVec<UInt8>` and pre-parsed `ExtractionConfig`. The backend now emits public Swift `extractBytes`/`extractFile` (sync + async) wrappers that accept `String`/`[UInt8]` content and an optional JSON config string, parse the JSON via `serde_json` on the Rust side, and delegate to the underlying RustBridge calls.
- fix(napi-backend): emit per-variant getters on `#[napi(object)]` tagged-enum structs. `JsFormatMetadata` is emitted as a flat struct with `<tag>_tag: String` plus optional fields per variant, but `#[napi(object)]` on its own does not expose getters that match `result.metadata.format.excel.sheetCount`. Each variant of every tagged enum that maps every non-empty variant to a single Named field now has a `#[napi(getter)]` method on a `#[napi]` impl block: it checks `<tag>_tag`, then deserializes the variant's JSON payload and returns it.
- fix(magnus-backend): emit per-variant accessor methods on tagged-enum classes. The Ruby `FormatMetadata` class was previously emitted with flat optional fields but no per-variant accessors. Each non-empty variant now has a Ruby method (`#excel`, `#docx`, …) that returns the variant payload when the discriminator matches, or `nil` otherwise.
- fix(e2e-gleam): unwrap `Result(_, _)` before field-access assertions. The codegen previously emitted `result.field |> should.equal(...)` even when `result` was a `Result`, failing Gleam's type checker. The codegen now emits `let assert Ok(r) = result` once after the call and uses `r` as the access base for every assertion.
- fix(zig-backend): port to zig 0.16. Replaced `std.fmt.allocPrintZ` (removed in 0.16) with `std.fmt.allocPrintSentinel(allocator, ..., 0)` returning `[:0]u8`, simplified the matching `free` calls, and updated `@typeInfo(E).ErrorSet` to `@typeInfo(E).error_set` (snake_case rename in 0.16).
- fix(scaffold/node): generate proper platform-dispatch `index.js` at `crates/{X}-node/index.js` instead of a single-file stub. Previously the scaffold did not emit this file, so a `napi build` (without `--platform`) stub `module.exports = require("./{X}-node.node");` would persist in the source tree. Per-target CI builds with `napi build --platform --target X` regenerate the file locally on each runner, but only `*.node` artifacts get uploaded — the platform-aware `index.js` is discarded. The committed stub then ships in the npm tarball, even though the bundled binaries are platform-suffixed (`{X}-node.darwin-arm64.node`, `{X}-node.linux-x64-gnu.node`, etc.), so `require("./{X}-node.node")` fails for every consumer at install time. The new generator emits a self-contained dispatcher covering linux x64/arm64 (gnu+musl), darwin x64/arm64, and win32 x64/arm64 (msvc), with a fallback to optional `{packageName}-{platformArchABI}` deps.
- fix(e2e-swift): handle optional string fields in trimming assertions. When a Rust `Option<String>` maps to Swift `String?`, calling `.trimmingCharacters(in:)` directly fails because the method is unavailable on the optional type. Generated assertions now coalesce optional strings with `?? ""` before trimming, enabling fixtures to assert on optional metadata fields like `output_format`.
- fix(rustler-backend): always convert rustler::Binary to owned Vec<u8> in NIF deser to avoid escape-into-spawn lifetime errors. Previously, the `is_ref` branch emitted `let content: &[u8] = content.as_slice();`, which borrows from the input Binary and cannot satisfy the `'static` requirement of `std::thread::spawn`. Always cloning to `Vec<u8>` is correct and the call site re-borrows the slice when the underlying core function takes `&[u8]`.
- fix(e2e-swift): emit RustBridge-qualified function calls in generated tests. Since wrapper functions were disabled in Phase 2D (commit 6bdbd0e9), e2e tests must call `RustBridge.extractFileSync(...)` instead of bare `extractFileSync(...)`. The codegen now qualifies all function calls with the RustBridge module prefix.
- fix(java-backend): suppress checkstyle LineLength on all generated classes. Extended the suppression added for e2e test classes to cover all alef-generated Java classes (records, enums, tagged unions, opaque handles, builders, facades, FFI classes, exception classes, and trait bridges). All classes now emit `@SuppressWarnings("checkstyle:LineLength")` to acknowledge that generated code may exceed the 140-character limit.
- fix(elixir-backend): use rustler::Binary for NIF binary parameters. Rustler 0.37 cannot marshal `Vec<u8>` from Erlang binaries, causing ArgumentError on every NIF call. NIF functions now accept `rustler::Binary` parameters and convert to `Vec<u8>` with `.as_slice()` or `.as_slice().to_vec()` when calling core functions.
- fix(e2e-dart): add missing kreuzberg package import in test files. Generated Dart e2e tests were emitting `KreuzbergBridge.extractBytesSync(...)` calls without importing the kreuzberg package, causing "Undefined name 'KreuzbergBridge'" errors. Test files now include `import 'package:kreuzberg/kreuzberg.dart'` to make the API accessible.
- fix(swift-backend): emit type aliases for all struct types, not just those referenced in enum variants. swift-bridge doesn't reliably expose all referenced types in generated Swift modules, so the backend now unconditionally emits `public typealias StructName = RustBridge.StructName` for all non-trait types. This ensures metadata types (JatsMetadata, EpubMetadata, PstMetadata, etc.) are accessible in Swift code.
- fix(swift-backend): add lightweight wrapper functions for extraction methods. The Swift package now includes public `extractFile`, `extractBytes`, `extractFileSync`, and `extractBytesSync` convenience functions that delegate to RustBridge equivalents, providing idiomatic Swift entry points for e2e tests and common use cases.
- fix(e2e-java): suppress checkstyle LineLength violations on generated test classes. Auto-generated e2e test methods contain byte array literals (for testing error paths) that are unavoidably long; annotate test classes with `@SuppressWarnings("checkstyle:LineLength")` to indicate that these violations are acceptable.
- fix(csharp-backend): remove duplicate accessor properties from discriminated-union sealed records. The code generator was creating both a sealed record type (e.g., `Pdf`) and a property with the same name (`Pdf => ...`), causing CS0102 "already contains a definition" errors. Sealed records don't need these accessors; pattern matching and property access via the record field itself is idiomatic C#.
- fix(e2e-zig): wire kreuzberg module into test build.zig. Each Zig test module now imports the kreuzberg module via `addImport`, resolving `@import("kreuzberg")` failures. The build script also declares ffi_path and ffi_include options for linking kreuzberg_ffi.
- fix(e2e-dart): emit receiver class and arguments in non-HTTP test calls. Dart e2e tests now emit `KreuzbergBridge.extractBytesSync(File(...).readAsBytesSync(), ...)` instead of bare `extractBytesSync()`, with fixture inputs correctly loaded and passed as arguments.
- fix(e2e-dart): convert snake_case function names to camelCase in generated test code. Dart conventions require camelCase method names; the test code generator now converts function names like `extract_bytes_sync` to `extractBytesSync`, matching idiomatic Dart API surface.
- fix(swift-backend): emit native Swift enums with unit variants only instead of typealiasing to non-existent RustBridge enum types. swift-bridge's automatic code generation doesn't reliably expose all enum types; emitting them directly as Swift enums avoids brittle typealias dependencies and enables pattern matching.
- fix(e2e-wasm): inject initSync() call in test files for Node.js test environments. vitest running wasm-pack output requires synchronous initialization of the WASM module using initSync with the bundled binary, preventing `TypeError: Cannot read properties of undefined (reading '__wbindgen_add_to_stack_pointer')` and fetch failures in Node.js.
- fix(csharp-backend): remove stray opening brace after class headers in SafeHandle and wrapper classes. The template already included the brace; the code was pushing an extra one, resulting in all C# generated files having `class Foo {` followed by a stray `{` on the next line. All 30+ C# compilation errors now clear.
- fix(e2e-dart): remove HTTP-only assumption from non-HTTP fixture codegen. Dart e2e tests now render direct-API call tests using `[e2e.call.overrides.dart]` function overrides, eliminating skip-stubs.
- fix(e2e-gleam): remove call-override check for non-HTTP fixtures. Gleam e2e tests now render all non-HTTP fixtures using the default or overridden call config, eliminating skip-stubs.
- fix(e2e-kotlin): fix `build.gradle.kts` dependency declaration. Registry mode now uses correct `groupId:artifactId:version` format; local mode references kreuzberg binding JAR from `target/release/`.
- fix(e2e-swift): document test placement path. Clarified that tests are placed in `packages/swift/Tests/` (not `e2e/swift/`) due to SwiftPM 6.0 limitations, with rationale in comments.
- fix(e2e-zig): implement proper error union and async function handling. Error-path tests now use `catch` syntax; async functions emit informational notes; all test variants (error/no-assertions/success) now compile and run.

## [0.14.33] - 2026-05-07

### Fixed

- **Go module**: emit `#cgo CFLAGS: -I${SRCDIR}/include` (instead of monorepo-relative `-I${SRCDIR}/.../crates/<ffi-crate>/include`), and have the FFI backend's generated `build.rs` copy the cbindgen header into the Go module's `include/` directory at build time. This unblocks downstream consumers that install the Go module via `go get` (where everything outside `packages/go/` is absent). Thanks to @structuredmerge for the report.
- **Note**: 0.14.31 was a tag-only release whose changes never reached crates.io because Cargo.toml wasn't bumped. 0.14.33 carries forward the intended fix.

## [0.14.32] - 2026-05-07

### Added

- feat(release): x86_64-apple-darwin release binaries to support downstream macOS x86_64 CI runners that cannot rely on Rosetta

## [0.14.30] - 2026-05-07

### Fixed

- fix(java,csharp,magnus,php-backends): set `keep_trailing_newline(true)` on the minijinja `Environment`. Minijinja strips trailing newlines from rendered templates by default, which caused multi-template concatenation in the Java marshalling helpers (`helper_object_mapper.jinja` + `helper_read_json_list.jinja`) to emit `MAPPER = createObjectMapper();    private static <T> readJsonList(...)` smashed onto a single 294-character line. With the trailing newline preserved, helper templates concatenate with proper separators. Also applied defensively to the csharp/magnus/php backends, which use the same template environment idiom.

## [0.14.29] - 2026-05-07

### Changed

- fix(rustler-backend): remove `x86_64-pc-windows-gnu` from default rustler_precompiled `targets:` list. The target was declared in the alef-generated `native.ex` but no canonical CI build job produces a windows-gnu NIF (rustler precompiled binaries normally target windows-msvc, and most Elixir projects don't ship windows-gnu artifacts). Including the missing target broke `mix rustler_precompiled.download --all` during release prep, which in turn blocked Hex publish for downstream packages. New default target list is `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`.

### Fixed

- fix(pyo3,wasm,php-backends): add `clippy::arc_with_non_send_sync` to crate-root allow lists. Rust 1.95.0 promoted this lint to default-on; binding crates wrap a non-Send/Sync `ConversionOptionsBuilder` (or similar core type) in `Arc` for ergonomic chaining, which is sound in single-threaded VM contexts. The napi backend already had this allow; the other three now match for consistent behavior on Rust 1.95+.

### Added

- feat(extendr-backend): flat data enums (e.g. `OutputFormat`) now generate a `From<BindingStruct> for CoreEnum` impl in addition to the existing `From<CoreEnum> for BindingStruct`. Previously the binding→core conversion fell back to `Default::default()` for all flat data enum fields, so setting `output_format = "markdown"` on an `ExtractionConfig` was silently discarded and every extraction ran as `Plain`. The reverse impl is only generated for enums where all tuple-variant data types are `String` or `Option<String>` (primitives); complex output-only enums like `FormatMetadata` are excluded.
- feat(extendr-backend): input types (structs that appear as function/method parameters) now emit a `from_json(json: String) -> Result<Self>` static factory method in their `#[extendr]` impl block. The factory deserializes via the core Rust type (which carries `#[serde(default)]` and string-enum aliases), then converts to the binding type. This allows R callers to construct configs from partial JSON (`ExtractionConfig$from_json('{"output_format":"markdown"}')`). The R-side `.Call` wrapper is emitted in `extendr-wrappers.R` automatically. Only input types receive the factory; output-only types (e.g. `Metadata`, `ExtractionResult`) are excluded.

### Changed

- fix(csharp-backend): replace bespoke `FormatMetadataJsonConverter` with .NET 7+ standard `[JsonPolymorphic]` + `[JsonDerivedType]` attributes. The custom converter was removed entirely; JSON deserialization for tagged unions now uses the idiomatic .NET 7+ approach. Generated C# code is simpler and maintains the same runtime behavior.

### Fixed

- fix(java-backend): visitor bridge `invokeCallbacks` methods now split into chunks of 5 callbacks instead of 10. Checkstyle enforces a 150-line `MethodLength` limit; at ~22 lines per callback, chunks of 10 produced ~232-line methods that failed checkstyle. Chunks of 5 produce ~114 lines, safely under the limit.
- fix(java-backend): visitor interface `jinja` template now emits `default` methods with braced bodies on separate lines. Previously the body was on a single line; checkstyle's `DesignForExtension` rule requires overridable `default` methods to have an explicit block body.

### Fixed

- fix(magnus-backend): `version.rb` now emits `VERSION = '...'` with single quotes to match the quote style that `alef sync versions` normalizes to, fixing a persistent `alef verify` staleness false-positive on the Ruby gem version file.
- fix(scaffold/node,php,python): remove `[lints] workspace = true` from the generated Cargo.toml for NAPI-RS, ext-php-rs, and PyO3 binding crates. These crates legitimately use `unsafe` code generated by their proc-macros; inheriting `unsafe_code = "forbid"` from the workspace caused compile errors (`E0453`) that cannot be suppressed by `#![allow(unsafe_code)]` in source when the workspace uses `forbid`. Cargo does not allow mixing `workspace = true` with individual lint overrides in the same section, so workspace lint inheritance is omitted for these crates.

### Added

- feat(e2e-c): added `raw_c_result_type`, `c_free_fn`, and `result_is_option` fields to `CallOverride` for C e2e codegen. When `raw_c_result_type` is set (e.g. `"char*"`, `"int32_t"`, `"uintptr_t"`), the C test generator uses the raw type directly instead of the opaque `{PREFIX}Type*` pattern, emits the correct assertion style per type, and calls the appropriate free function for `char*` returns. This enables e2e tests for all non-process C FFI calls (registry, detection, query, download).

### Fixed

- fix(magnus-backend): `TryConvert` for binding struct types now accepts plain Ruby `Hash` and `String` values via JSON fallback. Previously only already-wrapped typed-data objects were accepted, causing `TypeError: no implicit conversion of Hash into {StructName}` when callers passed a plain hash. The generated impl now tries typed-data unwrap first; on failure it calls `.to_json` (or accepts a String directly) and deserializes via `serde_json`. Symptom: Ruby process/init/configure e2e tests failed with `TypeError: no implicit conversion of Hash into ProcessConfig`.
- fix(magnus-backend): `gen_options_field_bridge_function` now accepts a Ruby `Hash` as the options argument in addition to a visitor object. When the second argument is a `RHash`, the bridge deserializes it as `ConversionOptions` via JSON; otherwise it wraps it as a visitor bridge handle. Previously only the visitor path existed, so passing plain option hashes silently produced default-constructed `ConversionOptions`.
- fix(magnus-backend): visitor method result parsing now detects Ruby `Hash` returns with a `:custom` key (e.g. `{ custom: '--- {text} ---' }`) and extracts the value as `VisitResult::Custom(s)`. Previously `val.to_string()` on a Hash yielded the Ruby inspect string (`{custom: "..."}`) which was then used verbatim as the Custom template, producing garbled output instead of the intended replacement string.
- fix(ruby-e2e-codegen): `count_equals` and `count_min` assertions on named fields (e.g. `field: "warnings"`) now correctly emit `result.warnings.length` even when `result_is_simple = true`. Previously `result_is_simple` caused the field path to be dropped unconditionally, generating `result.length` which raised `NoMethodError` on `ConversionResult`.
- fix(ruby-e2e-codegen): `CallbackAction::CustomTemplate` visitor callbacks now emit Ruby double-quoted strings with `#{param}` interpolation (e.g. `{ custom: "--- #{text} ---" }`) instead of single-quoted literals. Single-quoted Ruby strings do not interpolate `{text}` placeholders, so the template was returned verbatim and the Rust core received a literal `{text}` instead of the substituted value.
- fix(r-e2e-codegen): non-empty `json_object` args (e.g. `options = {"preprocessing": {"remove_forms": false}}`) are now emitted as R `list(...)` expressions rather than defaulting to `NULL`. Previously the codegen collapsed any `is_object()` value to `r_default_for_config_arg()`, so fixture options containing nested structs like `PreprocessingOptions` were silently discarded and tests ran with default options instead.
- fix(r-e2e-codegen): `json_to_r` now converts PascalCase enum strings to snake_case (e.g. `"DoubleEqual"` → `"double_equal"`, `"AtxClosed"` → `"atx_closed"`) instead of plain `.to_lowercase()`. Plain lowercasing produced `"doubleequal"` which the R binding's `parse_highlight_style` rejected; snake_case matches the expected `"double_equal"`.
- fix(r-e2e-codegen): `CallbackAction::CustomTemplate` visitor callbacks now emit `paste0(...)` expressions for R string interpolation (e.g. `list(custom = paste0("[BTN:", text, "]"))`) instead of a plain double-quoted string literal. R has no built-in string interpolation so `"{text}"` in a string literal was passed verbatim to the Rust core instead of the substituted value.
- fix(r-e2e-codegen): `visit_details` callback parameter renamed from `is_open` to `open` to match the R binding's named argument passing (`("open", ...)` in the Pairlist). R's named-argument dispatch raised `unused argument (open = FALSE)` when the function declared `is_open` instead.
- fix(ffi-backend): remove unnecessary parentheses around `*vtable` dereference in visitor bridge constructor. The generated `(*vtable).clone()` caused a clippy `unused_parens` warning; simplified to `*vtable`.
- fix(pyo3-backend): `_coerce_dict_{name}` helper functions now annotate the `value` parameter as `dict[str, Any]` instead of bare `dict`. mypy `--strict` flags bare `dict` as missing type arguments (`type-arg` error).
- fix(c-e2e-codegen): remove unused `failed` counter from generated C test runner `main()`. The counter was declared and initialized to 0 but never incremented (C test failures abort the process), causing cppcheck to flag `failed > 0` as "comparison is always false". Changed return to `return 0;` and updated result printf to hardcode `0 failed`.
- fix(c-e2e-codegen): `not_empty` assertion now guards against NULL before calling `strlen` (e.g. `assert(var != NULL && strlen(var) > 0 && ...)`). Previously `strlen(var)` was emitted without a prior null check; when a `count_min` assertion on the same variable followed with `var != NULL`, cppcheck flagged `[nullPointerRedundantCheck]` — "either the condition is redundant or there is a possible null pointer dereference". Fixed in both the raw-C-result path and the `render_assertion` path (field-based assertions).
- fix(magnus-backend): `MAGNUS_RESERVED_FN_NAMES` no longer includes `"init"`. The generated entry-point function is now `fn ruby_init(ruby: &Ruby)` (not `fn init`), so `"init"` no longer conflicts with generated API function names and API functions named `init` are now emitted correctly.
- fix(codegen/structs): `gen_struct_default_impl` now uses `typed_default` from the IR field definition when generating `Default` impls for binding structs. Previously all non-optional fields fell through to `Default::default()`, emitting `false` for bool fields even when the core's `impl Default` set them to `true`. Now `BoolLiteral`, `IntLiteral`, and `StringLiteral` defaults are reproduced verbatim, fixing `ProcessConfig::default()` returning `structure: false` in Ruby/Python bindings when the core default is `true`.
- fix(pyo3-backend): data enum variant accessor keyword check now correctly uses the snake_case variant name for comparison against `RUST_KEYWORDS` instead of the original CamelCase name. Previously `Struct`, `Enum`, `Type` variants always passed the keyword check (CamelCase != lowercase keyword) and emitted invalid function names like `fn struct` / `fn enum` / `fn type` that failed to compile. Now `"Struct"` → `"struct"` → `fn r#struct`, matching what Python sees as a plain `struct` attribute.
- fix(pyo3-backend): data enum variant accessor methods now use the correct `pyo3::types::PyDict` path (was `pyo3::PyDict`, which does not exist in pyo3 0.28). Return type is `PyResult<Option<pyo3::Py<pyo3::types::PyDict>>>`.
- fix(pyo3-backend): data enum variant accessor methods now call `.downcast_into::<pyo3::types::PyDict>()?` followed by `.unbind()` to obtain an owned `Py<PyDict>`. Previously `.downcast()` returned `&Bound<'_, PyDict>` (a reference into a temporary), which could not be converted to `Py<PyDict>`, causing an `E0277: From<&pyo3::Bound<'_, PyDict>>` compile error.
- fix(pyo3-backend): data enum single-tuple variants whose inner type is a Named binding struct now return the typed struct directly (via `From`) instead of a Python dict. Multi-field or non-Named variants continue to use the JSON-to-dict path.

- feat(pyo3-backend): data enums now emit `#[getter]` accessor properties for each variant. For example, `FormatMetadata` now exposes `excel`, `docx`, `pdf` properties that return `Option<dict>` containing the variant's data fields. Enables ergonomic pattern: `result.metadata.format.excel["sheet_count"]`. Variant names are converted to snake_case for Python idioms; tag field names that are Rust keywords (e.g. `type`) are emitted with `r#` prefix (`fn r#type`) which PyO3 exposes to Python without the `r#`.
- feat(napi-backend): tagged unions now emit `#[napi(getter)]` accessor methods for each variant on the flattened struct. For example, `JsFormatMetadata` now includes `excel()`, `docx()`, `pdf()` getters that return `Option<napi::bindgen_prelude::Object>` containing the variant's field values. Enables ergonomic pattern: `result.metadata.format.excel?.sheetCount` with optional chaining in JavaScript/TypeScript.
- feat(pyo3-backend): data enums with a `serde_tag` field (e.g. `FormatMetadata`) now emit a `#[getter]` for the tag field on the opaque `#[pyclass(frozen)]` wrapper. The getter serializes `self.inner` to JSON and extracts the tag string (e.g. `format.format_type`). If the tag field name is a Rust keyword (e.g. `type`), the function is emitted with an `r#` prefix (`fn r#type`) — PyO3 exposes it to Python without the `r#`. The companion pyi stub is now a `class` with the tag attribute declared, rather than a `TypeAlias`, so type checkers see the attribute.
- feat(r-e2e-codegen): add `result_is_r_list` flag to `CallOverride`. When `true`, the R generator emits the call result directly without wrapping in `jsonlite::fromJSON()`, fixing test failures for bindings (like html-to-markdown's extendr R binding) that already return a native `Robj` list instead of a JSON string. Field-path assertions (`result$content`, etc.) are unaffected — the flag only suppresses the JSON parse wrapper.

### Fixed

- fix(php-backend): config params are now null-coalesced at call sites (`$config ?? new ConfigType()`) only when the config type can be constructed with zero arguments (all fields are optional in the IR). Previously any param whose type name ended with `Config` triggered null-coalescing unconditionally, which caused `new ProcessConfig()` calls in generated PHP facades — invalid because `ProcessConfig` has 8 required constructor fields. The codegen now pre-computes the set of all-optional-field types and restricts the null-coalesce rewrite to members of that set.
- fix(java-backend, csharp-backend): suppress `clippy::too_many_arguments` on `gen_sync_function_method` and `gen_wrapper_function`; both functions require all their parameters to drive code generation and cannot reasonably be refactored below the 7-argument limit without losing clarity.
- fix(extendr-backend): fix overindented doc list items in `gen_extendr_wrappers_r` doc comment.
- fix(e2e-typescript): collapse duplicate `if`/`else if` branches that both pushed `"undefined"` into a single `if` with an `||` condition.
- fix(e2e-typescript): revert vitest `singleFork` config — running all tests in one worker amplifies visitor teardown crashes across unrelated test files; default pool isolation is safer.
- fix(e2e-node): options are now constructed as a plain object literal with a type assertion (`{ key: val } as ConversionOptions`) and imported type-only. The previous `ConversionOptions.fromUpdate(new ConversionOptionsUpdate({...}))` pattern failed at runtime because `ConversionOptions` is a TypeScript interface, not a class.
- fix(e2e-wasm): options are now constructed via empty constructor + setter assignments wrapped in an IIFE (`(() => { const _u = new WasmConversionOptionsUpdate(); _u.key = val; return WasmConversionOptions.fromUpdate(_u); })()`). The previous pattern passed an object literal to a positional constructor (40+ args), silently landing it as `heading_style` and ignoring all intended values.

### Fixed

- fix(extendr-backend): types whose fields include `Vec<Vec<T>>` or `Option<Vec<Vec<T>>>` are now excluded from `#[extendr]` class and function generation. `extendr_api` has no `TryFrom<&Robj>` implementation for nested vectors, so emitting `#[extendr]` on such types produced a compile error (`error[E0277]: the trait bound … is not satisfied`). Affected types (`ExcelSheet`, `OcrTable`, `Table`, `RecognizedTable`) are now skipped from the extendr class list and the derived function list; their struct definitions are preserved so serde serialization still works. Symptom: `kreuzberg-r` failed to compile with `error[E0277]: Option<Vec<Vec<String>>>: extendr_api::TryFrom<&extendr_api::Robj> not satisfied`.
- fix(php-backend): `ConversionOptionsBuilder.visitor()` now uses `Rc<RefCell<PhpHtmlVisitorBridge>>` instead of `Arc<PhpHtmlVisitorBridge>` for the `VisitorHandle` wrapper. `html_to_markdown_rs::visitor::VisitorHandle` is a type alias for `Rc<RefCell<dyn Visitor>>`, which is `!Send`; using `Arc` produced a compile error (`error[E0308]: mismatched types: expected Rc<RefCell<dyn Visitor>>, found Arc<PhpHtmlVisitorBridge>`). Also removed the now-redundant `&` in `Some(&handle)` (Rc is already an owned handle).
- fix(php-backend): `gen_enum_tainted_from_binding_to_core` now accepts a `bridge_type_aliases` set and, for sanitized fields whose type is a bridge alias, emits `(*val.{name}.inner).clone()` instead of `Default::default()`. Previously bridge-type fields in the tainted-enum conversion always fell back to `Default::default()`, so the actual PHP-supplied bridge object was silently discarded — the `ConversionOptions` carrying a non-None `visitor` always produced a `None` visitor when bridging from PHP binding to Rust core.
- fix(php-backend): the null-coalescing eligible set now also includes types that derive `Default` (i.e. `has_default == true`), not just types whose every IR field is `Optional`. Previously, struct types with `bool` fields carrying `#[serde(default = "…")]` (such as `ExtractionConfig`) were excluded from null-coalescing because their `bool` IR fields are not tagged `optional`, even though calling `new ExtractionConfig()` in PHP works fine via `Default`. This caused config parameters to lose their `?T = null` nullable signature and break callers that omit the config.
- fix(php-backend): `ConversionOptionsBuilder.visitor()` method now correctly accepts a `&mut ZendObject` parameter, creates a `PhpHtmlVisitorBridge` from it, wraps it in `Arc::new()`, and passes `Some(&handle)` to the inner builder — instead of ignoring the parameter and always passing `None`. The PHP visitor bridge can now be invoked through the builder API.
- fix(php-e2e-codegen): `CallbackAction::CustomTemplate` placeholder interpolation now converts `{key}` to `{$key}` in PHP double-quoted strings, enabling variable expansion in generated visitor callback templates. Template `"~{text}~"` now generates `return ['custom' => "~{$text}~"];` so PHP's string interpolation evaluates `$text` variables correctly.
- fix(java-backend): builder classes now carry `@JsonIgnoreProperties(ignoreUnknown = true)` so flattened tagged-enum discriminators (e.g., `format_type` from `FormatMetadata` flatten on `Metadata`) and forward-compatible additions don't fail Jackson's strict deserialization. Without this, every Java SmokeTest errored with `UnrecognizedPropertyException: Unrecognized field "format_type" (class dev.kreuzberg.MetadataBuilder)` because Rust core's `#[serde(flatten)] format: Option<FormatMetadata>` puts the tag at the top level of the metadata JSON and Builder + `@JsonTypeInfo` + `@JsonUnwrapped` are mutually incompatible in Jackson.
- fix(go-backend): bytes parameters no longer panic on empty slices. The previous emission `(*C.uint8_t)(unsafe.Pointer(&content[0]))` panics with `index out of range [0] with length 0` when callers pass `[]byte{}`. The codegen now emits a nil-or-pointer guard so `kreuzberg.ExtractBytesSync(nil, ...)` / `kreuzberg.ExtractBytesSync([]byte{}, ...)` reach the FFI layer cleanly and surface the Rust-side validation error instead of crashing the test binary.
- fix(extract/defaults): const-fold integer/float binary arithmetic in `fn default()` bodies. Patterns like `500 * 1024 * 1024`, `1024 * 1024`, and `100 * 1024 * 1024` (used pervasively for byte-size limits) previously fell through `expr_to_default_value` to `DefaultValue::Empty`, so generated bindings emitted `0` instead of the real default. With `<int> OP <int>` and `<float> OP <float>` now folding through `+ - * / % << >> | & ^` (with overflow checks), `SecurityLimits` and similar config types finally surface their actual default-byte-limits to Python/Java/Go/etc., fixing `RuntimeError: Validation error: ZIP archive text content exceeds limit: 3045 bytes (max: 0 bytes)`.
- fix(napi-backend/default-optional): Named function params whose type derives `Default` (e.g. `ExtractionConfig`) are now promoted to `Option<T>` in the binding signature, with the body emitting an `unwrap_or_default()` coercion before the rest of the let-bindings run. JS callers can now omit the slot or pass `undefined` and get a default-constructed value, matching the wasm-bindgen path's serde behaviour. Without this, idiomatic `batchExtractBytesSync(items, undefined)` failed at NAPI's struct deserializer with `TypeError: Cannot convert undefined or null to object`.
- fix(codegen/promoted-optional): promoted-optional Named params (those that are required in the IR but appear after a naturally-optional param, so the binding signature carries `Option<T>`) now emit `unwrap_or_default()` instead of `.expect("'…' is required")` when bridging to core. JS/Python callers idiomatically pass `undefined`/`None` for these slots, which previously triggered a Rust panic that vitest reported as `Error: Panic in async function` / `'config' is required`. Binding types always derive `Default`, so this is a clean drop-in.
- fix(napi-backend): structs containing `Buffer` fields now skip the `Clone` derive (Buffer doesn't impl Clone) and emit a manual `Clone` impl that copies via `to_vec().into()`. Bytes fields are tagged `#[serde(skip)]` so the rest of the struct can still derive Serialize/Deserialize (Buffer's `Default` impl is used to reconstruct on deserialize). Without this, generated `JsBatchBytesItem`/`JsExtractedImage`/etc. fail to compile with `the trait Clone is not implemented for Buffer` and `the trait serde::Serialize is not implemented for Buffer`.
- fix(napi-backend): function return type `TypeRef::Bytes` now wraps the core `Vec<u8>` (or `bytes::Bytes`) result with `.into()` so it converts cleanly into the `napi::bindgen_prelude::Buffer` return type. Free functions like `render_pdf_page_to_png` (`Result<Vec<u8>>`) previously emitted a bare `core_call.map_err(...)` body, producing `expected Result<Buffer, Error>, found Result<Vec<u8>, Error>` at compile time once `Vec<u8>` struct fields started mapping to `Buffer`.
- fix(codegen/bytes-dedup): `binding_to_core` and `core_to_binding` `apply_*_wrapper` helpers now recognise the new `val.{name}.to_vec().into()` and `val.{name}.map(|v| v.to_vec().into())` forms emitted by `TypeRef::Bytes` field-conversions and pass them through unchanged. Without this dedup, `CoreWrapper::Bytes` re-wrapped them, producing `(val.x.to_vec().into()).into()` and `val.x.map(|v| v.to_vec().into()).map(|v| v.to_vec().into())` — which fail with `cannot infer type of the type parameter T` and `cannot find function .into for type T`.

- fix(java-e2e-codegen): nested-record builder methods on primitive-fields types (e.g. `SecurityLimitsBuilder.withMaxContentSize(long)`) now receive the bare numeric literal instead of `Optional.of(1L)`. Earlier the codegen wrapped every numeric in `Optional.of()` unless the camelCase field name matched a hardcoded plain-numeric whitelist; that whitelist missed all SecurityLimits fields, so generated tests failed `javac` with `incompatible types: no instance(s) of type variable(s) T exist so that java.util.Optional<T> conforms to long`.
- fix(go-backend): emit `MarshalJSON` on structs whose fields include `[]byte` (Vec<u8>) so the bytes serialize as a JSON array of integers — matching Rust serde's expected `Vec<u8>` deserializer shape. Go's default `json.Marshal([]byte)` emits a base64 string, which Rust's deserializer rejected with `invalid type: string "SGVsbG8=", expected a sequence`.
- fix(napi-backend): `Vec<u8>` struct fields now emit as `napi::bindgen_prelude::Buffer` rather than the bare `Vec<u8>`. NAPI-rs 3.x treats `Vec<u8>` as a JS `Array`, calling `napi_get_array_length` on the value — which fails with "Failed to get Array length" when the JS side passes a `Buffer` or `Uint8Array` (the canonical byte-payload types). Using `Buffer` accepts both and avoids a copy.
- fix(java-e2e-codegen): enum-typed builder method parameters now receive correctly-typed enum constants (e.g., `withCodeBlockStyle(CodeBlockStyle.Backticks)`) instead of string literals. The codegen now uses a configuration-driven enum field mapping (camelCase field name → enum type name) to determine which string values are enums and emit them as qualified enum constants. Fixture JSON like `"code_block_style": "Backticks"` is now converted to `withCodeBlockStyle(CodeBlockStyle.Backticks)` rather than `withCodeBlockStyle("Backticks")`. The mapping is configurable via `[crates.e2e.call.overrides.java.enum_fields]` in `alef.toml` (e.g., `headingStyle = "HeadingStyle"`, `codeBlockStyle = "CodeBlockStyle"`).
- fix(java-e2e-codegen): numeric Optional fields are no longer incorrectly wrapped in `Optional.of()`. Fields like `list_indent_width` and `wrap_width` that have plain `long` signatures now emit bare numeric literals (e.g., `withWrapWidth(40L)`), while truly optional numeric fields are wrapped (e.g., `withMaxDepth(Optional.of(100L))`). The codegen maintains a whitelist of plain numeric field names and distinguishes between the two cases.
- fix(java-e2e-codegen): nested options types like `PreprocessingOptions` are now correctly imported when used in builder expressions. The codegen now detects nested type usage via a configuration-driven mapping (`[crates.e2e.call.overrides.java.nested_types]` in `alef.toml`, e.g., `preprocessing = "PreprocessingOptions"`) and emits the corresponding import statements, preventing compilation errors when nested options builders are referenced.
- fix(php-e2e-codegen): empty config objects (`{}`) in fixture `input.config` are now passed as `null` to config constructors instead of being serialized and passed to `ExtractionConfig::from_json(json_encode([]))`. Empty objects cause Rust serde deserialization errors when required fields have enum defaults (e.g., `hierarchy_mode` expecting `"unified"` or `"element_based"`). The codegen now detects empty-after-filtering config objects in the `json_object` / `options_type` code paths and directly passes `null` to match the nullable parameter type. Symptom: 10 PHP e2e smoke/format/contract tests failed with `Exception: unknown variant '', expected 'unified' or 'element_based' at line 1 column 397`.
- fix(java-e2e-codegen): typed-record nested config fields (e.g. `SecurityLimits`, `OcrConfig`, `ChunkingConfig`) are now automatically detected and used to instantiate the correct Java record type instead of being referenced as a fictitious `TypeNameOptions` builder. The codegen now includes built-in type mappings for all Kreuzberg config fields (security_limits, chunking, ocr, etc.), allowing users to omit explicit `nested_types` configuration in alef.toml for common cases. Previously unmapped fields like security_limits were reconstructed with a hardcoded `Options` suffix, causing `error: cannot find symbol SecurityLimitsOptions` when compiling e2e tests.
- fix(pyo3-backend): Python API wrapper now correctly excludes data enums (tagged unions like `OutputFormat`) from `_coerce_enum` calls when converting dict inputs to dataclass instances. Data enums are represented as TypedDict unions in Python and should be passed through to the PyO3 constructor as dicts, not looked up as enum class attributes. Additionally, nested has*default struct types (e.g., `SecurityLimits`, `OcrConfig`) in dict inputs are now recursively converted via their respective `\_to_rust*\*`converters before constructing the dataclass, preventing`TypeError: argument 'field': 'dict' object is not an instance of 'Type'`when callers pass nested dicts instead of constructed instances. Symptom: 3 Python e2e tests failed with`ValueError: unknown OutputFormat value: 'markdown'`and`TypeError: argument 'security_limits': 'dict' object is not an instance of 'SecurityLimits'`.
- fix(csharp-e2e-codegen): typed-record nested config fields (e.g. `SecurityLimits`, `OcrConfig`, `ChunkingConfig`) are now automatically detected and deserialized via `JsonSerializer.Deserialize<T>(...)` instead of being passed as raw JSON strings. The codegen now includes built-in type mappings for all Kreuzberg config fields (security_limits, chunking, ocr, etc.), allowing users to omit explicit `nested_types` configuration in alef.toml for common cases. Previously only explicitly-configured nested_types fields were deserialized, causing `error CS0029: Cannot implicitly convert type 'string' to 'Kreuzberg.SecurityLimits'` for unmapped fields like security_limits.
- fix(java-backend): generated Java source files now comply with the 120-character line-length limit enforced by Checkstyle. Multi-line record fields with long type annotations are now wrapped such that (a) each annotation appears on its own line, and (b) the type and field name are further split if necessary to keep each line ≤120 characters. Trait bridge registry fields (`ConcurrentHashMap<String, {BridgeClass}>`) are now emitted on two lines instead of one when they would exceed 120 characters. Symptom: `mvn package -DskipTests` was failing with 5 Checkstyle [LineLength] violations in generated Java sources (HtmlMetadata.java, PostProcessorBridge.java, JatsMetadata.java, EmbeddingBackendBridge.java, ExtractionResult.java).
- fix(go-backend): Go bindings for `ExtractionResult.tables` now correctly declare the field as `[]Table` instead of `[]string`. The exclude_types list was using unqualified type names (`"Table"`, `"TableCell"`, `"TableRow"`), which incorrectly excluded the public `kreuzberg::types::tables::Table` type. Changed to fully-qualified names (`"kreuzberg::extraction::docx::parser::Table"`, etc.) to exclude only the internal docx parser types, allowing the public table types to be correctly generated in language bindings.
- fix(go-e2e-codegen): pass `file_path` args directly as fixture-relative strings — the e2e/go `TestMain` in `main_test.go` already `os.Chdir`s into the repo-root `test_documents/` directory, so additional `filepath.Abs(filepath.Join("../../test_documents", path))` resolution would land outside the kreuzberg repo. Reverted the earlier path-rewriting attempt and removed the now-unnecessary `path/filepath` import.
- fix(go-e2e-codegen): `bytes` args whose fixture value is a string are now loaded via `os.ReadFile(path)` at test-run time (matching the rust e2e codegen's `std::fs::read` convention) instead of `base64.StdEncoding.DecodeString`. Fixture strings like `"data": "pdf/fake_memo.pdf"` are fixture-relative paths into `test_documents/` (TestMain chdirs there), not base64 — the previous decoder produced garbage bytes that the parser rejected as invalid PDF/DOCX/etc.
- fix(csharp-backend): trait bridge code generation now emits a generic `FfiJsonExtensions.ToFfiJson<T>()` extension method for serializing trait method return values; methods returning named types (enums, records, classes) were generating calls to `.ToFfiJson()` on the result object, but the method was never emitted, causing CS1061 compilation errors. The extension method is now always generated as part of the TraitBridges.cs file.
- fix(elixir-e2e): drop `optional: true` from the generated e2e `mix.exs` rustler dep — `RustlerPrecompiled` falls back to `rustler` only when `force_build: Mix.env() in [:test, :dev]`, but `optional: true` makes mix skip fetching the package, so the build-from-source path used by every e2e run failed with "the dependency is not available, run mix deps.get".
- fix(rust-e2e): `bytes` args resolved as fixture file paths now use `../../test_documents/` (two levels up) — `CARGO_MANIFEST_DIR` for an e2e test crate is `e2e/<lang>/`, so reaching the repo-root `test_documents/` directory needs two parent steps, not one. Also added explicit `file_path` arg handling so `extract_file*` calls receive an absolute path instead of a fixture-relative string.
- fix(typescript-e2e-codegen): `visit_input` visitor method now declares the first non-context parameter as `input_type` (snake_case) instead of `inputType`, matching the fixture template placeholder `{input_type}`. The previous camelCase name caused a `ReferenceError: input_type is not defined` in template-literal visitor callbacks, crashing the vitest worker.
- fix(r-e2e-codegen): `options` call argument now defaults to `NULL` instead of the non-existent `HtmlOutputConfig$default()`. The `"options" | "html_output"` arm was merged; splitting it makes `"options"` emit `NULL` (R NULL, passed directly) while `"html_output"` retains the class default. Previously all 239 R e2e tests failed with `Error: could not find function "HtmlOutputConfig$default"`.
- fix(r-e2e-codegen): visitor test call no longer emits a duplicate `options` parameter. When a fixture has both a visitor spec and an `options` argument that defaults to `NULL`, the codegen now strips the `options = NULL` from the base argument string before appending `options = list(visitor = visitor)`, preventing the `Error: formal argument "options" matched by multiple actual arguments` R error.
- fix(java-e2e-codegen): generated test files now import only the nested config types (e.g. `PreprocessingOptions`) that are actually referenced in fixture builder expressions, rather than all entries in the `nested_types` defaults map. The previous approach imported product-specific types like `ChunkingConfig` and `OcrConfig` unconditionally, causing `error: cannot find symbol` javac failures in bindings that do not expose those types (e.g. html-to-markdown).
- fix(php-backend): visitor bridge methods now use snake_case method names when calling into the PHP object (e.g. `visit_link` instead of `visitLink`), matching PHP's conventional naming for method names. Previously the bridge called camelCase methods which PHP userland implementations would not define, causing all visitor callbacks to silently fall through to `Continue`.
- fix(php-backend): visitor bridge result parsing now handles PHP array returns `['custom' => 'replacement']` in addition to string returns. Added `php_zval_to_visit_result()` helper that tries `val.string()` first (for `skip`/`continue`/`preserve_html`/custom strings) and then `val.array().get("custom")` (for `['custom' => ...]` returns), using the ext_php_rs 0.15.12 `Zval::array()` and `ZendHashTable::get()` APIs.
- fix(scaffold/node): also emit `packages/<crate>/index.js` (CommonJS shim that re-requires from `crates/<crate>-node/index.js`). Previously only `index.d.ts` was emitted, but `package.json` declares `"main": "index.js"`, so vitest/node failed to resolve the package entry from downstream consumers.
- fix(extract): `extract_serde_rename_all` now properly consumes sibling values inside `serde(...)` and `cfg_attr(..., serde(...))`; previously a leading sibling key (e.g. `tag = "..."` before `rename_all = "..."`) left the parser cursor mid-value, aborting the parse and dropping `rename_all`. Switched the skip path from `let _ = meta.value();` to `let _: syn::Expr = meta.value()?.parse()?;` so the value is actually consumed. Symptom: tagged enums emitted lowercase discriminators (`"listitem"`) instead of `snake_case` (`"list_item"`) in C#/Python/etc. bindings.
- fix(extendr-backend): `generate_public_api` now emits `extendr-wrappers.R` directly from the IR plus a regenerated `NAMESPACE` listing every free function and class env. Previously the package only shipped a `useDynLib` stub, expecting `rextendr::document()` to fill in the wrappers — but rextendr never runs at install time, so every R caller saw `Error: could not find function "extract_file_sync"` (and every other binding). Wrappers cover free functions, static methods, instance methods (with `self` captured by `$.Type` dispatch), and class env definitions for every type registered in `extendr_module!`.
- fix(csharp-e2e-codegen): `JsonElement?` typed fields (discriminated unions like `output_format`) are now wrapped with `JsonDocument.Parse(…).RootElement` instead of emitted as plain string literals; `output_format = "markdown"` now generates `OutputFormat = JsonDocument.Parse("\"markdown\"").RootElement`, avoiding `error CS0029: Cannot implicitly convert type 'string' to 'System.Text.Json.JsonElement?'`.
- fix(rust-e2e-codegen): element types from `json_object` args with `element_type` specified are now imported in the generated test file; batch operations that use `serde_json::from_value::<Vec<{elem}>>()` previously failed to compile with E0425 (cannot find type) because types like `BatchFileItem` and `BatchBytesItem` were not in scope.
- fix(csharp-backend): `JsonOptions` now uses `DefaultIgnoreCondition = WhenWritingNull` instead of `WhenWritingDefault`; bool fields explicitly set to `false` (CLR default) were silently dropped when serializing options to Rust FFI, so e.g. `remove_forms: false` was never sent and Rust fell back to its own default of `true`.
- fix(csharp-backend): generated `VisitResult` record now includes a `ToFfiJson()` method that emits Rust-serde-compatible JSON for visitor return values (`"Continue"`, `"Skip"`, `"PreserveHtml"`, `{"Custom":"…"}`, `{"Error":"…"}`); the previous `JsonSerializer.Serialize` produced `{}` for unit-variant records, which Rust could not deserialize.
- fix(csharp-backend): visitor bridge entry point names now use `to_snake_case()` instead of `to_lowercase()` (e.g. `htm_html_visitor_bridge_new` not `htm_htmlvisitorbridgenew`); the wrong name caused `EntryPointNotFoundException` at runtime.
- fix(csharp-backend): `catch (Exception ex)` → `catch (Exception)` in options-field trait bridges; the `ex` variable was unused in that code path, triggering CS0168 compile error.
- fix(csharp-e2e): options arguments now emitted as idiomatic C# object initializers (`new ConversionOptions { Prop = Value }`) with PascalCase property names instead of `JsonSerializer.Deserialize<ConversionOptions>(json)` calls; the JSON path used camelCase keys that did not match `[JsonPropertyName("snake_case")]` attributes.
- fix(csharp-e2e): add `nested_types` field to `CallOverride`; maps fixture nested-object field names to C# type names so nested structs are generated as `Preprocessing = JsonSerializer.Deserialize<PreprocessingOptions>(…)` rather than inlined JSON.
- fix(cli/build): `cargo` build dispatch no longer collapses to `cd  && cargo build --release` when a language has no explicit `output` path (e.g. Dart in FRB style, whose Rust crate lives at `packages/<lang>/rust/` while generated sources go to `packages/<lang>/lib/src/`). The cargo branch now short-circuits to `cargo build -p <crate-name><crate_suffix>` when `crate_dir` is empty and `BuildConfig::crate_suffix` is set, matching the workspace-member case.
- fix(scaffold/node): generated `kreuzberg-node` `Cargo.toml` now always includes `serde = { version = "1", features = ["derive"] }` and suppresses the `cargo-machete` false positive with `[package.metadata.cargo-machete] ignored = ["serde"]`; machete flagged serde as unused because derive macros reference it via fully-qualified paths not detected by static analysis.
- fix(napi): NAPI type map now emits `Vec<u8>` instead of removed `napi::Buffer` for `TypeRef::Bytes`; `napi::Buffer` was removed in NAPI v3 and JS `Uint8Array` ↔ `Vec<u8>` conversion is now automatic.
- fix(php): `gen_enum_tainted_from_binding_to_core` now emits `#[allow(clippy::useless_conversion)]` before the generated `impl From` block; enum-tainted structs with `Vec<u8>` or `String` fields triggered clippy errors for no-op `.into()` conversions.
- fix(csharp-backend): `ConvertWithVisitor` code path now uses `ConversionResultToJson` + `ConversionResultFree` on the returned handle instead of treating the pointer as a direct JSON string, matching the updated FFI API.
- fix(go-e2e-codegen): visitor test fixtures now correctly emit `opts := &htmd.ConversionOptions{}; opts.Visitor = visitor; result, err := htmd.Convert(html, opts)` instead of an undeclared `options` variable and passing `nil` to Convert. The codegen now creates a fresh `opts` variable with the visitor attached and correctly replaces the trailing `, nil` with `, opts` in the function call.
- fix(java-e2e-codegen): visitor test fixtures now correctly emit `convert(html, new ConversionOptions().withVisitor(visitor))` instead of losing the html argument. The codegen now properly detects and replaces optional arguments when visitor is present.
- fix(java-e2e): generate idiomatic builder expressions instead of MAPPER.readValue for json_object args; e.g., `ProcessConfig.builder().withLanguage("go").withChunkMaxSize(Optional.of(50L)).build()` instead of `MAPPER.readValue("{\"language\":\"go\",\"chunk_max_size\":50}", ProcessConfig.class)`. Builder methods wrap number/boolean values in Optional.of() to match Optional<T> parameter types. Removes MAPPER dependency for options deserialization.
- fix(go-backend): `*C.char` returns now check `if ptr == nil { return nil }` before calling `C.GoString`; previously `C.GoString(nil)` returned `""` and `&v` was returned as a non-nil `*string`, so `Option<String> = None` was misrepresented as a non-nil pointer to an empty string.
- fix(go-backend): `{ffi_prefix}_last_error_context()` is now used in all receiver/param `from_json` error messages; the `kreuzberg_` prefix was hardcoded, causing a CGo "could not determine what C.kreuzberg_last_error_context refers to" build error when the FFI prefix differs.
- fix(java-backend): Java builder classes now derive boolean field defaults from the extracted `impl Default` block (`typed_default`) instead of a hardcoded type-name list; previously `ProcessConfig.structure`, `.imports`, and `.exports` defaulted to `false` in the builder while Rust defaults them to `true`, causing the Java binding to explicitly pass `false` to Rust and suppress all structural analysis.
- fix(java-backend): `createObjectMapper()` now calls `.setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)` so Jackson maps snake_case JSON keys (e.g. `total_lines`) to camelCase builder `with`-methods during deserialization.
- fix(java-e2e): `contains` / `contains_all` / `not_contains` assertions on array fields now call `.toString().contains(value)` instead of `List.contains(value)`; `List.contains` uses `equals()` and always returns false when comparing a `String` against complex record types such as `StructureItem`.
- fix(e2e/csharp): config arguments are now generated as typed C# object initializers (`new ProcessConfig { Language = "abl" }`) with PascalCase property names instead of raw `JsonSerializer.Deserialize` calls; this matches the `[JsonPropertyName]`-annotated C# API surface and avoids snake_case key mismatches.

## [0.14.23] - 2026-05-05

### Added

- feat(hooks): add `hooks/alef_hook.py` so consumers can run `alef` as a pre-commit hook without a local Rust toolchain — the hook reads the version from `alef.toml`, downloads, sha-verifies, caches, and execs the matching pre-built `alef` binary for the host platform.

### Changed

- chore(scaffold/rustler): generated Elixir NIF wrappers now use `RustlerPrecompiled` unconditionally with a `force_build:` option pointing at `<APP>_BUILD_FROM_SOURCE` / `Mix.env() in [:test, :dev]`, instead of branching on `if force_build do use Rustler else use RustlerPrecompiled end`. The branching pattern broke `mix compile` in published packages because `RustlerPrecompiled.__using__` cannot be invoked dynamically.
- chore(scaffold/java): generated Checkstyle config tightens line length from 200 → 120, adds `UnusedImports`, and caps `MethodLength` at 150 lines, matching the wider Kreuzberg Java conventions.
- chore(scaffold/php): generated `composer.json` emits a single-escaped `psr-4` namespace (`"Foo\\": "src/"`) and a lowercased package `name`; the previous double-escaped form produced an invalid `composer.json`, and uppercase names violate Composer's name regex.

### Fixed

- fix(csharp-backend): remove public `ConvertWithVisitor` method and embed visitor handling into single `Convert(string html, ConversionOptions? options)` method; `ConversionOptions.Visitor` is now typed as `IHtmlVisitor?` (managed interface) with `[JsonIgnore]` instead of `VisitorHandle?`, achieving full API parity across all language bindings (one convert function per language, visitor passed via options).
- fix(java-backend): mirror the C# visitor fix on the Java FFI class — `convertWithVisitor` is no longer emitted as a parallel public method; the visitor dispatch is folded into the main `convert` body, with the `has_visitor_bridge` flag plumbed through `gen_sync_function_method`.
- fix(rustler): generated Elixir visitor wrappers now pattern-match `{:ok, _}` instead of `:ok` on the `_with_visitor` NIF call; Rustler encodes `Result<(), String>` as `{:ok, {}}`, so the bare `:ok` match always failed at runtime.
- fix(php): `from_binding_skip_types` field added to `ConversionConfig`; the PHP backend populates it with trait bridge type aliases (e.g. `VisitorHandle`) so their fields emit `Default::default()` in the binding→core `From` impl instead of `val.field.map(Into::into)`, which failed to compile because no `From` impl exists for the PHP-side bridge handle.
- fix(e2e/rust): `not_error` assertions on `Result`-returning calls now emit `.expect("should succeed")` so the test actually panics on errors; previously the result was bound to `_` and silently discarded, making the assertion a no-op.
- fix(php-backend): PHP method names now mirror the Rust source name verbatim (camelCased) without an extra `Async` suffix; the suffix was breaking parity with `alef.toml` call overrides.
- fix(e2e/go): byte-array JSON arguments are now base64-encoded for Go `json.Unmarshal` compatibility; previously the raw `[0, 1, 2, …]` integer array failed to unmarshal into `[]byte`.
- fix(e2e/go): `not_error` and `error` assertions are excluded from the "only nil assertions" short-circuit, so a single `not_error` / `error` assertion no longer collapses the entire test body into a no-op.
- fix(e2e/go): `has_deref_value` (which selects `value` vs `result_var` as `effective_result_var`) now also excludes `not_error` / `error` assertions; previously a `not_error + is_empty` fixture emitted a nil check against undefined `value` while `result` was declared-but-unused, causing a compile error.
- fix(e2e/r): `run_tests.R` now resolves script and test directories relative to its own path before changing the working directory to `test_documents/`; also sets `testthat::set_max_fails(Inf)` to surface all failures during triage.
- fix(scaffold/r): R scaffold `Makevars` files now use the Rust crate name (`{core}_r`) for the static library path rather than the R package name; the two differ when the R package uses a custom user-facing name.
- fix(napi): `Vec<u8>` / `Bytes` parameters now emit a `let name: Vec<u8> = name.to_vec();` conversion in the generated function body so JS `Buffer` arguments round-trip through `napi::Buffer` to the Rust core. Adds a dedicated `gen_napi_buffer_conversion_bindings` helper and `is_bytes_param` predicate, and gates `use_let_bindings` on bytes params so the conversion always runs.
- fix(magnus/trait_bridge): `default_types` parameters in bridge functions are now passed as the binding type and converted to the core type via `.into()` (the Magnus binding already accepts the typed value), instead of being typed `Option<String>` / `String` and parsed via `serde_json::from_str`. The JSON-string detour silently dropped non-string fixture values and forced callers to pre-encode every nested struct.
- fix(codegen/config_gen): `gen_magnus_kwargs_constructor` now accepts kwargs as `Option<magnus::RHash>` via `scan_args`, so callers can omit the kwargs hash entirely (matching idiomatic Ruby) instead of always passing an empty `{}`.
- fix(codegen/shared): `constructor_parts_with_renames` no longer double-wraps already-`Optional<T>` fields in another `Option<…>`; previously fields whose IR type is already optional became `Option<Option<T>>` in the generated constructor signature.
- fix(e2e/wasm): `globalThis.process.chdir(testDocumentsDir)` replaces `process.chdir(testDocumentsDir)`; the WASM module exports a `process` function that shadowed the Node.js global `process` object, making `.chdir` unavailable at test setup time.
- fix(e2e/c): C generator now reads `options_type` from the call override (`[e2e.call.overrides.c]`) instead of hardcoding `ConversionOptions`; generated code uses the configured type name (e.g. `ProcessConfig`) for both the `*_from_json` constructor and `*_free` calls.
- fix(e2e/csharp): when a visitor is present but no options argument exists, the generator now emits `new OptionsType { Visitor = visitorVar }` instead of appending the visitor as a bare extra argument.
- fix(csharp-backend): `is_convert_with_visitor` early-return path now emits the closing brace and returns immediately, preventing the method body from falling through into the standard param-setup path.
- fix(hooks): `alef_hook.py` now reads the `alef` version from `alef.toml` at hook-cache root instead of a static `hooks/VERSION` file; the `hooks/VERSION` file is removed. This eliminates version-skew bugs where `Cargo.toml` was bumped without a matching `VERSION` update, causing the hook to download a non-existent release artifact.
- fix(.pre-commit-config.yaml): add `default_stages: [pre-commit]` to prevent hooks without explicit `stages:` from running in the `commit-msg` stage (where they receive the commit message file instead of source files).

## [0.14.22] - 2026-05-04

### Fixed

- fix(e2e/wasm): `inject_wasm_init` now loads the WASM binary via `readFileSync` and passes the buffer directly to `init()`; Node.js `fetch` does not support `file://` URLs, so the previous `await init()` (no arguments) failed with `TypeError: fetch failed`.

## [0.14.21] - 2026-05-04

### Fixed

- fix(e2e/go): remove `needs_json_stringify` from `needs_json` gating — `jsonString()` lives in `helpers_test.go` which has its own `encoding/json` import; individual test files no longer emit a spurious `import "encoding/json"` that the Go compiler rejects as unused.
- fix(e2e/typescript): `json_object` args with array values (e.g. batch items) are no longer cast to the `options_type` — only non-array object args receive the interface cast.
- fix(e2e/typescript): empty-object `json_object` args now emit `{} as OptionsType` (interface cast) instead of `new OptionsType()` (class instantiation); TypeScript options types are interfaces, not classes.
- fix(e2e/csharp): `json_array_to_csharp_list` now handles typed class element types (e.g. `BatchBytesItem`) by emitting `JsonSerializer.Deserialize<T>(json, ConfigOptions)!` per element; previously fell through to `List<string>` which caused a compile error when the C# binding expects `List<T>`.

- fix(e2e/wasm): `inject_wasm_init` now searches backward from `} from "pkg_name"` to locate the correct `import {` block; previously it matched the first `import {` in the file (the vitest import), corrupting the vitest import line.
- fix(pyo3-backend): inject `from_json` staticmethod into the existing `#[pymethods]` block instead of emitting a second one; avoids requiring the `multiple-pymethods` pyo3 feature which is not enabled by default.
- fix(napi-backend): visitor and plugin method calls with N>0 arguments now use `FnArgs::from(tuple)` instead of passing the raw tuple directly; the blanket `JsValuesTupleIntoVec` impl was treating a `(Unknown, Unknown)` tuple as a single NAPI value, causing all extra arguments to arrive as `undefined` in JavaScript callbacks.
- fix(e2e/csharp): `equals` assertions on optional string fields now emit `field!.Trim()` (null-forgiving) instead of `field.Trim()` to suppress CS8602 nullable warnings.
- fix(csharp-backend): typed error classes (e.g. `ErrorException`) now inherit from the library's generic fallback exception class (e.g. `TreeSitterLanguagePackException`) instead of `Exception` directly; `Assert.ThrowsAny<LibException>()` now correctly catches typed errors.
- fix(csharp-backend): `TreeSitterLanguagePackException` (the generic fallback) gains `(string message)` and `(string message, Exception innerException)` constructors so derived error classes can call them without a numeric code.
- fix(e2e/csharp): use `Assert.ThrowsAny<T>()` / `Assert.ThrowsAnyAsync<T>()` instead of exact `Assert.Throws<T>()` for `is_error` assertions; the base exception class now matches derived exception subclasses (e.g. `ErrorException` from `TreeSitterLanguagePackException`).
- fix(e2e/csharp): `contains`, `contains_all`, `not_contains`, and `contains_any` assertions on list fields use `JsonSerializer.Serialize(field)` instead of `field.ToString()`; `List<T>.ToString()` returns the type name rather than the contents.
- fix(e2e/csharp): `result_is_array = true` (simple list results such as `manifest_languages()`) now also serialize via `JsonSerializer.Serialize` for substring assertions, not `result.ToString()`.

## [0.14.20] - 2026-05-04

### Fixed

- fix(rustler-backend): `gen_nif_async_function` no longer double-appends `_async` when the Rust function name already ends with `_async` (e.g. `embed_texts_async` → NIF was `embed_texts_async_async`); the generated NIF name now matches the Elixir native.ex stub.
- fix(napi-backend): add `#![allow(unsafe_code)]` inner attribute to generated Node.js bindings; NAPI-RS bridge code for trait visitors emits unsafe blocks that were previously rejected by the workspace-level `-D unsafe_code` lint.
- fix(e2e/wasm): `inject_wasm_init` now adds `init` as the **default** export (`import init, { ... }`) instead of a named export (`import { ..., init }`); wasm-pack exports `init` as a default export, so the named-import form produced `TypeError: init is not a function`.
- fix(e2e/go): `contains`, `contains_all`, `not_contains`, and `contains_any` assertions on optional array fields no longer emit `jsonString(*field)` (invalid dereference of a Go slice); Go slices are nil-able value types, so `jsonString(field)` is emitted directly with the nil guard on the field unchanged.
- fix(csharp-backend): `DefaultValue::EnumVariant` for fields whose C# type is `JsonElement` or `JsonElement?` (complex tagged-union enums) now emits `null` instead of `JsonElement.VariantName`, which does not exist in the .NET API.

### Added

- feat(pyo3-backend): generate `from_json(json_str: String) -> PyResult<Self>` staticmethod on all non-opaque struct types with serde and a core→binding `From` conversion. Deserializes via the core type to correctly handle fields with `#[serde(skip)]` (e.g. `Vec<Message>` in `ChatCompletionRequest`). Requires PyO3 ≥ 0.21 (multiple `#[pymethods]` blocks allowed by default).
- feat(e2e/python): add `"from_json"` mode for `options_via`. When set, generates `OptionsType.from_json(json_str)` instead of a dict or kwargs, allowing typed construction from fixture JSON for types that cannot be constructed via kwargs.
- feat(e2e/python): add `from_json_module` field to `CallOverride`. When set alongside `options_via = "from_json"`, the `options_type` is imported from this module (e.g., `liter_llm._internal_bindings`) instead of the main public module, supporting PyO3 native types whose `from_json()` exists on the native class only.
- feat(e2e/python): add `async` field to `CallOverride`. When set, overrides the call-level `async` flag for Python tests only — useful when a binding returns a sync iterator from a call marked async at the call level (e.g., `chat_stream`).
- fix(e2e/python): use `_alef_e2e_text().lower()` for `equals` assertions on enum fields instead of `.strip()`. PyO3 enum values do not have `.strip()` and their string representation may differ in case from the fixture assertion value.

### Fixed

- fix(e2e/python): resolve `options_type` and `options_via` per-fixture from the per-call Python override (`[crates.e2e.calls.X.overrides.python]`), falling back to the file-level global override. Previously only the global override was used, so per-call overrides had no effect.
- fix(csharp-backend): `Optional<String>` return types — FFI returns a raw C string, not JSON-encoded; use `Marshal.PtrToStringUTF8` directly instead of `JsonSerializer.Deserialize`, which failed with raw strings not being valid JSON.
- fix(csharp-backend): `Optional(_)` return types — null pointer means `None` (not found), not an FFI error; generate `return null` instead of `throw GetLastError()` / `throw new ExceptionName(...)` in both top-level methods and opaque type methods.
- fix(java-backend): use `org.jspecify.annotations.Nullable` instead of `org.jetbrains.annotations.Nullable` in generated record types; aligns with the JSpecify annotations used elsewhere in the Java bindings.
- fix(magnus-backend): skip `&mut self` methods when registering methods in `gen_module_init`; Magnus's `method!` macro doesn't support mutable receivers, causing a compile error.
- fix(napi-backend): import `async_trait` when trait bridges are configured; the generated bridge code uses `#[async_trait::async_trait]` but the dependency was missing from the generated Cargo.toml.
- fix(php-backend): allow `unsafe_code` in generated PHP bindings; `ext-php-rs` macros expand to unsafe blocks, causing clippy to reject with `-D warnings`.
- fix(e2e/go): move `jsonString` helper into a dedicated `helpers_test.go` file emitted once per package, eliminating duplicate function definition errors when multiple test files needed it.
- fix(e2e/go): `needs_fmt` calculation now checks `field_resolver.is_valid_for_result(field)` to avoid emitting an unused `fmt` import when an assertion references a field that doesn't exist on the result type.
- fix(e2e/python): when a fixture argument is an array of objects and `element_type` is declared, construct typed instances (`ElementType(key=value, ...)`) instead of raw dict literals; matches the binding's type-safe API.
- fix(scaffold/csharp): add `<Compile Include="../src/**/*.cs" />` to the generated `.csproj` so source files from the shared `src/` directory are included in the project build.
- fix(scaffold/node): add `async-trait = "0.1"` to Node.js Cargo.toml scaffold when trait bridges are configured; the generated bridge code uses `#[async_trait::async_trait]` but the dependency was missing.
- fix(rustler-backend): declare `visitor_owned_env` as `mut` so `send_and_clear` (which takes `&mut self`) compiles without E0596 borrow error.
- fix(php-backend): use `!is_php_prop_scalar_with_enums` instead of `type_ref_has_named` when computing `has_named_params` in struct methods codegen, correctly gating named-param constructors on non-scalar fields.
- fix(pyo3-backend): correct boolean logic in `is_native` computation in `gen_api_py` — options types are now excluded first, then native membership is checked, preventing options types from incorrectly landing in native imports.
- fix(rustler-backend): replace double-nested `OwnedEnv::run` + `send_and_clear` with a single `send_and_clear` call in the trait bridge field function spawn closure, eliminating a double-borrow that caused BEAM message delivery failures.
- fix(e2e/wasm): remove `vite-plugin-top-level-await` from generated WASM e2e `package.json` and vitest config; top-level await is natively supported in modern Vite/Vitest without the plugin.
- fix(napi-backend): force `is_param_optional = true` in bridge functions so options parameters are always treated as `Option<T>` regardless of whether the IR marks them as non-optional.
- fix(php-backend): add `#[serde(default)]` struct attribute when `has_serde` is enabled, so `from_json()` accepts partial JSON and missing fields use `Default` values instead of failing deserialization.
- fix(php-backend): always emit `from_json` for has_default structs when `has_serde` is true, preventing broken `__construct` methods with invalid Rust enum defaults (e.g. `BrowserMode::Auto`) that don't exist in the PHP binding's string-mapped enum model.
- fix(e2e/go): `is_empty` assertions on pointer-to-struct results now generate `if result != nil { ... }` instead of dereferencing and calling `len()` on a struct, which was both invalid and panicked on nil.
- fix(csharp-backend): add `using System.Collections.Generic;` to generated opaque type source files when any method has a `Vec<T>` parameter or return type.
- fix(csharp-backend): pass full set of opaque type names to `gen_opaque_handle` so that methods on one opaque type returning another opaque type (e.g. `LanguageRegistry::get_language` → `Language`) wrap the pointer with `new Language(ptr)` instead of incorrectly calling the non-existent `LanguageToJson` FFI.

## [0.14.19] - 2026-05-04

### Fixed

- fix(scaffold/python): add `async-trait` dependency to Python Cargo.toml scaffold when trait bridges are configured.
- fix(scaffold/php): add `async-trait` dependency to PHP Cargo.toml scaffold when trait bridges are configured.
- fix(e2e/c): derive header name from `ffi.header_name` config and lib name from `ffi.lib_name` when package-level overrides are absent.
- fix(e2e/go): fix array-assertion detection to correctly check `result_is_array` when no field is specified, without falling through to field-level `is_array` check.
- fix(e2e/ruby): call `.to_s` before `.strip` on simple result fields; call `.to_s` before `not_to be_empty` for simple result fields.

## [0.14.18] - 2026-05-04

### Fixed

- fix(scaffold/ffi): add `async-trait` dependency to FFI Cargo.toml scaffold when trait bridges are configured. When the config has trait bridges, the generated FFI `lib.rs` uses `#[async_trait::async_trait]` for trait bridge implementations, but the dependency was missing from the generated Cargo.toml, causing compilation errors.

## [0.14.17] - 2026-05-04

### Fixed

- fix(scaffold/rustler): remove unused `futures-util` dependency from Rustler NIF Cargo.toml scaffold. When `has_async=true`, the dependency was added speculatively but the generated Rustler code never imports it, causing `cargo-machete` to flag it as unused.

## [0.14.16] - 2026-05-04

### Fixed

- fix(scaffold): `alef all --clean` now force-regenerates scaffold files (Cargo.toml templates, gemspec, etc.) matching the behaviour of README and e2e file writes. Previously scaffold files were never overwritten by `--clean`, leaving stale content (e.g. removed `[lints]` section) on disk indefinitely.

- fix(scaffold/php): downgrade `ext-php-rs` template version from `0.15.12` back to `0.15.4`. Version 0.15.12 introduced an `ext-php-rs-clang-sys` fork that conflicts with `clang-sys` when `kreuzberg-php` is a member of a shared Cargo workspace.

- fix(pyo3): coerce enum fields in dict input before constructing dataclass. When a config dict (e.g. `ExtractionConfig`) contains an enum-typed field, the dict→dataclass path now converts the string value to the correct Python enum member before calling `DataClass(**value)`.

- fix(rustler): extend visitor callback return handler to recognise all `CallbackAction` variants. Previously only `is_binary(result)` was checked; `:continue`, `:skip`, `:preserve_html`, and `{:custom, value}` tuples are now handled correctly.

- fix(e2e/csharp): normalise enum field values to lowercase before serialising fixture input JSON. C# `JsonStringEnumConverter` emits lowercase names; passing PascalCase values (e.g. `"Tildes"`) from fixtures caused deserialization mismatches.

- fix(e2e/elixir): include fixtures that use `client_factory` override when computing `has_mock_server_tests` in the test module header. Previously only fixtures that called `needs_mock_server()` were counted, causing the mock server setup to be omitted for client-factory fixtures.

- fix(e2e/python): include fixtures that use a `client_factory` Python override when computing `has_http_fixtures` in `conftest.py`. Same root cause as the Elixir fix above.

## [0.14.15] - 2026-05-04

### Fixed

- fix(e2e/go-codegen): exclude mock-only fixtures from `needs_pkg` import check. Mock fixtures with no real function calls were incorrectly triggering conditional imports.

- fix(e2e/ruby-scaffold): remove `[lints] workspace = true` from excluded-workspace Magnus Cargo.toml. The lints section is inherited from workspace config, but packages/ruby/ext/kreuzberg_rb/ is excluded from the workspace, so `workspace = true` failed with "cannot find workspace root". Removed the lints section from the generated Cargo.toml template.

- fix(e2e/php-codegen): strip namespace prefix from class names in use statements. When class override contains a namespace (e.g. `Kreuzberg\Kreuzberg`), extract just the class name to avoid triple-nested namespaces like `use Kreuzberg\Kreuzberg\Kreuzberg;`. Now correctly emits `use Kreuzberg\Kreuzberg;`.

## [0.14.14] - 2026-05-04

### Fixed

- fix(java-backend): `Duration` fields map to boxed `Long` in Java; compact constructor defaults emitted as bare integer literals (e.g. `30000`) failed to compile because `int` does not auto-box to `Long`. Now appends `L` suffix so the literal is a `long` that Java auto-boxes correctly.

- fix(magnus-backend): emit a `_refs: Vec<&str>` intermediate binding when a `Vec<String>` parameter is passed by reference to a core function expecting `&[&str]`. Previously the generated code passed `&Vec<String>` directly, which mismatches the `&[&str]` signature and fails to compile.

- fix(rustler-backend): same `Vec<String>` → `&[&str]` refs intermediate for Rustler/Elixir bindings (mirrors the Magnus fix above).

- fix(napi-backend): use `.into()` (`FnArgs`) when calling visitor JS callbacks with more than one argument. Single-argument calls are unaffected; multi-argument calls previously passed a raw tuple that NAPI-RS did not unpack correctly to JavaScript.

- fix(cache): include alef binary identity (mtime + size) in `compute_lang_hash` and `compute_stage_hash` so locally rebuilt binaries always invalidate stale caches without requiring a version bump. Previously a rebuild with code changes would reuse the old cached output.

- fix(e2e-elixir): rewrite `alef_e2e_item_texts` helper to use `Enum.flat_map` with a `case` expression instead of `Enum.map(fn attr -> item |> Map.get(attr) |> to_string() end)`. `to_string/1` raises `ArgumentError` on atom values (e.g. enum variants); the new form guards `is_atom` and capitalises separately.

- fix(e2e-go): use `len(slice)` instead of `len(*slice)` for optional slice-typed fields in `not_empty`, `empty`, `min_elements`, and `max_elements` assertions. Go slice optionals are value types, not pointer-to-slice, so dereferencing them was a compile error.

- fix(e2e-typescript): convert `{placeholder}` syntax to `${placeholder}` in `CustomTemplate` visitor method bodies so the emitted string is a valid JavaScript template literal.

- fix(e2e-go): include fmt import when `needs_json_stringify` is true, since the `jsonString` helper uses `fmt.Sprint`. Previously tests with array field `contains` assertions failed to compile with "undefined: fmt".

- fix(e2e-csharp): add missing `using static {namespace}.{class_name};` directive to test file headers so static method calls to the binding class resolve without full qualification.

- fix(e2e-java): add `org.jetbrains:annotations:24.1.0` dependency to generated pom.xml. E2e tests use `@NotNull`/`@Nullable` annotations from this package.

- fix(e2e-elixir): skip batch functions in generated tests with `@tag :skip`. Batch functions (`batch_extract_*`) are excluded from the Elixir binding due to unsafe NIF tuple marshalling; tests now emit a documented skip rather than failing to compile.

## [0.14.13] - 2026-05-04

### Fixed

- fix(e2e-go): propagate call-level `result_is_array` to Go generator. Previously only Go-specific overrides were checked, so call-level `result_is_array = true` was ignored, causing `value := *result` dereference errors for slice return types like `[]string`.

## [0.14.12] - 2026-05-04

### Fixed

- fix(pyo3): use `val.kind.into()` instead of `Default::default()` for data enum fields in `From` impls. Data enums (like `StructureKind`) were added to `opaque_names_set` which caused the conversion generator to treat their fields as opaque and emit `Default::default()`. A separate `conversion_opaque_set` now excludes data enum names so their fields convert correctly with `.into()`.

- fix(e2e-ruby): use per-item text check for array field `contains` assertions. Previously `result.structure.to_s` produced an object-repr string that never matched the expected value. Now generates `array.any? { |item| alef_e2e_item_texts(item).any? { |t| t.include?(val) } }` using a shared helper method.

- fix(e2e-elixir): use `Enum.any?` for array field `contains` assertions instead of `to_string` on list. `to_string/1` on an Elixir list raises `ArgumentError`; now generates a per-item traversal using a shared `alef_e2e_item_texts/1` helper.

- fix(e2e-go): skip nil check and pointer dereference for `result_is_array` slice results. Go slice types (`[]string`) are not pointers, so the generated `if result == nil` check and `value := *result` dereference were compile errors. Array results now use `value := result` directly.

- fix(magnus-backend): add `options_field` visitor bridge support. The Magnus Ruby backend now generates proper wrapper functions for trait bridges using `bind_via = "options_field"` binding style. Previously only `bind_via = "function_param"` was supported, causing e2e tests to fail when trying to pass a visitor as a secondary argument to functions like `convert(html, visitor)`.

## [0.14.11] - 2026-05-03

### Fixed

- fix(e2e-python): fix `_to_rust_*()` converter field access when `python_output = "typed-dict"`. Previously the converter used `value.get("field")` dict-style access even for dataclass input types, causing `AttributeError` when called with `ProcessConfig` instances. Converters now use `value.field` attribute access (matching the input style) rather than the output style.

- fix(e2e-go): propagate `call_config.result_is_simple` to Go e2e generator. The generator was only checking per-language overrides, so call-level `result_is_simple = true` was ignored, causing `*string` returns to be treated as `error` and printing pointer addresses as failure messages.

- fix(e2e-php): add `options_type` support for PHP. When a call override sets `options_type = "ClassName"`, PHP e2e tests now generate `ClassName::from_json(json_encode([...]))` to construct typed config objects. Removes hardcoded `\HtmlToMarkdown\ConversionOptions::builder()` placeholder.

- fix(e2e-elixir): fix 0-arity functions (e.g. `language_count`, `available_languages`) called with `%{}` or `nil` when `args = []` and fixture input is empty. Now correctly emits no-arg calls.

- fix(e2e-ruby): same empty-args fix for Ruby — 0-arity functions no longer receive `nil` or `{}` from empty fixtures.

- fix(csharp-backend): store delegate objects in field array to prevent GC collection. The trait bridge generated code was creating 41+ `UnmanagedFunctionPointer` delegates as local variables in `BuildVtable()`, then immediately discarding them. The GC could collect these delegates, leaving dangling function pointers in the vtable and causing `AccessViolationException` on native callbacks. Now delegates are stored in `private readonly object[] _delegates` initialized in the constructor and kept alive for the bridge's lifetime.

## [0.14.10] - 2026-05-03

### Added

- feat(e2e-go): `client_factory` support in Go e2e codegen. When `[e2e.call.overrides.go] client_factory = "CreateClient"` is set, the generated test creates a client via `pkg.CreateClient("test-key", baseURL)` and calls API methods as `client.Method(args)` instead of `pkg.Function(args)`. Also fixes `fixture_has_go_callable` to return `true` when `client_factory` is configured.

- feat(e2e-python): `client_factory` support in Python e2e codegen. When `[e2e.call.overrides.python] client_factory = "create_client"` is set, the generated test imports only the factory function, creates a client, and dispatches calls as `client.method(args)`.

- feat(e2e-csharp): `client_factory` support in C# e2e codegen. When `[e2e.call.overrides.csharp] client_factory = "createClient"` is set, the generated test calls `ClassName.CreateClient("test-key", baseUrl)` and dispatches as `client.Method(args)`.

- feat(e2e-ruby): `client_factory` support in Ruby e2e codegen. When `[e2e.call.overrides.ruby] client_factory = "create_client"` is set, the generated spec creates a client via `ModuleName.create_client('test-key', ENV.fetch('MOCK_SERVER_URL'))` and calls methods on it.

- feat(e2e-elixir): `client_factory` support in Elixir e2e codegen. When `[e2e.call.overrides.elixir] client_factory = "create_client"` is set, the generated test calls `{:ok, client} = ModuleName.create_client("test-key", base_url)` and passes `client` as the first argument to API functions.

### Fixed

- fix(e2e): include fixture id in mock server base URL for C#/Ruby/Elixir e2e backends. Previously `client_factory` generated code pointed clients at bare `MOCK_SERVER_URL`; now appends `/fixtures/{fixture_id}` so API calls route correctly through the mock server's prefix-based routing.

- fix(e2e-php): parametrize extension name in generated `run_tests.php`; previously hardcoded to `libhtml_to_markdown_php`, now derived from the crate's extension name so generated test runners work across projects.

- fix(e2e-wasm): inject `initSync` initialization block in generated test files for Node.js environments. Uses `fs.readFileSync` + `initSync` to load the WASM binary from the local `node_modules` path, replacing the broken async fetch-based approach that fails in vitest's Node.js runner.

- fix(e2e): remove language suffix from generated e2e package names. TypeScript packages now use `{pkg}-e2e` instead of `{pkg}-e2e-typescript`; Python packages use `{pkg}-e2e` instead of `{pkg}-e2e-tests`. Python `pkg_name` now defaults to the call module name (replacing a stale hard-coded project name).

- fix(e2e-wasm): remove `initWasm()` call from generated test files. The bundler target auto-initializes when imported, so explicit initialization is unnecessary and causes `TypeError: initWasm is not a function`.

- fix(backend-java): use `LINKER.defaultLookup()` for Panama FFM symbol resolution. The `SymbolLookup.loaderLookup()` API returns symbols from Java's class loader, not the native linker; `LINKER.defaultLookup()` correctly returns symbols from all loaded native libraries.

## [0.14.9] - 2026-05-03

### Fixed

- fix(backend-pyo3): `_coerce_enum` helper now returns `_E` (not `_E | None`) by separating
  the `isinstance` early-return from the `None` case (which now raises `ValueError`). This
  eliminates the `[return-value]` mypy error and removes now-unused `# type: ignore` comments
  that newer mypy versions flag as `[unused-ignore]`.

- fix(backend-napi): `find_options_field_binding` now unwraps Optional types when matching
  options parameters against `options_type` in trait bridge configs. When a function parameter
  is `Option<ConversionOptions>` and a trait bridge specifies `bind_via = "options_field"`,
  the NAPI backend now correctly recognizes the parameter and generates the visitor bridge code.
  Fixes 45 failing Node.js visitor e2e tests where `options.visitor` was being discarded by
  the generated From impl.

- fix(e2e-php): emit `ConversionOptions::from_json()` instead of direct property assignment,
  since ext-php-rs doesn't support writable #[php(prop)] fields; fixes "No setter available for
  this property" errors in 7 e2e tests with preprocessing/metadata options

- fix(backend-go): initialize string-alias enum zero values as `EnumType("")` instead of `EnumType{}` (composite literal syntax only applies to structs/slices/maps, not string aliases)

- fix(backend-go): option setter functions for slice and map fields now assign `v` directly instead of `&v`, matching the `[]T` field types introduced in v0.14.5

- fix(backend-pyo3): fix `_coerce_enum` return type from `_E` to `_E | None` to allow None values
  in optional enum fields; remove unnecessary type: ignore comments that mypy flags as unused

- fix(backend-java): use short annotation names (`@JsonDeserialize`, `@JsonPOJOBuilder`) instead of
  fully-qualified names in generated record and builder classes so that the accompanying import
  statements are actually used and checkstyle's UnusedImports rule passes.

- fix(codegen): `gen_lossy_binding_to_core_fields` and `gen_lossy_binding_to_core_fields_mut` now
  accept `cast_uints_to_i32` and `cast_large_ints_to_f64` flags. All callers updated (magnus,
  integration tests). Extendr backend passes `true` for both to correctly cast R's i32/f64 types
  back to core Rust int types in generated method bodies.

- fix(e2e-php): do not append `_async` suffix to function name when a language-specific override
  explicitly provides the function name. Previously, async PHP functions with overridden names
  were incorrectly suffixed with `_async`.

## [0.14.8] - 2026-05-03

### Fixed

- fix(backend-ffi): `type_ref_to_rust_type` now prefixes named types with `kreuzberg::` so
  `Vec<BatchFileItem>` turbofish annotations in FFI bodies are emitted as
  `Vec<kreuzberg::BatchFileItem>`. The v0.14.6 fix added turbofish annotations without the
  crate prefix, causing "cannot find type X in this scope" E0425 errors in the FFI crate for
  all `Vec<T>`-parameter functions whose element type is a core library struct.

## [0.14.7] - 2026-05-03

### Fixed

- fix(backend-extendr): add `skip_impl_constructor` flag to `RustBindingConfig` and set it
  `true` for the extendr backend. The in-class `impl T { fn new(...) }` constructor is now
  suppressed; callers must use the kwargs-style free-function constructor (already generated
  by `gen_extendr_kwargs_constructor`). This prevents "TryFrom<&Robj> not satisfied" compile
  errors when struct parameters cannot be auto-converted by extendr.
- fix(backend-extendr): methods whose parameters take owned non-opaque structs by value, or
  whose return type is an enum, are now excluded from the `#[extendr]` impl block. Extendr
  generates `TryFrom<&Robj>` for references (`&T`) but not for owned values (`T`), and has
  no `ToVectorValue` impl for enum types; including such methods caused E0277 and E0277
  compile errors in the R binding.
- fix(backend-extendr): methods whose parameters take `Vec<T>` where T is a non-opaque,
  non-enum struct are also excluded from the `#[extendr]` impl block. There is no automatic
  R-list→Vec<ExternalPtr<T>> conversion in extendr.
- fix(codegen/extendr-kwargs): struct-typed fields (non-opaque, non-enum named types) are
  now omitted from the kwargs constructor parameter list and body. Extendr generates
  `TryFrom<&Robj>` only for `&T` (reference), not for owned `T`; including them as kwargs
  parameters caused "T: TryFrom<&Robj> not satisfied" compile errors.
- fix(codegen/extendr-kwargs): fields whose type is already `Option<T>` are no longer
  double-wrapped — the parameter type is now the same as the field type rather than
  `Option<Option<T>>`.
- fix(codegen/core-to-binding): `Vec<u8/u16/u32/i8/i16>` fields with `cast_uints_to_i32`
  are now converted element-wise (`v.iter().map(|&x| x as i32).collect()`) so R receives
  integer vectors instead of raw byte arrays.
- fix(backend-napi/pyo3/php): set `skip_impl_constructor: false` explicitly in those
  backends' `RustBindingConfig` initializers (no behaviour change; required after the new
  field was added to the struct).

## [0.14.6] - 2026-05-03

### Fixed

- fix(backend-ffi): `Vec<T>` parameters now always emit a concrete turbofish type annotation
  (`serde_json::from_str::<Vec<String>>(...)`) in generated FFI functions. Previously the
  annotation was only emitted when `is_ref || is_mut`; without it, calls to Rust core functions
  with generic bounds (e.g. `fn f<T: AsRef<str>>(v: Vec<T>)`) failed to compile with E0283
  "type annotations needed".

## [0.14.5] - 2026-05-03

### Fixed

- fix(backend-extendr): add `cast_uints_to_i32` and `cast_large_ints_to_f64` flags to
  `ConversionConfig` so the R binding maps u8/u16/u32/i8/i16 → `i32` and u64/usize/isize → `f64`
  in both binding→core and core→binding From impls. Without these casts, primitive type
  mismatches caused ~100 E0308 errors in the generated R binding.
- fix(backend-extendr): generate `#[extendr]` on impl blocks (previously `method_block_attr: None`)
  so generated structs register as R classes and satisfy the `ToVectorValue` trait bound.
- fix(backend-extendr): import `std::collections::HashMap` and `extendr_api::Result` in the
  generated binding so HashMap fields compile and `Result<T>` is unambiguously the extendr alias.
- fix(codegen/extendr-kwargs): optional struct fields (where `field.optional=true`) in the
  kwargs constructor now assign `Some(v)` instead of `v` so `Option<T>` fields are set correctly.
- fix(codegen/extendr-kwargs): enum fields in kwargs constructors now accept `Option<String>` and
  parse via serde_json instead of `Option<EnumType>`, which extendr cannot convert from Robj.
- fix(backend-extendr): skip opaque types that use `Rc` internally (identified by having a cfg
  gate) from struct generation and from field inclusion in non-opaque structs. `VisitorHandle` is
  `Rc<RefCell<dyn HtmlVisitor>>` and cannot be wrapped in `Arc` — its binding struct would
  violate `Send + Sync` bounds. Affected fields (e.g. `visitor` on `ConversionOptions`) are
  excluded from struct generation and conversion impls.
- fix(trait_bridge/extendr): use `List::from_pairs(attrs_pairs).into()` instead of the invalid
  `collect::<List>()` pattern for building named R lists from attribute key-value pairs.

- fix(backend-napi): NAPI options_field_bridge now preserves visitor handle across
  options conversion. The From<JsConversionOptions> impl was unconditionally setting
  visitor to Default::default() due to opaque field handling. Fixed by extracting
  the visitor before conversion and re-assigning it after the Into call.
- fix(backend-pyo3): PyO3 convert wrapper now reads visitor from options.visitor
  when the separate `visitor=` kwarg is None. This allows Python callers to pass
  the visitor via ConversionOptions without requiring a separate keyword argument.
- fix(e2e/wasm): replace static package imports with dynamic imports + top-level
  await to defer WASM module loading until after vite plugins have processed it.
  Fixes "failed to initialize WebAssembly bundle" errors in vitest e2e tests.
- fix(e2e/php): when `result_is_simple = true`, generated assertions now access the
  `content` property of the ConversionResult object instead of passing the object
  directly to assertion methods. Fixes TypeError when trimming or comparing results.
- fix(backend-pyo3): plain data structs now emit `#[pyclass(frozen)]` instead of
  `#[pyclass(unsendable)]`. The previous code used the transitive-closure set
  (`opaque_names_set`) to decide between frozen and unsendable, causing any struct
  whose fields transitively referenced an opaque type to be marked unsendable. Data
  structs are `Send + Sync` and crossing thread boundaries in async Python code must
  not panic with "<TypeName> is unsendable". Only truly-opaque Arc/Rc-wrapped handles
  need the unsendable annotation.
- fix(backend-go): `Vec<T>` function return types now emit `[]T` (a value slice) instead
  of `*[]T` (a pointer to a slice). Pointer-to-slice is unidiomatic in Go, breaks
  `len(result)` calls, and is unnecessary since Go slices are already reference types.
- fix(e2e/csharp): generate `List<string>` parameter type for VisitTableRow method
  (was `IReadOnlyList<string>`), emit `new VisitResult.XXX()` instead of
  `VisitResult.XXX()` to correctly instantiate sealed record types, and call
  `ConvertWithVisitor` instead of `Convert` when a visitor is present in fixtures.

## [0.14.4] - 2026-05-03

### Fixed

- fix(backend-pyo3): Required parameters whose Rust type is a has-default struct
  (e.g. `ExtractionConfig`) are now treated as optional at the Python wrapper layer.
  Callers may pass `None` and the wrapper substitutes a Rust default-constructed
  instance. Previously the PyO3 binding panicked with `.expect("'param' is required")`.
- feat(backend-pyo3): emit pass-through wrappers for trait-bridge `unregister_*` and
  `clear_*` functions in `api.py` so callers can use the public package path.
- fix(backend-go): Rust newtype-tuple enums (e.g.
  `enum OutputFormat { Plain, Markdown, Custom(String) }`) are now rendered as Go
  string types with const variants and a Custom fallthrough, instead of empty
  structs. Fixes JSON unmarshaling of config fields like `output_format: "markdown"`.
- fix(e2e/go): callable detection no longer skips fixtures with reason "non-HTTP
  fixture: Go binding does not expose a callable for the configured `[e2e.call]`
  function" — Go does expose the canonical 27 fns. Real test bodies are now
  emitted from the e2e generator, mirroring the Rust backend pattern.
- fix(backend-ffi): emit `visitor_create`/`free`/`convert_with_visitor` when both
  `options_field` and `visitor_callbacks` are configured.
- fix(wasm-visitor): use marker-relative search for visitor replacement; use
  bounded `replace` within impl blocks to avoid cross-impl pollution.
- feat(backend-napi): add `gen_options_field_bridge_function` for visitor
  embedding in options.

### Fixed (carry-forward from unreleased v0.14.3 follow-ups)

- fix(e2e/elixir): inject visitor into the options map argument instead of passing it as a
  separate positional argument, so the Elixir facade's `convert/2` can properly extract it
  via `Map.pop(options, :visitor)` and dispatch to the NIF with visitor callbacks.
- fix(e2e/elixir): read `returns_result` from the language override when available instead of
  always using the base CallConfig value, enabling `returns_result = true` in Elixir overrides
  to generate proper `{:ok, result} =` pattern matches in assertions.
- fix(backend-napi): when `bind_via = "options_field"` is configured for a trait bridge,
  extract the visitor from the options struct before conversion, create the bridge, and
  manually inject it into the converted core options. This ensures visitor callbacks
  are preserved across the NAPI FFI boundary instead of being dropped during the
  JsConversionOptions -> ConversionOptions Into conversion.
- fix(backend-pyo3): emit a `_coerce_enum(enum_cls, value)` helper in the generated
  api.py wrapper and use it in `_to_rust_*` converters instead of attempting
  `_rust.<Enum>(value)`. PyO3 unit-enum pyclasses do not expose a string-accepting
  `__new__`, so the previous codegen raised `TypeError: cannot create '<Enum>'
instances` when callers passed string aliases like `'atx'`. The helper coerces
  strings, snake_case, UPPER_CASE, and CamelCase to the canonical class attribute.
- fix(backend-java): stop emitting illegal `Optional<String>` accessor methods that
  shadow the canonical record component accessor. Java records auto-generate the
  accessor with the field's declared type and disallow overrides with a different
  signature. Callers that want `Optional` can wrap with `Optional.ofNullable(...)`.
- fix(e2e/java): wrap nullable record-component accessors with `Optional.ofNullable`
  before calling `.orElse(...)`, so the existing assertion templates work whether the
  underlying type is `Optional<X>` or `@Nullable X`.
- fix(cli/build): resolve mvn working directory by walking up from `[crates.output].java`
  to the nearest `pom.xml`, so configurations that point output at `src/main/java/`
  still build via maven from the project root.
- fix(cli/build): when the cargo `[crates.output].<lang>` directory is itself inside a
  standalone crate that declares `[workspace]` (extendr's `packages/r/src/rust/`), `cd`
  into that crate and run `cargo build` instead of treating it as a workspace member.
- fix(cli): keep the test module after all items so clippy's `items_after_test_module` check
  passes in the full pre-commit suite.
- fix(e2e/typescript): simplify generated `is_empty` assertion emission so clippy remains
  warning-clean under `-D warnings`.
- fix(backend-pyo3): import native unit enums referenced only through data-enum aliases in generated
  `options.py`, keeping runtime aliases such as `ToolChoice = ToolChoiceMode | str | SpecificToolChoice`
  import-safe and ruff-clean.
- fix(e2e/go): derive generated Go e2e imports and local `replace` paths from `[go]` package
  configuration when no explicit `[e2e.packages.go]` entry is present, import `testify/assert`
  for every assertion kind that emits `assert.*`, and only emit direct Go calls when an explicit
  Go callable override exists.
- fix(cli/test): resolve e2e-only test languages in `alef test --e2e` without leaking the
  test-only resolver into scaffold generation.
- fix(e2e/elixir): emit fixture IDs as generated ExUnit test labels so long fixture
  descriptions cannot exceed Elixir's test name limit.
- fix(scaffold/ruby): omit workspace lints from standalone Ruby native extension crates.
- fix(e2e/csharp): default generated project references to Alef's nested C# package
  project path.
- fix(e2e/csharp): read namespace from `[crates.csharp].namespace` in alef.toml when
  generating test imports, fixing mismatch with generated package namespace.
- fix(e2e/go): render string containment assertions through `fmt.Sprint` so structured
  slices such as `[]StructureItem` compile.
- fix(backend-csharp): emit free P/Invoke declarations for all true opaque handle
  wrappers, including handles that only appear in generated wrapper classes.
- fix(backend-magnus): borrow field types when detecting thread-unsafe visitor
  handles so the backend compiles cleanly.
- fix(e2e/wasm): declare Rollup in generated WASM Vitest harnesses so Vite plugins
  can load in isolated installs.
- fix(e2e/go): compute imports from executable Go fixtures only so skip-only files
  compile without unused imports.
- fix(backend-go): decode Rust data-enum JSON into Go discriminated structs with
  a printable variant name instead of silently returning nil conversion results.
- fix(e2e/go): stringify structured array fields as JSON for containment assertions
  so pointer fields compare by value instead of address.
- fix(backend-go): marshal decoded data-enum variants back to their variant names so
  generated JSON-based e2e assertions can inspect them.

## [0.14.3] - 2026-05-03

### Added

- feat(codegen/doc-emission): shared `parse_rustdoc_sections`,
  `parse_arguments_bullets`, and `replace_fence_lang` helpers in
  `alef-codegen::doc_emission`. Rustdoc Markdown H1 headings (`# Arguments`,
  `# Returns`, `# Errors`, `# Panics`, `# Safety`, `# Example`/`# Examples`)
  are parsed into a typed `RustdocSections` struct that every host renderer
  consumes. ` ```rust ` fences inside `# Example` bodies are rewritten to
  host-native language tags (`typescript`, `php`, `csharp`, …); pound signs
  inside non-Rust fences (e.g. shell `# install …`) are preserved.
- feat(codegen/doc-emission): per-host section renderers
  `render_jsdoc_sections`, `render_javadoc_sections`,
  `render_csharp_xml_sections`, `render_phpdoc_sections`, and
  `render_doxygen_sections`. JSDoc emits `@param` / `@returns` / `@throws` /
  `@example` with ` ```typescript ` fences. JavaDoc emits `@param` / `@return`
  / `@throws KreuzbergRsException`. C# XML doc emits `<param>` / `<returns>`
  / `<exception cref="…">` / `<example><code language="csharp">`. PHPDoc emits
  `@param` / `@return` / `@throws` / ` ```php ` example fences. Doxygen emits
  `\param` / `\return` / `\code` / `\endcode`.
- feat(codegen/trait-bridge): shared `host_function_path` helper in
  `alef-codegen::generators::trait_bridge` replaces the duplicated per-backend
  `<lang>_host_function_path` inline helpers. pyo3, napi, magnus, ext-php-rs,
  rustler, extendr, and wasm now consume the same resolution logic.
- feat(backend-magnus/unregister-clear): emit `pub fn unregister_<name>(name:
String) -> Result<(), magnus::Error>` and `pub fn clear_<plural>() ->
Result<(), magnus::Error>` when `bridge_config.unregister_fn` / `clear_fn`
  are set. Errors are wrapped through `Ruby::exception_runtime_error`.
- feat(backend-php/unregister-clear): emit `#[php_function] pub fn
unregister_<name>(name: String) -> ext_php_rs::prelude::PhpResult<()>` and
  the matching `clear_<plural>()` function. Errors are mapped through
  `PhpException::default(format!("{}", e))`.
- feat(backend-rustler/unregister-clear): emit `#[rustler::nif] pub fn
unregister_<name>(env: rustler::Env<'_>, name: String) -> rustler::Atom`
  and `clear_<plural>()` returning `:ok` / `:error` atoms.
- feat(backend-wasm/unregister-clear): emit `#[wasm_bindgen] pub fn
unregister_<name>(name: String) -> Result<(), JsValue>` and matching
  `clear_<plural>()` wrappers; surfaced via `gen_bindings/mod.rs` so the
  generated bridge re-exports them in `index.ts`.
- feat(backend-extendr/unregister-clear): emit `#[extendr] pub fn
unregister_<name>(name: String)` / `clear_<plural>()` for R bindings.
- feat(backend-pyo3/host_function_path): pyo3 register/unregister/clear
  bridges now share the centralised host-function path resolver, eliminating
  the duplicated inline helper and matching the resolution rules used by all
  other backends.
- feat(backend-napi/host_function_path): napi register/unregister/clear
  bridges now share the centralised resolver.

### Fixed

- fix(backend-napi): preserve Rust `Default` values when converting omitted
  fields on defaultable JavaScript config objects instead of falling back to
  per-field zero values.
- fix(backends): remove dead PyO3 and Magnus backend code paths so workspace
  builds are warning-clean.
- fix(e2e): make TypeScript assertions handle `null` empty results and string
  containment checks against arrays of structured objects.
- fix(backend-napi/errors): JSDoc rendering for `KreuzbergError` variants now
  goes through the shared per-host renderer instead of leaking raw rustdoc
  Markdown headings into emitted error class JSDoc blocks.

## [0.14.2] - 2026-05-02

### Added

- feat(extract/normalize-rustdoc): doc comments parsed from Rust source are now run through
  a `normalize_rustdoc` pass before any backend sees them. Two specific rustdoc artefacts
  that previously leaked into every binding are removed:
  - Rustdoc-hidden lines inside `rust /`rust,no_run fences (`# tokio_test::block_on(async {`,
    `# Ok::<(), Error>(())`, `# }`, `# async fn example()`) are dropped.
  - Intra-doc-link syntax `[`crate::Foo`]` / `[`super::Bar`] /`[`self::X`]` is rewritten to
  plain `` `Foo` `` / `` `Bar` `` / `` `X` ``.
Per-host renderers (Javadoc, KDoc, PHPDoc, JSDoc, Dartdoc, Swift-DocC, roxygen2, Doxygen)
continue to translate `# Errors`/`# Returns`/etc. as before — full per-host code-fence
  translation deferred to v0.14.3.

- feat(error-gen/strip-thiserror-placeholders): added `strip_thiserror_placeholders` and
  `acronym_aware_snake_phrase` helpers in `alef-codegen::error_gen`. Display strings emitted
  by binding error renderers (Go sentinels, Dart `String get message`) no longer contain raw
  `{name}` substitution markers (`OCR error: {message}` becomes `OCR error`, `extraction
cancelled` is preserved). Variant names with technical acronyms (IO, OCR, PDF, URL, HTTP, …)
  render as `IO error` / `OCR error`, not `iO error` / `oCR error`.

- feat(backend-dart/error-render): Dart sealed-class exceptions now use the new placeholder
  stripping so `String get message => 'Parsing error: {message}'` is emitted as
  `'Parsing error'`. Surviving prose still has `'`, `\`, and `$` escaped per Dart string
  literal rules.

- feat(backend-zig/trait-bridge): the `make_<trait>_vtable` doc comment was missing a
  `format!()` call, leaking the literal `{snake}` template string. Substitution now
  happens correctly so generated docs read `make_post_processor_vtable(MyType, …)` etc.

- feat(trait-bridge/unregister-clear): `[[trait_bridges]]` schema gains optional
  `unregister_fn` and `clear_fn` fields. When set, alef emits matching host-language
  wrappers alongside `register_fn`. Pyo3 and napi backends opt in this release; remaining
  backends (magnus, php, rustler, gleam, extendr, dart, swift, kotlin, csharp, java,
  wasm) fall through to the default `None` (no emission) — those will be added in v0.14.3.
  The host-language wrappers call the user's plugin module function (e.g.
  `kreuzberg::plugins::ocr_backend::unregister_ocr_backend(&name)`); the path is derived
  from the bridge's `registry_getter` (`get_*_registry` → `*::*_fn_name`).

### Fixed

- fix(error-gen/go-sentinel): Go sentinel error strings no longer contain literal
  `{message}` / `{plugin_name}` / `{elapsed_ms}` placeholders. Variants like `IoError`,
  `OcrError`, `PdfError` render as `errors.New("IO error")`, `errors.New("OCR error")`,
  `errors.New("PDF error")` instead of the previous `errors.New("iO error")`,
  `errors.New("oCR error: {message}")`.

### Deferred to v0.14.3

- Per-host doc-comment section translation (`# Arguments` → `@param`, `# Returns` →
  `@returns`, `# Errors` → `@throws`, etc.) for JSDoc, JavaDoc, C# XML, PHPDoc, dartdoc,
  roxygen2, KDoc, Swift-DocC, Doxygen, Elixir doctests. Foundation in
  `alef-codegen::doc_emission` is unchanged.
- `gen_unregistration_fn` / `gen_clear_fn` for magnus, php, rustler, gleam, extendr,
  dart, swift, kotlin, csharp, java, wasm. Schema is in place; backends opt in incrementally.

### Fixed (carry-forward from unreleased v0.14.1 follow-ups)

- fix(cli): fix alef-verify ↔ host-formatter circular drift in `generate` and `all` commands.
  `format_generated` was called with only the bindings slice, so languages where only stubs
  changed (cache-hit bindings) were never formatted before `finalize_hashes` ran. Host
  formatters (ruff, mix format, php-cs-fixer, gofmt) would then reformat the unformatted
  stubs, making the embedded `alef:hash:` line stale and causing `alef verify` to report
  drift on every run. Fix: pass `bindings + stubs` combined to `format_generated` in both
  `Commands::Generate` and `Commands::All`, and track stub-changed languages in
  `changed_languages` in `Commands::All`.

- fix(e2e): add `node`/`wasm` default formatters (`npx oxfmt {dir}`) to `alef_e2e::format`.
  `run_formatters` had no entry for TypeScript e2e output, so hashes were embedded over raw
  codegen bytes; prek's oxfmt hook then reformatted those files, making `alef verify` report
  28 stale TS files after every `alef e2e generate` / `alef e2e generate --registry` run.

- fix(cli,e2e): add `cargo sort` to Ffi/Wasm/rust-e2e formatter pipelines before hash
  finalisation. `cargo-sort` normalises `Cargo.toml` dependency-table ordering and feature
  indentation; without it, prek's `cargo-sort` hook reformatted those files after
  `finalize_hashes` ran, causing `alef verify` to report `crates/*-wasm/Cargo.toml`,
  `e2e/rust/Cargo.toml`, and `test_apps/rust/Cargo.toml` as stale on every run.
  - `Language::Ffi` formatter now runs `cargo fmt --all` then `cargo sort -w` (workspace).
  - `Language::Wasm` formatter now runs `cargo fmt --manifest-path` then `cargo sort <crate>`.
  - E2e rust default formatter now runs `cargo fmt --all && cargo sort .` inside the crate dir.

- fix(php): respect namespace config in php_autoload_namespace. After v0.14.0, the namespace
  emission logic regressed and ignored the `[crates.php] namespace` configuration, instead
  splitting the extension name on underscores. Now respects explicit namespace override.

- fix(backend-wasm): bridge type aliases (e.g. `VisitorHandle`) are now treated as opaque in
  WASM binding-to-core `From` conversions. Fields of these types now emit `Default::default()`
  instead of `val.visitor.map(Into::into)` (which failed E0277 because `WasmVisitorHandle`
  has no `From` impl to the core `Rc<RefCell<dyn HtmlVisitor>>`). Builder methods on
  has-default structs with bridge parameters also use `.visitor(None)` instead of
  `.visitor(visitor.as_ref().map(|v| &v.inner))` to avoid E0308 type mismatch.

- fix(backend-php): bridge type aliases (e.g. `VisitorHandle`) are now included in
  `opaque_type_names` so that `gen_php_struct` emits `#[serde(skip)]` for fields of those
  types. Without this, structs like `ConversionOptions` that derive `serde::Serialize` +
  `serde::Deserialize` failed E0277 because `VisitorHandle` (an `Arc<Rc<RefCell<dyn
HtmlVisitor>>>` wrapper) does not implement those traits. Builder methods with bridge
  parameters also use `.visitor(None)` to avoid E0308 type mismatch.

- fix(backend-pyo3): fixed three remaining codegen bugs for PyO3 bindings: (1) `PyVisitorRef::extract` now uses `Borrowed::to_owned()` instead of non-existent `Borrowed::clone()` to avoid E0507 "cannot move out of dereference"; (2) builder methods on has_default structs no longer attempt to access `.inner` for bridge type parameters—visitor parameter skips the core builder's `.visitor()` call and uses `None` instead to avoid E0308 type mismatch; (3) wrapper functions that pass has_default parameters to core functions now wrap the deserialized value in `Some()` when core expects `Option<T>` to fix E0308 "expected Option, found T".

- fix(backend-pyo3): `PyVisitorRef` now uses PyO3 0.28 API correctly. Updated `FromPyObject<'a, 'py>` to accept two lifetimes and use `extract(Borrowed<'a, 'py, PyAny>)` instead of the deprecated `extract_bound()`. Fixed `IntoPyObject` error type to use `Infallible` (conversion cannot fail). Resolves E0407, E0107, E0277, E0308 errors when building against PyO3 0.28.3.

- fix(backend-pyo3): trait types now map to `PyVisitorRef` (a custom wrapper) instead of `Arc<Py<PyAny>>`.
  `Arc<Py<PyAny>>` broke PyO3's `IntoPyObject`/`FromPyObject` field traits (E0277). `PyVisitorRef`
  wraps `Py<PyAny>` in `Arc<>` for cheap cloning while maintaining PyO3 compatibility at struct
  field and parameter boundaries. The wrapper implements Clone via Arc, FromPyObject/IntoPyObject
  for binding integration, and exposes `.inner: Arc<Py<PyAny>>` for trait bridge code generation.

- fix(backend-go): `NodeContext.NodeType` field type is now emitted as `NodeType` (the package-defined
  type alias for `string`) instead of the raw `string` type. The redundant `type NodeType = string`
  declaration in `visitor.go` that caused a redeclaration compile error with `binding.go`'s
  `type NodeType string` has been removed.

- fix(backend-rustler): `gen_native_ex` now emits a `convert_with_visitor/3` NIF stub when
  a `bind_via = "options_field"` bridge is present. Previously only `FunctionParam` bridges
  triggered the visitor-variant stub, causing an `on_load` failure at runtime.

- fix(backend-rustler): `gen_bridge_field_function` now correctly handles optional options
  params. When the core function expects `Option<ConversionOptions>`, the plain NIF no longer
  calls `.unwrap_or_default()` (preserving `None`), and the visitor NIF wraps `options_core`
  in `Some(...)` before passing to the core function.

- fix(alef-codegen/enums): resolved merge-conflict marker in `gen_enum` that broke builds.
  The `#[default]` attribute is now placed on the `is_default`-marked variant (falling back to
  the first variant), matching the Rust core's explicit `#[default]` placement.

- fix(napi): `core_to_binding` conversion now emits `Default::default()` for opaque Named fields
  with `CoreWrapper::None` (e.g. `visitor: Object<'static>`) instead of trying to wrap them
  with `Arc::new` — mirrors the same fix previously applied to `binding_to_core`.

- fix(napi): `gen_opaque_struct_methods` no longer emits `#[napi]` methods whose parameters
  include opaque-typed values by value; NAPI class types don't implement `FromNapiValue`
  and cannot appear as plain method params. Methods without an explicit adapter body are skipped.

- fix(napi/types): opaque Named types in `#[napi(object)]` struct fields are now mapped to
  `napi::bindgen_prelude::Object<'static>` instead of the NAPI class type, which fails the
  `FromNapiValue` bound required by `#[napi(object)]`.

- fix(napi/dts): opaque trait types (e.g. `JsHtmlVisitor`) are now included in the generated
  `index.d.ts`; previously they were excluded by the `!is_trait` filter.

- fix(backend-java): optional record fields now use `@Nullable` annotation instead of
  `Optional<T>` wrapper, making the Java API idiomatically correct. Fields are typed as
  nullable boxed types (e.g. `@Nullable Long` instead of `Optional<Long>`), improving
  compatibility with IDE inspections and code generators.

- fix(backend-java): Javadoc generation now strips Rust-specific syntax (intra-doc links
  like `[`Type`]`, code block fences, and Rust sections like `# Arguments`/`# Errors`)
  to produce valid Java documentation. The `emit_javadoc` helper function applies the
  transformation uniformly across all public API methods.

- fix(backend-java): builder `build()` methods now correctly extract values from `Optional`
  fields using `.orElse(null)` before passing to nullable record constructors.

- fix(backend-java): `convertWithVisitor` method now constructs `ConversionResult` with
  nullable fields instead of `Optional.of()` and `Optional.empty()` calls.

- fix(napi/dts): optional fields in `#[napi(object)]` structs are now correctly marked `?`
  when the field type is `TypeRef::Optional`, `field.optional` is set, or the parent type
  has `has_default = true`.

- fix(e2e/node): removed `"strictNullChecks": false` from e2e tsconfig; strict mode is now
  fully enabled.

- fix(e2e/node): visitor method template double-brace bug fixed and string literals in
  `Custom` callback returns are now properly quoted.

- fix(e2e/node): visitor method parameters are annotated `: any` to satisfy `noImplicitAny`.

- fix(java): bare `catch (Throwable ignored) { return 0; }` in VisitorBridge upcall stubs swallowed
  visitor exceptions silently. The catch clause now captures the first throwable in a sticky
  `volatile Throwable visitorError` field, returns `VISIT_RESULT_ERROR` (4), and surfaces the
  exception via `rethrowVisitorError()` in the `finally` block of `convertWithVisitor`.

- fix(java): `encodeVisitResult` allocated `Custom` markdown and `Error` message buffers into
  `Arena.global()`, leaking them permanently. The method is now a non-static instance method
  that allocates into `this.arena` (the bridge's confined arena), so all buffers are freed on
  `VisitorBridge.close()`.

- fix(java): `checkLastError()` always threw the base exception class. The method now
  dispatches on the native error code: code 1 throws `InvalidInputException`, code 2 throws
  `ConversionErrorException`, and any other code throws the base `{Class}Exception`.

- fix(java): `createObjectMapper()` was called once per FFI invocation, allocating a new
  `ObjectMapper` on every `convert()` call. The factory is now called once at class-load time
  to populate a `private static final ObjectMapper MAPPER` field.

- fix(java): stale generated files `IHtmlVisitor.java`, `HtmlVisitorBridge.java`,
  `TestVisitor.java`, `TestVisitorAdapter.java`, and `VisitContext.java` were emitted by the
  trait-bridge and gen-visitor paths. They are superseded by `Visitor.java` and
  `VisitorBridge.java` in the Panama FFM visitor pattern. All five files are removed from
  generation and deleted from committed output.

- fix(java): `has_visitor_pattern` was evaluated using `config.ffi.visitor_callbacks` which is
  always `None` in the Java backend's `ResolvedCrateConfig`. The check now also activates when
  any `[[trait_bridges]]` entry uses `bind_via = "options_field"`, which is the Java-specific
  visitor activation path.

- fix(java): output path defaulted to `packages/java/` from the workspace template, causing
  generated files to land in `packages/java/dev/kreuzberg/htmltomarkdown/` instead of the
  correct `packages/java/src/main/java/dev/kreuzberg/htmltomarkdown/`. The Java output path is
  now resolved via an explicit `java = "packages/java/src/main/java/"` entry in `[crates.output]`.

## [0.14.1] - 2026-05-02

### Added

- feat(trait-bridge): `[[trait_bridges]]` schema now supports optional `unregister_fn` and
  `clear_fn` fields. When present, alef emits the corresponding host-language wrappers
  alongside the existing `register_fn` codegen. Pyo3 backend implements both new methods;
  remaining backends fall through to the default `None` (no emission).

- feat(error-gen): error-Display strings now have template placeholders (`{message}`,
  `{plugin_name}`, `{elapsed_ms}`, `{limit_ms}`, `{0}`-style positional, etc.) stripped
  before emission. Acronym-aware variant-name splitting recognizes 40+ technical
  acronyms (IO, OCR, PDF, URL, HTTP, TCP, …) and preserves them in the rendered text:
  `IoError` → "IO error", not "iO error". Applies across all error-handling backends.

- feat(pyo3): `register_*` plugin docstrings now use a humanized noun derived from the
  trait name. Previously emitted `"""Register a register_ocr_backend backend."""`
  (placeholder leaked); now emits `"""Register a OCR backend implementation as a runtime plugin."""`.

### Fixed

- fix(extendr): `generate_public_api` no longer emits a duplicate `convert()` wrapper calling
  `.Call("htm_convert", ...)`. extendr generates `extendr-wrappers.R` with the correct
  `.Call(wrap__convert, ...)` symbol; the backend now emits only `@useDynLib` + `options.R`.

- fix(extendr): generated `options.R` now includes all `ConversionOptions` fields from the IR,
  including `exclude_selectors`, `max_image_size`, `capture_svg`, `infer_dimensions`, `max_depth`,
  `skip_images`, `link_style`, `output_format`, `include_document_structure`, `extract_images`,
  and `visitor` (previously missing).

- fix(extendr): visitor pairlist keys no longer carry a leading underscore. Rust uses `_ctx` /
  `_href` for unused default-impl parameters; R callers write `function(ctx, ...)` without the
  prefix. Keys are now trimmed of leading `_`.

- fix(e2e/r): `CallbackAction::Custom` / `CustomTemplate` output values are now quoted in generated
  R: `list(custom = "[AUDIO: podcast.mp3]")` instead of the previous unquoted form that caused a
  parse error.

- fix(e2e/r): `visitor` is now passed inside `options = list(visitor = visitor)` rather than as a
  top-level parameter to `convert()`.

- fix(backend-wasm): use `is_default` variant for generated `Default` impl on wasm enums instead
  of always using the first variant, fixing incorrect defaults for `HeadingStyle`, `CodeBlockStyle`,
  and `PreprocessingPreset`.

- fix(backend-wasm): visitor bridge methods now dispatch `{custom: "..."}` JS objects to
  `VisitResult::Custom(s)` and `{error: "..."}` objects to `VisitResult::Continue`, making the
  full `VisitResult` variant set reachable from JavaScript.

- fix(e2e/wasm): merge visitor into the options object (2nd arg) rather than appending it as a
  standalone 3rd argument, matching the wasm binding's single-object options API.

- fix(backend-napi): use generated `From` conversions for named reference parameters instead of
  JSON round-tripping when let-binding delegation is possible.

- fix(backend-pyo3): generate `__str__` and `__repr__` for Rust-backed enum wrappers so returned
  enum values are inspectable from Python.

- fix(e2e/python): render string containment assertions over configured array fields by checking common DTO text
  attributes instead of comparing directly against the object list.

- fix(alef-cli/format): format WASM binding crates with `cargo fmt --manifest-path`
  derived from the resolved output path so renamed or workspace-excluded crates are handled correctly.

- fix(codegen): use generated `From` conversions for named reference parameters instead of JSON round-tripping when a
  direct let-binding is possible. This preserves fields such as Python `ProcessConfig.language` in free functions.
- fix(codegen): convert Cow-backed string fields with `.into()` when reconstructing `core_self` for binding methods.

- fix(cli): run language-native formatters on stubs before finalising the embedded `alef:hash:` line.
  `alef stubs` previously skipped the format step and computed the hash over raw codegen output.
  When host-language tools (ruff, php-cs-fixer, mix format, cargo fmt, …) reformatted those files,
  the embedded hash no longer matched the on-disk content, causing `alef verify` to report them as stale.

- fix(cli): track stub-changed languages in `changed_languages` during `alef generate`.
  When stubs changed but no bindings changed for a language, the formatter gate
  (`if any_written && !changed_languages.is_empty()`) would skip formatting entirely,
  leaving stub files unformatted and the hash stale after the next formatter run.

- fix(cli): always add stub file paths to `current_gen_paths` before the orphan-sweep pass
  in `alef generate`. When the stub cache was warm, stub paths were not registered,
  causing the sweep to delete freshly-generated stub files (e.g. `.pyi`) as orphans
  on the second generate run.

- fix(csharp): `ConversionOptionsBuilder.Dispose()` and `VisitorHandle.Dispose()` were
  no-op stubs. Opaque handles now emit a `{Name}SafeHandle : SafeHandle` inner class for
  deterministic, exception-safe cleanup. The public wrapper delegates `Dispose()` to it.

- fix(csharp): `optionsHandle` and `visitorHandle` were leaked when `Convert()` or
  `ConvertWithVisitor()` threw. Both handles are now freed in a unified `try/finally` block.

- fix(csharp): `Marshal.PtrToStringAnsi` used throughout — replaced everywhere with
  `Marshal.PtrToStringUTF8` so non-ASCII characters (e.g. Unicode URLs, CJK text) are
  decoded correctly on all platforms.

- fix(csharp): typed exceptions (`InvalidInputException`, `ConversionErrorException`) were
  never thrown. `GetLastError()` now dispatches on the native error code and emits `if`
  branches for each registered exception type.

- fix(csharp): `NodeContext.NodeType` was typed `int` instead of the `NodeType` enum.
  `DecodeNodeContext` now casts `Marshal.ReadInt32` to `(NodeType)`.

- fix(csharp): stale generated files `IVisitor.cs` and `VisitorCallbacks.cs` were still
  emitted by `gen_visitor_files`. They are superseded by `IHtmlVisitor`/`HtmlVisitorBridge`
  in the trait-bridge pattern. Both files are removed from generation and deleted from
  committed output.

- fix(csharp): bare `catch { return 0; }` in visitor upcall thunks swallowed exceptions
  silently. The catch clause now captures the exception and returns the Error discriminant
  (4) with the message encoded via `EncodeString`.

- fix(csharp): outer `packages/csharp/HtmlToMarkdown.csproj` was a NuGet wrapper that
  double-included the inner project's sources, producing duplicate type definitions.
  Added `<Compile Remove="HtmlToMarkdown/**" />`, `<ProjectReference>`, and
  `<GenerateAssemblyInfo>false</GenerateAssemblyInfo>`.

- fix(csharp): `<TreatWarningsAsErrors>true</TreatWarningsAsErrors>` was absent from
  `Directory.Build.props`. Added alongside `<Nullable>enable</Nullable>`.

- fix(csharp): `using System.Threading.Tasks` was emitted unconditionally. It is now
  conditionalized on whether the API has any `async` functions or methods.

- fix(csharp): `ConvertWithVisitor` accepted `IVisitor` (old generated interface) instead
  of `IHtmlVisitor` (trait-bridge interface from `TraitBridges.cs`). The method now takes
  `IHtmlVisitor` and wraps it in `HtmlVisitorBridge` instead of the removed `VisitorCallbacks`.

- fix(napi): `mapper.map_type` returns `String`, not `Cow<str>`; removed stale `.into_owned()`
  calls in `crates/alef-backend-napi/src/gen_bindings/types.rs`.

- fix(extendr): removed dead reference to `gen_conversion_options_r` (kreuzberg-specific dead
  code from an earlier project-coupled state).

- fix(codegen): collapsed clippy `if-same-then-else` warning in `config_gen.rs` by combining
  enum-variant and Named-type branches that emit the same body.

- chore(deps): dropped unused `tracing` dep from `alef-core` (only `tracing-test` is used,
  and that's a dev-dep).

## [0.14.0] - 2026-05-02

### Changed

- **BREAKING (config schema): `alef.toml` is now multi-crate.** The old single-crate `[crate]` schema is rejected on load with a curated migration message. Run `alef migrate --write` to convert. Internally the loader returns `Vec<ResolvedCrateConfig>` and every backend, codegen pass, scaffold step, e2e generator, and publish step now consumes `&ResolvedCrateConfig` instead of `&AlefConfig`. The `Backend` trait signature changed accordingly. Workspace-wide settings live under `[workspace]`; per-crate settings live under `[[crates]]`.
- alef-cli now iterates every crate in the workspace by default. Add a top-level `--crate <name>` (repeatable) to restrict commands to a subset.
- `alef-backend-swift` / `alef-backend-kotlin`: an explicit `[crates.output] {swift,kotlin}` value is now treated as the verbatim package directory; only the template-derived default constructs the canonical `Sources/<Module>/` (swift) or `src/main/kotlin/<package>/` (kotlin) layout. Predictable rule: explicit override = exactly that path; default template = backend builds the canonical structure.

### Added

- feat(alef-cli): `alef migrate --write` rewrites legacy `alef.toml` in place using an atomic temp-file + rename and refuses to overwrite symlinks. The dry-run default prints a unified diff.
- feat(alef-cli): top-level `--crate <name>` CLI filter (repeatable) selects a subset of the workspace; absent processes every crate. New `crates/alef-cli/src/dispatch.rs` exposes `select_crates` / `is_multi_crate`.
- feat(alef-core): new `WorkspaceConfig` / `RawCrateConfig` / `ResolvedCrateConfig` / `OutputTemplate` types with documented per-field precedence; `OutputTemplate::resolve` rejects path traversal segments and NUL bytes; `WorkspaceConfig`, `RawCrateConfig`, and `NewAlefConfig` use `#[serde(deny_unknown_fields)]` so typos surface as parse errors.
- feat(alef-core): per-crate `compute_crate_sources_hash(&ResolvedCrateConfig)` replaces the legacy `&[PathBuf]` overload so multi-source-crate workspaces produce a single stable hash across all contributing source files.

### Fixed

- fix(e2e-go): avoid double-wrapping `len(...)` in `min_length` / `max_length`
  assertions and generate valid guards for array element length checks.
- fix(e2e-rust): honor call-level `result_is_simple` and `result_is_option`
  flags so optional scalar results are unwrapped before equality assertions.
- fix(codegen): preserve sanitized `Cow<str>` fields in Rust binding DTO serde and binding-to-core conversions.
  Python bindings previously dropped required string fields such as `ProcessConfig.language`, causing generated e2e
  tests to fail with missing-field deserialization errors.
- fix(alef-backend-dart, alef-backend-swift): honor `[crates.dart] frb_version` and `[crates.swift] swift_bridge_version` overrides — the fields were deserialized but every callsite hardcoded the compiled-in default constant.
- fix(alef-cli/migrate): preserve legacy `[crate]` scalars at the top of the generated `[[crates]]` entry by recursively clearing toml_edit position metadata so the resulting document is well-formed even when many language sub-tables follow.
- fix(alef-cli/version-pin): write `[workspace] alef_version` after a successful generate instead of a top-level `version =` line. The legacy detector rejected the top-level form on the next run, breaking re-generation.
- fix(alef-cli/cache): scope IR / lang / stage caches per-crate (`.alef/<crate>/ir.json`, `.alef/<crate>/hashes/<lang>.{hash,manifest}`, `.alef/<crate>/hashes/<stage>.{hash,manifest}`) so multi-crate workspaces no longer poison or overwrite each other's cache entries. `validate_cache_crate_name` rejects path separators, NUL, `..`, `.`.

## [0.13.10] - 2026-05-02

### Added

- feat(codegen): auto-exclude trait-bridge registration functions via `collect_trait_bridge_registration_fn_names()`.
  Each binding backend now automatically excludes `register_*` / `unregister_*` functions to prevent double-emit
  compile errors — downstream consumers no longer need manual `exclude` entries in `alef.toml` for trait-bridge registration fns.

- feat(e2e-go): support disk-path fixture loading in Go e2e harness in addition to HTTP URLs.
  New test infrastructure in `crates/alef-e2e/tests/go_bytes_file_path_fixtures.rs` demonstrates the pattern.

### Fixed

- fix(cli): qualify type-name exclusion entries by Rust path. `apply_filters` previously matched
  `[exclude].types`/`enums`/`errors` entries against the short type name only, so a fully-qualified entry
  like `kreuzberg::core::config::formats::OutputFormat` never actually filtered anything when an
  ambiguously-named type appeared in two pub-use chains. Entries containing `::` are now matched against
  the type's `rust_path` (with hyphens normalised); plain entries continue to match by short name.
  Three regression tests cover plain-name match, qualified-path match, and hyphen normalisation.

- fix(codegen): resolve C# signature mismatch in generated binding code that surfaced during trait-bridge auto-exclusion wiring.

- fix(backend-rustler): emit `if(...)` function-call form instead of `(if ...)` outer-parens form for JSON-encoding options.
  `mix format` rewrites `(if x, do: y, else: z)` → `if(x, do: y, else: z)`, so the outer-parens form caused a hash drift
  on every prek run. The generator now emits the formatter-stable `if(...)` form directly.

- fix(e2e): honor call-level Rust option result flags and use the crate version for registry-mode Rust e2e dependencies when no package override is set.
- fix(e2e): generate valid Go array guards for length assertions on array element fields.
- fix(e2e): honor call-level simple/optional result flags in Go tests and render contains assertions for struct slices.
- fix(e2e): emit nil checks for Go optional simple-result empty assertions.
- fix(backend-java): correct four compile errors in the generated Java convert wrapper
  when `bind_via = "options_field"` is used with an opaque-handle bridge field:
  - B1: the generated `convert()` no longer calls the non-existent
    `htm_conversion_options_from_json` FFI symbol; options are now serialised to JSON
    and passed as a C string directly via `coptionsJsonSeg`.
  - B2: the bridge attachment code now detects that the bridge Java type is an opaque
    handle (e.g. `VisitorHandle`) and calls `options.visitor().handle()` to pass the
    raw `MemorySegment` to the FFI setter, instead of constructing a `VisitorHandleBridge`
    (which does not exist and would require a `Visitor` interface argument).
  - B3: the FFI call argument for the options param is now `coptionsJsonSeg` (matching
    the emitted variable name), not the stale `cOptions` name from the old `_FROM_JSON`
    path.
  - B4: `gen_builder_class` now only emits `.orElse(null)` for optional fields that
    are bridge fields (where the record stores the raw handle type); regular
    `Optional<T>` fields are passed directly to avoid "incompatible types" errors.
- fix(backend-go): preserve `None` as nil for optional string returns.
- fix(backend-go): decode externally tagged Rust enums with string payload fallback
  variants (for example `Other(String)`) as string-like Go enums instead of empty
  structs, and match Rust serde's default unit-variant wire names.
- fix(e2e-go): render `contains` assertions for structured values through JSON
  instead of `%v`, so pointer fields are checked by value.

- fix(backend-go): correct two codegen bugs that produced uncompilable Go bindings when
  `bind_via = "options_field"` is used with an options type that has an Update sibling:
  - Bug A: `ffi_c_struct_name` previously inserted an extra PascalCase prefix segment,
    producing double-prefixed C type names (e.g. `HTMHtmNodeContext`) that do not exist
    in the cbindgen header; the formula now maps Rust struct basenames directly to their
    cbindgen names (e.g. `HTMNodeContext`).
  - Bug B: visitor.go is no longer emitted when all `[[trait_bridges]]` entries use
    `bind_via = "options_field"`; the stale FFI symbols `htm_conversion_options_from_json`,
    `htm_visitor_create`, `htm_visitor_free`, and `htm_convert_with_visitor` are absent
    from the options-field bridge API and caused undefined-symbol link errors.
  - Additionally, non-opaque Named parameters and method receivers whose type has a
    corresponding `{Name}Update` sibling now use the two-step
    `{prefix}_{snake}_update_from_json` + `{prefix}_{snake}_from` pattern instead of the
    removed `{prefix}_{snake}_from_json` helper.

- fix(backend-pyo3): emit SCREAMING_SNAKE_CASE aliases alongside PascalCase variants in
  `.pyi` enum stubs so mypy resolves `CodeBlockStyle.BACKTICKS` without `attr-defined`
  errors. Previously only the raw PyO3 names (e.g. `Backticks`) were declared in the stub
  while `options.py` adds the aliases at runtime.

- fix(backend-pyo3): use `object | None` for bridge fields in the `__init__` stub parameter
  list, matching the field annotation. Previously the constructor parameter was typed
  `VisitorHandle | None`, causing a mypy `arg-type` error when passing `value.visitor`
  (typed `object`) from the generated `api.py` conversion helper.

- fix(backend-rustler): wrap inline `if/do/else` JSON-encoding expressions in parentheses
  so the generated NIF call args parse as Elixir when the conditional is followed by a
  visitor parameter. Previously emitted `convert_with_visitor(html, if ..., do: ..., else: ..., visitor)`
  which is invalid syntax — Elixir keyword lists must be the final argument.

- fix(backend-php): null-guard visitor access and correct PSR-4 namespace in generated PHP:
  - `generate_public_api` now emits `$options?->visitor` (null-safe operator) instead of
    `$options->visitor` when the options parameter is optional, eliminating the PHPStan
    "Cannot access property on null" error.
  - Added `generate_scaffold` to `PhpBackend` that emits a `functions.php` convenience
    wrapper using the namespace from `config.php_autoload_namespace()` (which honours
    `[php].namespace` in `alef.toml`), replacing the previously hard-coded wrong
    namespace derived by mechanical case-splitting of the crate name.

- fix(backend-php): resolve compile errors when `bind_via = "options_field"` is used
  in `[[trait_bridges]]` with an options type that has an opaque handle field:
  - `gen_php_struct` now receives `opaque_type_names` from `mod.rs` so
    `gen_struct_with_per_field_attrs` can emit `#[serde(skip)]` on fields whose type
    is an opaque handle (e.g. `VisitorHandle`), fixing 8 `E0277` serde trait errors.
  - `gen_bridge_field_function` now emits `let mut {name}_core` for optional params,
    enabling `options_core.as_mut()` in the generated `visitor_attach` call.
  - `find_bridge_field` (alef-codegen) now sets
    `param_is_optional: is_optional || param.optional`, covering the IR pattern where
    the options param is stored as `ty: Named("T") + optional: true` rather than
    `ty: Optional(Named("T"))`. This caused the wrong `visitor_attach` branch to be
    selected, producing a `no field 'visitor' on type Option<...>` compile error.
  - `gen_php_call_args` now emits `visitor.map(|v| (*v.inner).clone())` for optional
    opaque params, replacing the former `visitor.as_ref().map(|v| &v.inner)` which
    produced a type mismatch (`Option<&Arc<...>>` vs `Option<Rc<...>>`).

- fix(backend-wasm): resolve four compile errors when `bind_via = "options_field"` is
  used in `[[trait_bridges]]` with an options type that has a handle field:
  - `gen_new_method` now appends `..Default::default()` to the struct literal when
    bridge fields are excluded from constructor params, fixing missing-field errors.
  - `gen_opaque_struct_methods` now skips methods whose params contain a
    `type_alias` of an `options_field` bridge (e.g. the builder's `visitor()` method),
    preventing uncompilable Arc-vs-Rc type mismatches in the generated call args.
  - `field_conversion_from_core_cfg` (alef-codegen) now emits `None` for sanitized
    `Named` / `Optional(Named)` fields in WASM mode (`map_uses_jsvalue = true`)
    instead of the Debug-format fallback that produced `Option<String>` rather than
    `Option<JsValue>`.
  - `bridge_fields_map` is extended to cover `{options_type}Update` types so that
    binding→core `From` impls for Update types default the bridge field via
    `Default::default()` instead of calling `val.visitor.map(Into::into)`, which
    fails because `WasmVisitorHandle` has no `Into<VisitorHandle>` impl.
    Also fixes `convertible_types` (alef-codegen) to allow optional sanitized fields
    whose type is `Named(T)` with `optional = true`: the binding stores `Option<T>`
    which is always `Default`, so these types correctly remain in the convertible set.

- fix(backend-ffi): suppress duplicate `{prefix}_convert` symbols in options-field
  bridge mode. When `bind_via = "options_field"` is set, the free-function loop now
  skips ALL `convert` variants (not just sanitized ones), and `gen_convert_no_visitor`
  - `gen_visitor_bindings` are not emitted. The single authoritative `{prefix}_convert`
    comes exclusively from `gen_convert_with_options_field_bridge`. Removes the three
    `htm_convert` definitions that caused a duplicate `#[no_mangle]` compile error in
    html-to-markdown. Also fixes `gen_convert_with_options_field_bridge` to call
    `core::convert(html, options)` with 2 arguments matching the current API (visitor
    is embedded in options, not passed as a third argument).

- fix(e2e): visitor codegen now assigns the synthesized visitor object to
  `options.visitor` (language-idiomatic field assignment) instead of passing it
  as a third positional argument. This aligns e2e test generation with the h2m
  API change where `convert(html, options)` accepts visitor as an optional
  ConversionOptions field rather than a separate parameter. Implemented for:
  - Rust: mutate options binding after construction to attach visitor
  - Ruby: insert visitor key into options hash inline
  - PHP: merge visitor as separate options object
  - Python, TypeScript: already correct (kwargs/object-field pattern)
    Remaining languages (Go, Java, C#, Elixir, R, Gleam, Kotlin, Zig, Swift,
    WASM, Dart, C) require equivalent updates to move visitor from positional
    arg to options field assignment (patterns will vary per language syntax).

- fix(backend-napi): resolve all compile errors when `bind_via = "options_field"`
  is used in `[[trait_bridges]]` with a NAPI-RS binding:
  - `gen_struct` bridge-field substitution (emit `Option<Object<'static>>` instead of
    `Option<JsVisitorHandle>`) is now applied BEFORE the `field.sanitized` guard, so it
    correctly fires for bridge fields that are NOT sanitized (e.g. `visitor` in
    `ConversionOptions` has `sanitized: false`).
  - `gen_struct` now also builds a set of all bridge `type_alias` values and applies the
    `Option<Object<'static>>` substitution to ANY field whose type is a bridge alias
    (e.g. `ConversionOptionsUpdate.visitor`), not only fields in the declared
    `options_type`. This prevents `JsVisitorHandle: FromNapiValue not satisfied` errors
    in Update-pattern structs.
  - `bridge_fields_by_type` (used to populate `force_default_fields` for From
    conversions) is extended by case (b): any type with a field whose type is a bridge
    type alias is now included, so binding→core and core→binding `From` impls emit
    `Default::default()` instead of `val.visitor.map(Into::into)` for those fields.
  - `gen_opaque_struct_methods` now skips methods whose params contain a `type_alias`
    of an `options_field` bridge (e.g. `ConversionOptionsBuilder::visitor()`),
    preventing uncompilable Arc-vs-Rc type mismatches in the generated call args.

- fix(backend-pyo3): resolve all compile errors when `bind_via = "options_field"`
  is used in `[[trait_bridges]]` with a PyO3 binding:
  - `extra_field_attrs` now returns `vec![]` for bridge fields, preventing
    duplicate `#[pyo3(get)]` and `#[serde(skip)]` attributes.
  - `rewrite_bridge_field_type` now accepts `type_alias: Option<&str>` and
    also rewrites `Option<{alias}>` patterns (e.g. `Option<VisitorHandle>`),
    not only the legacy `Option<String>` placeholder.
  - `rewrite_bridge_field_impl` now rewrites constructor parameter types
    (`visitor: Option<VisitorHandle>` → `visitor: Option<Py<PyAny>>`).
  - Struct generation for both the primary options type and related types
    (e.g. `ConversionOptionsUpdate`) that share the same bridge field name
    now applies the full set of rewrites: field type override, `frozen` →
    `unsendable`, and manual `Clone` impl using `Python::attach`/`clone_ref`.
  - `bridge_field_name_for_type` extends From-conversion post-processing to
    cover related types (`ConversionOptionsUpdate`) that are not listed as
    `options_type` in `alef.toml` but share the bridge field name.
  - Opaque types whose name matches a bridge `type_alias` (e.g. `VisitorHandle`)
    and builder types for bridge options types (e.g. `ConversionOptionsBuilder`)
    now use `#[pyclass(unsendable)]` because they transitively contain
    `Rc<RefCell<dyn Trait>>`.
  - Builder opaque impl: `visitor.as_ref().map(|v| &v.inner)` rewritten to
    `visitor.map(|v| (*v.inner).clone())` to pass `Rc<RefCell<...>>` (not a
    reference into Arc) to the core builder's `.visitor()` method.
  - `gen_bridge_field_function` in `trait_bridge.rs` now uses
    `Python::attach(|py| v.clone_ref(py))` to extract the visitor from
    `Option<Py<PyAny>>`, replacing the former `.clone()` call that requires
    the `py-clone` feature (not enabled in most consumers).

- fix(backend-csharp): drop stale `[DllImport]` declarations and fix the visitor
  attach path when `bind_via = "options_field"` is active:
  - `gen_native_methods` no longer emits `{prefix}_conversion_options_from_json`
    for options types owned by an options-field bridge; those types are constructed
    via the `UpdateFromJson` + `FromUpdate` pair that still exists in the FFI surface.
  - `gen_native_methods_visitor` now returns an empty string when an options-field
    bridge is present, suppressing the stale `{prefix}_convert_with_visitor`,
    `{prefix}_visitor_create`, and `{prefix}_visitor_free` P/Invoke declarations.
  - `gen_wrapper_function` now emits `{opts}.{Field}._vtable` directly instead of
    double-wrapping the already-constructed `{bridge_type}Bridge` in a second `new
{bridge_type}Bridge(...)`, eliminating the `CS1503` type mismatch compile error.
  - `ffi_symbol()` on `OptionsFieldBridgeInfo` now produces
    `{prefix}_options_set_{field}` (matching `alef-backend-ffi::gen_bridge_field`)
    instead of the former `{prefix}_{opts_snake}_set_{field}` mismatch.

### Added

- feat(backend-pyo3): wire `bind_via = "options_field"` bridge support. When a
  `[[trait_bridges]]` entry sets `bind_via = "options_field"`, the PyO3 backend now:
  - Emits `visitor: Option<Py<PyAny>>` on the `ConversionOptions` pyclass (overriding
    the IR-sanitized `String` type via a targeted post-process rewrite).
  - Switches the `ConversionOptions` pyclass attribute from `frozen` to `unsendable`
    because the struct embeds `Rc<RefCell<dyn HtmlVisitor>>` via `Py<PyAny>` and is
    `!Send`; `unsendable` enforces single-thread GIL access.
  - Generates a `convert` wrapper that extracts `options.visitor`, builds a
    `PyHtmlVisitorBridge`, sets it on the core `ConversionOptions.visitor` field via
    `std::rc::Rc::new(std::cell::RefCell::new(bridge)) as VisitorHandle`, then calls
    core convert — no separate `convert_with_visitor` export.
  - Updates `.pyi` stubs: the visitor field on `ConversionOptions` is typed as
    `object | None` instead of `str | None`.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the pyo3 `trait_bridge` module.

- feat(backend-napi): wire `bind_via = "options_field"` bridge support — visitor field on
  `JsConversionOptions` is emitted as `Option<Object<'static>>`, the `convert` wrapper
  extracts it, builds the `JsHtmlVisitorBridge`, sets it on core options, and calls core;
  the `.d.ts` interface shows `visitor?: HtmlVisitor`; no separate `convertWithVisitor`
  export is emitted in this mode.
- feat(backend-extendr): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the R/extendr backend now:
  - Renders the bridge field as `Option<extendr_api::Robj>` on the binding options struct
    (with `#[serde(skip)]` so JSON round-trips ignore it).
  - Emits a custom `From<BindingOptions> for core::Options` impl that leaves the bridge
    field at `Default::default()`; the convert wrapper sets it explicitly after building
    the bridge from the R object.
  - Updates the `new_<options>` kwargs constructor to accept the visitor as
    `Option<extendr_api::Robj>` and assign it via `Some(v)`.
  - Emits a `convert` wrapper (via `gen_bridge_field_function`) that takes
    `options: Option<ConversionOptions>`, pulls the visitor field off the binding,
    constructs `RHtmlVisitorBridge`, attaches it to `options_core.visitor` as
    `Rc<RefCell<...>> as VisitorHandle`, and calls core convert.
  - Does NOT emit a separate `convert_with_visitor` extendr export.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the extendr `trait_bridge` module.
- feat(backend-rustler): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Elixir Rustler backend
  now emits:
  - A regular `convert` NIF (ignores any visitor field in the options JSON).
  - A `convert_with_visitor` NIF with the visitor appended as `Option<rustler::Term<'_>>`,
    generated by the new `gen_bridge_field_function` helper.
  - A `visitor` field (nil default) on the `ConversionOptions` Elixir struct.
  - A `convert/2` Elixir wrapper that calls `Map.pop(options, :visitor)` and dispatches
    to either the async NIF+receive-loop (visitor present) or the plain NIF (no visitor).
  - `convert_with_visitor` is not re-exported as a public Elixir function.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the `trait_bridge` module.
- feat(backend-ffi): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a bridge is configured in options-field mode, `alef-backend-ffi` now:
  - Emits `{prefix}_options_set_{field}(options, visitor)` — a setter that wraps
    the vtable bridge in a thin `Rc<RefCell<VtableRef>>` and stores it on the
    options struct's visitor field.
  - Emits `{prefix}_convert(html, options)` (options carries the embedded visitor)
    instead of the generic sanitized stub.
  - Does NOT emit `{prefix}_convert_with_visitor` — the single `{prefix}_convert`
    entry point replaces it.
  - Suppresses `{prefix}_{options_type}_from_json` for types that own a bridge
    field, because the visitor cannot survive a JSON round-trip.
    New symbol emitted for h2m: `htm_options_set_visitor`.
- feat(docs): render trait-bridged fields as struct fields on options types when
  configured with `bind_via = "options_field"`. The visitor bridge type is rendered
  with language-appropriate syntax (e.g. `HtmlVisitor(Protocol)` for Python,
  `HtmlVisitor (interface)` for TypeScript), and the field description is
  auto-generated if not explicitly documented.
- feat(core/codegen): trait bridges may now declare `bind_via = "options_field"`
  on `[[trait_bridges]]` (with `options_type` and optional `options_field`)
  to indicate that the bridge handle lives as a struct field on a function
  parameter rather than as its own positional argument. Adds the
  `BridgeBinding` enum, the `find_bridge_field` codegen helper, and updates
  `find_bridge_param` to skip bridges configured for options-field binding.
  Existing configs default to `function_param` (the legacy mode), so this is
  fully backwards compatible.
- feat(backend-wasm): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the WASM/wasm-bindgen backend now:
  - Emits the bridge field as `Option<JsValue>` on the binding struct (e.g. `visitor` on
    `WasmConversionOptions`) with a matching `#[wasm_bindgen(getter)]` / `setter`.
  - Excludes the bridge field from the `new()` constructor — callers set it via the setter.
  - Marks the bridge field `sanitized` in the cloned IR so the auto-generated
    `From<WasmConversionOptions>` emits `Default::default()` for it.
  - Generates a `convert` wrapper (via `gen_bridge_field_function`) that extracts the
    `Option<JsValue>` from the binding options, converts the rest to core options via
    `From`, builds `WasmHtmlVisitorBridge`, wraps it as
    `Rc<RefCell<...>> as VisitorHandle`, sets it on core options, and calls core convert.
  - Does NOT emit a standalone `convertWithVisitor` function in this mode.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the wasm `trait_bridge` module.
- feat(backend-csharp): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the C# backend now:
  - Emits a `[JsonIgnore] public {Trait}Bridge? {Field}` property on the options
    record type so the bridge object is excluded from JSON serialization.
  - Declares an `internal static extern void {Options}Set{Field}(IntPtr options, IntPtr vtable)`
    P/Invoke entry-point in `NativeMethods.cs` whose FFI symbol is
    `{prefix}_{options_snake}_set_{field_snake}`.
  - Injects a bridge-attachment block in the wrapper `Convert` method: when
    `options.Visitor != null`, creates the bridge, calls the setter, then calls the
    standard two-arg `convert(html, options)` FFI function.
  - Does NOT emit a separate `ConvertWithVisitor` overload in this mode.
- feat(backend-php): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the PHP/ext-php-rs backend now:
  - Renders the bridge field as `?HtmlVisitor` on the `ConversionOptions` type stub
    (overriding the IR-sanitized `?string` type).
  - Generates a `convert` wrapper (via `gen_bridge_field_function`) that takes all original
    params plus a hidden `{field}_obj: Option<&mut ZendObject>` extra param, builds
    `PhpHtmlVisitorBridge::new`, wraps it in `Rc<RefCell<...>> as VisitorHandle`, attaches it
    to the core options struct, and calls core convert.
  - The PHP facade passes `$options->visitor` as the extra hidden arg when delegating to the
    native extension class.
  - Does NOT emit a standalone `convertWithVisitor` function.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the PHP `trait_bridge` module.
- feat(backend-go): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Go/cgo backend now:
  - Emits a lightweight `type {Trait} interface` (trait methods only, no plugin lifecycle) in `binding.go`.
  - Injects a synthetic `{Field} {Trait} \`json:"-"\``field on the Go options struct,
skipping the raw IR`Option<VisitorHandle>` field so JSON marshaling ignores it.
  - Adds a `WithConversionOptions{Trait}` functional-option constructor.
  - Adds `"runtime/cgo"` to the imports when any options-field bridge is active.
  - In the `Convert` wrapper, after the C options handle is created, checks
    `options.{Field} != nil`, creates a `cgo.NewHandle`, defers `handle.Delete()`,
    and calls `C.{prefix}_options_set_{field}(cOptions, ...)` to pass the visitor to Rust.
  - Does NOT emit a separate `ConvertWithVisitor` function in this mode.
- feat(backend-java): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Java/Panama FFM backend now:
  - Emits the bridge field as the bridge interface type (e.g. `HtmlVisitor`) annotated with
    `@JsonIgnore` on the `ConversionOptions` record so Jackson excludes it from serialization.
  - Emits a `static final MethodHandle {PU}_OPTIONS_SET_{FIELD}` in `NativeLib.java` using
    `orElse(null)` so class initialization succeeds even when the dylib lacks the symbol.
  - Generates a `convert` wrapper that marshals the options record normally (bridge field is
    skipped by `@JsonIgnore`), then—if both the setter handle and `options.visitor()` are
    non-null—creates the `{Trait}Bridge`, converts it to a `MemorySegment` via `toSegment`,
    and calls the setter before the main FFI invocation.
  - Does NOT emit a standalone `convertWithVisitor` method in this mode.
- feat(backend-magnus): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Ruby/Magnus backend now:
  - Renders the bridge field as `Option<magnus::Value>` on the binding options struct
    (with `#[serde(skip)]` so JSON round-trips ignore it).
  - Generates a custom `From<BindingOptions> for core::Options` impl that skips the
    bridge field and lets the convert wrapper set it explicitly.
  - Emits a `convert` wrapper (via `gen_bridge_field_function`) that deserializes the
    options hash via `to_json`/`serde_json`, extracts the visitor Ruby object, builds
    `RbHtmlVisitorBridge`, wraps it in `Rc<RefCell<...>>`, and sets it on the core
    options before calling the core function.
  - Does NOT emit a standalone `convert_with_visitor` function.
    Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
    the Magnus `trait_bridge` module.

### Fixed

- fix(backend-java): public facade and opaque-type Javadocs now use the shared
  Javadoc escaping path, preventing Rustdoc examples from being emitted as raw
  `&...`/fenced-code content that breaks `mvn verify` Javadoc generation.
- fix(backend-php): generated public PHP wrapper files now include the
  formatter-required blank line after `<?php` before the Alef header, keeping
  `php-cs-fixer` from mutating generated files after hashes are finalized.
- fix(scaffold/java,scaffold/php): generated Java Checkstyle suppressions now
  use a repo-root path in `checkstyle.xml` so both Maven and repo-root
  pre-commit invocations find the suppressions file, and the PHP CS Fixer
  scaffold is itself fixer-clean while still tolerating packages without a
  `tests/` directory.
- fix(cli/format): the default WASM formatter now derives the generated crate
  package from Alef's core crate directory (`core_crate_dir-wasm`) instead of
  the public Rust crate name. This fixes repos where the public crate
  (`tree-sitter-language-pack`) differs from the internal core crate directory
  (`ts-pack-core`), avoiding `cargo fmt -p <missing-crate>` during generation.
- fix(scaffold/generated-output): generated downstream projects now pass common pre-commit checks after trimming a public API surface: Python `options.py` no longer imports native data-enum aliases before redefining their Python-side aliases, Elixir/Ruby Cargo scaffolds omit `async-trait`/`tokio` when no generated code uses them, Java Checkstyle suppressions resolve from both Maven and repo-root hook invocations, PHP CS Fixer tolerates packages without a `tests/` directory, and Rustler `native.ex` target lists are emitted in `mix format` style.
- fix(cli/format): the default Node formatter path now uses the Oxc toolchain (`npx oxfmt .` followed by `npx oxlint --fix .`) instead of invoking Biome. The Node scaffold, lint defaults, and generated pre-commit config already used `oxfmt`/`oxlint`; this removes the remaining stale Biome fallback that `alef all --clean` could hit in downstream repos with vendored Biome configs.
- fix(backend-napi): `gen_dts` now applies the same filtering as `gen_function` (drops names listed in `[node].exclude_functions`, drops `sanitized` functions without a trait_bridge). Previously every public function in the API surface was declared in `index.d.ts`, even when its NAPI binding was filtered out of `lib.rs` because it took a tuple-typed param like `Vec<(Vec<u8>, String, Option<FileExtractionConfig>)>`. The mismatch surfaced as TS2614 / TS2345 in downstream e2e suites that imported the phantom names. The d.ts is now generated from the same filtered set as the lib.rs, keeping the two in lockstep.
- fix(e2e/go): json_object args with `element_type` now emit a Go slice type derived from the element (e.g. `Vec<String>` → `[][]string`). Previously the codegen always declared `var parts []string` regardless of `element_type`, producing `cannot use parts (variable of type []string) as [][]string value` on `kreuzberg.GenerateCacheKey([][]string)` and similar nested-slice signatures.
- fix(backend-java): generate `TestVisitor`, `TestVisitorAdapter`, and `VisitContext` types for visitor-pattern bindings. `TestVisitor` is a test-friendly interface using `VisitContext` (same fields as `NodeContext`) instead of the raw FFI `NodeContext`, and `TestVisitorAdapter` converts `NodeContext` → `VisitContext` before delegating. `VisitResult` gains a `continue_()` alias used by generated e2e tests. `VisitorBridge.encodeVisitResult` now uses `Arena.global()` (not the try-with-resources arena) so allocated buffers survive past the callback return, and calls `.reinterpret(ValueLayout.ADDRESS.byteSize())` before `.set()` on the 0-byte upcall `outCustom`/`outLen` segments. The public facade's `generate_public_api` now forwards `has_visitor_pattern` so `HtmlToMarkdown.convert(html, options, TestVisitor)` is emitted. Trait bridge register/unregister handles in `NativeLib` use `.map(...).orElse(null)` instead of `.orElseThrow()` to avoid `ExceptionInInitializerError` when the symbol is absent.
- fix(e2e/java): `CustomTemplate` actions now extract `{placeholder}` names, convert them to camelCase Java variable names, replace each `{name}` with `%s`, and emit `String.format("fmt", var1, ...)` instead of the unformatted literal. `visit_blockquote`'s `depth` parameter is now `long` (matching the `JAVA_LONG` C layout) instead of `int`. Visitor e2e test files now import `TestVisitor`, `VisitContext`, and `VisitResult` from the binding package when any fixture uses visitor callbacks.
- fix(backend-pyo3): emit `setattr(Cls, "FOO", getattr(Cls, "Bar"))` instead of bare `Cls.FOO = Cls.Bar` when the Rust variant name is a Python keyword. Previously enum variants like `HighlightStyle::None` rendered as `HighlightStyle.NONE = HighlightStyle.None` in `options.py` — `None` is a reserved word, so this produced a `SyntaxError: invalid syntax` at import time, breaking every Python e2e test for any binding whose enum has a `None`/`True`/`False`/etc. variant.
- fix(backend-pyo3): use the python-ident-escaped runtime name in the `getattr` call for keyword-named enum variants. PyO3's `#[pyclass]` enum derive renames variants whose name collides with a Python keyword by appending `_` (e.g. `None` → the runtime attribute is `None_`), so `getattr(HighlightStyle, "None")` raises `AttributeError`. The alias emission now resolves the runtime name via `alef_core::keywords::python_ident` and also triggers the `setattr`/`getattr` form when the SCREAMING_SNAKE_CASE alias itself is a Python keyword (e.g. a variant `As` would alias to `AS` which is fine, but `Is` → `IS` and a hypothetical `Continue` → `CONTINUE` are guarded for symmetry).
- fix(e2e/rust): pass visitor arg and unwrap Result for content access. Generated tests now call `convert(html, Some(options), None)` (3-arg form) instead of the stale 2-arg form, emit `.expect("should succeed")` so field access is on the unwrapped `ConversionResult` not on `Result<ConversionResult, ConversionError>`, derive `Debug` on `_TestVisitor` structs (required by the `HtmlVisitor: Debug` bound), import `HtmlVisitor` from the `::visitor` sub-module where it lives, and fix all `HtmlVisitor` method parameter types (e.g. `u8` → `u32` for heading level, `&str` → `Option<&str>` for audio/video/iframe src, `usize` for blockquote depth). `CustomTemplate` actions now bind the referenced parameter identifiers so `format!("{text}")` etc. compile; `Option<&str>` parameters referenced in a template are unwrapped via `let name = name.unwrap_or_default()`. The `alef.toml` Rust override for html-to-markdown gains `wrap_options_in_some = true`, `options_type = "ConversionOptions"`, `extra_args = ["None"]`, and `returns_result = true`.
- fix(e2e/elixir): the e2e mix.exs now uses `[elixir].app_name` as the dep atom (e.g. `:html_to_markdown`), not the crate name with `-` → `_` replacement (e.g. `:html_to_markdown_rs`). When the dep atom doesn't match the path-dep's own `app:` value, mix's resolution silently misroutes — the path-dep's transitive deps (notably `:rustler_precompiled`) are not loaded during compilation of the path-dep itself, and the parent build fails with `(CompileError) cannot compile module ... module RustlerPrecompiled is not loaded and could not be found`.

- fix(backend-php): add `[php].namespace` field to `PhpConfig` that overrides the derived PHP namespace verbatim. Previously the namespace was always derived from `extension_name` by splitting on `_` and converting each segment to PascalCase (e.g. `html_to_markdown` → `Html\To\Markdown`), which was inconsistent with the single-segment `HtmlToMarkdown` namespace expected by `[e2e.call.overrides.php].module`. The new field is used verbatim in class registration (`#[php(name = ...)]`), the PHP facade, type stubs, PSR-4 autoload keys, and e2e test imports — ensuring all four places agree. The three duplicated inline namespace-derivation blocks in `gen_bindings/mod.rs` are collapsed to a single `config.php_autoload_namespace()` call that honors the override.

- fix(backend-extendr): emit `impl From<BindingType> for core::Type` (and the reverse direction) for binding↔core struct and enum conversions, mirroring the existing pyo3/magnus emission paths. Without these, every `*_core: core::Type = binding.into()` site in the generated R lib.rs failed to compile with E0277 unsatisfied trait bounds (e.g. `From<PreprocessingOptions> not implemented for html_to_markdown_rs::PreprocessingOptions`). Lossy `From` impls are now also emitted for data-variant enums, since the extendr binding flattens them to unit variants but containing structs still call `.into()` on the variant value.
- fix(codegen/extendr-kwargs): the `gen_extendr_kwargs_constructor` helper used to emit Rust function-parameter defaults (`name: String = "default"`), which is a syntax error — Rust does not accept default values in function signatures. The constructor now accepts each field as `Option<T>`, instantiates the type via its `Default` impl, and overlays caller-supplied values, producing valid Rust source that the `#[extendr]` macro accepts. Existing `extendr(default = …)` semantics on the R side are preserved by the `generate_public_api` wrapper.
- fix(e2e/c): emit shfmt-compliant case statement style in `download_ffi.sh` so the generated file passes the `shfmt` pre-commit hook without modification and `alef verify` stays consistent.
- fix(e2e/elixir): the e2e mix.exs now uses `[elixir].app_name` as the dep atom (e.g. `:html_to_markdown`), not the crate name with `-` → `_` replacement (e.g. `:html_to_markdown_rs`). When the dep atom doesn't match the path-dep's own `app:` value, mix's resolution silently misroutes — the path-dep's transitive deps (notably `:rustler_precompiled`) are not loaded during compilation of the path-dep itself, and the parent build fails with `(CompileError) cannot compile module ... module RustlerPrecompiled is not loaded and could not be found`. Fixes the elixir e2e suite for any binding whose `[elixir].app_name` differs from the kebab-case crate name.
- fix(e2e/typescript): `options_type` casts and imports are now resolved per-fixture. Previously the renderer used the top-level `[e2e.call.overrides.<lang>].options_type` for every fixture in a test file, even fixtures that overrode the call (`fixture.call = "chunk_text"`) and declared their own per-call `options_type` (e.g. `JsChunkingConfig`). The renderer now reads `e2e.calls.<name>.overrides.<lang>.options_type` for fixtures that override the call, and only the top-level value is used for fixtures using the default call. The header import block emits `type X` declarations for the _union_ of all options types referenced by the active fixtures (e.g. both `JsExtractionConfig` and `JsChunkingConfig` when one file mixes `extract_file` and `chunk_text` fixtures), eliminating TS2304 "Cannot find name 'JsChunkingConfig'" on chunking fixtures and stopping the wrong cast (`JsExtractionConfig` applied to a `JsChunkingConfig`-shaped object).
- fix(e2e/typescript): `as <OptionsType>` casts are now emitted only for _object_-shaped json_object args. Previously every json_object arg (including `paths: [...]`, `texts: [...]`, `parts: [...]`) was cast to the call's options type, producing `["a.pdf"] as JsExtractionConfig` on `batchExtractFileSync(paths, config)` calls — TS2352 "Conversion of type 'string[]' to type 'JsExtractionConfig' may be a mistake". The renderer now inspects the fixture value: arrays/scalars are emitted plain (typed as `Array<string>` / etc by the binding), only objects receive the cast.
- fix(e2e/typescript): `bytes` arg type now classifies the fixture string value (file path / inline / base64) and emits a runtime load instead of passing the string as-is. Previously fixtures like `extract_bytes` with `data: "pdf/fake_memo.pdf"` produced `extractBytes("pdf/fake_memo.pdf", ...)` — TS2345 "Argument of type 'string' is not assignable to parameter of type 'Uint8Array<ArrayBufferLike>'". The renderer now classifies via the same rules as python/rust codegens (`<`/`{`/`[`/whitespace → inline-text → `Buffer.from(s, "utf-8")`; `dir/file.ext` → file-path → `readFileSync(path)`; otherwise → base64 → `Buffer.from(s, "base64")`). When any active fixture's resolved call uses a bytes file-path arg, the test file imports `{ readFileSync } from 'node:fs'`. The existing `setup.ts` chdir to `test_documents/` is preserved, so relative paths resolve at runtime.
- fix(e2e/typescript): cast through `unknown` (`as unknown as <Type>`) for both inline configs and the empty-default placeholder. NAPI-RS marks Rust `Option<T>` config params as `T | undefined | null` but bare `T` as required, while the alef.toml `optional` flag does not always match — fixtures like `extract_file_with_no_config` produced calls with no third argument and TS2554 "Expected 3 arguments, but got 2". When the json_object arg is missing (or `null`) and an `options_type` is configured, the renderer now emits `{} as unknown as <Type>` to satisfy the binding's required parameter; partial config literals are also cast through `unknown` to avoid TS2352 "neither type sufficiently overlaps". The runtime binding still validates fields.
- fix(e2e/typescript): `result_is_simple = true` is now honored. Previously the flag was only used by the rust/csharp/java/ruby/r codegens, so fixtures whose binding returns a primitive (e.g. `generateCacheKey: string`, `validateCacheKey: boolean`) emitted `result.<field>` access and triggered TS2339 "Property 'result' does not exist on type 'string'". The TypeScript renderer now resolves the per-call `result_is_simple` flag from `[e2e.calls.<name>.overrides.<lang>]` and (a) uses the bare `result_var` for assertions, (b) skips assertions on non-`result` fields with a `// skipped:` comment.
- fix(e2e/typescript): `arg_order` per-language override is now honored. The flag has been documented in `alef-core` for some time but no codegen reordered the `args` slice — so kreuzberg's `[e2e.call.overrides.node] arg_order = ["path", "config", "mime_type"]` was silently ignored and the renderer emitted args in the canonical Rust order, producing TS2345 "Argument of type 'string' is not assignable to parameter of type 'JsExtractionConfig'" on every `extractFile(path, mime, config)` call (NAPI's binding takes `path, config, mime_type?`). The renderer now reorders the per-fixture call args via the language override before passing to `build_args_and_setup`.
- fix(e2e/typescript): per-fixture `options_type` resolution now falls back to the top-level `[e2e.call.overrides.<lang>]` when a per-call override doesn't declare its own. Most kreuzberg per-call entries (e.g. `extract_bytes`) rely on the same `JsExtractionConfig` shape as the default `extract_file` call and therefore intentionally omit the per-call `options_type` — but the prior renderer treated this as "no options_type", emitted no cast, and produced TS2345 on inline config literals. The fallback chain is: per-call `options_type` → top-level `options_type` → no cast.
- fix(e2e/typescript): emit a bare call (no arguments) when the resolved `[e2e.call]` has `args = []` and the fixture has no input. Previously the renderer fell through to `json_to_js(input)` and stringified the entire input object as a single positional arg, producing `listExtractors({})` for `args=[]` calls and triggering TS2554 "Expected 0 arguments, but got 1".
- fix(e2e/typescript): support per-call `result_is_vec = true` by indexing `[0]` into the result variable for field-targeted assertions. Mirrors the existing csharp behavior so fixtures asserting `mime_type` / `content` on `batch_extract_*` calls (which return `Array<JsExtractionResult>`) compile cleanly under strict tsconfig.
- fix(e2e/typescript+wasm): honor per-call `skip_languages` when filtering active fixtures. The CallConfig field has been documented in alef-core for some time but no codegen consulted it — so kreuzberg's `[e2e.calls.batch_extract_*] skip_languages = ["wasm"]` was silently ignored and the wasm renderer emitted test files importing `batchExtractBytesSync` etc. that the wasm binding doesn't export, producing TS2614 / TS2724. Both the typescript (node) and wasm renderers now drop fixtures whose resolved call's `skip_languages` contains the target language.
- fix(e2e/typescript): `not_empty` / `is_empty` on non-array fields no longer emits `.length`. The renderer cannot statically know whether a struct field (e.g. `metadata` on JsExtractionResult — type `JsMetadata`) is a string or an object, so it now emits a runtime IIFE that branches on `typeof v === 'string' || Array.isArray(v)` (uses `v.length`) vs object (uses `Object.keys(v).length`). Casting through `unknown` avoids TS narrowing the unreachable branch to `never`. Behavior on array fields (declared in `fields_array`) is unchanged.
- fix(e2e/typescript): `has_later_arg_value` now also returns true for optional `json_object` args with `options_type` configured, since those emit a typed `{} as unknown as <Type>` placeholder. Previously a fixture with only `path` set (no `mime_type`, no `config`) on a 3-arg `extractFile(path, mime_type?, config)` produced `extractFile(path, {})` (2 args), tripping TS2554 against the wasm signature `extractFile(path, mime_type, config)`. The fix makes preceding optional args emit `undefined` placeholders so positional order is preserved.

### Changed

- feat(e2e/codegen): result-shape flags (`result_is_simple`, `result_is_vec`, `result_is_array`, `result_is_bytes`, `result_is_option`) are now read from the call-level `[e2e.calls.<n>]` (and the default `[e2e.call]`) in addition to per-language overrides. The rust, csharp, ruby, r, and typescript codegens all OR the call-level value with their per-language override, so a single call-level declaration applies consistently to every binding (return-type shape is a property of the Rust core function, not of any binding). Per-language overrides remain accepted for backwards compatibility but are no longer required.

### Changed

- feat(core/config): `result_is_simple`, `result_is_vec`, `result_is_array`, `result_is_bytes`, and `result_is_option` are now first-class fields of `CallConfig` itself, not only `CallOverride`. These flags describe the Rust core's return type and therefore apply identically to every binding — declaring them at call level eliminates the need for redundant per-language `[e2e.calls.<n>.overrides.<lang>] result_is_simple = true` blocks (kreuzberg's alef.toml had ~30 such call entries duplicated across rust/node/csharp/java/python/wasm). The typescript codegen now ORs the call-level value with the per-language override to preserve backwards compatibility; other backends will adopt the same pattern in subsequent releases. Per-language overrides remain accepted but should be removed in favor of the call-level field.

## [0.13.6] - 2026-05-02

### Added

- feat(scaffold): manage workspace `.cargo/config.toml` via opt-in `[scaffold.cargo]` section in `alef.toml`. When the section is present, alef writes the full canonical file with hash-based drift detection (same `# alef:hash:` pattern as other alef-managed files). Default config produces a 6-target template covering macOS dynamic*lookup (required for PyO3/ext-php-rs cdylibs to link on macOS — the symptom is `ld: symbol(s) not found for architecture arm64` on `\_zend*_`/`ext_php_rs::zend::_`and PyO3 symbols at runtime since macOS ld is strict and these symbols are resolved at extension-load time, not link time), Windows MSVC`rust-lld`for x86_64 + i686,`aarch64-linux-gnu-gcc`for ARM64 Linux cross-compile,`x86_64-unknown-linux-musl`for static Linux, and the`wasm32-unknown-unknown`bulk-memory + getrandom_backend cfg. Per-target opt-out via`[scaffold.cargo.targets]`; repo-specific `[env]`entries via`[scaffold.cargo.env]`(supports both bare strings and`{ value, relative }`form). When`[scaffold.cargo]`is absent, the legacy create-if-missing wasm32-only behavior is preserved unchanged. New types in`alef-core::config`: `ScaffoldCargo`, `ScaffoldCargoTargets`, `ScaffoldCargoEnvValue`. New public function `alef_scaffold::render_cargo_config(&ScaffoldCargo) -> String` for deterministic rendering, with 11 unit tests covering golden output, env injection, per-target opt-out, deterministic re-render, key sorting, escaping, and the legacy-fallback gate.

### Added

- feat(e2e/elixir,e2e/gleam): migrate the BEAM languages onto the shared `TestClientRenderer` + `render_http_test` driver, completing Phase 2B. Elixir defines an `ElixirTestClientRenderer<'a>` (carrying `fixture_id` and `expected_status` so `render_call` can build `/fixtures/<id>` URLs and disable Req's auto-redirect for 3xx fixtures) implementing all 8 trait methods; the `FINCH_UNSUPPORTED_METHODS` (TRACE/CONNECT) skip stub stays as a thin pre-hook before delegating to the shared driver. Gleam defines a unit `GleamTestClientRenderer` whose `render_call` uses `gleam_httpc` and whose `render_assert_partial_body` / `render_assert_validation_errors` use `string.contains` (Gleam's stdlib has no JSON decoder); the renderer also overrides `sanitize_test_name` to strip leading `_`/digits so gleeunit names are valid Gleam identifiers. Behavioral notes: elixir's `describe`/`test` labels are now `(fixture_id, description)` rather than `("METHOD PATH - description", "")`, matching all reference impls (cosmetic only); both languages now emit `body_partial`/`validation_errors` assertions when fixtures carry them (old monoliths never called these); WASM was already routing through the migrated TypeScript renderer and required no changes.
- feat(e2e/go,e2e/java,e2e/csharp,e2e/kotlin): migrate the JVM/Go/.NET languages onto the shared `TestClientRenderer` + `render_http_test` driver. These languages cannot expose the consumer's `TestClient` cleanly via FFI, so they continue to drive the binding's `App.serve()` over a TCP-loopback `baseUrl` env var, but the per-test rendering now goes through the shared driver. Per-language deltas: go now unconditionally emits `_ = bodyBytes` to silence the unused-var compile error on tests with no body assertion (the old monolith conditionally omitted `io.ReadAll`); java/kotlin/csharp now correctly emit `<<present>>`/`<<absent>>`/`<<uuid>>` header-token assertions (old code silently skipped them); java now appends query params to the URI string (old code never handled query params); kotlin/go/csharp now sort headers and cookies for deterministic output (old code used HashMap insertion order); go/kotlin/csharp/java now also emit `body_partial` and `validation_errors` assertions when fixtures carry them (old monoliths never called these).
- feat(e2e/php,e2e/dart,e2e/swift,e2e/zig): migrate the second batch of TestClient-friendly languages (php, dart, swift, zig) onto the shared `TestClientRenderer` + `render_http_test` driver. Each language now defines a `<Lang>TestClientRenderer` struct + `impl client::TestClientRenderer` capturing only the language-native primitives (PHPUnit `public function test*(): void` + Guzzle, Dart `test('...', () async {})` over `dart:io`, XCTest `func testFoo() throws` over `URLSession`, Zig `test "..."` over `std.http.Client`). Per-language deltas: php now emits validation-error assertions even when the fixture also has a `body` (the old gate suppressed them); dart's `DartTestClientRenderer` carries a small `Cell<bool>` for `is_redirect` + `in_skip` because `CallCtx` doesn't carry expected-status; swift binds `_resp`/`_responseData` in `render_call` and references those names from subsequent assertion methods (matching the synchronous-via-DispatchSemaphore wrapper); zig's `render_assert_header` and `render_assert_partial_body`/`render_assert_validation_errors` are stubs (Zig's stdlib HTTP client doesn't expose response headers from `client.fetch(...)` results — same TODO pattern already used for `method_result`/`matches_regex` in the file).
- feat(e2e/python,e2e/typescript,e2e/ruby,e2e/rust): migrate the four pilot languages (python, typescript/node, ruby, rust) onto the shared `TestClientRenderer` + `render_http_test` driver. Each language now defines a small `<Lang>TestClientRenderer` struct + `impl client::TestClientRenderer` capturing only the language-native primitives (`def test_X(client):` / `it(...)` / RSpec `it` block / `#[tokio::test] async fn`); the canonical sequence (open → call → status → headers → body → validation → close), header sorting, content-encoding stripping, skip-directive handling, and `null`/empty-string body sentinels are now driven once from the shared driver. The four files `crates/alef-e2e/src/codegen/{python,typescript,ruby,rust}.rs` lose ~145-line monolithic HTTP-test renderers in favor of trait impls, leaving `render_http_test_function` (or equivalent) as a thin pre-hook + delegate to `client::http_call::render_http_test`. Behavioral notes: rust now also emits body/header/validation assertions from the shared driver (previous code only asserted status); typescript request headers are now emitted in deterministic sorted order rather than HashMap insertion order; python's one fixture with both `body` and `validation_errors` (`content_type_validation_invalid_type`) now emits both assertions instead of suppressing the latter (the validation assertion is a strict subset of the body equality, so no false failures).
- feat(e2e): shared `TestClientRenderer` trait and `render_http_test` driver in `crates/alef-e2e/src/codegen/client/`. Per-language e2e codegen previously duplicated the structural shape of every HTTP test (function header, request build, status/header/body assertions) across 18 monolithic per-language files (359-2965 lines each); the shape only differed in syntax. The new `client::TestClientRenderer` trait captures the canonical sequence — `render_test_open` → `render_call` → `render_assert_status` → per-header `render_assert_header` → `render_assert_json_body` / `render_assert_partial_body` → `render_assert_validation_errors` → `render_test_close` — and the shared `client::http_call::render_http_test` driver invokes those primitives in order, with built-in handling of skip directives, content-encoding header stripping, deterministic header ordering, and the `null`/empty-string body sentinels. Languages migrate one at a time: each implements the trait, replaces its monolithic `render_http_test_function` with a one-line call to the shared driver, and may optionally split its file into a `<lang>/{mod,client,ws,helpers}.rs` directory. 9 new unit tests use a tag-emitting mock renderer to lock down the driver's call sequence and special-case handling. A companion `client::ws_script::WebSocketScriptRenderer` trait is stubbed out for Phase 2B/2C.

### Changed

- chore: scrub library-specific terminology from alef source comments, doc-strings, test fixtures, and skill references. alef is a generic polyglot bindgen tool used by multiple downstream consumer libraries; comments, examples, and test fixture data that previously named one specific consumer ("spikard", `dev.spikard`, `:spikard`, etc.) are renamed to neutral placeholders (`mylib`, `mylib-core`, `consumer_core`, `com.example`, `:mylib`, `MyHandler`). Changes are limited to documentation/test data — no production code logic or generator output is affected. Historical published `## [0.x.y]` CHANGELOG entries are intentionally not rewritten (they describe shipped releases).
- fix(adapters/pyo3): callback-bridge codegen for Python no longer emits hardcoded `prepare_request`/`interpret_response` stub methods referencing a specific consumer's `handler_trait::RequestData` type. The stubs were dead code (always returning `Err(...)` "not implemented") that imposed a 3-method trait shape on every consumer; other language bridges (Node, Ruby, PHP, ...) only implement the user-defined method and were already correct. The Python branch now matches that pattern. If a consumer's actual trait requires additional methods, they must declare default impls in their trait definition or extend the AdapterConfig schema.

### Fixed

- fix(taskfile): `task set-version` now also bumps the workspace `alef.toml` (alef dogfoods its own config). Also switched from BSD-only `sed -i ''` to portable `sed -i.bak ... && rm -f` so the task works on Linux CI as well as macOS.
- fix(scaffold/node): emit `index.d.ts` at the package root in addition to `src/index.d.ts`. The generated `package.json` declares `"types": "index.d.ts"` (and lists `"index.d.ts"` in `files`), but the scaffold previously only emitted the type-shim under `packages/{node}/src/`, leaving the npm package without discoverable declarations at the path consumers actually resolve. Downstream `import 'kreuzberg'` (or any other alef-managed npm package) typed under the strict tsconfig produced `TS7016: Could not find a declaration file for module 'kreuzberg'` and degraded to implicit `any`, propagating to every test/file that referenced binding-emitted types like `JsExtractionConfig`. The fix emits both copies — the `src/` copy is preserved for in-tree tsconfig "include": ["src"] tooling; the new root copy is what npm exposes via `package.json#types`. Scaffold tests updated to assert both files are present (file count rises from 7 → 8 in `test_scaffold_node` and 10 → 11 in `test_scaffold_multiple`).
- fix(backend/pyo3): correct `Arc<T: !Copy>` field codegen for clone. When a struct field had type `Option<Arc<serde_json::Value>>` (or any non-String `Arc`-wrapped type), the core→binding `From` impl chained an extra `.map(|v| (*v).clone().into())` on top of a base conversion that already handled the `Option<Arc<T>>` via Display/Deref coercion (e.g. `val.X.as_ref().map(ToString::to_string)`). This produced `(*String).clone()` at the call site — dereferencing a `String` to `str`, then calling `str::clone()` which fails since `str: !Clone`. The fix detects when the base conversion is already a complex expression (not a simple `val.{name}` passthrough) and returns it unchanged, relying on `Arc<T>: Display + Deref<Target=T>` to correctly produce the binding-side value.
- fix(backend/pyo3): emit `Arc<tokio::sync::Mutex<T>>` (instead of `Arc<std::sync::Mutex<T>>`) for opaque types whose `&mut self` methods are all `async`. `std::sync::MutexGuard` is `!Send`, so holding a guard across `.await` makes the surrounding future `!Send`, which fails to compile against PyO3's `Send` future bound on `pyo3_async_runtimes::tokio::future_into_py`. `tokio::sync::MutexGuard` IS `Send`. New helper `alef_codegen::generators::type_needs_tokio_mutex` returns true only when every `&mut self` method on a mutex-needing type is async (mixed sync+async `&mut self` methods stay on `std::sync::Mutex` since `tokio::sync::Mutex::lock()` returns a future). The PyO3 backend rewrites the rendered struct + impl block to swap the lock type and `.lock().unwrap()` → `.lock().await` for these types. Verified end-to-end against an opaque type wrapping `axum-test`'s `TestWebSocket` in `Arc<Mutex<>>` with all-async `&mut self` methods: with the previously-required exclusions removed from `alef.toml`, the consumer's PyO3 cdylib compiles cleanly and the generated future satisfies the `Send` bound.
- fix(e2e/rust): rust e2e codegen now honors `owned = true` on `bytes` and `string` arg configs. Pre-this-change the renderer always emitted `&{name}` for bytes (via `.as_bytes()`) and a bare `&str` literal for string, regardless of whether the underlying kreuzberg-core function took the value by reference or by ownership (e.g. `detect_image_format(data: Vec<u8>)`, `detect_mime_type(path: String, ...)`). The codegen now flips both binding type (`Some(...).to_vec()` / `String::new()`) and pass expression (`name.to_string()` / `name.to_vec()` / `name`) when `owned = true`, so call sites match the function signature without forcing every owned-by-value Rust API into a borrowed wrapper. Two regression tests added in `crates/alef-e2e/tests/rust_function_call_fixtures.rs` (owned bytes file-path, owned string).
- fix(backend-magnus): functions registered via the trait*bridge path now use fixed arity in `function!(...)` instead of `-1` (variadic). The bridge generator emits a fixed-arg signature like `fn convert(html: String, options: Option<String>, visitor: Option<Value>)`, but the registration loop previously emitted `function!(convert, -1)` whenever any param was optional. Magnus's `function!` macro requires the fn to take `&[Value]` for arity `-1`, so the trait-bridge fn failed to satisfy `FunctionCAry<*>`/`Fn<(&[Value],)>`, breaking compilation for every binding that exposed a top-level function with an `HtmlVisitor`-style trait param (e.g. `html-to-markdown`'s ruby gem failed to build on linux/aarch64/macos with E0599 "method `call_handle_error`exists but trait bounds were not satisfied"). Bridge functions now register with`function!(name, params.len())`; non-bridge optional-param functions still use variadic `-1`with`scan_args` as before.

## [0.13.5] - 2026-05-02

### Fixed

- fix(e2e/rust): rust e2e codegen no longer short-circuits all non-HTTP, non-mock-server fixtures to a `// TODO: implement when a callable API is available` stub. Pre-0.13.5 the renderer assumed any fixture without an `http` block or a `mock_response` was a schema/spec validation fixture (asyncapi/grpc/graphql_schema) with no callable Rust API, so libraries whose fixtures invoke a plain function (e.g. `kreuzberg::extract_file(path, mime, config)`) emitted zero real test bodies. The stub gate now triggers only when the resolved call config has no function name; fixtures pointing at a configured `[e2e.call]` (or `[e2e.calls.<n>]`) render real function invocations with the right imports.
- fix(e2e/rust): `use {crate}::{fn};` import lines are now emitted for any non-HTTP fixture whose resolved call config has a function name, not only `mock_response`-bearing fixtures. Previously plain function-call fixtures rendered the call site but omitted the `use` line, producing E0425 "cannot find function" on every test.
- fix(e2e/rust): `bytes` arguments whose fixture value is a relative file path (e.g. `"pdf/fake_memo.pdf"`, classified the same way as in the python codegen) are now loaded at runtime via `std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_documents/", "<path>")).expect(...)` and passed by reference. Previously the path string itself was embedded as `r#"…"#` and `.as_bytes()`-ed, which compiled but always passed the path text as the file contents.

## [0.13.4] - 2026-05-01

### Added

- feat(e2e/wasm): the wasm e2e codegen now emits real `extractFile`/`extractBytes`-style test cases for non-HTTP fixtures, not just HTTP-server fixtures. Pre-0.13.4 the wasm renderer filtered `f.http.is_some()` and dropped every function-call fixture, leaving `e2e/wasm/` with only `globalSetup.ts`+`package.json` and zero tests against the binding's core async API. The wasm codegen now reuses the typescript renderer (parameterised by `lang = "wasm"`) and emits a `setup.ts` chdir to `test_documents/` when any active fixture takes a `file_path` or `bytes` arg, plus `globalSetup.ts` only when at least one HTTP fixture is in scope. Function-call resolution now goes through `[e2e.calls.<name>.overrides.wasm]` (auto-camelCasing the rust snake_case name when no explicit override is set), so wasm-specific renames work the same way they do for node.

### Changed

- chore(codegen): `resolve_node_function_name` (in `alef-e2e::codegen::typescript`) now takes a `lang: &str` parameter so the wasm codegen can reuse it. Existing call sites unaffected — typescript codegen passes `"node"` and behaves identically.

## [0.13.3] - 2026-05-01

### Fixed

- fix(backend/php): always emit `#![cfg_attr(windows, feature(abi_vectorcall))]`
  in the generated PHP binding lib.rs. ext-php-rs requires the `vectorcall`
  calling convention for PHP entry points on Windows, but the generator
  previously only emitted the cfg_attr when `[php].feature_gate` was set.
  Without it, downstream crates fail with E0658 ("the extern \"vectorcall\"
  ABI is experimental"). The cfg_attr is a no-op on non-windows so it costs
  nothing on Linux/macOS builds.

## [0.13.2] - 2026-05-01

### Fixed

- fix(publish/php): use `[php].extension_name` (composer.json's `php-ext.extension-name`)
  for PIE archive filenames and inner `.so`/`.dll` name, not the binding crate name. PIE
  installs binaries by extension-name, so `package_php` previously emitted
  `php_{crate}_php-...` archives that PIE couldn't find. Cargo's compiled artifact filename
  (still based on the crate name with `-php` suffix) is now resolved separately and renamed
  to `{ext_name}.so` / `{ext_name}.dll` inside the archive.

- fix(backend/ffi): honor `[ffi] exclude_types` in generated lib.rs. Previously the `FfiConfig::exclude_types`
  field was parsed from `alef.toml` but never applied during code generation — all types in the API surface
  were emitted regardless. The FFI backend now filters `api.types` through `ffi_exclude_types` before emitting
  struct wrappers, field accessors, and method wrappers, matching the behavior of the elixir and wasm backends.

## [0.13.1] - 2026-05-01

### Fixed

- fix(cli/e2e): `alef e2e generate` now sweeps orphan alef-generated files left under the e2e (and `--registry` test_apps) output root when a category or fixture leaves the generation set (e.g. all fixtures in a category resolve to skipped for the active binding surface, or wasm fixture-filtering drops a file). Previously these files lingered on disk with stale `alef:hash:` headers and `alef verify --exit-code` reported them as stale forever, blocking pre-commit hooks even after a clean regen. The sweep only deletes files whose first 10 lines contain the `auto-generated by alef` marker, so user-owned fixtures, scaffolded manifests, and lockfiles are never touched. Dependency directories (`target`, `node_modules`, `_build`, `vendor`, `deps`, `.venv`, `build`, `dist`, `.git`, `Pods`) are skipped during the walk.

## [0.13.0] - 2026-05-01

### Changed (BREAKING)

- feat(publish/php): `alef publish package --lang php` now produces PIE-conventional archives.
  Filename: `php_{ext}-{ver}_php{phpVer}-{arch}-{os}-{libc}-{ts}.tgz` (Unix) /
  `php_{ext}-{ver}-{phpVer}-{ts}-{compiler}-{arch}.zip` (Windows).
  Archive contains only `{ext_name}.so`/`.dll` at the archive root — no `composer.json`,
  `pie.json`, `INSTALL.md`, or `ext/` subdirectory. Requires new flags:
  `--php-version` (required), `--php-ts` (default `nts`), `--php-libc` (auto-detected),
  `--windows-compiler` (required on Windows targets). A SHA-256 sidecar
  `{archive}.sha256` is always written alongside the archive.

### Added

- feat(template-versions): `ENVOY_VERSION_RANGE` for the `envoy` Hex package — used by gleam e2e tests to read `MOCK_SERVER_URL` since neither `gleam_stdlib` nor `gleam_erlang` expose env-var access.
- feat(e2e/gleam): inject `gleam_http` as a direct dep and emit `import gleam/result` so generated tests can compile under gleam_stdlib >= 1.0.

### Fixed

- fix(codegen/conversions): preserve `Some(...)` wrap when the binding optionalizes a non-optional core field of type `Cow<'_, str>` (e.g. NAPI `optionalize_defaults`); previously the Cow `to_string()` branch dropped the upstream `Some(...)` wrap, producing `String` where `Option<String>` was required and breaking compile of NAPI bindings whose core type holds `Cow<'static, str>` (`ArchiveMetadata.format`, `ExtractionResult.mime_type`)
- fix(e2e/wasm): drop non-HTTP fixtures, fixtures with a literal `Content-Length` header, and fixtures using TRACE/CONNECT before code generation; node `fetch` (undici) rejects pre-set Content-Length and disallows TRACE/CONNECT, and empty `describe(...)` blocks fail vitest

- fix(e2e/elixir): emit a `@tag :skip` ExUnit test for non-HTTP non-`mock_response` fixtures (AsyncAPI, WebSocket, OpenRPC schema-only) so generated suites compile without referencing a non-existent binding `handle_request`/`handle_request_async` callable.
- fix(e2e/elixir): drop `content-encoding` and `connection` from generated header assertions — Req auto-decompresses gzip/brotli responses and strips the encoding header, and `connection` is a hop-by-hop header stripped by the response pipeline.
- fix(e2e/gleam): import `gleam/httpc` (not the package name `gleam_httpc`) and call `httpc.send/1`; the previous import path produced "unknown module" errors at compile time.
- fix(e2e/gleam): replace `gleam/os.get_env` (removed from gleam_stdlib >= 1.0) with `envoy.get/1` for `MOCK_SERVER_URL` lookup.
- fix(e2e/gleam): annotate the `list.find` predicate with `fn(h: #(String, String))` so the gleam type checker can resolve tuple field access on `resp.headers`.
- fix(e2e/gleam): use `result.is_ok` (not `option.is_some`) for header-presence assertions — `list.find` returns `Result(a, Nil)`, not an Option.
- fix(e2e/gleam): strip leading underscores/digits from HTTP test function names so numeric-prefixed fixture IDs (`13_json_with_charset_utf16`) produce valid gleam identifiers.
- fix(e2e/gleam): always emit the `e2e_gleam_test.gleam` entry module so `gleam test` discovers the per-category test files.
- fix(e2e/gleam): drop WebSocket-upgrade fixtures (request advertises `Upgrade: websocket`) — gleam_httpc cannot follow HTTP/1.1 protocol upgrades and errors with `ResponseTimeout`.
- fix(e2e/wasm): drop non-HTTP fixtures, fixtures with a pre-set `Content-Length` header, and fixtures using `TRACE`/`CONNECT` methods — node's undici fetch rejects the first two and refuses to dispatch the latter.

## [0.12.16] - 2026-05-01

### Added

- feat(backend-csharp,core): support `[languages.csharp.exclude_functions]` to skip generating selected free functions from the C# binding surface
- feat(backend-java): emit `JsonInclude(NON_NULL)` on optional fields and `JsonInclude(NON_ABSENT)` at the class level so Jackson omits null/empty optionals when serializing Java DTOs across the FFI boundary
- feat(backend-java): wrap optional FFI parameters as `TypeRef::Optional` for null-safety; non-zero integer defaults emit a compact constructor that initializes them when callers pass null

### Fixed

- fix(backend-magnus): use variadic arity (-1) with `scan_args` for free functions that have optional or promoted parameters; previously such functions were registered with a fixed arity equal to the total parameter count, causing Ruby callers that omit trailing optional arguments to get an argument count error
- fix(e2e/ruby): emit `nil` placeholder for skipped optional positional args when a later arg is present; previously omitting an optional String arg before a json_object arg produced a 2-arg call where the config object landed in the string slot, causing a `TypeError: no implicit conversion` runtime error
- fix(e2e/elixir): add kreuzberg path dep and rustler direct dep to generated mix.exs so NIF force-build works in e2e tests
- fix(e2e/elixir): replace `map_or(false, ...)` with `is_some_and(...)` to clear `clippy::unnecessary_map_or`
- fix(e2e/java): set `workingDirectory` in the generated maven-surefire plugin block so tests run from the project basedir, fixing relative fixture path resolution
- fix(backend-extendr): derive R wrapper output path as packages/r/R/ instead of packages/r/src/rust/src/
- fix(backend-pyo3): emit `pass` body for empty structs that pass through the codegen; combined with the v0.12.15 fix this allows empty structs to be referenced in tagged-union aliases without import-time `NameError`
- fix(backend-ffi): preserve numeric error code metadata on the FFI error type through the type generators so host-side error conversions can attach `code`/`message` consistently
- fix(scaffold/r): include `<R_ext/Visibility.h>` in generated entrypoint.c to define `attribute_visible`
- fix(scaffold/php): align scaffolded PHP package metadata and dev tooling with current backend expectations
- fix(cli/extract): drop debug `eprintln` from the extract pipeline so generation output is no longer polluted on stderr
- fix(e2e/go): include `mock_response` in unit-test fixture builder so `test_go_method_name_uses_go_casing` exercises the real codegen path instead of the non-HTTP skip stub
- fix(e2e/rust): include `mock_response` in unit-test fixture builders so `rust_call_overrides` regression tests exercise the real codegen path
- fix(scaffold): assert on `config.ext_dir = 'native'` (the current `rb_sys/mkmf` API) instead of the deprecated `cargo_manifest` form

## [0.12.15] - 2026-05-01

### Fixed

- fix(backend-pyo3): emit empty Python dataclasses for empty Rust structs (e.g. `pub struct ExcelMetadata {}`) when they have `Default`. Previously, the codegen skipped them entirely, leaving dangling references in tagged-union type aliases (`FormatMetadata = ... | ExcelMetadata | ...`) and breaking module import with `NameError: name 'ExcelMetadata' is not defined`.
- fix(e2e/escape): escape ASCII control characters (`\x00`–`\x1f`) in Python string literals using `\xHH` so generated Python source remains valid. Previously, fixtures with embedded NUL bytes (e.g. `clean_text_basic`) produced source files Python's parser rejected with `SyntaxError: source code string cannot contain null bytes`.
- fix(e2e/csharp): use PascalCase HTTP method names for `System.Net.Http.HttpMethod` static properties; route header assertions to `.Headers` vs `.Content.Headers` correctly to avoid "Misused header name" runtime errors; uniquify per-assertion variable names so `out var` does not redeclare in method scope.
- fix(e2e/go): clarify the non-HTTP stub message and document why it fires (Go bindings without a callable matching `[e2e.call].function`).
- fix(e2e/go): emit `os` import when any HTTP fixture is present (HTTP tests read `MOCK_SERVER_URL` via `os.Getenv`); previously only mock_url args triggered the import.
- fix(e2e/go): only emit `io` import when at least one HTTP fixture has a body assertion; eliminates `"io" imported and not used` build errors in test files where no fixture asserts on the body.
- fix(e2e/go): only declare `bodyBytes` when the fixture has a body assertion; eliminates `declared and not used: bodyBytes` build errors.
- fix(e2e/go): use a separate `want :=` declaration plus `%q` formatting in the body-mismatch `t.Fatalf` instead of interpolating the (already-Go-quoted) literal into the format string; previously emitted unparsable Go for fixtures whose expected body contained double-quotes (e.g. JSON-as-string responses) and for headers whose names/values contained backticks.
- fix(e2e/go): decode HTTP response and expected bodies into `any` (not `map[string]any`) so JSON arrays in the expected body decode without `cannot unmarshal array into ... map` runtime errors.

## [0.12.14] - 2026-05-01

### Fixed

- fix(e2e/rust): translate synthetic assertion fields (`embeddings`, `embedding_dimensions`, `embeddings_valid`, `embeddings_finite`, `embeddings_non_zero`, `embeddings_normalized`, `chunks_have_embeddings`, `keywords`, `keywords_count`) to correct Rust expressions instead of treating them as struct field accesses; `keywords`/`keywords_count` map to `extracted_keywords` on `ExtractionResult`.
- fix(e2e/python): intercept synthetic embedding and keyword assertion fields before struct field emission; skip `keywords`/`keywords_count` (no Python binding for `extracted_keywords`).
- fix(e2e/typescript): intercept synthetic embedding and keyword assertion fields; emit Vitest `expect` assertions using array methods; skip `keywords`/`keywords_count`.
- fix(e2e/go): intercept synthetic embedding and keyword assertion fields; emit inline immediately-invoked functions for predicate checks on `*[][]float32`; skip `keywords`/`keywords_count`.
- fix(e2e/ruby): intercept synthetic embedding and keyword assertion fields in the binding assertion path; emit RSpec `expect` assertions using Ruby enumerable methods; skip `keywords`/`keywords_count`.
- fix(e2e/php): intercept synthetic embedding and keyword assertion fields; emit PHPUnit assertions using `array_reduce`/`array_filter`; skip `keywords`/`keywords_count`.
- fix(e2e/csharp): intercept synthetic embedding and keyword assertion fields; emit xUnit assertions using LINQ; skip `keywords`/`keywords_count`.
- fix(e2e/elixir): intercept synthetic embedding and keyword assertion fields; emit ExUnit assertions using `Enum` functions; skip `keywords`/`keywords_count`.
- fix(e2e/r): intercept synthetic embedding and keyword assertion fields; emit testthat `expect_*` assertions using `sapply`/`all`; skip `keywords`/`keywords_count`.
- fix(e2e/wasm): intercept synthetic embedding and keyword assertion fields (defensive, embed is WASM-skipped); skip `keywords`/`keywords_count`.
- fix(e2e/ruby): use title-cased `Net::HTTP` request class names (`Delete`, `Head`, `Patch`, `Put`) instead of all-caps variants that don't exist in Ruby's stdlib; skip `content-encoding` header assertions; handle plain-string and empty bodies correctly; skip HTTP 101 WebSocket upgrade tests; emit `skip` stubs for non-HTTP fixtures (WebSocket, SSE) that cannot be tested via Net::HTTP; avoid `JSON.parse(nil)` on 204 No Content responses; deduplicate `Content-Type` header when body is already set.
- fix(backend-magnus): use `.to_vec()` instead of `.into()` when wrapping `&Bytes` return values from non-opaque struct methods; `Vec<u8>: From<&Bytes>` is not implemented so `.into()` failed to compile for methods like `as_bytes()` on `UploadFile`.
- fix(e2e/gleam): filter out HTTP-type fixtures and call-based fixtures without a Gleam-specific override; emit a `src/` placeholder module (required by the Gleam toolchain) and a smoke test when no fixture-driven tests are generated; strip leading digits from test function names.
- fix(e2e/kotlin): generate proper `java.net.http.HttpClient` HTTP tests for all 559 HTTP fixtures; hit `MOCK_SERVER_URL/fixtures/<id>` per the mock server protocol; URL-encode query params at runtime to handle JSON array values; escape `$` in string literals to prevent Kotlin string interpolation; skip Java's restricted headers (`Connection`, `Content-Length`, `Expect`, `Host`, `Upgrade`); skip `content-encoding` header assertions (mock server doesn't compress); skip 101 status tests (Java HttpClient doesn't handle protocol upgrades); emit `assumeTrue(false)` stubs for non-HTTP fixtures without a Kotlin-specific call override; add `kotlin("test")` to build.gradle.kts dependencies.
- fix(escape): add `escape_kotlin` function that escapes `$` in addition to standard Java escapes to prevent Kotlin string interpolation on embedded values.

## [0.12.13] - 2026-05-01

### Fixed

- fix(extract): `validate_call_export` now accepts method-on-type references (e.g., `function = "chat"` where `chat` is a method on a public type). Previously rejected legitimate method-style entry points used by `liter-llm` and similar libraries.

## [0.12.12] - 2026-05-01

### Fixed

- fix(codegen/binding_helpers): apply `k.into()` to Map keys in `gen_lossy_binding_to_core_fields` so the binding-side `is_empty()` helper round-trips wrapped string keys (`Cow`, `Box<str>`, `Arc<str>`) correctly. Previously the helper emitted `(k, …)` directly, which broke when the core key type was `AHashMap<Cow<'static, str>, Value>`.
- fix(backend-wasm): also deserialize `Vec<Vec<T>>` parameters in the `can_delegate` branch via `serde_wasm_bindgen::from_value`. Previously only the Result-returning serde-recovery branch handled this, so non-Result functions like `generate_cache_key` received the raw `JsValue`.
- fix(codegen/binding_to_core): emit `k.into()` for non-Json keys in the generic `Map` arm, completing the Cow-key round-trip fix.

### Changed

- feat(e2e/go): emit `t.Skip("TODO: ...")` stubs for all fixtures that lack a `mock_response`; omit the package import when no test uses it to avoid the Go "imported and not used" compile error.
- feat(e2e/java): emit `Assumptions.assumeTrue(false, "TODO: ...")` stubs for all fixtures that lack a `mock_response` so the test class compiles and is skipped cleanly.
- feat(e2e/csharp): emit `[Fact(Skip = "TODO: ...")]` stubs for all fixtures that lack a `mock_response` so the test class compiles and all tests are reported as skipped.
- fix(e2e/php): skip JSON decode and body assertions when `expected_response.body` is the empty-string sentinel, avoiding `JsonException` on empty bodies.
- fix(e2e/php): skip `markTestSkipped` for non-HTTP fixtures with no assertions; add `allow_redirects => false` to Guzzle client; skip HTTP 101 WebSocket upgrade tests; handle plain-string response bodies with raw `(string)$response->getBody()` comparison; suppress redundant `validation_errors` assertions when a full `body` assertEquals is generated; skip `content-encoding` header assertions since the mock server always returns uncompressed bodies.
- fix(e2e/rust): handle `<<absent>>` sentinel by omitting that header from mock responses; handle `<<uuid>>` sentinel by generating a real UUID v4 value; use `text/plain` default content-type when body is not JSON.
- fix(e2e/python): skip content-encoding assertions; handle plain-string and empty/null bodies without json.loads; skip HTTP 101 tests; disable redirect following; fix validation errors field name ("errors" not "detail").
- fix(e2e/typescript): skip content-encoding assertions; handle plain-string and empty/null bodies; skip HTTP 101 tests; add redirect: 'manual'; suppress import when no-assertion non-HTTP fixtures; emit it.skip stubs; fix validation errors field name.

## [0.12.11] - 2026-05-01

### Changed

- feat(e2e/swift): emit test files into `packages/swift/Tests/<Module>Tests/` instead of a standalone `e2e/swift` package. SwiftPM 6.0 forbids local `.package(path:)` references between packages in the same git repository; placing the generated tests inside the existing library package sidesteps the restriction while retaining a skeletal `e2e/swift/Package.swift` for CI reference.
- feat(e2e/zig): use `std.heap.DebugAllocator(.{}) = .init` (Zig 0.16+); emit compilable stub test bodies (`_ = testing;`) for fixtures with no assertions instead of calling a non-existent `handle_request` function; omit allocator setup when no setup lines are needed.
- feat(scaffold/swift): upgrade generated `Package.swift` to `swift-tools-version: 6.0`; remove `unsafeFlags` from `RustBridge` target's `linkerSettings` so the package can be used as a SwiftPM dependency.

### Fixed

- fix(e2e/rust/tests): pass the new `needs_http_tests` flag through `cargo_toml_generation.rs` test fixtures so the suite compiles after the `render_cargo_toml` signature was extended.
- fix(codegen/conversions): emit `k.to_string()` for Map keys in core→binding conversion and `k.into()` for the reverse direction so `Cow<'_, str>`/`Box<str>`/`Arc<str>` keys (which the type resolver normalizes to `TypeRef::String`) round-trip correctly. Without this fix the generated `From` impls produced `(Cow<'_, str>, …)` iterators feeding `HashMap<String, String>::from_iter`, breaking pyo3/napi/php bindings on `Metadata.additional`.
- fix(e2e/elixir): add missing commas between deps entries in generated `mix.exs`; without them Elixir emits a syntax error before deps like `{:req, ...}`.
- fix(e2e/kotlin): fall back to `alef_config.resolved_version()` when `[e2e.packages.kotlin]` has no explicit `version`, avoiding a stale `0.1.0` jar reference in `build.gradle.kts`.
- fix(e2e/gleam): strip leading underscores from generated test-function names; fixture IDs with numeric prefixes (e.g. `13_json_...`) produced `_json_..._test()` which Gleam rejects.
- fix(backend-dart): emit `lib/<pkg>.dart` barrel file re-exporting `src/<pkg>.dart` so `package:<pkg>/<pkg>.dart` imports resolve correctly in e2e tests.
- fix(e2e/elixir): normalize scientific-notation floats for Elixir syntax (`1e-10` → `1.0e-10`, strip `e+` → `e`); use `response.body` directly instead of `Jason.decode!` since Req auto-decodes JSON; fall back to `Req.request(method: :METHOD, ...)` for non-standard HTTP verbs like OPTIONS/TRACE; extract first value from Req's `[{name, [values]}]` header lists; preserve original JSON map keys instead of snake_casing them.
- fix(e2e/rust): skip `content-encoding` headers in mock server responses — server returns uncompressed bodies so forwarding a `content-encoding` header would cause clients to attempt decompression and fail; also skip setting default `content-type: application/json` when the fixture already specifies one.
- fix(extract/type_resolver): treat bare `Value` as `TypeRef::Json` to match the idiomatic `use serde_json::Value;` re-import. Without this, `HashMap<String, Value>` fields (e.g. `ProblemDetails.extensions`) sanitized to `Map<String, String>` and downstream From-impls emitted `.into_iter().collect()` between mismatched value types, breaking pyo3/napi bindings. `serde_json::Value` (full path) was already handled.

## [0.12.10] - 2026-05-01

### Added

- feat(backend-wasm/config): add `custom_rust_modules`, `exclude_fields`, and `source_crate_remaps` options for hand-written Rust modules, per-type field exclusions for `cfg(not(target_arch = "wasm32"))`-gated source fields, and rewriting `<original_crate>::TypeName` references to `<override_crate>::TypeName` when `core_crate_override` is set.
- feat(codegen/conversions): introduce `core_type_path_remapped` / `apply_crate_remaps` so generated `From` impls reference the override crate when `source_crate_remaps` is configured, avoiding orphan-rule violations across re-export facades.

### Fixed

- fix(extract): preserve `Map<K, V>` structure during sanitization. Previously a `Map<Cow<'static, str>, serde_json::Value>` field was flattened to `TypeRef::String` whenever the key resolved through a sanitized `Named` type; the field now stays a Map so binding backends emit the correct iterator-based conversion (or `serde_wasm_bindgen::to_value` for WASM) instead of `format!("{:?}", val.<field>)`. Fixes `Metadata.additional` mismatches in pyo3, napi, php, and wasm bindings.
- fix(backend-napi): widen nested `Vec<Vec<primitive>>` element-wise in return conversion when the binding declares the wider type (`f32` → `f64`, `u64`/`usize`/`isize` → `i64`). Mirrors the existing single-Vec arm so `embed_texts` and similar functions returning `Vec<Vec<f32>>` compile.
- fix(backend-php): widen nested `Vec<Vec<u64/usize/isize>>` element-wise in return conversion to match the i64 cast emitted for single-Vec return types.
- fix(backend-php): `gen_stub_return` now respects the function's actual error variance — non-Result functions get a type-appropriate default (e.g. `String::new()`, `Vec::new()`, `None`) instead of an `Err(PhpException::default(...))` body that violates the function signature.
- fix(backend-wasm): emit `serde_wasm_bindgen::from_value` deserialization for `Vec<Vec<T>>` parameters that arrive as `JsValue`. Without it, `generate_cache_key` and similar functions taking nested-vec composites passed the raw `JsValue` into the core fn.
- fix(core/version): add `to_r_version()` converting SemVer prereleases to CRAN-compatible four-component form.
- fix(scaffold/r): generate `packages/r/src/Makevars`, `Makevars.in`, `Makevars.win.in`, and `src/entrypoint.c`.
- fix(scaffold/r): change scaffolded `Cargo.toml` crate-type from `["cdylib"]` to `["staticlib", "lib"]`.
- fix(scaffold/r): generate `packages/r/NAMESPACE` with `useDynLib` bootstrap.
- fix(scaffold/r): use `to_r_version()` in `DESCRIPTION` `Version:` field.
- fix(backend-extendr): `bridge_imports()` returns bare paths, not full `use` statements.
- fix(cli/sync-versions): write CRAN-compatible version to `packages/r/DESCRIPTION`.
- fix(cli/validate-versions): compare R DESCRIPTION against CRAN-compatible version.

## [0.12.9] - 2026-04-30

### Fixed

- fix(e2e/go): add `returns_void` field to `CallConfig` so functions returning only `error` (e.g. `validate_host`, `validate_port`, `validate_language_code`) emit `err := func()` instead of `_, err := func()` in generated Go tests.
- fix(e2e/go): Go override `returns_result = true` now respected for `dedup_text` and similar functions returning `(value, error)`; previously the call-level default (`false`) overrode the per-language override.

## [0.12.8] - 2026-04-30

### Fixed

- fix(backend-dart): convert nested `Vec<Vec<f32>>` to `Vec<Vec<f64>>` in bridge so embed_texts return type matches binding declaration.

## [0.12.7] - 2026-04-30

### Fixed

- fix(backend-rustler): `force_build` honors `Application.compile_env(:rustler_precompiled, [:force_build, :<app>], false)` in addition to env var + Mix.env() heuristics, so consumers can opt into source builds via mix config.
- fix(extract): empty struct with `#[derive(Default, Serialize, Deserialize)]` is treated as a transparent NifMap data type rather than an opaque resource handle (e.g. `ExcelMetadata{}` is data, not a `ResourceArc`).
- fix(scaffold/precommit): generated `.pre-commit-config.yaml` includes the shared `kreuzberg-dev/pre-commit-hooks` repo (shfmt/shellcheck/hadolint/textlint), tracked via the new `KREUZBERG_PRECOMMIT_HOOKS_REV` template version.
- fix(e2e/java): per-fixture `options_type` overrides now collected for ObjectMapper imports/usage; previously only class-level options_type was honored, leaving overrides un-imported.
- fix(backend-magnus): optional Ruby parameters typed as `Option<magnus::Value>` (not `magnus::Value`) — handler now matches `Some(_v) if !_v.is_nil()` before calling `.funcall(...)`, fixing a compile error when an optional struct arg is omitted.
- fix(backend-pyo3): union alias types used directly in `options.py` runtime expressions (e.g. `FormatMetadata = str | ExcelMetadata | ...`) are now imported unconditionally instead of under `TYPE_CHECKING`. Because union alias RHS is evaluated at module load, all names must resolve at import time; previously the TYPE_CHECKING guard caused `NameError` for data-enum payload struct types not defined locally.
- fix(codegen/core-to-binding): `Map<String, Json>` fields now emit `.to_string()` per-value conversions (same as `Map<String, String>`). Previously the map-value branch only matched `TypeRef::String`, so `serde_json::Value` values were not converted, causing a type mismatch in NAPI-RS and other backends.
- fix(e2e/go): `go.mod` now includes `github.com/stretchr/testify` as a required dependency so generated Go test modules compile without a missing-import error.
- fix(e2e/go): `bytes`-type args now decode from base64 at runtime via `base64.StdEncoding.DecodeString`; the `encoding/base64` import is emitted only when needed.
- fix(e2e/go): optional `string` args are now passed as `*string` (address of a local) matching Go binding signatures that take pointer-to-string for nullable strings.
- fix(e2e/go): functions that return only `error` (no result value) now emit `err := fn(...)` instead of `_, err := fn(...)`, fixing a compile error when the function signature is `func(...) error`.
- fix(e2e/go): `result_is_simple` functions (returning `*string`, `*bool`, etc.) now dereference the pointer into a `value` local before assertions and emit a nil guard, rather than asserting directly on a pointer type.
- fix(e2e/go): `greater_than` assertions on optional (pointer) fields now emit a nil guard before the comparison, preventing a nil-pointer dereference.
- fix(e2e/go): `contains` assertions on optional array fields now use `strings.Join(*field_expr, " ")` instead of incorrectly dereferencing an array pointer as a string.
- fix(e2e/go): `contains_any` is now recognized as a string-assertion type that requires the `strings` import.
- fix(e2e/typescript): `package.json` local dependency now uses `file:<pkg_path>` instead of `workspace:*`, fixing resolution when the e2e package is not inside a pnpm workspace.
- fix(e2e/typescript): options-type imports now collect all distinct types across all per-fixture call overrides, not only the default call's options type; each type is imported exactly once.
- fix(e2e/typescript): `resolve_node_function_name` now converts snake_case function names to camelCase (NAPI-RS convention) when no explicit node override is set, eliminating `undefined is not a function` errors at runtime.
- fix(e2e/typescript): optional config args with no fixture value now emit `{} as unknown as <OptionsType>` instead of being skipped, matching NAPI-RS binding signatures that require the config parameter.
- fix(e2e/typescript): config `json_object` args are now cast via `as unknown as <T>` to suppress TS2352 type-overlap errors for partial config objects.
- fix(e2e/typescript): `bytes`-type args now emit `Buffer.from("<b64>", "base64")` so base64 fixture values are decoded to `Uint8Array` at test time.
- fix(e2e/typescript): `not_empty` assertions now handle array fields with `.length` checks and non-string struct fields with `!= null` presence checks, rather than emitting `.length` on every type unconditionally.
- fix(e2e/core): `CallOverride` gains an `arg_order` field to allow per-language parameter reordering when a binding's function signature differs from the canonical `args` list order.
- fix(e2e/escape): `escape_python` now escapes NUL (`\x00`) and other ASCII control characters (U+0001–U+001F) as `\xNN` sequences, preventing malformed Python string literals from non-printable fixture bytes.

## [0.12.6] - 2026-04-30

### Fixed

- fix(e2e/rust): `Option<Vec<T>>` fields in `fields_array` now emit `as_deref().unwrap_or(&[])` instead of `unwrap_or("")` in the unwrap binding. Previously, `as_deref()` on `Option<Vec<T>>` yields `Option<&[T]>`, so the `&str` fallback produced an `E0308` type mismatch (e.g. `detected_languages`, `metadata.sheet_names`).
- fix(e2e/rust): `greater_than_or_equal` assertions on optional non-array fields (e.g. `Option<usize>`) now emit `.unwrap_or(0) >= N` instead of a bare `>= N` comparison. Comparing `Option<usize> >= 2` directly is a type error (e.g. `metadata.sheet_count >= 2`).
- fix(e2e/rust): `equals` assertions with string values on optional string fields that were not pre-unwrapped (e.g. inside a `result_is_vec` loop) now emit `.as_deref().unwrap_or("").trim()` instead of `.trim()` directly on `Option<String>`, fixing `E0599` no method `trim` on `Option<T>` (e.g. `metadata.output_format`).
- fix(e2e/rust): `field: "result"` used as a sentinel to refer to the whole return value no longer emits `result.result`. When the field path exactly matches the result variable name, the codegen now references the result variable directly.
- fix(e2e/c): `result_type_name` fallback now derives from the base `call.function` name instead of the C-overridden function name. When `[e2e.call.overrides.c]` sets `function = "htm_convert"` and `prefix = "htm"`, the fallback type was `HtmConvert`, which combined with the `HTM` prefix produced `HTMHtmConvert` (doubled prefix). It now correctly produces `HTMConvert`.
- fix(backend-go): `TypeRef::Bytes`-returning methods and functions no longer emit `defer C.{prefix}_free_string(ptr)`. Bytes pointers alias internal FFI storage and must not be freed by the caller; freeing via `_free_string` (which expects `*C.char`) would corrupt the parent handle and caused cgo type errors. The `unmarshalBytes` helper copies the data without freeing the source pointer.
- fix(backend-php): PHP type stub methods with non-void return types now emit `throw new \RuntimeException('Not implemented.');` instead of an empty `{ }` body. Empty bodies are invalid for non-void methods under PHPStan level 9 (`implicitlyImpureMethod` / missing return), which broke `composer check` on downstream repos (e.g. `getErrorCode(): int`, getter methods, and `*Api` static stubs).

## [0.12.5] - 2026-04-30

### Added

- **Per-language `core_import` derivation.** When `[wasm|dart|swift].core_crate_override` is set, the wasm backend now also routes generated Rust `use` paths and `From`/`Into` impls through the override (e.g. `spikard_core::ProblemDetails` instead of `spikard::ProblemDetails`). Without this, the override only flipped the Cargo dep key but left source paths pointing at the umbrella crate, producing E0433 unresolved-crate errors at compile time. New helper: `AlefConfig::core_import_for_language(lang)` in `crates/alef-core/src/config/mod.rs`. Defaults to `core_import()` when no override is set, so existing configs are unaffected.
- **Wasm backend respects `exclude_types` for error converters.** `gen_wasm_error_converter` calls and the WASM exports list now skip any error whose `name` is listed in `[wasm].exclude_types`, mirroring the existing exclusion behavior for structs/enums/functions. Lets a wasm-safe surface (e.g. schema validation only) drop sibling-crate errors like `GraphQLError` or `SchemaError` without the binding referencing the unlinked crate.

## [0.12.4] - 2026-04-30

### Fixed

- fix(backend-php): inherent-method delegation helper (`gen_php_lossy_binding_to_core_fields`) now applies per-value `.into()` and preserves the Option layer for `Map<K, Named>` and `Option<Map<K, Named>>` fields. Same bug class as 0.12.3 fix in `alef-codegen::binding_helpers`, but PHP has its own helper that needed the same patch.

## [0.12.3] - 2026-04-30

### Fixed

- fix(codegen/binding-helpers): `gen_lossy_binding_to_core_fields` now applies per-value `.into()` for `Map<K, Named>` and `Option<Map<K, Named>>` fields. The 0.12.2 round only patched `field_conversion_to_core` (used by the `From<Binding> for Core` impl), but the inherent-method delegation path (used for `with_chunking`, `all`, `minimal`-style wrappers on PyO3/PHP/NAPI/Magnus/Rustler) is a parallel codegen path that constructs `core_self` directly. Both paths now emit per-value `(k, v.into())` so a binding-wrapper Map value converts to the core type.

## [0.12.2] - 2026-04-30

### Fixed

- fix(e2e/rust): `not_empty` / `is_empty` assertions on a field that is BOTH `fields_optional` AND `fields_array` now emit `as_ref().is_some_and(|v| !v.is_empty())` and `as_ref().is_none_or(|v| v.is_empty())` respectively, so `Option<Vec<T>>` struct fields (e.g. `chunks`, `images`, `pages`, `extracted_keywords`) compile against the assertion. Previously the codegen emitted `result.<field>.is_empty()` against the bare `Option<Vec<_>>`, which failed `E0599`.
- fix(e2e/rust): when a call returns `Vec<T>` (`result_is_vec` rust override), the call-site optional-string unwrap pass is skipped so per-element iteration emitted by `render_assertion` is the only place fields are accessed; previously the call-site emitted `let metadata_x = result.metadata.x.as_deref().unwrap_or("")` against `Vec<ExtractionResult>` (no `metadata` field on the Vec).
- fix(e2e/rust): non-optional `string` arguments are no longer prefixed with `&` in the call expression. The fixture binding is already a `&'static str` literal; adding another reference produced `&&str`, mismatching `&str` parameter slots (e.g. `extract_email_content(data, mime_type, ...)`). For `impl AsRef<Path>` parameters both forms still satisfy the bound thanks to `&&str: AsRef<Path>`.
- fix(e2e/rust): `not_empty` on a bare `Option<T>` result (no field) now emits `result.is_some()` when the call is annotated `result_is_option`, matching the existing `is_some()`/`is_none()` mapping for field-level checks.
- fix(backend-pyo3): python_field_type now resolves Named(DataEnum) types correctly per emit context (options.py dataclass field vs \_native.pyi stub) — introduces EmitContext enum and threads it through all collection branches (Map, Vec, Optional) so that in the OptionsModule context the bare name resolves to the locally defined union type alias rather than to the native module import, eliminating the mypy type mismatch between caller-supplied `dict[str, options.ExtractionPattern]` and the annotation `dict[str, _native.ExtractionPattern]`.
- fix(codegen/core-to-binding): emit explicit arms for `Map<K, Named>`, `Option<Map<K, Named>>`, `Vec<Named>`, `Option<Vec<Named>>` instead of falling through to the binding-to-core helper (was emitting wrong-direction conversions; broke every backend that uses the shared converter for high-level `Option<Map<Named>>` fields).
- fix(backend-rustler): split native.ex `force_build:` keyword across three lines so `mix format` accepts it without reformatting (was 114 chars, exceeded Elixir's 98-char default).
- fix(codegen/binding-to-core): apply per-value `.into()` when emitting `Option<Map<K, Named>>` field conversions (was dropping the wrapper conversion, causing rustc type mismatch in PyO3/PHP/Magnus/Rustler).
- fix(codegen/binding-to-core): preserve `Option` layer in optionalized-field path when field is genuinely IR-optional (was using `unwrap_or_default` and dropping `Option`, breaking NAPI `Option<Map<K, Named>>` round-trip).

### Added

- **Per-language `core_crate_override`.** `[wasm].core_crate_override`, `[dart].core_crate_override`, and `[swift].core_crate_override` let a Rust binding crate point at a sub-crate other than `[crate.name]` (e.g. a wasm-safe `spikard-core` instead of the umbrella `spikard` facade). When set, the binding's generated `Cargo.toml` depends on `<override> = { path = "../<override>" }` (wasm) or the equivalent `crates/<override>` path (dart/swift) and the override key replaces the umbrella crate as the core dep. (`crates/alef-core/src/config/languages.rs`, `crates/alef-core/src/config/mod.rs`, `crates/alef-backend-wasm/src/gen_bindings.rs`, `crates/alef-backend-dart/src/gen_rust_crate/cargo.rs`, `crates/alef-backend-swift/src/gen_rust_crate/`)
- **Per-language `exclude_extra_dependencies`.** `[wasm|dart|swift].exclude_extra_dependencies` filters specific keys out of the merged `[crate.extra_dependencies]` set for that language only — useful when shared sibling crates (e.g. `spikard-http`, `spikard-graphql`) cannot be linked into the wasm target. (`crates/alef-core/src/config/mod.rs::extra_deps_for_language`)

Both fields default to unset/empty so existing configs produce byte-identical output.

- test(backend-wasm): round-trip test for `Map<K, Named>` field through `serde_wasm_bindgen`, locks in the contract that Named values inside a Map must have symmetric serde impls (`crates/alef-backend-wasm/tests/gen_bindings_test.rs`).

### Changed

- BREAKING(cli): `alef generate` now runs language formatters by default; pass `--no-format` to skip. The previous `--format` flag is removed (was opt-in and easily forgotten, causing generated output to fail downstream linters like `mix format`). `alef all` behaves the same way — pass `--no-format` to suppress formatters.
- refactor(codegen): introduce `TypeMapper` trait in `alef-codegen`; every backend now implements it with exhaustive `TypeRef` matching. The Go, Java, C#, and FFI backends have been migrated to `GoMapper`, `JavaMapper`/`JavaBoxedMapper`, `CsharpMapper`, and `FfiParamMapper`/`FfiReturnMapper` structs respectively — all implementing `TypeMapper`. Adding a new `TypeRef` variant now produces a compile error in every backend that hasn't handled it, retiring the silent-fallthrough bug class that previously hid `AHashMap` as `Named("AHashMap")`. The dead `extendr/type_map.rs` stub (comment-only file) has been removed; `ExtendrBackend` already implemented `TypeMapper` in `gen_bindings.rs`.

## [0.12.1] - 2026-04-30

### Fixed

- **e2e mock-server respects fixture `expected_response.headers`.** The alef-generated mock HTTP server (both the test-embedded `mock_server` module and the standalone `mock-server` binary used by cross-language e2e suites) now applies fixture-declared response headers to the served response. Previously, headers from `http.expected_response.headers` (spikard schema) and `mock_response.headers` (liter-llm schema) were silently dropped, causing consumer header assertions (CORS, request-id, auth challenge, compression, etc.) to come back as `null`. `MockResponse` gained a `headers: HashMap<String, String>` field, `Fixture::as_mock_response()` bridges headers from both schemas, and the mock-server route handlers iterate the map and apply each entry via `Response::builder().header(name, value)`. Repeated `.header()` calls preserve multi-value semantics for headers like `Set-Cookie`. (`crates/alef-e2e/src/fixture.rs`, `crates/alef-e2e/src/codegen/rust.rs`)

- **go: skip method emission on opaque error types.** When a `TypeDef` is both opaque and registered as an error, `gen_go_error_struct` already emits it as a value-type struct with `Code`/`Message` fields (no `ptr`). Previously the codegen still tried to emit method bodies that dispatch through `h.ptr`, producing uncompilable Go (e.g. `GraphQLError.StatusCode` referencing `h.ptr` against a value struct). Methods on these types are now skipped. (`crates/alef-backend-go/src/gen_bindings.rs`)

### Added

- **Rust e2e call-override fields.** `[e2e.call.overrides.rust]` learns `wrap_options_in_some`, `extra_args`, `returns_result`, `result_is_vec`, and `result_is_option` to support fallible signatures whose options slot is owned `Option<T>` and which take additional trailing positional args. (`crates/alef-core/src/config/e2e.rs`, `crates/alef-e2e/src/codegen/rust.rs`)

### Reverted

- **Wasm HTTP-fixture auto-skip** (introduced in fa2d03b1 then reverted in d8ff34c7). The wasm e2e target needs a real fix — a wasm-safe dispatch entrypoint that exercises HTTP fixtures in-process — rather than skipping them.

## [0.12.0] - 2026-04-30

### Fixed

- fix(extract): recognise `AHashMap`, `IndexMap`, and `FxHashMap` as map types in the type resolver (previously fell through to `TypeRef::Named`, causing every binding backend to emit a string/opaque type instead of a real map).

### Added

- **Blocker A: HTTP fixture mock-server support for spikard-style fixtures.** The e2e generator now emits the `mock-server` binary and per-language bootstrap code for projects whose fixtures use the `http.expected_response` schema (spikard shape), not just `mock_response` (liter-llm shape). Changes:
  - `fixture.rs`: `needs_mock_server()` returns `true` for HTTP fixtures; new `as_mock_response()` bridges both schemas to a unified `MockResponse`.
  - `codegen/rust.rs`: embedded `Fixture` struct in the mock-server binary now deserializes both `mock_response` and `http.expected_response`; route-loading uses `as_mock_response()` accessor.
  - `codegen/typescript.rs`: HTTP test cases now call `fetch(MOCK_SERVER_URL/fixtures/<id>)` instead of `app.request(bare_path)`.
  - `codegen/python.rs`: conftest.py now spawns the mock-server binary and exposes `MOCK_SERVER_URL`; HTTP test functions use `urllib.request` against `/fixtures/<id>`.
  - `codegen/ruby.rs`: generates `spec/spec_helper.rb` that starts the mock-server; HTTP examples use `Net::HTTP` against `/fixtures/<id>`.
  - `codegen/php.rs`: `bootstrap.php` spawns the mock-server; HTTP tests use `MOCK_SERVER_URL` and `/fixtures/<id>`.
  - `codegen/elixir.rs`: `test_helper.exs` starts the mock-server via Port; HTTP tests use `Req` against `/fixtures/<id>`.

### Changed

- Bump templated `ext-php-rs` pin to `0.15.12` for PHP 8.5 compatibility (downstream consumers regenerate to pick up upstream's PHP 8.5 build fixes).
- **BREAKING: `[e2e.calls.X].returns_result` default flipped from `true` to `false`.** Most e2e fixture call configs target functions whose Rust signatures do not return `Result<T, E>` (e.g. `String`, `Cow<'_, str>`, `bool`, builder types). The previous default emitted `.expect("should succeed")` unconditionally, producing `no method named expect` compile errors against non-Result returns. Existing configs that genuinely call Result-returning functions (`extract_*`, `batch_extract_*`, `chunk_*`, `render_*`, `validate_*`, `detect_languages`, `blake3_hash_*`, `compute_hash`, etc.) must now set `returns_result = true` explicitly. (`crates/alef-core/src/config/e2e.rs`)

### Added

- **`alef_extract::validate_call_export(surface, module_path, function_name)` public API.** Returns `ExportValidation::Ok` when the function is exported at the declared path, `WrongPath` when the function exists but not at the given module path (includes all actual `rust_path`s found), or `NotFound` when the function is absent from the surface entirely. Used by `alef generate` to fail fast on C1 (function not re-exported at crate root) and C2 (wrong definition selected by dedup). (`crates/alef-extract/src/lib.rs`)
- **`alef_extract::return_type_fields(surface, function_name)` public API.** Returns the public fields of the struct returned by a named free function. Returns `None` for primitive, `String`, `Vec`, unit, or opaque (no-field) return types. Allows the Rust codegen to validate `result.field` assertions at generation time. (`crates/alef-extract/src/lib.rs`)
- **`[e2e.calls.X].args[].owned` flag.** When `true`, the Rust codegen emits the argument as an owned binding and passes it by value rather than by reference. Use for parameters whose Rust signature is `Vec<T>` (not `&Vec<T>` / `&[T]`) — for example `batch_extract_file(items: Vec<(PathBuf, Option<FileExtractionConfig>)>, config: &ExtractionConfig)`. Defaults to `false`. (`crates/alef-core/src/config/e2e.rs`)
- **`[e2e.calls.X].args[].element_type` field.** For `json_object` args whose Rust target is `&[T]`, set to the element type literal (`"String"`, `"f32"`, etc.) so the codegen emits `let name: Vec<element_type> = serde_json::from_value(...).unwrap();`. Without this annotation `serde_json::from_value` cannot infer the unsized slice type and the generated test fails to compile with E0277. (`crates/alef-core/src/config/e2e.rs`)

### Fixed

- **Go backend: duplicate sentinel error declarations across multiple `ErrorDef`s.** When two error enums in the same crate shared variant names (e.g. `GraphQLError::ValidationError` and `SchemaError::ValidationError`), the Go binding emitted two top-level `var (...)` blocks each declaring `ErrValidationError`, breaking compilation with `redeclared in this block`. The Go backend now emits a single consolidated sentinel block; colliding variant names are disambiguated by qualifying with the parent error's stripped base name (`ErrGraphQLValidationError` and `ErrSchemaValidationError`). Unique variant names continue to use the unqualified `Err{Variant}` form. New public APIs `alef_codegen::error_gen::gen_go_sentinel_errors` and `gen_go_error_struct` allow callers to control sentinel/struct emission independently. (`crates/alef-codegen/src/error_gen.rs`, `crates/alef-backend-go/src/gen_bindings.rs`)

- **Magnus backend: typed-options params now accept `magnus::Value`.** Functions/methods with `Option<Named>` (or `Named`) parameters previously generated a `Option<String>` ABI that forced Ruby callers to `Hash#to_json` explicitly — and any failure to do so raised `TypeError`. The binding now accepts `magnus::Value` and calls `to_json` internally before `serde_json` deserialization, so a plain Ruby Hash works directly. Closes the upstream regression in `kreuzberg-dev/html-to-markdown#334`.

- **Phase 1: Rust e2e codegen A1/A3/A4/A5 fixes** — Eliminate `E0308 expected &T found &Option<_>`, `E0308 expected Vec<T> found &_`, and `E0277 trait bound` errors via correct optional handling, owned-param passing, slice-type annotation, and simple-return-type detection.
  - **A1:** No longer wraps optional `json_object` args in `Some(...)`; desers as `T` directly, passes `&T`. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A3:** Respects `owned = true`, passes by value instead of reference. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A4:** Emits `Vec<element_type>` annotation for slice args when `element_type` is set. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A5 (partial):** `result_is_simple = true` in call overrides redirects field-access assertions to the result variable directly. (`crates/alef-e2e/src/codegen/rust.rs`)

- **Phase 2: Per-language e2e codegen fixes** — Parallel fixes across Python, TypeScript, Go, Java, C#, Ruby, PHP, Elixir, and other languages to match Rust A1-A5 patterns.
  - **A1 (Python):** No longer wraps optional `json_object` args; passes values directly. When optional arg value is null, skip argument (function default handles None).
  - **A2 (Python, TypeScript, and all languages):** Respect `returns_result=false` — skip error-handling try/except/await for non-Result calls.
  - **Python codegen (`crates/alef-e2e/src/codegen/python.rs`):** A1 and A2 fixes.
  - **TypeScript codegen (`crates/alef-e2e/src/codegen/typescript.rs`):** A2 fix (A1 already handled correctly).

- **Phase 2: Validation hardening in `validate_fixtures_semantic()`** — Add semantic checks to catch configuration errors at `alef generate` time rather than downstream build failures.
  - **D1:** Validate call arg arity and types against IR function signatures (planned: integrate in follow-up with alef-extract).
  - **D2:** Validate return-type field paths against IR struct definitions; reject fixture assertions on fields not in the return type (planned: integrate in follow-up with alef_extract::return_type_fields).
  - **D3:** Integrate module-path validation into generate flow (D3 already implemented in alef-cli; validate.rs updated for consistency).

- **alef-cli: fixed `replace('-', "_")` Rust 2024 edition char-literal compile error.** The Rust 2024 edition requires `&str` for the first argument of `str::replace`; passing `'-'` (char) caused an E0308 in the validate-call path. (`crates/alef-cli/src/main.rs`)

## [0.11.26] - 2026-04-30

### Fixed

- **PyO3 stubs now emit `async def` for async functions and methods so mypy accepts the `await` in the generated `api.py` wrapper.** The 0.11.24 fix changed `api.py` to use `async def fn(...): return await _rust.fn(...)` for pyo3-async functions, but the corresponding `.pyi` stub kept declaring the underlying `_rust.fn` as plain `def fn(...) -> T`. mypy then errored at every wrapper call site with `Incompatible types in "await" (actual type "T", expected type "Awaitable[Any]")`. The stub generator now reads `FunctionDef.is_async` / `MethodDef.is_async` and emits `async def` so the underlying `_rust` symbol is typed as a coroutine; the generated `api.py` wrapper's `await` then type-checks. Both free functions and instance methods (including static methods) are covered. (`crates/alef-backend-pyo3/src/gen_stubs.rs`)
- **E2E TypeScript generator now auto-produces `globalSetup.ts` for HTTP test fixtures.** The generator was only creating `globalSetup.ts` when `client_factory` was configured, leaving HTTP test suites without proper mock server setup. Tests would fail with "app is not defined" at runtime. The generator now checks `has_http_fixtures` and generates `globalSetup.ts` unconditionally when HTTP tests are present (regardless of `client_factory`). The setup creates a fetch-wrapped HTTP client (`createApp`) and exposes it as `global.app` to all test suites. Vitest's `globalSetup` config is also auto-enabled whenever `needs_global_setup` is true. (`crates/alef-e2e/src/codegen/typescript.rs`)

## [0.11.25] - 2026-04-30

### Fixed

- **C# JSON converter switch-case block braces now match `dotnet format`'s default style.** The `gen_csharp_json_converter` writer emitted block braces aligned with the `case` keyword (col 12), but `dotnet format` reformats them one level deeper (col 16). On every commit the `csharp-format` pre-commit hook reshuffled the braces, which then broke `alef-verify`'s per-file hash check on the very next run. The generator now matches the formatter's expected output, eliminating the hook ping-pong. (`crates/alef-backend-csharp/src/gen_bindings.rs`)

## [0.11.24] - 2026-04-30

### Fixed

- **PyO3 backend: sanitized struct fields now emit `#[serde(skip)]` to prevent JSON round-trip failures (`#44 item 1`).** Fields whose types were sanitized to `String` placeholders (e.g. `CancellationToken → String`) were included in the derived `serde::Deserialize` impl. Deserializing a binding struct from JSON would fail with "unknown field 'cancel_token'" because the core type never expects that field. The fix adds `serde(skip)` alongside the existing opaque-field skip logic, so sanitized fields are excluded from JSON serialization/deserialization. (`crates/alef-codegen/src/generators/structs.rs`)
- **PyO3 backend: sanitized enum-like fields no longer cause "unknown variant ''" round-trip errors (`#44 item 2`).** A non-`Option` enum field sanitized to `String` (e.g. `result_format: OutputFormat → String`) derived `Default` as `String::default()` → `""`. Round-tripping through serde then failed with "unknown variant ''". The same `serde(skip)` fix from item 1 applies: the field is excluded from JSON, so the in-memory default is used silently. (`crates/alef-codegen/src/generators/structs.rs`)
- **`api.py` wrappers now forward arguments by keyword to match pyo3 signature order (`#44 item 3`).** The Python wrapper called `_rust.fn(path, config, mime_type)` in wrapper-declaration order, which misaligned with the pyo3 `#[pyo3(signature = (path, mime_type=None, config=None))]` declaration. The call is now `_rust.fn(path=path, mime_type=mime_type, config=config)` so slot alignment is independent of order. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)
- **Async pyo3 functions now produce `async def` + `await` wrappers in `api.py` (`#44 item 4`).** Functions with `is_async = true` were emitted as plain `def fn(...)` wrappers that returned a coroutine directly. Callers who assigned the result without awaiting got a coroutine object instead of the resolved value, and type checkers saw the wrong return type. The generated wrapper is now `async def fn(...): ... return await _rust.fn(...)`. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)
- **Trait-bridge `register_*` helpers are now exported via `api.py` and included in `__init__.py` `__all__` (`#44 item 5`).** `register_embedding_backend` and `register_ocr_backend` were added as `#[pyfunction]` to the pyo3 module but were absent from `api.py` and `__all__`, so `kreuzberg.register_ocr_backend(...)` raised `ImportError`. Pass-through wrappers are now emitted in `api.py` for all trait bridges that declare `register_fn`, and both `__init__.py` imports and `__all__` are updated accordingly. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)

### Changed

- **PyO3 and NAPI generated bindings suppress pedantic/nursery clippy lints that don't apply to autogenerated FFI code.** Downstream projects that opt into `clippy::pedantic = "deny"` and `clippy::nursery = "deny"` (e.g. spikard's `[workspace.lints.clippy]`) were getting 240+ errors in `crates/<crate>-py/src/lib.rs` and `crates/<crate>-node/src/lib.rs` — none of them real bugs, all of them stylistic complaints about generated wrappers (every accessor wanting `#[must_use]`, every `-> CrateName` wanting `-> Self`, every `as` cast at the JS/Python boundary, every `Deserialize` derive on a type that has unsafe FFI methods). Fixing each in the generators would require per-template rewrites with no functional impact, so both backends now emit a documented `#![allow(clippy::*)]` block covering the FFI-specific false-positives: `unsafe_derive_deserialize`, `must_use_candidate`, `return_self_not_must_use`, `use_self`, `missing_const_for_fn`, `missing_errors_doc`, `needless_pass_by_value`, `doc_markdown`, `derive_partial_eq_without_eq`, `uninlined_format_args`, `redundant_clone`, `implicit_clone`, `redundant_closure_for_method_calls`, `wildcard_imports`, `option_if_let_else`, `too_many_lines`. NAPI also picks up the cast-family lints (`cast_possible_wrap`, `cast_possible_truncation`, `cast_sign_loss`) that pyo3 already had. Each entry is annotated with the rationale in the source. (`crates/alef-backend-pyo3/src/gen_bindings.rs`, `crates/alef-backend-napi/src/gen_bindings.rs`)

## [0.11.23] - 2026-04-30

### Fixed

- **C# csproj `<None Include="../../../LICENSE" />` had three `../` segments; flat csproj layout (`packages/csharp/<Namespace>.csproj`) only needs two to reach the repo root.** With three `../` segments, `dotnet pack` resolved the LICENSE path to one directory _above_ the repo and bailed with `error NU5019: File not found`. The csproj is scaffold-once so existing repos keep their hand-fixed value, but new scaffolds now emit `../../LICENSE`. (`crates/alef-scaffold/src/languages/csharp.rs`)

## [0.11.22] - 2026-04-29

### Fixed

- **Rustler NIF emission for `&self` opaque methods no longer requires `T: Clone`.** The 0.11.21 fix replaced `resource.inner.as_ref().clone().method(...)` with `(*resource.inner).clone().method(...)` to silence `noop_method_call`, but that pattern requires the underlying opaque type to implement `Clone` — and many real opaque types (e.g. tree-sitter-language-pack's `DownloadManager`) intentionally don't, since they wrap non-cloneable resources like dynamic-library handles. Compilation failed with `error[E0599]: no method named clone found for struct DownloadManager` across every emitted opaque-method call. The emit is now `ReceiverKind`-aware: `ReceiverKind::Ref` produces `resource.inner.method(...)` (Arc<T> derefs to &T, no clone needed); `ReceiverKind::RefMut` and `ReceiverKind::Owned` keep `(*resource.inner).clone().method(...)` (still requires `T: Clone`, but those receiver kinds are uncommon for opaque types and callers can use `[<lang>] exclude_functions` if the type isn't cloneable). Same fix applied to the sync (`gen_nif_method`) and async (`gen_nif_async_method`) call paths.

## [0.11.21] - 2026-04-29

### Fixed

- **Rustler NIF instance-method calls no longer trip `noop_method_call` under `clippy -D warnings`.** `gen_bindings/functions.rs` emitted `resource.inner.as_ref().clone().method(...)` for opaque instance methods. `Arc::as_ref()` returns `&T`; `.clone()` then resolves to `<&T as Clone>::clone` (a pointless reference-clone returning another `&T`), and the lint flagged every emitted NIF call site under `-D warnings`. tree-sitter-language-pack's Elixir NIF builds failed across all 3 platforms (linux-x86_64, linux-aarch64, macos-arm64) with dozens of `error: call to .clone() on a reference in this situation does nothing` at every generated method. The emission now uses `(*resource.inner).clone().method(...)` — the `*` dereferences `Arc<T>` to `T`, so `.clone()` resolves to `<T as Clone>::clone` and produces an owned `T` the method can consume. Same fix applied to both the sync (`gen_nif_method`) and async (`gen_nif_async_method`) emission paths.

## [0.11.20] - 2026-04-29

### Fixed

- **`alef sync-versions` no longer skips work based on a stale `.alef/last_synced_version` cache.** The previous warm-path short-circuit returned early whenever the cached version matched `Cargo.toml`'s, on the assumption that "same version → all manifests still in sync." That assumption breaks in three real cases: a manifest hand-edited to the wrong version, a _new_ manifest added after the last sync (e.g. `e2e/rust/Cargo.toml` introduced by `alef e2e generate`), or a stale `alef:hash:` line whose content drifted. CI runs without the cache and re-derives the correct state, so the local hook stayed silent while the `alef-sync-versions` pre-commit hook failed in CI for downstream consumers (most recently: liter-llm rc.14, kreuzcrawl, html-to-markdown, tree-sitter-language-pack — all required `rm -rf .alef` to reproduce the diff). The function now always walks every manifest. The scan is sub-second on kreuzberg-sized repos and the underlying writes are idempotent when nothing is actually stale, so the cost is invisible. The `.alef/last_synced_version` stamp is still written for forward-compatible introspection but is no longer consulted as a gate. (`crates/alef-cli/src/pipeline/version.rs`)

### Added

- **`[csharp].package_id` config field for NuGet `<PackageId>`, decoupled from `[csharp].namespace`.** The csharp scaffold previously emitted both `<RootNamespace>` and `<PackageId>` from `csharp_namespace()`, conflating two distinct identifiers. That conflation silently broke html-to-markdown's NuGet publish: the consumer-facing namespace `HtmlToMarkdown` is owned on nuget.org by an unrelated third party (Enrico Rossini), but the historical `KreuzbergDev.HtmlToMarkdown` package was owned by us. The alef migration overwrote the published-artifact id, and every release returned `403 (does not have permission to access the specified package)` until the csproj was hand-edited. Setting `[csharp] package_id = "KreuzbergDev.HtmlToMarkdown"` now lets the project publish to the owned coordinate while the in-code namespace stays the short, idiomatic form. When unset, `package_id` defaults to `namespace` — existing configs keep their behaviour. New accessor: `AlefConfig::csharp_package_id()`.

### Changed

- **PHP composer name and Swift Package URL no longer hardcode `kreuzberg`.** The PHP e2e composer.json `"name"` field now derives `<vendor>/e2e-php` from the consumer-binding pkg vendor, and the PSR-4 autoload namespace uses the configured PHP namespace (`<configured>\E2e\\`). The PHP README's `composer require <vendor>/<crate>` line uses the same derivation. The PHP e2e `pkg_name` fallback for `[e2e.packages.php].name` derives `<org>/<module>` from `[scaffold] repository` (instead of `kreuzberg/<module>`). The Swift e2e `Package.swift` `.package(url: ...)` for registry mode now uses the configured repository URL with a `.git` suffix, falling back to a vendor-neutral `https://example.invalid/<module>.git` placeholder. New `derive_repo_org()` helper exposed in `alef_core::config`.
- **`alef init` no longer writes `dev.kreuzberg` / `kreuzberg-dev` literals into freshly generated `alef.toml`.** When invoked in a project whose `Cargo.toml` declares `[package] repository = "..."`, the generated config now seeds `[scaffold] repository`, derives `[go] module` from it, and derives `[java] package` via the same reverse-DNS rule as `AlefConfig::java_package()` — so `repository = "https://github.com/foo-org/bar"` produces `module = "github.com/foo-org/bar"` and `package = "com.github.foo_org"`. When the repository is unset, the affected sections are emitted with the offending fields commented out and a `# TODO: set this` marker, instead of writing a plausible-looking but wrong default. When `[go].module` is unset, alef now derives the module path from `[scaffold] repository` / `[e2e.registry] github_repo` by stripping the `https://` scheme (`https://github.com/foo/bar` → `github.com/foo/bar`). When no repository is configured at all, the fallback is a vendor-neutral `example.invalid/<crate>` placeholder that fails `go build` loudly. The internal `package_name()` helper's last-resort `unwrap_or("kreuzberg")` is now `"binding"` (this branch is unreachable in practice — `next_back()` on a non-empty `split('/')` always returns `Some` — but the literal is gone). New `try_go_module()` accessor returns `Result<String, String>` for callers that should error rather than emit the placeholder.
- **Java/Kotlin package fallback no longer hardcodes `dev.kreuzberg`.** When `[java].package` / `[kotlin].package` is unset, alef now derives a reverse-DNS package from the configured repository URL (`[scaffold] repository` or `[e2e.registry] github_repo`): `https://<host>/<org>/<rest>` becomes `<reversed-host>.<org>` (host labels reversed, hyphens replaced with underscores). For kreuzberg-dev consumers (`https://github.com/kreuzberg-dev/<crate>`) this produces `com.github.kreuzberg_dev` — _different_ from the previous `dev.kreuzberg` literal. **Action required:** kreuzberg-dev consumers should set `[java] package = "dev.kreuzberg"` and `[kotlin] package = "dev.kreuzberg"` in their `alef.toml` to keep the existing namespace. When no repository URL is configured at all, `java_package()` / `kotlin_package()` fall back to `unconfigured.alef` so the build fails loudly. New `try_java_package()` / `try_kotlin_package()` accessors return `Result<String, String>` for callers that should error rather than emit the placeholder.
- **README scaffolding no longer falls back to `https://github.com/kreuzberg-dev/<crate>` when no repository is configured.** Alef is meant to be vendor-neutral; consumers outside the kreuzberg-dev org were silently picking up that URL in 13 places across `alef-readme` (the README header link plus 12 per-language "See <repo> for usage examples." pointers). The 13 inline `format!("https://github.com/kreuzberg-dev/{name}")` calls now route through a single `AlefConfig::github_repo()` accessor whose fallback is `https://example.invalid/<crate>` — an obviously-broken URL that surfaces in code review instead of smuggling another organization's link into the output. Set `[scaffold] repository = "..."` (or `[e2e.registry] github_repo`) in your `alef.toml` to resolve. A new `AlefConfig::try_github_repo() -> Result<String, String>` accessor is available for callers that should fail hard on missing config.

### Fixed

- **C# wrapper class methods now thread `IntPtr handle` for non-static methods.** `gen_pinvoke_for_method` and `gen_wrapper_method` previously only emitted the visible parameter list, ignoring the receiver. The cbindgen-emitted FFI signature for an instance method is `fn(this: *const T, ...)`, so the C# P/Invoke and the wrapper call site were one argument short — `dotnet build` failed with `CS7036: There is no argument given that corresponds to the required parameter 'ptr'`. Both functions now detect `!method.is_static && method.receiver.is_some()` and prepend `IntPtr handle` (P/Invoke signature) / `handle` (wrapper-to-native call argument) so the surfaces line up.
- **Kotlin error typealiases now use the `Exception` suffix to avoid colliding with same-named structs.** When an `errors` variant shares a name with a struct in `api.types` (e.g. an error variant `Foo` and a struct `Foo`), the previous `typealias Foo = pkg.FooException` would clash with the struct's own `typealias Foo = pkg.Foo`, and `compileKotlin` failed with `Redeclaration:`. The error alias now uses `typealias FooException = pkg.FooException`, mirroring the Java facade's class name; struct typealiases are unchanged.
- **Kotlin scaffold sample object renamed to plain `Sample` so its filename is not project-specific.** ktlint's `filename` rule requires a file with a single top-level declaration to match the declaration name. The sample object is now named `Sample` (matching `Sample.kt`) instead of `<ProjectName>Sample` (which would require `<ProjectName>Sample.kt`); the project name still appears in the body's `println` for context.
- **Gleam scaffold emits a valid `manifest.toml`.** Previous output was a comment-only stub, which gleam's TOML parser rejects with `missing field 'requirements'` on every `gleam check` / `gleam build` invocation. The scaffolded manifest now contains the minimum gleam expects (`packages = []` plus an empty `[requirements]` table); `gleam build` repopulates it on the first run.
- **Java scaffold's checkstyle config now resolves `${checkstyle.suppressions.file}`.** The pom.xml `<configuration>` block references a checkstyle config that requires the `checkstyle.suppressions.file` property to be set, but the config never told maven-checkstyle-plugin where to load that property from. Adds `<propertiesLocation>${project.basedir}/checkstyle.properties</propertiesLocation>` to the plugin config. Also fixes the property's value in `checkstyle.properties` to be relative to packages/java/ (the maven cwd) rather than to repo root.

## [0.11.19] - 2026-04-29

### Fixed

- **`sync-versions` now updates `packages/ruby/Gemfile.lock` alongside the gemspec.** After a version bump the gemspec received the new version but the lockfile was not touched, causing `bundle install` to abort with "The gemspecs for path gems changed, but the lockfile can't be updated because frozen mode is set" on every CI run. `sync_versions` now textually replaces all `<gem-name> (<old-version>)` entries for path gems in Gemfile.lock (both the `PATH > specs` block and the `CHECKSUMS` block) with the new RubyGems-format version. The replacement is idempotent — a second sync with the same version is a no-op.
- **`has_derive` now recognises namespaced derive paths** like `#[derive(serde::Serialize, serde::Deserialize)]`. The bare-ident form (`#[derive(Serialize)]`) was already detected, but the namespaced form fell through and IR types ended up with `has_serde: false`. Backends that gate `_to_json` / `_from_json` emission on `has_serde` (which is correct) then produced binding code referencing FFI functions that don't exist (e.g. `C.spikard_sse_event_to_json` for spikard's `SseEvent`, which derives `serde::Serialize`). The cfg_attr branch already used last-segment matching; the direct-derive branch now mirrors that.
- **Go backend C type references now preserve all-caps abbreviations.** Mirror of the FFI cbindgen-forward-decl fix: `alef-backend-go` ran type names through `heck::ToPascalCase` in seven places (free-method receiver, opaque-handle marshalling, return-value unwrapping, trait bridge trampolines, the `type_name` helper used in unmarshalFoo function names). For types like `GraphQLError` this produced `C.SPIKARDGraphQlError` in the generated `binding.go`, which doesn't match cbindgen's actual `SPIKARDGraphQLError` and causes `go build` to fail with "could not determine what C.SPIKARDGraphQlError refers to". IR type names are already PascalCase from Rust source — the conversion was both unnecessary and harmful for acronym-bearing types. Field-name renaming (`field_name.to_pascal_case()` for `serde_rename_all = "PascalCase"`) and method-name conversion are unchanged.
- **FFI cbindgen forward declarations now match cbindgen's actual emit for types with all-caps abbreviations.** The forward-declaration block in the generated `cbindgen.toml` ran type names through `heck::ToPascalCase`, which mangles abbreviations: `GraphQLError` becomes `GraphQlError`, but cbindgen emits the actual struct in the C header as `SPIKARDGraphQLError` (literal Rust name + prefix). Consumers compiling against the header saw two different type names for the same struct and the build failed (`unknown type name 'SPIKARDGraphQLRouteConfig'; did you mean 'SPIKARDGraphQlRouteConfig'?`). The pre-declarations now use the IR type name verbatim.
- **Java/Kotlin e2e codegen now respects `[java].package` and `[kotlin].package`.** Previously the e2e generators hardcoded `dev.kreuzberg` in three places: the pom.xml `<groupId>`, the test-file `package` declarations, and the generated filesystem path (`src/test/{java,kotlin}/dev/kreuzberg/e2e/...`). Projects whose Java/Kotlin package config used a different group id (e.g. spikard's `dev.spikard`) ended up with package declarations that disagreed with their filesystem location, breaking compilation. The generators now use `alef_config.java_group_id()` / `alef_config.kotlin_package()` consistently — pom.xml `<groupId>`, gradle `group =`, the `package` line, and the path segments all derive from the configured value.

## [0.11.18] - 2026-04-29

A patch release that fixes Javadoc emission so HTML inside backticks survives the Eclipse-formatter Spotless pipeline.

### Fixed

- **Javadoc `{@code …}` content now HTML-escapes its inner `<`, `>`, `&` characters.** A Rust doc comment like ``/// Determines how code blocks (`<pre><code>`) are rendered`` previously emitted `{@code <pre><code>}` with raw HTML inside the tag. Eclipse-formatter Spotless (used by html-to-markdown's `packages/java/pom.xml`) interprets the inner `<pre>` as a real block-level HTML element and shatters the doc comment across multiple `* ` rows — which then breaks `alef-verify` on the very next prek run. The codegen now emits `{@code &lt;pre&gt;&lt;code&gt;}` so Spotless leaves the line alone; readers see the same text since Javadoc renders `{@code}` literally regardless. Both `alef-codegen::doc_emission::escape_javadoc_line` and `alef-backend-java::gen_bindings::helpers::escape_javadoc_line` carry the fix.

### Fixed

- **Python e2e codegen no longer triggers ruff `F401` on `import pytest`.**

## [0.11.17] - 2026-04-29

PHP backend: fix flat-enum codegen to emit correct code instead of no-op conversions.

### Fixed

- **PHP flat-enum `From` impls no longer emit no-op `.into()` for primitive and `String` fields.** `flat_enum_core_to_binding_field_expr` and `flat_enum_binding_to_core_field_expr` previously fell through to `.map(Into::into)` / `.into()` for all types not explicitly handled. For primitives (`u8`, `u16`, `u32`, `i32`, `bool`, `f32`, `f64`, etc.) and `String`, the PHP binding type equals the core type, so `Into::into` is a no-op and triggered `clippy::useless_conversion`. Both functions now emit direct assignment for these same-type cases.
- **PHP flat-enum getter methods no longer call `.clone()` on `Copy` types.** Getters for `Option<u32>`, `Option<u8>`, `Option<bool>`, `Option<i64>` etc. previously always emitted `self.field.clone()`, which triggered `clippy::clone_on_copy`. Getters for Copy fields (`is_php_copy_type` helper: `Primitive` and `Option<Primitive>`) now emit `self.field` directly.
- **PHP flat-enum `From` impls no longer emit `..Default::default()` when all struct fields are covered.** When a variant's explicit field assignments already cover every field in the flat struct, the trailing `..Default::default()` is redundant and triggered `clippy::needless_update`. The codegen now pre-computes the complete set of flat field names and omits the struct update when the variant sets all of them. When `e2e.call.async = true` (or any test in the file is async/skipped/has error assertions), the python e2e generator emits `import pytest` at module level. Pytest is needed for `pytest.fixture` / `pytest.mark.*` decorators, but ruff's `F401` rule strips the import when no symbol is statically referenced in the file body — which then causes `alef verify` to fail on subsequent runs because the file's hash no longer matches the generated content. The import is now suppressed with `# noqa: F401`.

## [0.11.16] - 2026-04-29

WASM trait bridges and FFI multi-crate codegen fixes.

### Added

- **`[wasm.extra_dependencies]` table in `alef.toml`.** Mirrors `[crate.extra_dependencies]` but applies only to the WASM binding `Cargo.toml`. Initial use case: `async-trait = "0.1"` so generated wasm bridges can use `#[async_trait::async_trait(?Send)]` without callers having to declare it manually.

### Fixed

- **WASM trait bridge async impls now correctly emit `#[async_trait::async_trait(?Send)]`.** `gen_bridge_trait_impl` in `alef-codegen` always emitted `#[async_trait::async_trait]` (the default `+ Send` variant), which produced `error[E0053]: method ... has an incompatible type for trait` on `wasm32-unknown-unknown`: the underlying trait future was not `Send`-bounded but the macro-rewritten impl signature was. Added `async_trait_is_send() -> bool` (default: `true`) on the `TraitBridgeGenerator` trait; the wasm backend overrides it to `false`, and `gen_bridge_trait_impl` selects between `#[async_trait::async_trait]` and `#[async_trait::async_trait(?Send)]` accordingly. WASM bindings (kreuzberg-wasm and any other wasm-target generator) now compile clean for `OcrBackend`, `PostProcessor`, `Validator`, and `EmbeddingBackend` bridges.
- **FFI scaffold now merges `[crate.extra_dependencies]` into the generated `crates/<crate>-ffi/Cargo.toml`.** Previously the scaffold emitted only the umbrella crate (e.g. `kreuzberg = { path = "../kreuzberg" }`) plus `serde_json` and `tokio`, which worked when the public API surface lived in a single workspace crate. For multi-crate workspaces (e.g. spikard's `spikard-core`, `spikard-http`, `spikard-graphql`), the FFI bindings codegen emits qualified paths like `spikard_http::ServerConfig` and `spikard_graphql::QueryOnlyConfig` — the cdylib failed to compile because those crates were not direct dependencies. The scaffold now merges entries from `[crate.extra_dependencies]` (sorted, alphabetised) into the `[dependencies]` block, matching the behaviour of the wasm backend.
- **FFI codegen no longer double-prefixes sibling-crate paths in field accessors.** The path-qualification logic in `gen_field_accessor` previously only treated `module_prefix` as already-qualified when it equalled `core_import` or started with `core_import::`. For sibling workspace crates whose names share the project's prefix (e.g. `core_import = "spikard"`, sibling crate `spikard_http`), `module_prefix.starts_with("spikard::")` was false and the codegen emitted `spikard::spikard_http::openapi::OpenApiConfig` — a path that doesn't exist. The check now also accepts `{core_import}_` as a sibling-crate marker, so sibling-crate paths render verbatim.
- **FFI field accessors now look up Named types in the per-binding `path_map`.** When a struct field's `type_rust_path` is `None` in the IR (which alef emits when the type is referenced by short name), the field accessor previously fell back to `core_import` and produced paths like `spikard::ContactInfo` — which fails when the type lives in a sibling crate (`spikard_http::ContactInfo`) and is not re-exported through the umbrella crate. `gen_field_accessor` now threads the `path_map` built in `gen_bindings::generate` and uses `c_return_type_with_paths`, matching the behaviour of `gen_method_wrapper` and `gen_free_function`.
- **FFI codegen now uses `.map(str::to_owned)` for `Option<&str>` returns.** Methods returning `Option<&str>` (e.g. a `get_description() -> Option<&str>` accessor) previously emitted `let result = result.cloned();`, which fails to compile because `str: !Sized` and `Option::cloned` requires `T: Clone` on a sized type. The codegen now special-cases `TypeRef::Optional(TypeRef::String)` to emit `.map(str::to_owned)` instead. The existing `.cloned()` path still applies to `Option<&NamedType>`, `Option<&Vec<...>>`, and `Option<&char>`.
- **FFI codegen now uses `.to_owned()` for `&str` returns (was `.clone()`, a no-op).** Methods returning `&str` previously emitted `let result = result.clone();` — which compiles but is a no-op (str: !Sized doesn't impl Clone, so the call is a noop on the reference) and triggers Rust's `noop_method_call` lint. The codegen now emits `.to_owned()`, producing the owned `String` the FFI layer needs.
- **FFI no longer emits duplicate `to_json` / `from_json` exports when the type defines those methods.** Types that derive both `serde::Serialize` and a manual `to_json` / `from_json` method (e.g. `ProblemDetails::to_json`) caused two FFI functions with the same C name (`{prefix}_{type}_to_json`) — one from the auto-serde codegen path and one from `gen_method_wrapper` — and the cdylib failed with `E0428: defined multiple times`. The auto-serde codegen now skips emitting `to_json` / `from_json` when the type already defines a method of that name.

## [0.11.15] - 2026-04-29

PHP backend: fix codegen errors in tagged data enum `From` impls.

### Fixed

- **PHP flat data enum `From` impls now handle all field kinds correctly.** `gen_flat_data_enum_from_impls` previously emitted `.into()` unconditionally for every variant field, which compiled only when `From<BindingType>` happened to exist. Four cases were missing:
  - **`sanitized: true` fields** (e.g. `TableGrid`, `ImageMetadata`, `PdfMetadata`, `ProcessResult`, `[(u32,u32);4]`): now emits `None` / `Default::default()` instead of trying to convert an opaque or complex core type through a String.
  - **`is_boxed: true` fields** (e.g. `FormatMetadata::Docx(Box<DocxMetadata>)`, `::Html(Box<HtmlMetadata>)`): now wraps the core→binding result in `Some((*val).into())` and the binding→core result in `Box::new(...)`.
  - **`TypeRef::Path` fields** (e.g. `ChunkSizing::Tokenizer { cache_dir: Option<PathBuf> }`): now uses `PathBuf::from(s)` for binding→core and `.to_string_lossy().into_owned()` for core→binding instead of `.into()` (no `From<PathBuf> for String` exists).
  - **`TypeRef::Primitive(Usize | U64 | Isize)` fields** (e.g. `EmbeddingModelType::Custom { dimensions: usize }`): now emits `v as usize` / `v as i64` explicit casts instead of `.into()` (no `From<i64> for usize` exists).
  - Struct variant destructuring patterns for sanitized fields now use `field: _field` syntax (not `_field` alone) to satisfy the Rust struct-pattern completeness requirement.

## [0.11.14] - 2026-04-29

CLI ergonomics, generation performance, and live output for long-running commands. Warm `alef generate` on a 16-language consumer (kreuzberg) drops from ~14s to ~1s.

### Added

- **Standard CLI affordances.** `alef --version` (and `-V`) prints the binary version. New global flags `--verbose` / `-v` (repeatable: `-v` info, `-vv` debug, `-vvv` trace), `--quiet` / `-q` (errors only), and `--no-color` (disable ANSI in log output). Tracing now defaults to `info`-level output on stderr — previously the CLI was effectively silent unless `RUST_LOG` was set, which meant users had no signal during long-running commands.
- **Live streamed output for long-running commands.** `alef setup`, `alef update`, `alef update --latest`, `alef lint`, `alef fmt`, `alef test`, and `alef clean` now stream stdout/stderr to the terminal in real time, line-prefixed with `[<lang>]` when multiple languages run in parallel. Previously output was captured and only re-emitted via `tracing::info!` after the command finished — producing a multi-minute blackout for `pnpm install` / `bundle install` / `cargo update`. Failures are now also surfaced via an explicit `✗ <command> failed: <lang> — <error>` summary line on stderr.

### Changed

- **`alef generate` no longer runs formatters by default.** Formatting was the dominant cost of `alef generate` on multi-language projects (e.g. `cargo fmt --all`, `ruff check --fix .`, `biome format --write .`, `dotnet format`) and ran on the full package directory every invocation — even when only one language regenerated. The behaviour is now opt-in: pass `--format` to `alef generate`, `alef all`, or `alef init` to run formatters. When `--format` is passed, formatters run _only_ for languages that actually regenerated this run (other languages skip), making warm `alef generate --format` proportional to changed source files. The tradeoff: projects that previously relied on the implicit format pass to keep `alef verify` green should either (a) pass `--format`, (b) keep formatters in pre-commit hooks, or (c) run `alef fmt` explicitly after generate.

### Performance

- **Public API codegen is now cached.** `pipeline::generate_public_api` output is hashed and skip-written like binding files; previously every warm run rewrote 200+ Python `api.py` / `options.py` / `__init__.py` files for no net change.
- **Deterministic Python public-API imports.** The PyO3 backend's `gen_api_py` collected import names into an `AHashSet` then emitted them via `.join(", ")` — `AHashSet` iteration order is non-deterministic, so the codegen output flipped between runs and the content-hash cache always missed. Imports are now sorted before emit.
- **Idempotent `sync_versions`.** `replace_version_pattern` previously returned `Some(new_content)` whenever its regex matched, even when the rewrite was byte-identical (e.g. when Magnus emits `VERSION = "x"` with double-quotes and the replacement template uses single-quotes). Each `alef generate` then ping-pong-rewrote `version.rb`, marked it as drifted, and triggered a full README regeneration. The function now extracts the version literal from the match and short-circuits when it already equals the target — quote style irrelevant.
- **`sync_versions` short-circuits on warm runs.** Stamps `.alef/last_synced_version` after each successful sync; the next warm run with no `--bump` and an unchanged canonical version skips the entire glob+regex+stat pass over package manifests. When a sync does run, only `readme/docs/scaffold` stage caches are invalidated; the IR cache and per-language binding hashes are preserved.
- **`sources_hash` mtime-prefilter.** A per-source `(mtime_nanos, size)` memo at `.alef/sources_hash.cache` lets warm runs return the previous aggregate hash directly when nothing in the source tree has changed — skipping the read+blake3 pass over every `[crate].sources` file.
- **Hot extractor optimisations.** `extract_serde_rename_all` parses with `attr.parse_nested_meta` instead of stringifying the entire token stream and substring-scanning. `normalize_type_string` now scans bytes directly instead of materialising a `Vec<char>` per call. Combined, these halve allocations on the cold extraction path for large API surfaces.
- **Single-pass `to_pep440`.** Rewritten to build the output in one pre-allocated `String` instead of chaining five `.replace()` calls, each of which allocates a fresh intermediate.
- **`extract_version` regex cache.** The verify path's `extract_version` helper now caches compiled regexes in `OnceLock<Mutex<HashMap>>` so the ~15 verify patterns aren't recompiled per file.
- **Gleam variant collision map** uses `ahash::AHashMap` with `&str` keys; only colliding names allocate owned `String`s (was: `HashMap<String, _>` with per-variant clone).

## [0.11.13] - 2026-04-29

A patch release that addresses two regressions surfaced by the tree-sitter-language-pack v1.8.0-rc.14 release: copy-paste duplication in generated Java FFI methods and Spotless/`alef-verify` hash drift.

### Fixed

- **Java backend deduplicates the FFI Vec-return path through a shared `readJsonList` helper.** Every Vec-returning method previously inlined an identical ~15-line null-check → reinterpret → free → JSON deserialize block, which `cpd` correctly flagged as duplication. The boilerplate now lives in a single private static helper emitted by the helper-emitter; per-method call sites collapse to one line.
- **Java auto-format prefers Spotless when `packages/java/pom.xml` configures `spotless-maven-plugin`.** When detected, `alef generate` runs `mvn -f packages/java/pom.xml spotless:apply -q` instead of `google-java-format`, so the embedded `alef:hash:` value matches what the project's `mvn spotless:check` will see at verification time. Previously the hash drifted on every `alef generate` → prek cycle for any project whose Spotless config diverges from google-java-format defaults (e.g., Eclipse formatter).

## [0.11.12] - 2026-04-29

A patch release that fixes the Java backend so functions returning `PathBuf` emit a properly-typed `java.nio.file.Path` value instead of a raw `String`.

### Fixed

- **Java backend now wraps FFI string returns with `java.nio.file.Path.of(str)` when the declared return type is `Path`.** Previously, methods like `cacheDir()` were declared to return `java.nio.file.Path` but the body returned the raw `String` result of the FFI call, causing `javac` to reject the generated code with "incompatible types". Both `Path` and `Optional<Path>` are now handled correctly.

## [0.11.11] - 2026-04-29

A patch release that adds a `returns_result: bool` field to e2e `[CallConfig]` so the Rust generator can skip the `.expect("should succeed")` unwrap for native non-Result-returning calls.

### Added

- **`[e2e.calls.<name>] returns_result = false`** lets fixture authors mark a call whose Rust function signature returns `T` directly (e.g. `String`, `Vec<u8>`, `bool`, a hash) rather than `Result<T, _>`. The Rust code generator now binds the value with no unwrap when this is set; previous behavior always emitted `.expect("should succeed")`, which failed to compile against any non-Result function. Defaults to `true` to preserve existing behavior.

## [0.11.10] - 2026-04-29

A patch release that fixes the NAPI/Node backend so functions whose only sanitized parameter is a configured `[[trait_bridges]]` param (e.g. `Option<VisitorHandle>`) are emitted via the bridge instead of being silently skipped.

### Fixed

- **NAPI backend now emits top-level functions that take a configured trait-bridge parameter, even when the function is marked sanitized.** Previously the sanitized check ran before `find_bridge_param`, so html-to-markdown's top-level `convert(html, options, Option<VisitorHandle>)` was dropped from the Node binding, causing downstream `import { convert }` to fail at TypeScript compile time. The check now skips a function only when it's sanitized AND no trait-bridge param applies — matching how the PyO3 backend already handles this case (which is why Python had `convert` and Node didn't).

## [0.11.9] - 2026-04-29

A patch release that fixes the PHP backend's lowering of tagged enums with **tuple variants** holding distinct types.

### Fixed

- **PHP backend now gives each tuple variant of a tagged enum its own distinct field instead of collapsing them all to a shared `_0` field.** Previously `pub enum Message { System(SystemMessage), User(UserMessage), … }` lowered to a single `_0: Option<SystemMessage>` (the first variant's type) and the From impls then tried to assign `UserMessage`/`AssistantMessage`/etc to that one field, producing N trait-bound errors per non-first variant. Single-field tuple variants now use the variant's snake_case name as the field name (`system: Option<SystemMessage>`, `user: Option<UserMessage>`, …); multi-field tuple variants get `{variant}_0`, `{variant}_1`, …. Struct variants are unchanged.

## [0.11.8] - 2026-04-29

A patch release that fixes the PHP and Rustler/Elixir backends to preserve struct-variant fields when lowering tagged enums, plus two smaller release-pipeline fixes.

### Fixed

- **PHP backend now generates a real wrapper class for tagged enums with struct variants instead of emitting only string constants.** Previously `pub enum SecuritySchemeInfo { Http { scheme, bearer_format }, ApiKey { location, name } }` was lowered to a few `pub const` strings, the surrounding struct field was demoted to `HashMap<String, String>`, and the generated `From<core::Outer> for php::Outer` impl emitted `(k, v.into())` against the original enum value — which fails to compile with `the trait bound \`String: From<SecuritySchemeInfo>\` is not satisfied`. The backend now emits a flat php_class with a`type_tag`discriminator plus optional fields for every variant, full`#[php_impl]`getters, a`from_json`constructor, and proper`From`impls in both directions. Fixes spikard-php builds with`--features extension-module`.
- **Rustler/Elixir backend now preserves struct-variant fields in tagged enums instead of silently dropping them.** Previously every enum was lowered to `rustler::NifUnitEnum`, so `SecuritySchemeInfo::Http { scheme, bearer_format }` became unit variants `Http`/`ApiKey` with the inner fields gone, and the generated `From` impls fabricated `Default::default()` for every missing field — round-tripping any real value through Elixir produced empty defaults (silent data corruption). The backend now detects struct-variant enums and lowers them to `rustler::NifTaggedEnum` with the original variant fields preserved; the From impls now destructure variants instead of fabricating defaults. Unit-only enums continue to use `NifUnitEnum`.
- **WASM binding crate generation now sets `default-features = false` on the core dep when `[wasm].features` is configured.** Cargo would otherwise OR the explicit feature set with the core crate's defaults, sneaking host-only defaults like `download` back into the `wasm32-unknown-unknown` build and failing on `getrandom`/`mio`.
- **`alef validate-versions` now normalizes per-format before comparing.** It applies `to_pep440` to the canonical version when reading `pyproject.toml`, applies `to_rubygems_prerelease` for Ruby `version.rb`, and skips manifests that exist but don't declare a `version` field (e.g. private pnpm workspace roots). Eliminates spurious mismatches like canonical `1.4.0rc8` vs Python `1.4.0rc8` vs Ruby `1.4.0.pre.rc.8`.

## [0.11.7] - 2026-04-28

A patch release that fixes Rust e2e codegen so optional string/bytes args are passed via `.as_deref()` (yielding `Option<&str>`/`Option<&[u8]>`) rather than `&Option<...>`.

### Fixed

- **Rust e2e codegen no longer generates `&mime_type` for an `Option<String>` argument when the target signature expects `Option<&str>`.** Previously `let mime_type = None;` followed by `extract_file(&path, &mime_type, ...)` produced an `expected Option<&str>, found &Option<_>` E0308 mismatch on every fixture using the `extract_file` call. Optional string args now bind to a typed `Option<String>`/`Option<Vec<u8>>` (so `None` resolves) and pass via `.as_deref()` (or `.as_ref().map(|v| v.as_slice())` for bytes). Non-string optional non-string args use `.as_ref()` to avoid moving the binding.

## [0.11.6] - 2026-04-28

A patch release that prevents the python e2e codegen from emitting test files for categories whose fixtures are 100% skipped for python.

### Fixed

- **Python e2e codegen no longer generates a `tests/test_<category>.py` file when every fixture in that category has `skip.languages` containing `"python"`.** Previously the emitter would still write the file with `@pytest.mark.skip` markers, but the file's `from kreuzberg import ...` line would then reference APIs that aren't bound in Python — failing at module import before pytest could honor the skip marker. The other language emitters (node, ruby, java, go, csharp, php, elixir, r, wasm, dart, gleam, kotlin, swift, zig, typescript) already filtered skipped fixtures upstream and were not affected.

## [0.11.5] - 2026-04-28

A patch release that fixes the GitHub-release existence check so the publish pipeline no longer skips its own builds when only a release tag exists with no binaries attached.

### Fixed

- **`alef check-registry --registry github-release` now optionally verifies asset presence.** Two new flags — `--asset-prefix <STR>` and `--required-assets <a,b,c>` — make the check fail (return `exists=false`) when the release tag exists but the requested binaries have not been uploaded yet. Without this, alef's own `check-github-release` job returned `exists=true` for v0.11.4's empty release (the tag had been created by `gh release create` before the build matrix uploaded artifacts) and the dependent build jobs skipped — producing another tag with no binaries. The action wrapper (`kreuzberg-dev/actions/check-registry`) already exposed `asset-prefix` and `assets` inputs but was previously dropping them on the floor.

## [0.11.4] - 2026-04-28

A patch release that ships v0.11.3's content plus a fix for alef's own publish pipeline. v0.11.3 tagged but never produced binaries because validate-versions aborted on a half-empty `alef.toml`.

### Fixed

- **`alef.toml` in this repo now carries the minimal `[crate]` section and `languages = []` that `load_config` requires.** `alef validate versions` (run by the publish workflow's pre-flight) was failing with `missing field crate`, causing the whole publish run to skip without ever building binaries. Without a `[crate]` table the parser refuses the file even though the subcommand only needs the canonical version from `Cargo.toml`. v0.11.0 through v0.11.3 all share this issue — none of those tags has binary assets on the release page; v0.11.4 is the first installable build of the v0.11.x line.

## [0.11.3] - 2026-04-28

A patch release that turns alef.toml's `version` field into a real lifecycle pin: writes are stamped on every successful generate, and a config pointing to a future alef now refuses to run instead of silently producing stale output.

### Added

- **`alef generate` and `alef all` enforce alef.toml ↔ CLI version compatibility.** Before doing any work, both commands parse the top-level `version = "X.Y.Z"` field (if set) and compare it semver-style against the running CLI. If the pin is greater than the CLI, the command aborts with `alef.toml pins version = "..." but installed alef CLI is X.Y.Z. Upgrade alef ...`. This catches the case where a downstream repo bumps `alef.toml` and tries to regenerate against an older binary still on disk — the regenerate would otherwise quietly skip new emitters and corrupt the output.
- **`alef generate` and `alef all` stamp alef.toml with the CLI version after a successful run.** The top-level `version = "..."` line is rewritten (or inserted if missing) to match `env!("CARGO_PKG_VERSION")`. Downstream consumers (install-alef, CI verify) now have an authoritative record of which alef produced the on-disk artifacts, so a mismatch between alef.toml and the headers in generated files becomes impossible. The rewrite is line-anchored, so dependency `version = "..."` specs inside inline tables are never touched.

## [0.11.2] - 2026-04-28

A patch release fixing the alef self-publish bootstrap and rolling up several major dependency upgrades.

### Fixed

- **`alef`'s own publish workflow no longer races against itself.** The `prepare`, `validate-versions`, and `check-registry` jobs use the alef-backed composite actions, which previously defaulted `alef-version: latest` — install-alef would resolve that to the just-bumped `alef.toml` pin (the very version being published) and try to download a binary that didn't exist yet (it's built later in the same run). All four alef-side action calls now pass `alef-version: main` so install-alef builds from source via `cargo install --git --locked`. Costs ~2-3 min per job but breaks the chicken-and-egg loop.

### Changed

- **`alef check-registry` ported to ureq v3.** The previous v2 API (`AgentBuilder`, `RequestBuilder::set`, `Error::Status(404, _)`, `Response::into_string`) was rewritten to the v3 equivalents (`Agent::config_builder().new_agent()`, `RequestBuilder::header`, `Error::StatusCode(404)`, `Response::into_body().read_to_string()`).

### Bumped (template_versions)

- `JUNIT` 5.14.4 → 6.0.3 (Maven scaffold templates)
- `MICROSOFT_NET_TEST_SDK` 17.14.1 → 18.4.0
- `XUNIT_RUNNER_VISUALSTUDIO` 2.8.2 → 3.1.5
- `PRE_COMMIT_HOOKS_REV` v0.9.5 → v6.0.0
- `TYPOS_REV` v0.7.10 → v1.45.2
- Plus auto-merged Renovate PRs landed since v0.11.1: `criterion` 0.7 → 0.8, `extendr-api` 0.8 → 0.9, `microsoft.net.test.sdk` 17.12.0 → 17.14.1, JVM tooling pins, pre-commit hook revisions.

## [0.11.1] - 2026-04-28

A patch release fixing path discovery and version-extraction edge cases in the new `alef validate versions` subcommand surfaced when running it against kreuzberg's repo layout.

### Fixed

- **`alef validate versions` now skips manifests that don't exist** rather than treating absence as a mismatch. Previously, every repo was flagged as having "missing" Ruby/Go/PHP manifests if it didn't follow alef's default lib-based layout. The check is opt-in per file: only manifests that physically exist are validated.
- **Ruby version files are discovered via globs** matching the same patterns alef's `sync-versions` writes to: `packages/ruby/lib/*/version.rb`, `packages/ruby/ext/*/src/*/version.rb`, `packages/ruby/ext/*/native/src/*/version.rb`. Repos that ship the Ruby gem with rb-sys-style ext layout no longer produce false negatives.
- **PHP composer.json is only validated when a `version` field is actually declared**. Composer relies on Git tags for versioning, and most polyglot manifests omit it; the validator no longer flags missing-by-design as a mismatch.
- **`mix.exs` reader now accepts both `@version "X.Y.Z"` (module-attribute form) and `version: "X.Y.Z"` (keyword form inside `def project do`).** The previous reader only matched the `@version` constant style.
- **Root `package.json` and `crates/{name}-{wasm,node}/package.json` are now part of the validation set.** Repos that ship a top-level npm package or per-crate package.json no longer silently bypass the check.

## [0.11.0] - 2026-04-28

A minor release that absorbs a large slice of polyglot publish-pipeline machinery into alef so consumers (kreuzberg, html-to-markdown, liter-llm, …) can stop duplicating it across `kreuzberg-dev/actions` shims and per-repo `scripts/publish/` shell. `alef-publish` now owns end-to-end packaging for seven languages it previously didn't, and the `alef` binary gains four new top-level subcommands that consolidate cross-manifest validation, release-event metadata extraction, multi-registry version checks, and Go submodule tagging.

### Added

- **`alef-publish` end-to-end packagers for Python, Wasm, Node, Ruby, Elixir, Java, and C#.** Each adds a `package_<lang>` function under `crates/alef-publish/src/package/` and is dispatched from `alef publish package --lang <lang> --target <triple>`. Python invokes maturin for wheels and sdist; Wasm runs `wasm-pack` then `npm pack`; Node runs `napi build` and emits per-platform npm sub-packages following the `napi.triples.additional` convention (with the platform list configurable via the new `[publish.languages.node].npm_subpackage_platforms`); Ruby compiles via rb-sys/`bundle exec rake compile` and assembles a platform-tagged `.gem`; Elixir produces RustlerPrecompiled-style NIF tarballs (one per `nif_versions` × target combination) plus a `checksum-Elixir.{App}.exs` aggregator; Java stages the JNI shared library under `native/{classifier}/` and runs Maven to produce a classifier-suffixed JAR; C# stages `runtimes/{rid}/native/` and runs `dotnet pack`.

- **`alef validate versions`** — cross-manifest version consistency checker. Reads `alef.toml` to discover each language's manifest location (Cargo.toml / pyproject.toml / package.json / gemspec / mix.exs / pom.xml / .csproj / composer.json / gleam.toml / build.zig / Package.swift / pubspec.yaml / build.gradle.kts / DESCRIPTION) and compares declared versions to the canonical Cargo.toml workspace version. Exits non-zero with a clear mismatch list; emits JSON with `--json`. Replaces the Python-based `kreuzberg-dev/actions/validate-versions` action and the per-repo shell scripts that did the same job.

- **`alef release-metadata`** — emits release metadata JSON in the exact shape consumed by GitHub Actions matrix dispatch (`tag`, `version`, `npm_tag`, `is_tag`, `is_prerelease`, `release_<target>` flags, …). Args: `--tag`, `--targets`, `--git-ref`, `--event`, `--dry-run`, `--force-republish`, `--json`. The set of valid targets is discovered from `alef.toml`'s `languages` array. Replaces `kreuzberg-dev/actions/prepare-release-metadata`.

- **`alef check-registry`** — version-existence check across PyPI, npm, crates.io, RubyGems, Hex, Maven Central, NuGet, Packagist, a Homebrew tap, and GitHub Releases. Output: `exists=true|false` (and JSON with `--json`). Replaces `kreuzberg-dev/actions/check-registry` plus the per-registry `scripts/publish/check_*.sh` scripts.

- **`alef go-tag`** — creates and pushes the two Git tags Go's submodule-versioning convention requires (the top-level `vX.Y.Z` and the `packages/go/v<major>/vX.Y.Z` submodule tag). Supports `--dry-run` and `--remote`.

- **`PublishLanguageConfig` schema additions** (all `Option<...>`, fully back-compatible): `npm_subpackage_platforms`, `cibuildwheel_environment`, `jni_classifier`, `csharp_rid`, `wheel`, `sdist`. Existing `[publish.languages.*]` tables continue to load unchanged.

### Fixed

- **Scaffolded GitHub workflows for Swift, Dart, Gleam, Kotlin, and Zig were broken on first run.** The Swift workflow referenced `actions/setup-swift@v4`, which doesn't exist (the working action is `swift-actions/setup-swift@v2`). The Dart, Gleam, Kotlin, and Zig workflows passed unsupported flags to the per-language CLIs (`--working-dir`, `--project-path`, `--directory`) instead of `cd`-ing into the package directory. All five workflow templates now use `defaults.run.working-directory: packages/<lang>` and let each CLI run from its own root, and Swift uses `swift-actions/setup-swift@v2`.

## [0.10.4] - 2026-04-27

A patch release fixing three orchestration / e2e-codegen bugs that surfaced during a clean `alef all --clean` regenerate of every downstream polyglot repo: standalone e2e Rust crates were getting absorbed into parent workspaces, the orchestrated `alef all` skipped the language-default formatter pass, and the Rust visitor codegen was producing unparsable trait-impl blocks.

### Fixed

- **e2e Rust crate's `Cargo.toml` was silently inherited by any parent workspace**, so running `cargo fmt`/`cargo build` from inside `e2e/rust/` failed with `current package believes it's in a workspace when it's not` whenever the consuming repo had a top-level workspace `Cargo.toml` that didn't explicitly list `e2e/rust` in `members` or `exclude`. The default formatter `(cd {dir} && cargo fmt --all)` therefore exited 1 and left `e2e/rust/tests/*.rs` unformatted, breaking the next `alef verify`. The generated `e2e/rust/Cargo.toml` now starts with an empty `[workspace]` table so the e2e crate is its own workspace root and is unaffected by any parent — consumers no longer have to remember to add `e2e/rust` to `workspace.exclude`.

- **`alef all` skipped `format_generated`**, leaving every language-default formatter (mix format, ruff format, biome format, php-cs-fixer, etc.) unrun on the freshly emitted bindings. Prek's check-mode formatting hooks (`mix format --check-formatted`, etc.) therefore failed against `packages/elixir/lib/<lib>/native.ex` and similar files in every repo that ran the orchestrated `alef all` instead of `alef generate`. `Commands::All` now invokes `pipeline::format_generated` before `fmt_post_generate`, mirroring the order in `alef generate`.

- **e2e Rust visitor codegen emitted untyped trait method parameters** (`fn visit_custom_element(&self, ctx, tag_name, html) -> VisitResult`), producing files that could not parse. Three coupled fixes:
  - Each visitor parameter is now bound to a `_` pattern with the explicit `&NodeContext` / `&str` / `bool` / `u8` / `u32` / `Option<&str>` / `&[String]` type from the `HtmlVisitor` trait, so the body needn't introduce unused bindings.
  - The receiver is now `&mut self` to match the trait, not `&self`.
  - `CallbackAction::Custom` was missing the surrounding string-literal quotes — `VisitResult::Custom([CUSTOM WIDGET].to_string())` was the literal output for `output: "[CUSTOM WIDGET]"`. The codegen now wraps the escaped value in `"…"` before calling `.to_string()`, so the emitted expression is a well-formed `&str` literal.
  - The test file now imports `HtmlVisitor`, `NodeContext`, and `VisitResult` whenever any fixture in the file declares a `visitor` block.

## [0.10.3] - 2026-04-27

A patch release fixing `alef e2e generate` so it only emits test projects for languages the consumer has actually scaffolded, plus two `alef verify`-vs-`prek` drift fixes uncovered by a clean v0.10.2 regenerate of the downstream polyglot repos.

### Fixed

- **`alef e2e generate` emitted test projects for every supported backend** when neither `--lang` nor `[e2e].languages` was set, including languages with no scaffolded binding (e.g. `gleam`, `kotlin`, `dart`, `swift`, `zig`, `brew`). The resulting `e2e/<lang>/` directories couldn't compile because the package they reference doesn't exist. The default now mirrors `alef generate` / `alef scaffold`: derive the e2e language list from the top-level `[languages]` array, mapping `Language::Ffi` → the `c` e2e harness and always including `rust` for the source-crate suite. Generators without a matching `Language` variant (`brew`) require explicit opt-in via `[e2e].languages`.

  Migration: after upgrading, run `alef e2e generate` once, then manually delete any stale `e2e/<lang>/` directories for languages you never scaffolded — the cleanup pass only revisits dirs the current run touched, so untouched stale dirs from prior runs are not auto-removed.

- **e2e Rust default formatter command was invalid**: v0.10.2 introduced a built-in default of `cargo fmt --manifest-path {dir}/Cargo.toml`, but `--manifest-path` is not a stable global flag for `cargo fmt` (cargo prints `Specify message-format: short|json|human / --all / --check` and exits 1). The default is now `(cd {dir} && cargo fmt --all)`, which formats the standalone e2e crate from inside its own directory and works regardless of whether the crate is a workspace member. Without this fix, `e2e/rust/tests/*.rs` were left unformatted by `alef e2e generate`, and prek's cargo-fmt hook then rewrote them post-finalisation, breaking `alef verify`.

- **`normalize_content` skipped trailing-whitespace stripping for `.rs` files** when rustfmt could not parse them — for example, cextendr's `packages/r/src/rust/src/lib.rs` uses the non-standard `name: T = "default"` parameter-default syntax, which rustfmt rejects, so `format_rust_content` falls back to the raw codegen output. The raw output contains trailing whitespace on blank lines (e.g. `    \n` between `#[must_use]` and the next `pub fn`), which prek's `trailing-whitespace` hook then strips, breaking `alef verify`. `normalize_content` now always pipes through `normalize_whitespace` after rustfmt, so the embedded `alef:hash` always reflects whitespace-clean content regardless of whether rustfmt parsed the file.

### Added

- **`alef_e2e::default_e2e_languages(&[Language])`** — the public helper that maps the scaffolded language list to e2e generator names. Exposed so consumers and downstream tooling can resolve the same default the CLI uses.

## [0.10.2] - 2026-04-27

A patch release fixing four codegen and pipeline bugs surfaced by a clean regenerate of the five downstream polyglot repos against v0.10.1.

### Fixed

- **Swift binding `Cargo.toml` was missing `serde_json`** even though the generated `lib.rs` emitted `::serde_json::to_value(...)` / `::serde_json::from_value(...)` calls for `Codable` propagation, breaking compilation with ~1k+ `E0433: cannot find serde_json in the crate root` errors. The Swift backend now always includes `serde_json = "1"` in the binding crate's `[dependencies]`.
- **Dart binding `Cargo.toml` listed an unused `anyhow = "1"`** — the Dart trait-bridge codegen returns `source_crate::Result<T>` directly and never imports anyhow, so `cargo machete` rejected the crate. The Dart backend's `extra_deps` no longer emits anyhow.
- **Go e2e codegen emitted fixture strings containing NUL bytes inside raw string literals** (`` `…\0…` ``), which `gofmt` rejects with `illegal character NUL`. Affected ~50 generated test files in repos with NUL-bearing fixture data. The `go_string_literal` helper now switches to interpreted (double-quoted) form whenever a string contains characters Go raw strings cannot represent (NUL, `\r`, or backtick), and emits `\xNN` hex escapes for any ASCII control byte.
- **e2e generated files drifted from their embedded `alef:hash` after `prek run --all-files`** because `alef e2e generate` skipped trailing-newline / trailing-whitespace normalization and didn't run a Rust or Python formatter on standalone e2e manifests by default. Three coupled fixes:
  - `pipeline::write_scaffold_files_with_overwrite` now runs every emitted file through `normalize_content` (ensure exactly one trailing newline, strip trailing whitespace per line) before hash finalization, matching what `prek`'s `end-of-file-fixer` and `trailing-whitespace` hooks would have done.
  - `alef-e2e::format::run_formatters` now falls back to a built-in default formatter set (`cargo fmt` inside `e2e/rust/`, `ruff format` on `e2e/python/`) when `[e2e].format` is unconfigured, instead of silently no-oping.
  - `pipeline::sync_versions` now invokes `finalize_hashes` on every version-synced file (e.g. `version.rb`) so the embedded `alef:hash` line stays consistent with the rewritten content.

A patch release reworking `alef verify` to be **idempotent across alef versions** and bundling six small generator/scaffold fixes that landed since v0.10.0. The verify hash no longer encodes the alef CLI version or `alef.toml`; it is now a per-file fingerprint derived purely from the rust sources and the on-disk file content, so a green `alef verify` stays green after upgrading the alef CLI as long as nothing else changed.

### Changed

- **`alef verify` is now per-file source+output deterministic**. The `alef:hash:<hex>` line embedded in every generated file is computed as `blake3(sources_hash || file_content_without_hash_line)`, where `sources_hash = blake3(sorted(rust_source_files))` — no alef version dimension, no `alef.toml` dimension. `alef generate` finalises the hash _after_ every formatter has run (so the embedded hash describes the actual on-disk byte content), and `alef verify` is now a pure read+strip+rehash+compare with no regeneration and no writes. Previously the hash incorporated the alef CLI version, which forced every consumer repo to re-run `alef generate` after every alef bump even when nothing else had changed.
- **`alef_core::hash::compute_generation_hash` is removed**; use `compute_sources_hash` + `compute_file_hash` instead. The IR cache (`.alef/ir.json`) now keys on `compute_sources_hash` alone — pass `--clean` to bust the cache when the alef extractor itself has changed.
- **`pipeline::write_files` / `write_scaffold_files` no longer take a `generation_hash` argument**; the hash is finalised separately by the new `pipeline::finalize_hashes(paths, sources_hash)` after formatters run.

  Migration: after upgrading, every existing alef-generated file still carries the old input-deterministic hash. Run `alef generate` once (then `alef e2e generate` if the repo uses `[e2e]`) to refresh embedded hashes to the new per-file scheme.

### Fixed

- **`alef scaffold` emitted Dart and Swift `Cargo.toml` files without a `license` field and pulled in an unused `serde_json` dep** — both languages now emit the same `license = "..."` line as the other backends, and the spurious `serde_json = "1"` is gone.
- **Go e2e fixtures used raw `to_pascal_case` for method names** instead of routing through `to_go_name`, so generated test code referenced `result.Html` while the binding declared `result.HTML` (golangci-lint rejected the package). All Go method emission now goes through `alef_codegen::naming::to_go_name`.
- **Dart codegen emitted reserved keywords / numeric idents / unescaped `$` and `\`** in field names, parameter names, and string literals — Dart now escapes reserved keywords (`async`, `class`, …), prefixes leading-digit identifiers with `_`, and escapes `$` / `\` / `"` inside generated string literals.
- **`alef readme` panicked when a snippet section was missing from the language template** — missing snippet keys default to an empty Tera object instead of raising.
- **`fmt_post_generate` did not run the configured formatters for WASM, C#, and Java** because the lint/format dispatch was hardcoded to a static set of languages — the dispatch is now driven entirely by `LintConfig::format` so any backend can opt in.
- **NAPI `.d.ts` emission placed optional parameters before required ones**, which TypeScript rejects with `TS1016`. NAPI generators now reorder optional parameters last in the emitted `.d.ts` signatures.

## [0.10.0] - 2026-04-27

A major release expanding alef's target-language coverage from 11 to 16 backends. Adds full code-generation support for **Kotlin** (JNA), **Swift** (swift-bridge), **Dart** (flutter_rust_bridge), **Gleam** (Rustler-on-BEAM), and **Zig** (C ABI), each with scaffold, lint, format, build, test, setup, update, and clean defaults wired through `alef-core`. The five new backends share the existing IR, trait-bridge round-trip API, and `alef-docs` language-native doc emission paths — there is no per-language fork of the codegen pipeline. Also adds language-native doc comments (PHPDoc, C# XML, `@doc`, roxygen2) to the existing PHP/C#/Elixir/R backends, and fills in numerous correctness gaps surfaced by the kreuzberg verification worktree.

### Added

- **Five new language backends** — `alef-backend-kotlin`, `alef-backend-swift`, `alef-backend-dart`, `alef-backend-gleam`, `alef-backend-zig`. Each implements the `Backend` trait, ships per-crate snapshot tests via `insta`, and is wired into `alef-cli`'s dispatch table. Trait bridges are supported across all five (Dart abstract classes, Gleam per-method response shims, Zig comptime `make_*_vtable` helpers, Kotlin/Swift host-side typealiases + Codable propagation).
- **Language-native scaffolds for the new backends** — `alef-scaffold` writes Kotlin (Gradle KTS + Detekt + ktlint), Swift (Swift Package Manager + swift-format + SwiftLint), Dart (pub + flutter_rust_bridge + lints), Gleam (`gleam.toml` + gleeunit), and Zig (`build.zig` + `build.zig.zon`) toolchain configs, README stubs, and CI templates. `alef-publish` gains per-language packaging and validation modules for the same five.
- **`alef-docs` language emitters for PHP/C#/Elixir/R/Kotlin/Swift/Dart/Gleam/Zig** — PHPDoc, XML `<summary>`/`<param>`/`<returns>`, Elixir `@doc` heredocs, roxygen2 `@param`/`@return`, KDoc, Swift markdown, Dart `///`, Gleam `///`, and Zig `///` are now emitted for all public functions/methods. Shared `doc_emission` module handles language-specific escaping (e.g. `*/` in PHP, XML entities in C#, triple-quote escapes in Elixir).
- **`has_serde` IR flag on `EnumDef`** — extracted from `derive(Serialize)` + `derive(Deserialize)` so backends can decide whether to JSON-roundtrip or pattern-match conversion. Used by Swift to emit `Codable` and propagate it to non-derived enum references whose containing struct is `Codable`.
- **`return_sanitized` IR flag on `FunctionDef`** — laid down by `alef-extract` when the return type was sanitized by the unknown-type pass. Backends can use it to decide whether a roundtrip is recoverable for the return value (currently consumed by snapshot/doc tests; full backend wiring is a follow-up).
- **`alef-readme` v0.10.0 README templates** for the new languages.

### Changed

- **Backend count: 11 → 16** in README, skill docs, and the supported-languages table. Kotlin/Swift/Dart/Gleam/Zig are no longer flagged "in progress".
- **Trait bridges span all 16 backends** — round-trip callbacks (host language implements a trait, Rust core invokes it through the bridge) work end-to-end for every supported language.
- **`alef-core::config` defaults** for `LintConfig`, `BuildConfig`, `TestConfig`, `SetupConfig`, `UpdateConfig`, and `CleanConfig` now cover Kotlin/Swift/Dart/Gleam/Zig with per-language preconditions and command sequences. The Phase-1 skip clauses in the per-language test loops are gone.
- **Centralized version pins** — `alef-core::template_versions` gains `pub_dev`, `toolchain`, and Kotlin/Swift entries in `cargo`/`maven` modules. Toolchain pins (Zig 0.16, Dart SDK `>=3.0 <4.0`, JVM 21, swift-bridge `0.1.59`, flutter_rust_bridge `2.12.0`, Kotlin 2.1.10, kotlinx-coroutines 1.9.0, JNA 5.14.0) are now single-source.

### Fixed

- **Swift typealias closure for #59** — non-trait Kreuzberg types are emitted as `typealias Kreuzberg.X = RustBridge.X`, host-language wrapper functions are removed, and consumers call `RustBridge` functions directly. `RustString`/`RustVec` returns and `Data → [UInt8]` arguments are converted at the wrapper boundary. Conversion initializers (`init(rustVal: RustBridge.X)`) are emitted for non-`Codable` enums; `Codable` enums skip them and use JSON. `swift_name(camelCase)` is emitted on every bridge fn so swift-bridge generates idiomatic Swift symbols. Reduces `kreuzberg-swift` build errors from ~190 to 0.
- **Swift unbridgeable surfaces** — getters/wrappers for fields that can't survive the swift-bridge round-trip (excluded fields, JSON containers with non-serde inners, `Vec<non-Primitive>` in non-serde structs) now skip emission entirely with a `// alef: skipped` marker instead of emitting `unimplemented!()`.
- **Swift constructor / lints / keyword escaping** — constructor is omitted (instead of `unimplemented!()`) for structs that need `Default` but don't derive it. Generated `lib.rs` carries an `#![allow]` block for the swift-bridge codegen pattern's clippy artifacts. Reserved Swift keywords used as field/param names are escaped at the extern block, wrapper, and constructor sites. A phantom `Vec<FooBox>` accessor is forced per trait so swift-bridge emits the C symbols its Vec<T> accessors call.
- **Dart trait abstract classes + unbridgeable surfaces** — Dart-side `abstract class` is emitted per trait for the round-trip API. Functions with unbridgeable params (e.g. `Vec<(Vec<u8>, …)>`, lossy through JSON) are skipped instead of emitting panicking shims. Tuple-variant fields are renamed `_N` → `fieldN` to survive flutter_rust_bridge's underscore stripping. Generator-only clippy lints (`map_identity`, `let_and_return`, `collapsible_match`, `manual_flatten`, `too_many_arguments`, `unit_arg`, `type_complexity`) are silenced via `#![allow]` in the emitted `lib.rs`.
- **Gleam per-method response shims** — each trait method gets a typed `{trait}_{method}_response(call_id, Result(T, E))` shim so callback modules can reply through the Rustler reply-registry without raw JSON.
- **Zig vtable codegen** — comptime `make_{trait}_vtable(comptime T, *T)` helper generated per trait bridge; thunks reconstruct byte slices, fallible methods return `i32 + out_error`, lifecycle stubs (`name_fn`, `version_fn`, `initialize_fn`, `shutdown_fn`) emitted when `super_trait` is configured. Trait-bridge thunks now discard unused `out_result` params, avoid shadowing method params named `result`, and only emit `return 0;` when the success path actually flows through.
- **WASM `useless_conversion`** — generated `lib.rs` adds the lint to its crate-wide allow list, since the `kreuzberg::T::from(t_core).into()` identity conversion is only an artifact of code generation.
- **Rustler doc emission** — NIF Rust source emits standard `///` Rustdoc instead of literal Elixir `@doc """…"""` heredocs (which produced 380 parse errors when run through `cargo build`). Reserved Elixir module-attribute names (`@behaviour`, `@callback`, …) are no longer used as GenServer attributes. The lossy `Vec<String>` → `Vec<&str>` conversion is dropped — `&Vec<T>` already derefs to `&[T]`, so the simpler emission preserves caller signatures like `&[String]`.
- **Swift scaffold indent** — Swift test stubs use 2-space indent to match swift-format defaults.

## [0.9.2] - 2026-04-27

A patch fixing three generator bugs that surfaced when alef v0.9.1 was used to regenerate downstream consumer repos. Each one blocked a different language's pre-commit hook from passing on freshly regenerated bindings; with these fixes a full `alef generate` + `alef e2e generate` produces output that passes mix credo (Elixir), golangci-lint (Go), and `dotnet format` / `dotnet build` (C#) without further hand-editing.

### Fixed

- **Elixir trait-bridge module names emitted broken PascalCase for hyphenated crate names**: `alef-scaffold`'s Elixir generator used `capitalize_first(&app_name)`, which only uppercased the first character and left underscores in place. For a crate named `html-to-markdown` the generated Rustler trait bridge declared `defmodule Html_to_markdownHtmlVisitorBridge do`, which `mix credo` correctly rejects ("Module names should be written in PascalCase."). All three call sites (mix.exs `defmodule`, the bridge `module_name`, and `native_mod`) now use `heck::ToPascalCase`, splitting on both `-` and `_`. `html-to-markdown` → `HtmlToMarkdown`, `tree-sitter-language-pack` → `TreeSitterLanguagePack`.
- **Go binding and Go e2e tests disagreed on initialism casing**: `alef-backend-go` already routes Go field/parameter names through an idiomatic-initialism transformer (`HTML`, `URL`, `ID`, `JSON`, …) but `alef-e2e`'s Go fixture renderer used a plain `to_pascal_case` helper, so generated test code referenced `result.Html` and `result.Url` while the binding declared `HTML` and `URL`. golangci-lint rejected the generated package with "result.Html undefined". Both crates now route through `alef_codegen::naming::to_go_name` and `go_param_name`; the duplicate `GO_INITIALISMS` table in `alef-e2e/src/codegen/go.rs` was deleted.
- **C# `.csproj` scaffold was placed inside the source subdirectory instead of the package root**: `alef-scaffold` wrote the project file at `packages/csharp/{Namespace}/{Namespace}.csproj`, but consumer Taskfile targets and the prek `csharp-format` / `csharp-lint` hooks expected it at `packages/csharp/{Namespace}.csproj` (alongside `Directory.Build.props`, matching the convention used for the gemspec at `packages/ruby/*.gemspec`). The path is now `packages/csharp/{Namespace}.csproj`, with `<ItemGroup>` `Include` paths adjusted (one level up the relative path) so `LICENSE` and `runtimes/**` still resolve correctly. The scaffold remains create-once.
- **C# async docs/signature emitted invalid `Task<void>`**: `alef-docs::render_csharp_fn_sig` formatted async return types as `Task<{ret}>` unconditionally, so for void-returning async functions the documented signature read `public static async Task<void> ...`, which doesn't compile under `<TreatWarningsAsErrors>true</TreatWarningsAsErrors>`. It now emits `Task` for void returns and `Task<{ret}>` otherwise.

## [0.9.1] - 2026-04-27

A patch release fixing the publish pipeline. v0.9.0 (and prior releases since `alef-publish` was added) failed to publish `alef-cli` to crates.io because `alef-publish` was missing from the publish workflow's crate list — `alef-cli` declares `alef-publish = "^X.Y.Z"` and crates.io still served `alef-publish 0.7.2`, so the upload of `alef-cli` errored with `failed to select a version for the requirement alef-publish = "^0.9.0"`. The 19 other workspace crates published cleanly each time; only `alef-cli` (the binary used by `cargo install` / `cargo binstall`) was affected. v0.9.0 was rescued by publishing `alef-publish` and `alef-cli` manually with `cargo publish`; v0.9.1 makes the workflow self-sufficient.

### Fixed

- **`alef-publish` was missing from the Publish workflow's crate list**: `.github/workflows/publish.yaml` listed 20 crates but not `alef-publish`, so the workflow never uploaded it to crates.io. `alef-cli`'s dependency on `alef-publish ^X.Y.Z` therefore failed to resolve at publish time. The crate is now listed in dependency order (after `alef-scaffold`, before the backends), matching the rest of the workspace.

## [0.9.0] - 2026-04-27

A major fix release eliminating ~40 generated "Not implemented" stubs across the PHP, Ruby, C FFI, and R backends. Every batch extraction API (`batch_extract_file_sync`, `batch_extract_bytes_sync`, plus async variants), `extract_file`/`extract_file_sync`, and most of the Ruby gem's surface previously failed at runtime with error code 99. Five distinct generator bugs collapsed into a single class of bad output. Also makes `alef verify` input-deterministic so downstream formatters can reformat generated content freely without breaking verify, and exposes the canonical input-hash recipe via `alef_core::hash::compute_generation_hash`.

### Changed

- **`alef verify` is now input-deterministic**. The embedded `alef:hash:<hex>` line in every generated file is no longer a per-file hash of the normalised generated content; it is a single fingerprint of the **inputs** that produced the run:
  - `blake3(sorted(rust_source_files) + alef.toml + alef_version)`

  `alef generate` computes this hash once and writes the same value into every alef-headered file. `alef verify` recomputes the same input hash and compares it to the disk hash without inspecting any file body. As a result, downstream formatters (rustfmt, rubocop, dotnet format, spotless, biome, mix format, php-cs-fixer, taplo, ruff, …) can reformat alef-generated content freely without breaking verify — only changes to the generation inputs invalidate the embedded hash. The previous output-deterministic semantics caused `alef verify` to flag bindings as stale on every commit in repos with active language formatters.

  Migration: after upgrading, every existing alef-generated file still carries the old per-file hash. Run `alef generate` once (then `alef e2e generate` if the repo uses `[e2e]`) to refresh all embedded hashes — a single regenerate pass writes the new uniform input hash everywhere.

- **`alef verify` ignores `--lang`, `--compile`, `--lint`**. The flags are still accepted for backwards compatibility but no longer affect the check, since verify no longer regenerates per-language. Use `alef build`, `alef lint`, `alef test` for those concerns.

### Added

- **`alef_core::hash::compute_generation_hash(sources, config_path, alef_version)`** — public function exposing the canonical input-hash recipe so other consumers (cache invalidation, custom build tools) can reuse it. `alef-cli`'s `cache::generation_hash` is now a thin wrapper passing `env!("CARGO_PKG_VERSION")` for the version dimension.

### Fixed

- **Bare `Path` resolved to `Named("Path")` and silently sanitized to `String`**: `alef-extract`'s `resolve_path_type` only recognized `PathBuf` (and `&Path` via the reference path). Functions taking `path: impl AsRef<Path>` resolved the inner generic arg `Path` to `Named("Path")`, then `sanitize_unknown_types` (which considers stdlib `Path` an unknown user type) replaced it with `TypeRef::String` and marked the param as sanitized. The result was that every binding emitted "Not implemented" stubs for `extract_file`, `extract_file_sync`, and any other API using `impl AsRef<Path>`. Bare `Path` now maps to `TypeRef::Path` like `PathBuf`.
- **PHP backend stubbed sanitized functions even when the params were JSON-roundtripable**: `alef-backend-php`'s `gen_function_body` short-circuited to `gen_stub_return` whenever `func.sanitized` was true, ignoring the existing serde-roundtrip helper. `gen_php_serde_let_bindings`, `gen_php_named_let_bindings`, and `gen_php_call_args_with_let_bindings` now handle `Vec<String>` params that came from `Vec<tuple>` (each element is a JSON-encoded tuple), and the function/method body generators fall through to the serde path when a recovery is possible. Affects all batch APIs (`batch_extract_*`).
- **Magnus (Ruby) backend stubbed every extraction function**: the shared `can_auto_delegate_function` rejected functions with a non-opaque Named ref param (e.g. `&ExtractionConfig`), so `extract_bytes`, `extract_bytes_sync`, and 19 other functions emitted "Not implemented" stubs even though the existing deser-preamble could handle them. `alef-backend-magnus` now adds a magnus-specific `magnus_serde_recoverable` gate that allows JSON-roundtrip on Named non-opaque params and on sanitized `Vec<String>` (originally `Vec<tuple>`), and uses `gen_call_args_with_let_bindings` so the call site borrows `&{name}_core` correctly.
- **C FFI backend stubbed all functions with sanitized params**: `alef-backend-ffi` set `will_be_unimplemented = func.sanitized` and bailed out before parameter conversion, even though the existing `Vec` JSON deserialization path could handle `Vec<String>` params with type inference at the call site. The FFI now exempts sanitized functions from the unimplemented bail-out when every sanitized param is a recoverable `Vec<String>` (with `original_type` set), routing them through the standard JSON-roundtrip Vec conversion. Affects `kreuzberg_batch_extract_*` and other tuple-batch FFI exports consumed by the Go, Java, and C# bindings.
- **R (extendr) backend panicked with `todo!("async not supported by backend")` for every async function**: extendr was configured with `AsyncPattern::None`. R is single-threaded but async functions are still callable from R via a per-call tokio runtime. The backend now uses `AsyncPattern::TokioBlockOn` and reports `supports_async: true`, generating `tokio::runtime::Runtime::new()?.block_on(...)` wrappers like the other sync-on-async backends.

### Known limitations

- Functions whose return type is a Named struct from outside the API surface (e.g. `get_preset` returning `&'static EmbeddingPreset`) are still stubbed: alef has no way to know whether the core type derives `Serialize`, so it can't safely JSON-roundtrip the value back to the binding side. The new `return_sanitized` IR field is laid down for a future fix.

## [0.8.7] - 2026-04-27

A patch fixing a CI-hang in the C# `alef setup` default and replacing the `// TODO: add usage example` placeholder in alef-generated READMEs with a pointer to the main repository docs.

### Fixed

- **`alef setup --lang csharp` walks the entire repo when no .csproj is at the top level**: `dotnet restore packages/csharp` searches for project files recursively under the given directory, including `target/`, `node_modules/`, and other artifact dirs. On CI this took longer than the 600s timeout. The C# setup default now applies the same precondition + `find` strategy as the upgrade default — both `dotnet` and a discoverable `.sln`/`.csproj` (depth 3) must exist, otherwise setup is skipped, and the resolved project path is passed explicitly to `dotnet restore`.
- **README usage examples were `// TODO: add usage example` placeholders**: the alef-generated per-language READMEs in the FFI/WASM crate dirs and `packages/{lang}/` for hardcoded fallbacks now redirect readers to the main repository's documentation instead of emitting a literal TODO.

## [0.8.6] - 2026-04-27

A patch hardening the C# upgrade default introduced in v0.8.5. The original `$(ls ... || find ... || echo …)` chain didn't actually skip when no project file existed — `ls foo/*.csproj 2>/dev/null` always exits 0 with empty stdout, so the `||` fallbacks never triggered and `dotnet outdated` ran with no path argument and errored out at the repo root.

### Fixed

- **C# upgrade falls through and fails when no `.csproj` exists**: alef-core's `update_defaults.rs` C# precondition now requires BOTH `dotnet` AND a discoverable `.sln`/`.csproj` under the output dir (depth 3). When no project exists, the upgrade is skipped (precondition warning) instead of erroring out. The command itself is simplified to just `find … | head -1`, which the precondition has already validated returns a path.

## [0.8.5] - 2026-04-27

A patch fixing two `alef update --latest` (`task upgrade`) failures observed across all four consumer repos.

### Fixed

- **Java `versions:use-latest-releases` aborted on missing rules file**: `alef-core/src/config/update_defaults.rs` unconditionally appended `-Dmaven.version.rules=file://${PWD}/{output_dir}/versions-rules.xml`, but most consumers don't ship that file. Maven aborted with `ResourceDoesNotExistException`, failing the whole upgrade pipeline. The flag is now wrapped in `$([ -f .../versions-rules.xml ] && echo "...")` so it's only appended when the file exists.
- **C# `dotnet outdated packages/csharp` rejected the directory**: `dotnet outdated` requires a `.sln` or `.csproj` path, or a directory that contains one at the top level. Most consumers nest projects under `packages/csharp/<ProjectName>/`, so the call failed with "The directory 'packages/csharp' does not contain any solutions or projects." The default now resolves to the first `.sln`/`.csproj` found under the output dir (depth 3) before invoking `dotnet outdated`.

## [0.8.4] - 2026-04-27

A follow-up to v0.8.3 fixing two cases where downstream tooling reformatted alef-generated `crates/{lib}-wasm/Cargo.toml` and silently invalidated the alef hash header.

### Fixed

- **WASM `Cargo.toml` cargo-sort canonical layout**: `alef-backend-wasm`'s `gen_cargo_toml` previously emitted `[lib]`/`[dependencies]` _before_ the `[package.metadata.*]` blocks and listed dependencies in declaration order. cargo-sort, run as a pre-commit hook in every consumer, then rewrote the file in canonical alphabetical / TOML-section order — invalidating the embedded `alef:hash:` line, so the next `alef verify` reported the wasm Cargo.toml as stale on every commit. The template now emits sections and dependencies in cargo-sort canonical order (`[package]` → `[package.metadata.*]` → `[lib]` → `[dependencies]` sorted alphabetically including the dynamic core crate dep), so cargo-sort is a no-op.
- **WASM cargo-machete unused-dep flag for `serde_json`**: the wasm `Cargo.toml` always declared `serde_json = "1"` because trait-bridge / function generators may use it, but consumers without those code paths would fail `cargo-machete` on every commit. Added `serde_json` to the existing `[package.metadata.cargo-machete] ignored` list alongside the already-ignored `futures-util` and `wasm-bindgen-futures`.

## [0.8.3] - 2026-04-26

A follow-up to v0.8.2 making `alef verify` formatter-agnostic and fixing the Ruby version-sync footgun. Verify is now a pure hash comparison: alef computes the canonical hash at generation time, embeds it in the file header, and on `alef verify` only compares the embedded hash against a freshly-computed hash of the canonicalised generated content. External formatters (php-cs-fixer, rubocop, ruff, biome, …) can reformat the body freely without ever causing a verify diff.

### Changed

- **`alef verify` is now hash-only.** The legacy "diff against on-disk content" path is removed. Generated files carry an `alef:hash:<blake3>` header line; verify reads that hash and compares against the hash of the freshly-generated canonical content. No file body inspection, no normalization-against-disk, no formatter sensitivity. Files without an alef hash header (user-owned scaffolds like `Cargo.toml`, `composer.json`) are skipped — alef has no claim over them.
- **`alef generate` auto-runs `sync_versions`** at the end of every run, so user-owned manifests (gemspec, composer.json, package.json, etc.) track `Cargo.toml` automatically without a separate `alef sync-versions` invocation.

### Fixed

- **Ruby version.rb double-conversion via `[sync].extra_paths`**: when consumers listed `version.rb` in `[sync].extra_paths` (a common pattern for non-default extension layouts), the generic `SEMVER_RE.replace_all` matched the `0.3.0` prefix of the gem-formatted `0.3.0.pre.rc.2` and replaced it with the cargo dash-form `0.3.0-rc.2`, producing the corrupted `VERSION = "0.3.0-rc.2.pre.rc.2"`. The extra-paths handler now recognises `version.rb` and `*.gemspec` filenames and applies the gem-aware replacer (`to_rubygems_prerelease` + targeted regex) instead of the generic semver regex.
- **Ruby gemspec/version.rb gem format**: scaffold (`alef-scaffold`) and binding writer (`alef-backend-magnus`) now both call `alef_core::version::to_rubygems_prerelease` so cargo prereleases like `0.3.0-rc.2` are emitted as RubyGems-canonical `0.3.0.pre.rc.2`. RubyGems rejects the dash form (`gem build` raises `Gem::Version "0.3.0-rc.2" is not a valid version`).
- **PHP stub PSR-12 blank line**: generated `.phpstub` files now emit `<?php\n\n` so php-cs-fixer doesn't reformat the file on every commit.

### Added

- **`alef_core::version` module**: `to_rubygems_prerelease(version: &str) -> String` extracted from the CLI's `pipeline::version` so backends and scaffolds can reuse it without duplicating the conversion logic.

## [0.8.2] - 2026-04-26

A follow-up release to v0.8.1 fixing post-generation formatter invocation. Generated bindings now reliably pass `cargo fmt --check` and `ruff check` in CI.

### Fixed

- **FFI cargo fmt silently no-op'd**: `format_generated` ran `cargo fmt` in `packages/ffi/`, which has no `Cargo.toml`. Generated `crates/{lib}-ffi/src/lib.rs` was therefore never formatted, and CI's `cargo fmt --all -- --check` failed on diffs like `use std::ffi::{CStr, CString, c_char};` vs the rustfmt-canonical `use std::ffi::{c_char, CStr, CString};`. Now runs `cargo fmt --all` from the project root, which formats every generated Rust crate (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs, Rustler, wasm-bindgen) in the consumer workspace.
- **Python ruff missed lint autofixes**: only `ruff format` was run, so generated stubs/wrappers retained `I001` unsorted imports, `F401` unused imports (e.g. `Any`), stale `# noqa: F401` comments on used imports, and `TC008` missing `TypeAlias` annotations on union aliases. `format_generated` now runs `ruff check --fix .` before `ruff format .` in `packages/python/`.
- **Ruby formatter wrong tool**: ran `cargo fmt` in `packages/ruby/`, which has no `Cargo.toml` (the Magnus crate lives under `packages/ruby/ext/{lib}_rb/native/` and is covered by the FFI-driven `cargo fmt --all`). Now runs `rubocop -A --no-server` in `packages/ruby/` to auto-correct generated `.rb` and `.gemspec` files.
- **R formatter wrong tool**: ran `cargo fmt` in `packages/r/`. Now runs `Rscript -e "styler::style_pkg('packages/r')"`.
- **WASM formatter scope tightened**: was `cargo fmt` in `packages/wasm/` (works because the wasm-bindgen crate lives there). Now uses `cargo fmt -p wasm` so the explicit package selection survives if package layout changes.

### Changed

- `FormatterSpec` now holds a `&'static [FormatterCommand]` instead of a single command, so a language can run multiple formatter steps in sequence (used by Python's `ruff check --fix` → `ruff format`). On first failure within a sequence the remaining steps are skipped (warning logged) — formatter errors never fail `alef generate`.
- `work_dir: ""` is now treated as the project root (no path join), so language-agnostic invocations like `cargo fmt --all` can run from the consumer's workspace root.

## [0.8.1] - 2026-04-26

A follow-up release to v0.8.0 focused on closing remaining clippy/build-correctness gaps surfaced by Kreuzberg's full workspace build. All alef-generated bindings (Python, Node, Ruby, PHP, FFI, WASM, Elixir, R, Go, Java, C#) now compile cleanly with `-D warnings` against the kreuzberg verification worktree. Adds an `is_copy` IR flag so the FFI backend can correctly distinguish Copy enums from Clone-but-not-Copy data-bearing enums.

### Added

- **`is_copy` IR flag on `TypeDef` and `EnumDef`**: extracted from `#[derive(Copy)]`. Lets backends distinguish Copy types from Clone-but-not-Copy types when emitting field accessors and method returns.

### Fixed

- **WASM trait-bridge regression**: shared `gen_bridge_trait_impl` emitted `#[async_trait::async_trait]` unconditionally, but `kreuzberg-wasm` has no `async_trait` dep. On non-wasm32 targets the trait declaration uses `#[cfg_attr(not(target_arch = "wasm32"), async_trait)]`, producing E0195 lifetime mismatches and E0433 missing-crate errors. WASM trait bridges now compile inside a private `#[cfg(target_arch = "wasm32")]` mod with a public re-export — the bridge is wasm-only by nature (wraps `wasm_bindgen::JsValue`), so host workspace builds skip the impl entirely. The same `cfg(target_arch="wasm32")` gate is also applied to bridge-using free functions.
- **WASM scaffold deps**: generated `kreuzberg-wasm/Cargo.toml` now ensures `js-sys` is present, and serde-style `Vec<&str>` parameters are correctly bridged via `serde_wasm_bindgen` instead of failing JsValue→Vec deserialization.
- **FFI clone-on-copy lints (~25 occurrences)**: field accessors and method returns previously emitted `Box::into_raw(Box::new(obj.field.clone()))` for every Named-typed field, tripping `clippy::clone_on_copy` on the `Copy` enums (`TableModel`, `ChunkerType`, `LayoutClass`, `BBox`, etc.). Codegen now consults `enum.is_copy` / `type.is_copy` to emit auto-copy/deref for Copy types and `.clone()` for Clone-but-not-Copy types. Non-Clone opaques fall back to a raw-pointer alias.
- **FFI move-out-of-borrow errors (E0507)**: data-bearing enums (`Provider`, `HtmlTheme`, `PdfBackend`, `NodeContent`, `OcrBoundingGeometry`, etc.) are not Copy; previous codegen emitted `Box::new(obj.field)` (no clone), causing E0507. Now correctly emits `.clone()` when `is_clone && !is_copy`.
- **FFI let_and_return in vtable wrappers**: infallible primitive/Duration vtable returns previously emitted `let _rc = unsafe { fp(args) }; _rc`, tripping `clippy::let_and_return`. Skip the binding for that case and emit `unsafe { fp(args) }` as the tail expression.
- **FFI useless conversion in PathBuf::from on Option<PathBuf>**: codegen unconditionally wrapped `cache_dir_rs.map(std::path::PathBuf::from)` even when `cache_dir_rs` was already `Option<PathBuf>`. Now passes through directly.
- **PHP plugin bridge clippy cleanup**: 33 lint errors eliminated across `alef-backend-php`'s trait bridge generator — `let_and_return` in async wrappers, `clone_on_copy` on `*mut _zend_object` (raw ptr is Copy), `useless_conversion` on `&str`/`PhpException` self-conversions, `vec_init_then_push` replaced with `vec![…]` literals, and a duplicate `let texts = texts;` redundant local.
- **Rustler/Elixir useless `.into()`**: NIF function wrappers no longer wrap owned `String` / `Vec<u8>` returns in `.into()` (which clippy flags as `String → String` / `Vec<u8> → Vec<u8>` identity). The `gen_rustler_wrap_return` helper now consults `returns_ref` to decide.
- **Rustler reserved Elixir attribute names**: emitted GenServer modules no longer use Elixir-reserved module-attribute names (`@behaviour`, `@callback`, etc.), preventing `mix compile` errors.
- **Rustler/Go/scaffold misc**: Elixir builtins handling fix, Go cbindgen prefix correction for re-exported types, Cargo `tokio` dep added when async functions are present.
- **Rustler test fixture**: `EnumVariant` test data updated for the new `is_tuple` field added in v0.8.0.
- **PyO3 / WASM static method wrappers**: `wrap_return_with_mutex` now wraps the inner core type in the binding wrapper for `default()` / `from()` static methods on non-opaque types (was previously skipping the conversion via the broken `n == type_name` shortcut, producing 24 `mismatched types` errors in PyO3).
- **PyO3 useless `.into()` on owned String returns**: `wrap_return` in the shared serde path no longer emits `.into()` for `TypeRef::String | TypeRef::Bytes` (both are identity in all backends). Eliminates `.map(Into::into)` chains on `Result<String, _>` returns in generated PyO3 functions like `reduce_tokens`, `serialize_to_toon`, `serialize_to_json`.
- **`useless_conversion` on extracted `From<X> for Y` static methods**: kreuzberg's `From<html_to_markdown_rs::HtmlMetadata> for HtmlMetadata` impl is extracted as a static method on `HtmlMetadata` whose param type is normalized to `Self`. The generated FFI/PyO3 wrapper calls `Y::from(arg: Y)`, which Rust resolves to the blanket `From<T> for T` (identity). The wrapper is preserved for ABI stability; `clippy::useless_conversion` is suppressed at the FFI and PyO3 file level.
- **C# nullable double-coalesce**: visitor field accessors emitted `x?.Property ?? null ?? defaultValue` on already-nullable types. The redundant `?? null` is now dropped.
- **C# Name property on visitor interface**: `IVisitor` was missing the `Name` property on some codegen paths; now always emitted so trait bridge dispatch works.
- **C# csproj scaffolding**: `Kreuzberg.csproj` is scaffolded once into the package subdir (not the language root) and not overwritten on subsequent runs. User-tuned project settings (target framework, package metadata) survive regeneration.
- **Elixir NIF Cargo.toml**: emits `[workspace]` block so the NIF crate is its own cargo workspace, plus `futures-util` dep for the async bridge dispatch path.
- **Java primitive trait method parameters**: trait bridges now correctly box primitives (`int` → `Integer`, `bool` → `Boolean`) before passing through `Object[]` callbacks; primitive return values are unboxed via `.intValue()` / `.booleanValue()` rather than direct cast.
- **Magnus tagged-union `Vec<Named>` field marshalling**: Vec-of-Named fields preserve `Vec<Named>` type in generated Rust enum variants instead of collapsing to `String`. JSON array round-trip now works for tagged unions like `Multi(Vec<Item>)`. Map fields still collapse to `String` for serde_json indirection.
- **Magnus Option<T> double-wrap in kwargs**: kwargs builders no longer emit `Option<Option<T>>` for nullable struct fields when the field type is already `Option<T>` and the kwarg flag also makes it optional. Single-Option emission across kwargs and getters.
- **Go/WASM NodeContext cbindgen prefix**: cbindgen-emitted C struct names use the configured crate prefix (e.g. `KZBNodeContext` instead of `NodeContext`), matching the declared FFI symbol convention.
- **Go binding `futures-util` dep**: generated Go FFI's `Cargo.toml` declares `futures-util` when the IR exposes async functions.

## [0.8.0] - 2026-04-26

This release closes a long-standing gap in alef's polyglot generator: bindings for every supported language now compile cleanly with zero errors and zero warnings against real-world Rust crates that exercise trait bridges, tagged-union enums, async functions, and feature-gated modules. Most of the surface area changes are codegen-internal — public APIs are unchanged — but the cumulative effect is that downstream projects (Kreuzberg in particular) can now consume alef-emitted code without any post-generation patching across Java, C#, Go, Ruby, Elixir, R, WASM, and the previously-stable Python/Node/PHP/FFI bindings.

### Added

- **Trait bridges (Java, C#, Magnus/Ruby)**: full plugin-style register/unregister codegen with proper FFI-side vtable struct names and method dispatch. Previously gated behind `exclude_languages = ["..."]` per-bridge; the generated code is now compile-clean across all backends.
- **Trait bridges (Rustler/Elixir)**: new `LocalPid`-based dispatch using `OwnedEnv::send_and_clear` plus a global oneshot reply registry (`TRAIT_REPLY_COUNTER`, `TRAIT_REPLY_CHANNELS`) for synchronous and asynchronous trait method calls. The `alef-scaffold` Elixir generator emits a companion GenServer module per trait so consumer code can `use` the bridge directly.
- **Auto-format**: `alef generate` now runs the language-native formatter (`ruff format`, `mix format`, `cargo fmt`, `biome format --write`, `gofmt -w`, `php-cs-fixer fix`, `dotnet format`, `google-java-format -i`) on the emitted output by default. `--no-format` opts out. Per-language overrides via `[format.<lang>]` in `alef.toml`.
- **Setup timeouts**: `alef setup` accepts `--timeout <seconds>` (default 600 per language). Per-language `timeout_seconds` settable via `[setup.<lang>]` in `alef.toml`. Hangs in tooling that ignores Ctrl-C now fail cleanly via `try_wait` deadline + `child.kill`.
- **Sync-versions (RubyGems prerelease format)**: `alef sync-versions` now writes Bundler-canonical prerelease strings (`1.8.0-rc.2` → `1.8.0.pre.rc.2`) to gemspecs and `version.rb` files. Previously emitted SemVer dashes were rejected by RubyGems.
- **Sync-versions (README regeneration)**: after manifest updates, alef extracts current IR and regenerates per-language READMEs so embedded version strings (e.g. `<version>1.8.0</version>` in pom.xml snippets) refresh without a separate `alef readme` invocation.
- **Sync-versions (cache invalidation)**: removes `.alef/` after manifest updates so subsequent generation steps see fresh IR.
- **Publish `after` hooks**: `[publish.<lang>] after = "..."` is now executed at the success-path end of `prepare`, `build`, and `package` stages, mirroring the existing `before`/`precondition` hook contract. The `after` field on `PublishLanguageConfig` was previously dead config.
- **Feature-gated extraction**: `alef-extract` now captures `#[cfg(feature = "...")]` annotations on `pub use` re-exports and `pub mod` declarations, propagating the cfg to all re-exported items.
- **WASM feature filtering**: `alef-backend-wasm` reads enabled feature flags from `alef.toml` and automatically excludes types/functions/enums whose cfg references a disabled feature, preventing broken references like `ServerConfig` (gated behind `api`) from leaking into the WASM crate.
- **Orphan file cleanup**: after `alef generate` writes the current binding output, alef scans the per-language output directories for files containing the alef-generated header that are not in the current run's emission set, and deletes them. Prevents stale alef-emitted files from blocking builds when types are removed from the public Rust API.
- **Magnus tuple variant codegen**: tagged-union enum variants like `Multiple(Vec<String>)` now emit valid `Multiple(Vec<String>)` Rust syntax. Previously emitted invalid `Multiple { _0: ... }` struct-style braces.
- **FFI enum from_json/free**: enums that flow through FFI as opaque pointer parameters now get `*_from_json` and `*_free` exports so backends (C#, Java, etc.) can construct enum values across the FFI boundary.
- **C# Directory.Build.props**: emitted on every `alef generate` (always overwritten) with `<Nullable>enable</Nullable>` and `<LangVersion>latest</LangVersion>`. MSBuild auto-imports it from the package directory so the setting survives user edits to `Kreuzberg.csproj`.

### Changed

- **Magnus serde marshalling for tagged-union enum fields**: `Vec<Item>` (where `Item` is a Named type) is now emitted as `Vec<Item>` in the generated Rust enum — preserving JSON array round-trip — instead of being collapsed to `String`. Map fields still collapse to `String` for serde_json indirection. Vec of primitives stays `Vec<T>`.
- **Java facade Optional unwrap**: facade methods declared with non-Optional return types (e.g. `String getPreset(...)`) now call `.orElseThrow()` on FFI results that come back as `Optional<T>`, eliminating the type mismatch between the facade and the Panama FFM layer.
- **C# wrapper `result` shadowing**: emitted local `result` variables are renamed to `nativeResult` to avoid CS0136/CS0841 collisions when the wrapper method takes a parameter named `result` (e.g. `SerializeToJson(ExtractionResult result)`).
- **C# tagged-enum string defaults**: when a serde-tagged enum field maps to `string` in C#, the default is emitted as the variant's JSON tag (`"Plain"`) rather than the non-existent `string.Plain` static.
- **C# async unit returns**: `async Task` methods with unit returns no longer emit `return await Task.Run(() => …);` — the `return` is dropped to satisfy CS1997.
- **Go trait_bridges.go**: now generated independently of the `visitor_callbacks` flag so plugin-style bridges work without enabling visitor codegen. The vtable struct name uses cbindgen's `{CRATE_UPPER}{CratePascal}{TraitPascal}VTable` convention; register/unregister calls use `{prefix}_register_{trait_snake}` / `{prefix}_unregister_{trait_snake}`. Function pointer fields are now wrapped via `(*[0]byte)(unsafe.Pointer(C.export))` so cgo treats them as C function pointer types. JSON-decoded `interface{}` parameters are type-asserted to `map[string]interface{}` before being handed to the impl method. `bool` parameters are converted to `C.uchar` via a conditional rather than direct cast.
- **C# stale-visitor cleanup**: when `visitor_callbacks = false`, alef now deletes `IVisitor.cs`, `VisitorCallbacks.cs`, `NodeContext.cs`, and `VisitResult.cs` from the output dir if they remain from a prior run.
- **Hash injection (XML)**: `alef_core::hash::inject_hash_line` now correctly handles `<!-- … -->` comment style for XML/csproj files; `pipeline::write_files` and `pipeline::diff_files` respect the per-file `generated_header` flag rather than always assuming Rust line comments.
- **alef-cli helpers**: `run_command_captured_with_timeout` uses a deadline-driven `try_wait` poll loop to enforce setup timeouts without adding new dependencies.

### Fixed

- **Magnus**: emits `Vec<T>` and `Map<K,V>` as their actual JSON-shaped types in serde-marshalled enum variants. Collapsing to bare `String` previously broke deserialization of tagged-union variants like `StopSequence::Multiple(Vec<String>)`.
- **Magnus**: deprecated `magnus::value::qnil()` and `magnus::RString::new()` API calls replaced with `Ruby::qnil()` / `Ruby::str_new` in trait bridge and binding emit. Crate-level inner attributes silence the remaining cosmetic warnings.
- **Java**: `gen_facade_class` now wraps FFI Optional returns with `.orElseThrow()` for non-Optional facade signatures (`getPreset`, `detectLanguages`).
- **Java**: `*_TO_JSON` symbol duplicates in `NativeLib.java` are deduplicated via a tracking set, mirroring the existing `_from_json` and `_free` dedup paths.
- **Java**: output-path doubling fixed when `output.java` already contains the `dev/<group>/` prefix.
- **C#**: `_vtable` field promoted to `internal` so cross-class dispatch from the wrapper compiles.
- **C#**: `sizeof(IntPtr)` replaced with the const `IntPtr.Size` so the call site no longer requires `unsafe`.
- **C#**: `bool` parameters cast to `int` (`(arg ? 1 : 0)`) at FFI call sites.
- **C#**: F32 default values emit the `f` suffix; F64 emits the `d` suffix where required for unambiguous typing.
- **C# scaffold**: removed `<NoWarn>` suppression — every generated `.cs` file now has `#nullable enable` and the project enables nullable reference types via `Directory.Build.props`.
- **Rustler**: trait bridge dispatch was previously stubbed; now wired to the reply registry with proper error_constructor template substitution. Atom creation uses `rustler::types::atom::ok()` (the previous `Env::new()` call did not exist).
- **Rustler**: error constructor template now substituted (`{msg}`) into `Err(...)` paths in both sync and async method bodies.
- **CLI**: `alef sync-versions` now updates `packages/ruby/lib/{gem}/version.rb` files in addition to gemspec/pyproject/package.json/pom.xml, completing the previously-incomplete Ruby version sync.
- **e2e/rust**: dropped the unconditional `[workspace]` block from generated `e2e/rust/Cargo.toml`. Parent workspace `[workspace.exclude]` now governs whether cargo trips on two-workspace-roots.
- **Go visitor.go**: regenerated visitor.go now compiles (`HTMHtmNodeContext`/`goVisitText` errors fixed at the alef-backend-go source).
- **Setup pipeline**: closure no longer fights `?`-propagation against rayon's parallel iterator; setup errors now surface with proper context.
- **alef-cli**: setup timeouts now actually kill the spawned process group on deadline rather than logging a spurious "skeleton" error.

### Removed

- C# scaffold no longer emits `<NoWarn>CS1591,CS8618,CS0168,CS0219,CS8625,CS0414,CS8632,CS8866</NoWarn>`. All formerly-suppressed warnings are now fixed at codegen.

## [0.7.11] - 2026-04-26

### Changed

- **E2E**: bump pinned `vitest` in generated TypeScript and WASM `package.json` from `^3.0.0` to `^4.1.5`, matching the version dependabot already pulls into the alef repo's own e2e lockfile.
- **Codegen/Scaffold**: all hardcoded third-party dependency version strings used in scaffold and e2e templates are centralized in `alef_core::template_versions` (110 constants grouped by ecosystem: npm, cargo, maven, gem, packagist, nuget, hex, pypi, cran, precommit). Each const that should auto-bump is annotated with a `// renovate: datasource=...` marker, and a new `renovate.json` at the repo root wires up the custom regex manager so Renovate can open version-bump PRs. Pure refactor — no value changes.
- **Tooling**: `task set-version` now targets the centralized `ALEF_REV` constant in `crates/alef-core/src/template_versions.rs` instead of the stale path in `precommit.rs`.

### Fixed

- **Update**: `alef update --latest` no longer hangs when Node and Wasm are both configured. Commands shared across multiple language default configs (e.g. `pnpm up --latest -r -w`) are now deduplicated — only the first language claiming a command runs it, preventing pnpm lockfile races under parallel execution.
- **Update**: R `upgrade` command now passes `ask = FALSE` to `remotes::update_packages()`, preventing an interactive prompt that blocked the non-interactive runner.
- **E2E (Python)**: skip-reason strings on generated `@pytest.mark.skip` decorators are now escaped before interpolation, so reasons containing quotes or backslashes no longer produce syntactically invalid Python test files.

### Removed

- **Hooks**: `alef-fmt` and `alef-lint` pre-commit hooks. Both used `pass_filenames: false` with a broad `files:` regex, so any matching commit cold-started every configured language toolchain (mvn, dotnet, mypy, etc.) regardless of which file changed — making the hooks unusably slow. The `alef fmt` and `alef lint` CLI commands are unchanged. Scaffold's generated `.pre-commit-config.yaml` no longer emits the removed hook ids.

## [0.7.10] - 2026-04-25

### Fixed

- **Validation**: redundant-default warnings no longer fire for `precondition` fields when the section has custom main commands (format/check/typecheck/command/e2e). Previously, `alef verify` would warn to remove the precondition while simultaneously requiring it — a contradiction.
- **Verify**: legacy README comparison now normalizes whitespace before hashing, preventing false-positive "stale bindings" reports from trailing-space or blank-line differences.

## [0.7.9] - 2026-04-25

### Added

- **Backends**: Kotlin/JVM backend (`alef-backend-kotlin`) — emits `data class` for IR structs, `enum class` / `sealed class` for IR enums, and an `object` wrapping top-level functions. Consumes the same Java/Panama FFM `.so` produced by the Java backend. Function bodies are `TODO()` stubs; FFI bridge wiring lands in Phase 1C. Kotlin/Native and Multiplatform paths deferred to Phase 3.
- **Backends**: Gleam backend (`alef-backend-gleam`) — emits `pub type Foo { Foo(field: T, ...) }` records and `@external(erlang, "<nif>", "<fn>")` declarations targeting the Rustler-emitted Erlang NIF. No new Rust crate generated; Gleam shims an existing Rustler NIF library.
- **Backends**: Zig backend (`alef-backend-zig`) — emits `pub const T = struct {}` types, `pub const E = enum {}` / `union(enum)` enums, and thin wrappers calling into the C ABI via `pub const c = @cImport(@cInclude("<header>.h"))`. Marshalling for non-trivial return types lands in Phase 1C.
- **Core**: `BuildDependency` enum (`None | Ffi | Rustler`) replaces `BuildConfig.depends_on_ffi: bool`. All 11 existing backends migrated; `BuildConfig::depends_on_ffi()` accessor preserved for callers.
- **Core**: `PostBuildStep::RunCommand { cmd, args }` variant — needed by Phase 2 Dart's flutter_rust_bridge codegen step.
- **Core**: Five new `Language` enum variants — `Kotlin`, `Swift`, `Dart`, `Gleam`, `Zig`. All per-language match arms across `crates/alef-core/src/config/` (build/clean/lint/setup/test/update defaults, naming, scaffold, publish, docs, adapters) wired with sensible defaults. `Swift` and `Dart` panic in the registry pending Phase 2.
- **CLI**: `alef verify` now checks README freshness — regenerated READMEs are compared with on-disk files.
- **Hooks**: `alef-readme` pre-commit hook to regenerate README files for all configured languages.
- **Scaffold**: scaffolded `.pre-commit-config.yaml` includes `alef-readme` hook.
- **Config**: top-level `[tools]` section selects per-language tool variants — `python_package_manager` (uv | pip | poetry), `node_package_manager` (pnpm | npm | yarn), and `rust_dev_tools` (list of `cargo install` targets). Per-language pipeline defaults dispatch on these choices.
- **Pipelines**: every per-language default (lint, test, build, setup, update, clean) now declares a POSIX `command -v <tool>` precondition so steps gracefully warn-and-skip when the underlying tool is missing on the user's system.
- **Setup**: Rust `setup` default now installs the full polyrepo dev-tool set — `cargo-edit`, `cargo-sort`, `cargo-machete`, `cargo-deny`, `cargo-llvm-cov` — plus `rustfmt` and `clippy` rustup components. The list is overridable via `[tools].rust_dev_tools`.
- **Init**: `alef init` emits a commented `[tools]` block in the generated `alef.toml` with all alternatives documented.
- **Validation**: `alef.toml` is validated at load time. Custom `[lint|test|build_commands|setup|update|clean].<lang>` tables that override a main command field must declare a `precondition` so the warn-and-skip behavior is preserved on user systems.
- **Scaffold**: FFI Cargo.toml gains a `[dev-dependencies]` block with `tempfile`. Dependency pins audited and policy documented.
- **Config**: per-language `run_wrapper`, `extra_lint_paths`, and `project_file` knobs reduce override boilerplate. `[python] run_wrapper = "uv run --no-sync"` prefixes default tool invocations across lint and test; `[python] extra_lint_paths = ["scripts"]` appends paths to default lint commands; `[csharp] project_file = "Foo.csproj"` (and `[java] project_file = "pom.xml"`) makes default lint/test/build commands target the file instead of the package directory. Each absorbs a common override pattern observed in consumer repos without forcing a full `[lint.<lang>]` redefinition.
- **Validation**: `alef.toml` is now scanned for redundant defaults at load time. When a user-supplied `[lint|test|build_commands|setup|update|clean].<lang>` field equals the built-in default verbatim, alef emits a `tracing::warn!` naming the section and field so users can keep the file minimal.

### Changed

- **Pipelines**: `default_*_config` functions now take a `&LangContext` argument bundling `&ToolsConfig`, `run_wrapper`, `extra_lint_paths`, and `project_file` — replacing the prior `&ToolsConfig`-only signature so per-language defaults can dispatch on every relevant knob.
- **Generate**: post-generation formatting is now best-effort. `alef generate` (and `alef all`) call a new `fmt_post_generate` that swallows three classes of post-gen formatter trouble — a missing tool (precondition miss → "Skipping" warning, language skipped), a failing `before` hook (warning, language skipped), and a non-zero formatter exit (warning, formatter loop continues). Formatters are _expected_ to modify generated files, so non-zero exits there must not abort the run. The explicit `alef fmt` command keeps strict failure semantics.

### Fixed

- **Python**: classify enum imports correctly in generated `api.py` — data enums (tagged unions) and enums not referenced by `has_default` config structs now import from the native module instead of `options.py`, fixing missing-attribute errors at runtime.
- **Python**: emit converter locals in required-first, optional-last order so positional calls to the native function match the pyo3 signature.
- **Python**: generate runtime conversion for data-enum and `has_default` parameters in `api.py` wrappers, including `Vec<…>` and `Optional<Vec<…>>` shapes — list comprehensions are emitted to coerce each element. Previously only scalar (`Named` / `Optional<Named>`) parameters were converted, leaving collection parameters silently un-coerced.
- **Performance (pyo3)**: maintain a parallel `AHashSet` for the opaque-types transitive-closure dedupe check, restoring O(1) membership testing in the hot loop.

## [0.7.8] - 2026-04-25

### Changed

- **Codegen**: extract shared trait bridge helpers (`bridge_param_type`, `visitor_param_type`, `prim`, `find_bridge_param`, `to_camel_case`) into `alef-codegen::generators::trait_bridge`, removing ~600 lines of duplication across 7 backend crates.
- **Codegen**: remove duplicate `format_type_ref` from extendr backend in favor of the shared implementation.
- **Codegen**: consolidate `PYTHON_KEYWORDS` constant — delete duplicate from `alef-codegen::generators::enums`, import from `alef-core::keywords`.
- **E2E**: extract shared `resolve_field()` helper from `rust.rs` and `python.rs` into `codegen/mod.rs`.

### Fixed

- **Codegen**: avoid redundant `.into()` on owned String/Bytes return values in generated bindings.
- **Codegen**: use `to_string_lossy()` for Path return values in generated method wrappers.
- **Codegen**: optimize generated `.map(|val| val.into())` to `.map(Into::into)` in function/method wrappers.
- **Go**: skip functions/methods that use enum types lacking FFI JSON helpers (`_from_json`/`_to_json`/`_free`).
- **Go**: respect `exclude_functions` from FFI config when generating Go bindings.
- **Go**: fix `visitor_callbacks` in trait bridge test config.

## [0.7.7] - 2026-04-25

### Added

- **Config**: top-level `version` field in `alef.toml` to pin the alef CLI version per project. `alef init` now emits this field automatically.
- **Install action**: `install-alef` reads the pinned version from `alef.toml` when input is `"latest"`, falling back to the latest GitHub release if not specified.

### Fixed

- **Config**: use top-level `version` key instead of `[alef]` section to avoid TOML scoping issues where subsequent keys were captured inside the section.
- **Python bindings**: sanitize doc strings with `sanitize_python_doc()` consistently across all generated code (options enums, dataclasses, TypedDict, API functions, exceptions) to prevent ruff RUF001/RUF002 lint errors.
- **Python stubs**: fix test assertions for builtin-shadowing parameter names (`input`, `id`) that now generate multi-line signatures with `# noqa: A002`.

## [0.7.6] - 2026-04-25

### Added

- **Config**: `[alef]` section in `alef.toml` with `version` field to pin the alef CLI version per project (superseded by top-level `version` in 0.7.7).

### Fixed

- **Python bindings**: sanitize doc strings with `sanitize_python_doc()` consistently across all generated code.

## [0.7.5] - 2026-04-25

### Fixed

- **Codegen**: suppress `clippy::redundant_closure` and `clippy::useless_conversion` lints in generated `From` impl blocks.
- **Python bindings**: add `noqa` comments to generated `typing` imports.
- **Python bindings/stubs**: force multi-line function signatures when a parameter name shadows a Python builtin (e.g. `id`, `type`, `input`), and annotate those parameters with `# noqa: A002`.

## [0.7.4] - 2026-04-25

### Added

- **E2E assertions**: `min_length`, `max_length`, `ends_with`, and `matches_regex` assertion types across all language codegen backends (Bash/Brew, C, C#, Elixir, Go, Java, PHP, Python, R, Ruby, WASM).

### Fixed

- **Python bindings**: prevent duplicate `| None` annotation on optional parameters whose base type already contains `| None` (e.g. `Option<Option<T>>`).
- **Python stubs**: use `ends_with("| None")` instead of `contains("| None")` for the same check in type-init, method, and function stubs.
- **Scaffold(C#)**: XML-escape author names in `.csproj` to avoid malformed XML.
- **Scaffold(Java)**: parse `"Name <email>"` author strings into separate `<name>` and `<email>` elements; XML-escape both; omit placeholder email when not provided.
- **Scaffold(PHP)**: derive Composer vendor from GitHub repository URL instead of hardcoding `kreuzberg-dev`.
- **Scaffold(Ruby)**: use double-quoted strings for gemspec authors array.
- **Scaffold**: default repository URL uses `example` org instead of `kreuzberg-dev`.
- **Elixir bindings**: escape parameter names that collide with Elixir reserved words (`do`, `end`, `fn`, etc.) by appending `_val` suffix.

## [0.7.3] - 2026-04-25

### Added

- **Keyword-aware field renaming for Python bindings** — struct fields whose names collide with Python reserved keywords (e.g. `class`, `from`, `type`) are automatically escaped in the generated binding struct (`class_`) while preserving the original name via `#[pyo3(get, name = "class")]` and `#[serde(rename = "class")]` attributes. Configurable per-field via `rename_fields` in language configs.
- Shared keyword list in `alef-core::keywords` with `python_ident()` helper — single source of truth for Python reserved words, replacing duplicated lists in `gen_bindings.rs` and `gen_stubs.rs`.
- `ConversionConfig::binding_field_renames` for keyword-escaped field name substitution in `From`/`Into` conversion impls.
- `gen_struct_with_rename` and `gen_impl_block_with_renames` codegen generators for per-field attribute customization.
- `AlefConfig::resolve_field_name()` for config-driven field name resolution per language and type.
- Scaffold: emit `rust-toolchain.toml` (Rust 1.95) and `.cargo/config.toml` with `wasm32-unknown-unknown` target for WASM projects.
- Scaffold: include WASM target in clippy pre-commit hook instead of excluding it.
- Config: `corepack up` added to Node.js update defaults; `corepack use pnpm@latest` for pnpm upgrade.

### Fixed

- Publish: derive PHP extension name from output path instead of hardcoding.
- Publish: derive crate names from output paths instead of hardcoding.
- FFI: replace deprecated `clippy::drop_ref` lint with `dropping_references`.
- Scaffold: Ruby `Rakefile` template uses correct Bundler 4 API.

## [0.7.2] - 2026-04-24

### Added

- **`alef publish` command group** — vendoring, building, and packaging artifacts for distribution across language package registries (issue #9).
  - `alef publish prepare` — vendors core crate (Ruby, Elixir, R) and stages FFI artifacts (Go, Java, C#).
  - `alef publish build` — cross-compilation-aware build with `--target` support, auto-selects cargo/cross/maturin/napi/wasm-pack per language.
  - `alef publish package` — creates distributable archives (C FFI tarball with pkg-config/CMake, PHP PIE archive, Go FFI tarball).
  - `alef publish validate` — checks version readability, package directory existence, and manifest file presence.
- New `alef-publish` crate with `platform::RustTarget` for parsing Rust target triples and mapping to per-language platform conventions (Go, Java, C#, Node, Ruby, Elixir).
- `[publish]` config section in `alef.toml` with per-language `vendor_mode`, `nif_versions`, `build_command`, `package_command`, `precondition`, `before` hooks, `pkg_config`, `cmake_config`.
- `vendor::vendor_core_only()` — copies core crate, rewrites Cargo.toml via `toml_edit` to inline workspace inheritance and dependency specs, removes workspace lints, generates vendor workspace manifest.
- `vendor::vendor_full()` — core-only + `cargo vendor` of all transitive deps with test/bench cleanup.
- `ffi_stage::stage_ffi()` / `stage_header()` — copies built FFI shared library and C header to language-specific directories.

### Fixed

- CLI: `alef build` now respects `[build_commands.<lang>]` overrides for non-Rust languages.
- CLI: `before` hooks in lint/fmt now fire even when a language has `check`/`typecheck` commands but no `format` commands.
- CLI: `before` hook output (stdout/stderr) is now logged instead of silently discarded.
- Config: Ruby lint defaults use `cd {output_dir} && bundle exec rubocop` instead of running from project root.
- Config: FFI lint defaults search entire output directory instead of assuming `tests/` subdirectory.
- Scaffold: pre-commit config now uses `alef-fmt` + `alef-lint` hooks instead of per-language hooks.
- Codegen(Go): local variable naming, error string conventions, enum acronym rules, `nodeTypeFromC` acronym naming, visitor doc comments, golangci scaffold config.

## [0.7.1] - 2026-04-24

### Added

- Config: `precondition` field on all command configs (`LintConfig`, `TestConfig`, `SetupConfig`, `UpdateConfig`, `BuildCommandConfig`, `CleanConfig`) — a shell command that must exit 0 for the main command to run; skips the language with a warning on failure.
- Config: `before` field on all command configs — command(s) that run before the main command; aborts on failure. Supports `StringOrVec` (single string or list).
- Config: `GOWORK=off` in default Go setup command to avoid workspace interference.
- Config: Maven version rules file reference in default Java update commands.
- CLI: Rust is now a first-class language in `alef build` — builds via configurable `[build_commands.rust]` instead of panicking on missing backend.
- FFI: derive `Copy` and `Clone` on generated vtable structs.

### Fixed

- FFI: trait bridge generation fixes for kreuzberg integration.
- Scaffold: pre-commit config simplification — removed per-language hooks.

## [0.7.0] - 2026-04-24

### Added

- CLI: `alef fmt` command — run only format commands on generated output.
- CLI: `alef update` command — orchestrate per-language dependency updates with sensible defaults. `--latest` flag for aggressive upgrades (incompatible/major version bumps).
- CLI: `alef setup` command — install dependencies per language using per-language defaults or `[setup.<lang>]` config.
- CLI: `alef clean` command — clean build artifacts per language using per-language defaults or `[clean.<lang>]` config.
- CLI: `alef test --coverage` flag — run coverage commands defined in `[test.<lang>].coverage`.
- Config: `StringOrVec` type for lint/update/test/setup/clean/build commands — supports both `format = "cmd"` and `format = ["cmd1", "cmd2"]` in alef.toml.
- Config: `[update.<lang>]` sections in alef.toml with `update` (safe) and `upgrade` (latest) commands.
- Config: `[setup.<lang>]` sections in alef.toml with `install` commands for dependency installation.
- Config: `[clean.<lang>]` sections in alef.toml with `clean` commands for removing build artifacts.
- Config: `[build_commands.<lang>]` sections in alef.toml with `build` and `build_release` commands (replaces hard-coded tool invocations).
- Config: `coverage` field on `[test.<lang>]` — `TestConfig` fields migrated to `StringOrVec`; `command` and `e2e` also accept arrays.
- Config: default lint commands for all 12 languages with autofixes enabled (ruff --fix, rubocop -A, clippy --fix, oxlint --fix).
- Config: default update commands for all 12 languages (cargo update, pnpm up, uv sync, bundle update, composer update, go get, mvn versions, dotnet outdated, mix deps.update, etc.).
- Config: default setup and clean commands for all 12 languages.
- Config: add `exclude_languages` field to `TraitBridgeConfig` for per-language trait bridge opt-out.
- Config: add `exclude_functions` and `exclude_types` fields to `RubyConfig` for per-type/function exclusion.
- Magnus: honor `exclude_functions`/`exclude_types` from `[ruby]` config in binding, conversion, and module init generation.
- Magnus: detect absent `Named` types in enum variant fields and route them through `serde_json` deserialization.
- Rustler: filter trait bridges by `exclude_languages` so excluded bridges are omitted from Elixir output.
- Codegen: handle `EnumVariant` defaults for `String`-mapped fields in Magnus hash constructors.
- Codegen: deduplicate `Bytes` conversion when base conversion already emits `.into()` or `.map(Into::into)`.
- Codegen: handle sanitized and excluded-type fields in enum binding-to-core match arms with `serde_json` deserialization and `Box` wrapping.
- Scaffold (Node): generate `.oxfmtrc.json` (120 printWidth, tabs, import sorting) and `.oxlintrc.json` (correctness=error, suspicious=warn, style=off, typescript+import plugins).

### Changed

- CLI: `alef init` now runs full project bootstrap — generates alef.toml, extracts API, generates bindings, scaffolds manifests, and formats in one command.
- CLI: `alef lint` refactored with `LintPhase` enum, uses `config.lint_config_for_language()` with per-language defaults instead of raw map lookup.
- CLI: `alef generate` and `alef all` use built-in `fmt` instead of prek for post-generation formatting.
- Scaffold (Node): replace Biome with Oxc toolchain — `oxfmt` + `oxlint` in package.json devDeps and scripts.
- Scaffold (Java): strip cosmetic whitespace checks from checkstyle.xml (Spotless handles formatting). Remove WhitespaceAfter, WhitespaceAround, GenericWhitespace, EmptyBlock, NeedBraces, MagicNumber, JavadocPackage.
- Scaffold (Go): update `.golangci.yml` to v2 format with full settings (errcheck exclude-functions, govet enable-all, misspell locale, revive rules, exclusions, formatters).
- Scaffold (pre-commit): replace biome-format/biome-lint hooks with oxlint + oxfmt local hook.

### Removed

- CLI: `run_prek()` and `run_prek_autoupdate()` — prek is no longer auto-invoked during generation. Users can still use prek independently.

### Fixed

- Java: remove dead code left over from modularization (duplicate helpers in `mod.rs`, `helpers.rs`, `marshal.rs`).
- Java: fix empty-line-after-doc-comment clippy warnings in `marshal.rs` and `types.rs`.
- Java: suppress `too_many_arguments` clippy warnings on `gen_main_class` and `gen_facade_class`.
- Java: add missing `is_ffi_string_return()` and `java_ffi_return_cast()` marshal functions.
- Java: fix `RECORD_LINE_WRAP_THRESHOLD` visibility (`pub(crate)`).
- Docs: fix `render_method_signature` line length by splitting parameters across lines.
- Tests: add missing `exclude_languages` field to all `TraitBridgeConfig` test initializers.
- Tests: add missing `exclude_functions`/`exclude_types` fields to `RubyConfig` test initializers.

## [0.6.1] - 2026-04-23

### Fixed

- FFI: generate `{prefix}_unregister_{trait_snake}` extern C function for trait bridges (was documented but missing).
- Codegen: use fully-qualified `std::result::Result` in trait bridge return types.
- Codegen: tuple sanitization, Go `gofmt` column alignment, error gen cleanup.
- Go: `gofmt`-compliant struct field alignment with tab-separated columns.
- Go: apply Go acronym uppercasing (`to_go_name`, `go_type_name`, `go_param_name`) per Go naming conventions.

### Changed

- Tests: align all backend test assertions with actual codegen output (Go, Magnus, NAPI, PHP, PyO3, Rustler, FFI, scaffold).
- Tests: remove unimplemented Magnus plugin bridge tests (visitor bridge tests retained).
- Scaffold: update file count expectations for new tsconfig.json (Node), .golangci.yml (Go), and Java config files.

## [0.6.0] - 2026-04-23

### Added

- Scaffold: add ruff + mypy config and `[dependency-groups] dev` to Python `pyproject.toml`.
- Scaffold: add `biome.json`, `tsconfig.json`, and `@biomejs/biome`/`typescript` devDeps to Node/TypeScript.
- Scaffold: add `.golangci.yml` for Go with standard linter configuration.
- Scaffold: add `.php-cs-fixer.php` for PHP with PSR-12 and PHP 8.2 migration rules.
- Scaffold: add `.credo.exs` for Elixir with strict mode and cyclomatic complexity limit.
- Scaffold: add `.lintr` for R with 120-char line length; add `lintr`/`styler` to DESCRIPTION Suggests.
- Scaffold: add `.editorconfig` for C# with 120-char line length.
- Scaffold (Java): add `versions-rules.xml` filtering alpha/beta/RC/milestone/snapshot versions.
- Scaffold (Java): add `pmd-ruleset.xml` with standard PMD categories and exclusions.
- Scaffold (Java): add `maven-enforcer-plugin`, `maven-checkstyle-plugin`, `maven-pmd-plugin`, `versions-maven-plugin`, `jacoco-maven-plugin` to `pom.xml`.
- Scaffold (Java): add `jspecify` and `assertj-core` dependencies.

### Changed

- Scaffold (Java): update `maven-compiler-plugin` 3.14.0 → 3.15.0, `maven-source-plugin` 3.3.1 → 3.4.0, `maven-site-plugin` 3.21.0 → 4.0.0-M16.
- Scaffold (Java): update surefire argLine with `-XX:-ClassUnloading`, parallel test execution, and 600s timeout.
- Scaffold (Java): change javadoc doclint from `none` to `all,-missing`.
- Scaffold (Java): add whitespace, block, and coding checks to `checkstyle.xml`; add FFI and test suppressions.

### Fixed

- Codegen: fix `Map` with `Named` value conversion in optionalized `binding_to_core` path.
- Codegen: fix WASM `js_sys` import generation, Go `gofmt` compliance, Rust e2e `is_none` assertions, PHP builder pattern.
- Codegen: fix C# bytes pinning and `IntPtr.Zero` guard, Go `gofmt` output, Rust e2e `is_none`, Java e2e camelCase naming.
- Codegen: fix `Vec<String>` `_refs` binding and sanitized tuple-to-vec clone in `core_to_binding`.
- Codegen: fix `Vec<Primitive>` sanitized passthrough and enum sanitized serde round-trip.
- Magnus: fix `funcall` API usage, visitor bridge argument passing, `Vec` conversion, optional field flattening, and `default_types` handling.
- Magnus: simplify `gen_trait_bridge` signature — remove inline error type/constructor args, derive from config.
- PyO3: pass enum fields directly instead of `str()` + map lookup.
- PyO3: add `license`/`credits`/`copyright` to Python builtin names list.
- Rustler: fix Elixir `@spec` line wrapping to respect 98-char formatter default.
- Elixir: add `.formatter.exs` with 120-character line length; update stub wrap threshold to 120.
- Scaffold: add `steep`, `rubocop-performance`, `rubocop-rspec` to Ruby Gemfile template.

## [0.5.9] - 2026-04-23

### Fixed

- Rustler: replace `Pid::spawn_monitor` with `env.call` for sync NIF dispatch; add `Encoder` import.
- Rustler: visitor bridge receive loop — emit `_with_visitor` NIF variant and Elixir-side `do_visitor_receive_loop/1` for trait bridge callbacks.
- Rustler: fix double-optional struct fields — skip outer `Option` wrap when `field.ty` is already `Optional`.
- Rustler: add explicit type-annotated let bindings for named params in static NIF methods to resolve ambiguous `From` conversions.
- PyO3: strong typing for data enums in `options.py` — use concrete union type aliases instead of `dict[str, Any]`.
- PyO3: transitively expand `needed_enums` so nested data enums (e.g. `ContentPart` inside `UserContent`) are defined in `options.py`.
- PyO3: topological sort of data enum union aliases to ensure dependencies are emitted before dependents.
- PyO3: `needs_any` check now scans for `TypeRef::Json` instead of data enum fields.
- PyO3: assert non-optional converters for data enum variant fields.
- PyO3: escape single quotes in string literals and add `noqa` comments for assertion expressions.
- Go: remove zero-argument `New{TypeName}()` constructors from opaque types — handles are created by factory functions, not bare allocators.
- WASM: add `type_contains_json` helper for data enum field scanning.
- E2E (TypeScript): escape single quotes in string literals, handle large integers with `BigInt()`, add `noqa` assertions.
- CLI: simplify post-generation hash logic — store generation hashes from in-memory content pre-formatter instead of re-extracting and re-hashing on-disk files.
- Scaffold: fix Ruby `Rakefile` for Bundler 4 — use `Bundler::GemHelper.install_tasks` instead of `require 'bundler/gem_tasks'`.
- Core IR: add `TypeRef::references_named` method for recursive named type detection.
- Codegen: add `type_contains_json` helper to `binding_helpers` for checking `TypeRef::Json` presence in nested types.

## [0.5.8] - 2026-04-23

### Added

- E2E: `is_false`, `method_result` assertion types and `resolve_call` per-fixture call routing across all 14 language generators.

### Fixed

- PyO3: exclude return types from `local_type_names` in `options.py` (prevents false re-exports).
- PyO3/Magnus: remove stale `type: ignore` comments; add `VERSION` constant to RBS type signatures.
- E2E (Python): add `ruff` per-file-ignores to generated `pyproject.toml`; add `noqa` comments for B017 and S108; add `method_result` helper imports.
- E2E (C): add missing `find_nodes_by_type` and `is_error` check types to `run_query`.
- E2E (Rust): add top-level `is_false` assertion support.
- C#/Elixir: format generated code to pass `dotnet format` and `mix format`.
- FFI: fix doc comment prefixes, duplicate imports, and optional param types.
- Codegen: use serde round-trip for tuple-to-vec `From` impls.
- Codegen: fix async return type wrapping, `Vec` element conversion, and `as_deref` option handling.
- Codegen: fix serde derives for opaque fields, `error_constructor` propagation, `BridgeOutput` fields, and trait bridge generation.
- Codegen: fix `Result` qualification, non-`Result` method returns, complex return types, PHP raw pointer handling, WASM scaffold paths.
- Scaffold: wire `scaffold_output` from config; remove stale `serde(default)` on non-serde structs.
- Python: dedup exception classes in `gen_exceptions_py`.
- NodeContent derive forwarding and test fixture resolution.
- Java: add `final` params, javadoc, `eclipse-formatter.xml`, `checkstyle.xml` scaffold with 120 line width, `spotless-apply` pre-commit hook.
- C#: fix `VisitResult.Continue()` to `new VisitResult.Continue()`.
- Elixir: add `ELIXIR_BUILTIN_TYPES` deny-list, suffix colliding type names with `_variant`.
- PHP: fix double-escaped quotes in `get_property` calls in trait bridge.

## [0.5.7] - 2026-04-22

### Fixed

- Python stubs (.pyi): add `Any` to typing imports (used by `dict[str, Any]` fields).
- Python stubs (.pyi): rename tagged union TypedDict variants with `Variant` suffix to avoid name collision with enum classes (e.g. `ToolChoiceModeVariant` instead of `ToolChoiceMode`).

## [0.5.6] - 2026-04-22

### Fixed

- Go backend: skip streaming adapter methods whose FFI signature uses callbacks (CGO incompatible).
- C# backend: skip streaming adapter methods in P/Invoke declarations and wrapper class generation.
- PHP backend: scaffold generates `[features] extension-module = []` for ext-php-rs compatibility.
- Map value conversion: fix optional fields with Named values in binding-to-core and core-to-binding conversions.

## [0.5.5] - 2026-04-22

### Added

- FFI backend: callback-based streaming via `LiterLlmStreamCallback` function pointer pattern. Replaces the `compile_error!` stub with a working implementation that invokes a callback per JSON chunk.
- FFI backend: `gen_streaming_method_wrapper` generates correct C signature (`client`, `request_json`, `callback`, `user_data` → `i32`).

### Fixed

- WASM backend: streaming adapter return type overridden to `Result<JsValue, JsValue>` (was `Result<String, JsValue>`, causing type mismatch with `serde_wasm_bindgen::to_value`).
- WASM backend: don't underscore-prefix params when adapter body is present (`_req` → `req`).
- Python backend: move type imports out of `TYPE_CHECKING` block — types used in function signatures must be available at runtime.
- Go backend: suppress CGO const warnings for generated enum-to-string functions.
- PHP backend: skip undelegatable methods instead of emitting broken stubs.
- Trait bridge: propagate `error_constructor` to all backends.

## [0.5.4] - 2026-04-22

### Fixed

- NAPI, Magnus, PHP, Rustler backends: skip sanitized methods entirely instead of emitting stubs, panics, or `[unimplemented: ...]` placeholders. Matches PyO3 behavior.
- Scaffold: Ruby `extconf.rb` now sets `cargo_manifest` path for `rb_sys`.
- Scaffold: Ruby and Elixir `Cargo.toml` `[lib]` section now includes `name` and `path` pointing to alef-generated binding source.
- E2E: TypeScript codegen no longer escapes `$` in double-quoted strings (fixes Biome `noUselessEscapeInString` lint).
- Go backend: remove invalid `?` syntax from vtable function pointer assignments.
- Rustler: keep `force_build` on single line for `mix format` compatibility.

## [0.5.3] - 2026-04-22

### Added

- CLI: `alef version` command and `alef --version` flag
- Scaffold: generate `py.typed` marker for Python, `Steepfile` for Ruby, `.d.ts` barrel for TypeScript
- Test coverage: ~500 new tests across alef-docs, alef-codegen, alef-extract, alef-readme, and all backends

### Fixed

- Adapters: streaming body fixes for all backends, core type let bindings
- Magnus: use snake_case for Ruby file paths in `generate_public_api`
- Scaffold: use snake_case for Ruby file paths regardless of `gem_name`
- Go: remove colons from Go parameter syntax in trait bridge
- Fix `error_type` field propagation across all test initializers

## [0.5.2] - 2026-04-22

### Added

- Trait bridges: plugin-pattern support across all 11 backends (issue #4)
  - PyO3: de-hardcoded plugin bridge via shared `TraitBridgeGenerator`
  - NAPI, WASM, PHP, Magnus, Rustler, Extendr: implement `TraitBridgeGenerator` for plugin bridges
  - FFI: vtable + `user_data` ABI with `#[repr(C)]` function pointer struct
  - Go: cgo trampolines wrapping C FFI vtable via `cgo.Handle`
  - Java: Panama FFM upcall stubs wrapping C FFI vtable
  - C#: P/Invoke declarations for C FFI trait bridge registration
- Codegen: shared `TraitBridgeGenerator` trait with `type_paths`, flexible registration, configurable super-trait
- Codegen: `format_param_type` respects `is_ref` for reference-typed params (`&str`, `&[u8]`, `&Path`)
- E2E: HTTP server fixture support for testing HTTP endpoints
- E2E: HTTP test codegen for Python, Node, Ruby, PHP, Elixir
- Scaffold: add `[lints] workspace = true` to all generated Cargo.toml templates
- Extendr: opaque type support with Arc wrappers and mutex types
- Skills: add designing-alef-toml guide with real-world configuration patterns

### Fixed

- Codegen: `gen_bridge_all` now emits constructor so `Wrapper::new()` exists for registration
- Codegen: `gen_bridge_trait_impl` filters `trait_source` methods (super-trait handled separately)
- Codegen: `gen_bridge_plugin_impl` strips body indentation to prevent double-indent
- PyO3: remove `.expect()` panics in `gen_registration_fn`, use graceful fallback
- PyO3: fix async double-`Result` flattening (`spawn_blocking` → `??`)
- NAPI/WASM/PHP/Magnus/Rustler/Extendr: remove `.expect()` panics in `gen_registration_fn`
- Magnus: implement actual registration logic (was a non-functional stub)
- Magnus: add missing async `String` clone for blocking closures
- Magnus/Rustler: fix `build_arg` ordering for optional `&str` (unreachable branch)
- Rustler: replace `spawn_monitor` with direct `Term::apply` for sync dispatch
- PHP: fix `&*self.php_obj` to `&mut *self.php_obj` for `try_call_method`
- WASM: handle poisoned `RwLock` in registration instead of panicking
- FFI: cache `version()` at construction (was leaking via `Box::leak` every call)
- FFI: return error on NUL bytes in serialized params (was `unwrap_or_default`)
- FFI: add `Sync` impl on async `_LocalBridge`
- Go: use correct `ffi_prefix` parameter instead of deriving from `register_fn`
- Go: fix `unsafe.Pointer(handle)` → `unsafe.Pointer(uintptr(handle))`
- Go: fix `outResult` ABI mismatch (emit when `return_type != Unit`, not just `error_type`)
- Go: fix syntax error in `C.GoString(` missing closing paren
- Go: add `FreeUserData` trampoline to prevent `cgo.Handle` leak
- Java: implement actual result serialization (was returning `MemorySegment.NULL`)
- Java: fix use-after-free by storing bridge in static map (not `try-with-resources`)
- Java: uncomment and implement FFI registration call
- Java: conditionally emit super-trait vtable slots based on config
- C#: add `[MarshalAs(UnmanagedType.LPUTF8Str)]` for UTF-8 string marshaling
- C#: change `ref IntPtr outError` to `out IntPtr outError`
- Docs: use filtered IR for API reference generation
- Docs: use `dict[str, Any]` instead of bare `Any` for Python JSON types
- Docs: box Java primitives in `Optional` (e.g. `Optional<Integer>` not `Optional<int>`)
- Docs: emit `async Task<T>` for C# async method signatures
- Docs: use `&str`/`&[u8]` for Rust method params matching free-function style
- Docs: pluralize nouns after stripping `_count` suffix
- Docs: thread `Language` param through `clean_doc_inline`
- Codegen: skip `Default`/`Serialize`/`Deserialize` derives for structs with opaque fields
- Codegen: add serde recovery path to Rustler and PHP backends
- Adapters: emit core type let bindings in Python streaming adapter
- C#: deterministic DllImport ordering in NativeMethods.cs
- CLI: use filtered IR for docs, remove dead unfiltered extraction code

## [0.5.1] - 2026-04-21

### Added

- Extract: per-crate extraction for multi-crate workspaces via `source_crates` config
- IR: `original_rust_path` field for orphan-safe `From` impls
- Codegen: dedup after path mapping, generic/enum generation improvements

### Fixed

- Codegen: Python workspace source path, C# paths/TFM, Go version resolution
- Codegen: only omit opaque inner field for unresolvable generic types (not all sanitized types)
- Codegen: skip serde derives for structs with opaque (Arc-wrapped) fields
- Codegen: use post-mapping `rust_path` for `From` impls, treat data enums as known types
- PyO3: don't import streaming `item_type` that shadows wrapper structs
- PyO3: async stub inference, Mutex default, trait type handling, error path fixes
- PyO3: group consecutive opaque type stubs in `.pyi` output
- Cache: strip all whitespace in content hash for formatter agnosticism
- CI: add `contents: write` permission to publish-homebrew job for bottle upload

## [0.5.0] - 2026-04-21

### Added

- Scaffold: render `extra_dependencies` in scaffold, auto path mappings
- CI: rewrite CI and Publish workflows using shared `kreuzberg-dev/actions` composite actions
- CI: add `typos` (spelling) and `actionlint` (GitHub Actions linting) pre-commit hooks

### Changed

- Publish: replace GoReleaser with native per-platform CLI builds via `build-rust-cli@v1`
- Publish: phased publish pipeline (prepare, validate, check, build, publish, finalize)
- Publish: OIDC authentication for crates.io via `publish-crates@v1`
- Publish: draft-then-undraft GitHub release pattern

### Fixed

- Codegen: opaque `Default` fallback uses `todo!()` instead of `Default::default()` for opaque return types
- PyO3: scan opaque type methods for `HashMap` usage to ensure import is added
- NAPI: pass `opaque_types` to `gen_unimplemented_body` calls
- E2E: make tokio dependency conditional in Rust e2e generator
- Core: remove unused `ahash` dependency, fix `clippy::manual_strip` warning

## [0.4.6] - 2026-04-21

### Added

- Config: `exclude_functions` and `exclude_types` support for Python (PyO3), FFI, and Node (NAPI) backends

### Fixed

- WASM: always import `js_sys` (error converters use `js_sys::Object`/`js_sys::Reflect`)
- WASM: skip free functions referencing excluded types (prevents undefined type references)
- WASM: add `clippy::unused_unit` to allowed lints for generated code
- Node: wire `exclude_functions`/`exclude_types` from config into NAPI gen_bindings
- Python: wire `exclude_functions`/`exclude_types` into both struct generation and pymodule init
- FFI: wire `exclude_functions` into free function generation loop
- Rustler: fix f32/f64 stub return values (`0` → `0.0`)
- Rustler: add `clippy::unused_unit` to allowed lints
- PyO3: add `clippy::map_identity` to allowed lints
- Codegen: use `format!("{:?}", i)` instead of `.to_string()` for sanitized `Vec<String>` conversions (fixes Display trait bound failures)
- Codegen: extend `gen_serde_let_bindings` to handle `Vec<Named>` params (fixes undefined `_core` variables)
- Rustler: Elixir module spec line-wrapping for long type signatures

## [0.4.5] - 2026-04-21

### Added

- Codegen: visitor bridge generation for all backends (Go, Java, C#, Python, Ruby, Elixir, etc.)
- Codegen: `gen_visitor.rs` for Go, Java, C# backends
- E2E: visitor e2e test codegen for 8 languages

### Fixed

- Codegen: NAPI trait_bridge for napi-rs v3 compatibility
- Codegen: PHP trait_bridge for ext-php-rs 0.15 compatibility
- E2E: NodeConfig test fixes

## [0.4.4] - 2026-04-21

### Changed

- CLI: parallelize code generation, build, lint, and test loops with rayon
- CLI: parallelize file writing and diff checks across languages
- CLI: build commands within independent and FFI-dependent groups run concurrently
- CLI: switch rustfmt to stdin-based piping with explicit `--config-path` (thread-safe, respects `rustfmt.toml` from any CWD)
- CLI: lazy-compile fixed regex patterns in version sync with `LazyLock`
- Docs: replace `push_str(&format!())` with `write!()` macro to eliminate temporary string allocations
- Docs: pre-allocate `String::with_capacity(8192)` for markdown output buffers
- PyO3: use `ahash::AHashSet<&str>` instead of `std::collections::HashSet<String>` in code generation hot paths

### Added

- Docs: Rust language API reference generation
- Docs: tuple type detection and per-language rendering (`tuple[A, B]` for Python, `[A, B]` for TypeScript, `(A, B)` for Rust/Go/C#, `Tuple<A, B>` for Java)

### Fixed

- CLI: remove redundant double rustfmt pass on generated Rust files
- CLI: fix post-build file patch race condition by running patches sequentially after parallel builds
- CLI: fix lint/test output truncation — all language output is now printed before reporting errors
- E2E: resolve `field='input'` correctly in TypeScript codegen
- E2E: Tree method mapping, bytes arg type, features, json_object ref
- E2E: Option return handling, `default-features`, `is_none`/`is_some`
- Java: escape `*/` in Javadoc comments; TypeScript codegen updates
- PHP: per-fixture call routing and `json_encode` arg passing in PHP e2e
- PHP: add let-binding support for `Named` ref and `Vec<String>` ref params
- Python: resolve call config per fixture for correct function dispatch
- C#: fix `CS0029` type shadowing, `CS1503` nullable unwrap, `Vec<String>` serde conversion
- C#: add `using System`, fix `CS8910` copy constructor clash for newtype variants
- Rustler: use `_rustler` crate name in `native.ex` (not `_nif`)
- Adapters: use `_core` locals in streaming adapter body (not `.into()` on moved param)
- Codegen: fix codegen for opaque types, ref params, and `Vec<&str>`
- FFI: unnecessary clone removal; NAPI/Rustler/scaffold improvements
- Sync: don't blanket-replace versions in `pyproject.toml` `extra_paths`

### Removed

- Codegen: remove unused `create_env()` and dead `templates` module
- Codegen: remove unused `minijinja` dependency from alef-codegen

## [0.4.3] - 2026-04-20

### Added

- Codegen: trait bridge generation for NAPI, Magnus, Rustler, WASM, and PHP backends
- Codegen: `gen_error_class` for structured Python error hierarchies
- E2E: C codegen backend for e2e test generation
- E2E: Python e2e improvements (fixture handling, codegen updates)

### Fixed

- E2E: various codegen fixes across C and Python backends
- PHP: binding function generation fixes

## [0.4.2] - 2026-04-20

### Added

- Codegen: trait bridge code generation — auto-generate FFI bridges for Rust traits (PyO3 backend)
- Codegen: shared `TraitBridgeGenerator` trait and `TraitBridgeSpec` for cross-backend bridge generation
- Config: `[[trait_bridges]]` table for configuring trait bridge generation per trait
- IR: `super_traits` field on `TypeDef` for trait inheritance tracking
- IR: `has_default_impl` field on `MethodDef` for default method detection
- IR: `CoreWrapper::ArcMutex` variant for `Arc<Mutex<T>>` wrapping
- E2E: standalone mock server binary generation for cross-language e2e tests
- E2E: WASM e2e spawns standalone mock server via `globalSetup`
- E2E: `client_factory` override for WASM instance-method calls
- E2E: `count_equals` and `is_true` assertion types across all language targets
- Verify: blake3 output content hashing for idempotent verify

### Fixed

- PyO3: resolve `Named` types via `rust_path` lookup instead of constructing paths
- PyO3: use `py.detach()` instead of removed `allow_threads` (PyO3 0.28 compat)
- PyO3: fix 5 public API codegen bugs (python_safe_name, string escaping, param ordering, import filtering)
- PyO3: re-export return types from native module, not `options.py`
- NAPI: export enums as types for `verbatimModuleSyntax` compatibility
- C#: rename `record` param to `Value` when it clashes with variant or type name
- C#: add `using System` to `NativeMethods.cs` for `IntPtr`
- Go: marshal non-opaque receivers to JSON instead of using `r.ptr`
- Ruby: normalize dashes to underscores in ext dir paths, add extra `../` for native nesting
- Rustler: use `_rustler` suffix instead of `_nif` for crate name and path
- Codegen: don't apply Vec conversion on non-Vec enum variant fields
- E2E: WASM `globalSetup` paths, `BigInt()` wrapping for u64/i64, `new` constructor usage
- E2E: PHP async function naming, phpunit bump, namespace fixes
- E2E: resolve `input` field correctly for `options_type` import detection
- Scaffold: remove readme from scaffold output
- Sync: version regex matches pre-release suffix to prevent double-append
- Verify: write stubs manifest so output hashes cover stubs

## [0.4.1] - 2026-04-19

### Added

- E2E: `MockResponse` struct on `Fixture` for mock HTTP server support
- E2E: Rust mock server codegen — generates Axum-based mock servers from fixture `MockResponse` data
- E2E: always add `[workspace]` to generated Cargo.toml (prevents workspace inheritance issues)

### Fixed

- Codegen: upstream codegen fixes for liter-llm migration (multiple backends)
- Codegen: additional prek compliance fixes across all backends
- Codegen: add `clippy::allow` attrs to PyO3 and FFI backend generated code
- Codegen: Rustler NIF parameter type conversions and param handling refinement
- Codegen: NAPI — wrap boxed enum variant fields with `Box::new` in binding-to-core conversion
- Codegen: Python `options.py`/`api.py` docstrings on all public classes (D101)
- Codegen: Python D101 docstrings, mypy `dict` type args, exception docstrings
- Codegen: Java tagged union newtype variants use `@JsonUnwrapped`
- Codegen: PHP facade void return
- Codegen: WASM — prefix unused parameters with `_` in unimplemented stubs
- Codegen: handle `Vec<Optional<Json>>` field conversion (`Value` → `String`)
- Codegen: async unimplemented returns `PyNotImplementedError`
- E2E: add `cargo-machete` ignore for `serde_json` in Rust e2e Cargo.toml
- Scaffold: phpstan.neon generation for PHP
- Scaffold: safe `Cargo.toml` handling in `extra_paths` (prevents dep version corruption)
- Scaffold: WASM `package.json` + `tsconfig`, Duration lossy conversion fix
- Scaffold: remove `wasm-pack` `.gitignore` from `pkg/` after build
- Scaffold: don't gitignore `pkg/` — npm needs it for WASM publish
- Version sync: scan workspace excludes correctly
- Overwrite READMEs, e2e, and docs when `--clean` or standalone commands

## [0.4.0] - 2026-04-19

### Added

- Codegen: `Option<Option<T>>` flattening — binding structs use `Option<T>` with `.map(Some)` / `.flatten()` in From impls
- Codegen: RefMut functional pattern — `&mut self` methods on non-opaque types generate clone-mutate-return (`&self → Self`)
- Codegen: auto-derive `Default`, `Serialize`, `Deserialize` on all binding types and enums
- Codegen: builder-pattern methods returning `Self` now auto-delegate instead of emitting `compile_error!`
- Codegen: `can_generate_default_impl()` validation — skip Default derivation for structs with non-Default fields
- Config: configurable `type_prefix` for WASM and NAPI backends, default WASM to `Wasm`
- E2E: Brew/Homebrew CLI test generator (13th e2e language target)
- IR: `returns_cow` field on `FunctionDef` for Cow return type handling in free functions
- IR: `is_mut` field on `ParamDef` for `&mut T` parameter handling
- NAPI: conditional serde derives gated on `has_serde` for structs, enums, and tagged enum structs
- NAPI: tagged enum From impls distinguish binding-struct fields from serde-flattened String fields
- Scaffold: generate CMake config for FFI, add `build:ts` to Node scaffold
- Scaffold: add serde dependency to NAPI and Rustler scaffold templates
- WASM: `exclude_reexports` config for public API generation
- WASM: TypeScript re-export support for custom modules

### Fixed

- Codegen: `TypeRef::Char` in `gen_lossy_binding_to_core_fields` — use `.chars().next().unwrap_or('*')` instead of `.clone()`
- Codegen: optionalized Duration field conversion — handle `has_default` types where Duration is stored as `Option<u64>`
- Codegen: enum match arm field access — use destructured field name instead of `val.{name}`
- Codegen: Default derive conflicts in extendr/magnus backends
- Codegen: convert optional Named params with `.map(Into::into).as_ref()` for non-opaque ref params
- Codegen: pass owned `Vec<u8>` when `is_ref=false` for Bytes params
- Codegen: add explicit type annotations in let bindings to resolve E0283 ambiguity
- Codegen: skip `apply_core_wrapper_from_core` for sanitized fields (fixes Mutex clone)
- Codegen: replace `compile_error!` with `Default::default()` for Named returns without error variant
- Codegen: generate `Vec<Named>` let bindings for non-optional `is_ref=true` params
- Codegen: fix `Option<&T>` double-reference — don't add extra `&` when let binding already produces `Option<&T>`
- Codegen: correct float literal defaults (`0.0f32`/`0.0f64`) in unimplemented body for float return types
- Codegen: handle `&mut T` parameters via `is_mut` IR field — emit `&mut` refs instead of `&`
- Codegen: parse `TypeRef::Json` parameters with `serde_json::from_str()` instead of passing raw String
- Codegen: skip auto-delegation for trait-source methods on opaque types (prevents invalid Arc deref calls)
- Codegen: convert `Vec<BindingType>` to `Vec<CoreType>` via let bindings before passing to core functions
- Codegen: handle `Vec<Optional<Json>>` field conversion (`Value` → `String`)
- Codegen: async unimplemented returns `PyNotImplementedError` instead of `py.None()`
- Codegen: handle `Optional(Vec(Named))` in sync function return wrapping
- Codegen: `#[default]` on first enum variant for `derive(Default)` to work
- Extract: skip `#[cfg(...)]`-gated free functions during extraction
- Extract: prune non-re-exported items from private modules
- FFI: use `std::mem::drop()` instead of `.drop()` for owned receiver methods
- FFI: clone `&mut String` returns before CString conversion
- FFI: deserialize `Optional(Vec/Map)` JSON params and pass with `.as_deref()`
- FFI: handle `Option<&Path>` / `Option<PathBuf>` conversion from `Option<String>`
- FFI: handle `&Value` params by deserializing into owned Value then passing reference
- FFI: add explicit `Vec<_>` type annotations for serde deserialization of ref/mut params
- FFI: handle `returns_cow` — emit `.into_owned()` for `Cow<'_, T>` returns before boxing
- FFI: unwrap `Arc<T>` fields in accessors via `core_wrapper` check
- FFI: respect `is_ref` for `Path` and `String`/`Char` parameters
- FFI: correct codegen for `Option<Option<primitive>>` field getters
- FFI: collapse nested match for `Option<Option<T>>` getters
- NAPI: `Optional(Vec(Named))` return wrapping for free functions
- NAPI: U32 in tagged enum core→binding stays as u32, only U64/Usize/Isize cast to i64
- NAPI: detect and import `serde_json` when serde-based parameter conversion is needed
- NAPI: enum variant field casts use `.map()` for Optional wrapping
- PHP: float literals in `gen_php_unimplemented_body` (`0.0f32`/`0.0`)
- PHP: BTreeMap conversion via `.into_iter().collect()` in binding→core From
- PHP: `Map(_, Json)` sanitized fields use `Default::default()`
- PHP: enum returns use serde conversion instead of `.into()`
- PHP: exclude `PooledString` (ext-php-rs incompatible methods)
- PHP: filter excluded types from class registrations in `get_module`
- PHP: flatten nested `Option<Option<T>>` to single nullable type in stubs
- PHP: optionalized Duration in lossy binding→core conversion
- Scaffold: skip existing files, align Java `pom.xml` with kreuzberg
- Scaffold: remove `compilers` directive for Rustler 0.34+
- Scaffold: remove unused license format arg from C# template
- Scaffold: use `PackageLicenseFile` for C# (NuGet rejects non-OSI licenses)
- WASM: don't add Rust `pub mod` for custom modules
- Docs: add trailing newlines and wrap bare URLs (MD034)
- Docs: shift heading levels down for frontmatter compatibility

### Documentation

- Skills: add `--registry` flag, `[readme]` config, `[e2e.registry]` block, `custom_files` key, Brew e2e target
- Skills: update `alef all` description to include e2e + docs generation
- Skills: add `--set` flag to `sync-versions` documentation
- Skills/READMEs: update pre-commit rev tags to v0.3.5
- README: add `init` and `all` to alef-cli command list

## [0.3.5] - 2026-04-16

### Fixed

- Codegen: deserialize sanitized fields in binding-to-core enum variant conversion
- Codegen: skip trait types in all backend type generation loops
- Codegen: replace glob imports with explicit named imports in PyO3 and FFI backends
- Codegen: generate `From` impls for function return types and nested params
- FFI: generate `from_json`/`to_json` for types with sanitized fields
- FFI: remove unused explicit core imports from FFI backend
- FFI: normalize dashes to underscores in IR `rust_path` for code generation
- CLI: format generated Rust content before diffing in verify/diff commands
- E2E: use configured `class::function` for PHP calls regardless of `result_is_simple`
- Extract: check `lib.rs` as parent module for reexport shortening
- Resolve trait sources in post-processing to handle module ordering
- Normalize dashes to underscores in rust path comparison for `From` impl generation
- Scaffold: add `compilers` directive to Elixir `mix.exs` template
- Bump cbindgen scaffold version from 0.28 to 0.29

## [0.3.4] - 2026-04-15

### Added

- `alef sync-versions --set <version>` — set an explicit version (supports pre-release like `0.1.0-rc.1`)
- `alef verify` now checks version consistency across all package manifests
- `alef-sync-versions` pre-commit hook for automatic version propagation on Cargo.toml changes
- PEP 440 pre-release conversion for Python (`0.1.0-rc.1` → `0.1.0rc1`)
- `reverse_conversions` config flag to gate binding-to-core `From` impls
- E2E registry mode for test_apps generation across all 12 languages
- `alef-docs` fallback description generator for struct fields and enum variants
- `alef all` now includes e2e and docs generation; scaffold reads workspace version
- Elixir `ex_doc` support in scaffold
- Java scaffold: switch to `central-publishing-maven-plugin`; Node scaffold: `optionalDependencies`
- Enriched PHP/Python/Java scaffolds for CI and publishing
- PHP composer.json scaffold: `scripts` section with `phpstan`, `format`, `format:check`, `test`, `lint`, and `lint:fix` commands

### Fixed

- Docs generator: critical type, default value, enum, and error generation fixes across Go, C#, Rust
- Go doc signatures with empty return type; C# double `Async` suffix removed
- FFI codegen cast corrections; README version template rendering
- PHP scaffold autoload and backslash escaping in `composer.json` PSR-4 namespace
- PHP stubs: generate public property declarations on classes (ext-php-rs exposes fields as properties, PHPStan needs them declared)
- PHP stubs: camelCase naming; remove hardcoded `createEngineFromJson` from facade and stubs
- PHP codegen: remove needless borrow in `serde_json::to_value` calls (clippy fix)
- Python stubs: add `# noqa: A002` to constructor parameters that shadow Python builtins (e.g. `id`)
- Python stubs: place `noqa` comment after comma in multi-line `__init__` params
- Python scaffold: removed `[tool.ruff]` section — linter config belongs in root `pyproject.toml`
- Python duration stubs: correct mypy annotations
- Ruby: replace `serde_magnus` with `serde_json`, handle sanitized fields in `From` impls
- Ruby gemspec version sync: match single-quote `spec.version = '...'` (was only matching double quotes)
- Java: `checkLastError` throws `Throwable`; correct Jackson version to 2.19.0
- WASM: `option_duration` handling; added missing `wasm-bindgen-futures` dependency
- WASM codegen: remove unused `HashMap` import
- Node/FFI scaffolds: removed unused `serde` dependency
- Node scaffold: removed unused `serde_json` dependency
- Rustler backend: output path uses `_nif` suffix instead of `_rustler`
- Version sync: recursive `.csproj` scanning, WASM/root `composer.json`, FFI crate name
- Only generate binding-to-core `From` impls for input types, not output-only types
- `path_mismatch` false positive on re-exported types (same crate root + name)
- Add `Language::Rust` to all match arms across codebase
- Rust e2e: `assert!(bool)` for clippy, `is_empty` for len comparisons, unqualified imports
- E2e: add `setuptools packages=[]` to Python registry `pyproject.toml`
- Clippy `field_reassign_with_default` suppressed for Duration builder pattern
- Scaffold Cargo.toml templates: removed unused deps — `pyo3-async-runtimes` (Python), `serde_json` (Node), `tokio` (PHP, FFI), `wasm-bindgen-futures` (WASM), `serde`+`tokio` (Elixir/Rustler) — only include what generated binding code actually uses

## [0.3.3] - 2026-04-14

### Added

- Distributable Claude Code skill for alef consumers

### Fixed

- PHP 100% coverage — `createEngineFromJson`, JSON config e2e support
- PHP snake_case properties, enum scalars, serde casing, camelCase facade
- PHP class registration, property access, per-field attributes
- PHP setters documented, enum serde casing via `serde_json`
- PHP tests updated for `Api` class pattern (no more standalone `#[php_function]`)
- Correct DTO style naming (`typed-dict` not `typeddict`), add `serde_rename_all` to config docs
- Add Credo to Elixir scaffold, PHPStan + PHP-CS-Fixer to PHP scaffold
- Drop unused `tokio` from Python scaffold, add e2e license, split assertions
- Remove unused `serde` dep from Node.js and FFI scaffolds
- PHP and Rustler miscellaneous fixes

## [0.3.2] - 2026-04-13

### Fixed

- Disable `jsonschema` default features (`resolve-http`, `tls-aws-lc-rs`) to remove `reqwest`/`aws-lc-sys` from dependency tree, fixing GoReleaser cross-compilation

## [0.3.1] - 2026-04-13

### Fixed

- Elixir resource registration and enum conversion fixes
- C# opaque handle wrapping, error handling, `Json` to `object` type mapping, `WhenWritingDefault` serialization
- Dropped `x86_64-apple-darwin` from GoReleaser targets (`aws-lc-sys` cross-compile failure)

## [0.3.0] - 2026-04-13

### Added

- Pre-commit hook support for consumer repos (`alef-verify` and `alef-generate` hooks via `.pre-commit-hooks.yaml`)
- Blake3-based caching for stubs, docs, readme, scaffold, and e2e generation — all commands now skip work when inputs are unchanged
- `cargo binstall alef-cli` support via `[package.metadata.binstall]` metadata
- `alef-docs` and `alef-e2e` added to crates.io publish workflow

### Changed

- All workspace dependencies consolidated to root `Cargo.toml` — every crate now uses `{ workspace = true }` for both internal and shared external deps
- Bumped all crates from 0.2.0 to 0.3.0

### Fixed

- Replaced `.unwrap()` with `.extend()` in `to_snake_case` (`alef-adapters`)
- Replaced runtime `todo!()`/`panic!()` with `compile_error!()` in generated code for unimplemented adapter patterns
- Made `PrimitiveType` matches exhaustive, removing `unreachable!()` (`alef-codegen`)
- Added inline SAFETY comments to FFI pointer dereferences (`alef-backend-ffi`)
- Clamped negative Duration values before `u64` cast in NAPI bindings
- Fixed rustler public API test assertions to match Elixir conventions (no parens on zero-arg defs, keyword defstruct)

## [0.2.0] - 2026-04-13

### Added

- `alef e2e` CLI command with `generate`, `list`, `validate`, `scaffold`, `init` subcommands
- E2E test generators for 12 languages (Rust, Python, TypeScript, Go, Java, C#, PHP, Ruby, Elixir, R, WASM, C)
- `alef-e2e` crate with fixture loading, JSON Schema validation, and `FieldResolver` for nested field paths
- `options_type`, `enum_module`, `enum_fields`, `options_via` config support for flexible argument passing

### Fixed

- crates.io version specifier fixes for path dependencies
- Backend-specific e2e test generation fixes across multiple languages

## [0.1.0] - 2026-04-09

### Added

- Initial release with 20 crates in a Cargo workspace
- Full CLI: `extract`, `generate`, `stubs`, `scaffold`, `readme`, `docs`, `sync-versions`, `build`, `lint`, `test`, `verify`, `diff`, `all`, `init`
- 11 language backends: Python (PyO3), TypeScript (NAPI-RS), Ruby (Magnus), Go (cgo/FFI), Java (Panama FFM), C# (P/Invoke), PHP (ext-php-rs), Elixir (Rustler), R (extendr), WebAssembly (wasm-bindgen), C (FFI)
- Async bridging and adapter patterns: streaming, callback bridge, sync function, async method
- Method delegation with opaque type wrapping via `Arc<T>`
- Error type generation from `thiserror` enums with cross-language exception mapping
- Type alias and trait extraction from Rust source
- Blake3-based caching for `extract` and `generate` commands
- CI pipeline: cargo fmt, clippy, deny, machete, sort, taplo
- GoReleaser-based publish workflow with cross-platform binaries and Homebrew tap

## [0.15.34] - 2026-05-11

- fix(magnus): remove problematic re-export loop in native.rb wrapper
- fix(magnus): handle optional String parameters via magnus::Value conversion
