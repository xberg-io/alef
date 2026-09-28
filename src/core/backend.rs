use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::validation::ValidatedApiSurface;
use std::path::PathBuf;

/// Build-time dependency for a language backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildDependency {
    /// Backend has no external build dependencies.
    #[default]
    None,
    /// Backend depends on the C FFI base being built first (Go, Java, C#, Zig).
    Ffi,
    /// Backend depends on the Rustler NIF being built first (Gleam).
    Rustler,
}

/// Build configuration for a language backend.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Build tool name (e.g., "napi", "maturin", "wasm-pack", "cargo", "mvn", "dotnet", "mix").
    pub tool: &'static str,
    /// Crate suffix for Rust binding crate (e.g., "-node", "-py", "-wasm", "-ffi").
    pub crate_suffix: &'static str,
    /// Build-time dependency for this backend.
    pub build_dep: BuildDependency,
    /// Post-processing steps to run after build.
    pub post_build: Vec<PostBuildStep>,
}

impl BuildConfig {
    /// Returns whether this backend depends on the C FFI base (backwards compatibility).
    pub fn depends_on_ffi(&self) -> bool {
        matches!(self.build_dep, BuildDependency::Ffi)
    }
}

/// In-process post-processor applied to a generated file after external build tools run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostProcessor {
    /// Rewrite frb-generated Dart sealed-class factory params from positional names (`field0`)
    /// to payload-derived names (e.g. `metadata` for a `PdfMetadata` payload).
    FrbDartSealedVariants,
    /// Filter excluded function definitions from frb-generated Dart lib.dart.
    /// Stores the set of function names to exclude.
    FrbDartExcludeFunctions(Vec<String>),
    /// Make struct constructor fields optional for types with Rust defaults.
    /// This handles Dart types that have #[serde(default)] fields in Rust.
    FrbDartOptionalFieldsWithDefaults,
    /// Fix FRB-generated Dart code that incorrectly calls executeSync/executeNormal
    /// on callback function parameters.
    FrbDartFixHandlerExecutorCalls,
    /// Inject display-as-text extensions on untagged union types so they can be
    /// stringified in assertions. Stores the set of type names.
    FrbDartInjectTextMethods(Vec<String>),
    /// Strip trailing whitespace from generated Dart files that `dart format`
    /// leaves untouched, such as `*.freezed.dart`.
    DartStripTrailingWhitespace,
}

/// A post-build processing step.
#[derive(Debug, Clone)]
pub enum PostBuildStep {
    /// Replace all occurrences of `find` with `replace` in `path` (relative to crate dir).
    PatchFile {
        /// File path relative to the binding crate directory.
        path: &'static str,
        /// Text to find.
        find: &'static str,
        /// Text to replace with.
        replace: &'static str,
    },
    /// Run an external command (e.g., for generated code post-processing via flutter_rust_bridge).
    RunCommand {
        /// Command to execute.
        cmd: &'static str,
        /// Command arguments.
        args: Vec<&'static str>,
    },
    /// Apply an in-process [`PostProcessor`] to the file at `path` (relative to crate dir).
    PostProcessFile {
        /// File path relative to the binding crate directory.
        path: PathBuf,
        /// In-process processor to apply.
        processor: PostProcessor,
    },
    /// Stage Dart native libraries from build artifacts into the package directory.
    /// Searches `{workspace}/target/{rust_target}/release/` for built libraries
    /// and copies them to `{package_root}/lib/src/native/{rid}/`.
    StageDartNatives {
        /// The library stem (e.g., "sample_lib_dart" for libsample_lib_dart.dylib).
        lib_stem: String,
    },
    /// Copy the just-built C FFI shared library (and its header, if present) into this
    /// backend's native-library directory (`crate::publish::ffi_stage::stage_ffi`).
    ///
    /// Backends with `build_dep: BuildDependency::Ffi` (go, java, csharp, kotlin,
    /// kotlin_android, jni, zig, and dart in FFI style) link against a cdylib
    /// their own build tool never places on disk itself — `cargo build` for the `-ffi` crate
    /// does, into `target/{release,debug}/`, a location none of `go build`/`mvn`/`dotnet build`
    /// know about. Without this step, that directory only ever gets staged by `alef test --e2e`
    /// or `alef publish`, so a plain `alef build` reports success while the staged artifact
    /// used by every other consumer of this package (IDEs, manual `go build`, CI steps that
    /// don't run through alef) keeps rotting at whatever version was last staged by one of
    /// those two commands, if any. Always attempted after a build, and always a fresh
    /// overwrite (never skipped because the destination already exists), so a stale staged
    /// copy can never survive a successful build silently. A missing built artifact (e.g. this
    /// step ran from `alef generate`'s post-build pass, which never invokes `cargo build`) is
    /// not an error — it is logged as a warning naming the destination, never a silent no-op. ~keep
    StageFfiLibrary,
    /// Re-run the swift-bridge file materialization (copy the freshly-built
    /// glue/headers from target/*/out into Sources/RustBridge{,C}). Must run
    /// AFTER the cargo build RunCommand so it picks up current output, not stale.
    MaterializeSwiftBridge {
        /// Hyphenated binding crate name (e.g. `sample-lib-swift`),
        /// matching the cargo build output dir prefix `{name}-swift-<hash>`.
        binding_crate_name: String,
        /// Swift package root (the dir containing `Sources/`), relative to the
        /// workspace base dir.
        package_root: String,
    },
    /// Scan `source_path` for `#[cfg(...)]`-gated free functions and carry those gates
    /// into `target_path`'s wire dispatch (both paths relative to the binding crate dir).
    ///
    /// Unlike [`PostProcessFile`](Self::PostProcessFile), this reads one file to determine
    /// what to rewrite in a *different* file, so it cannot be expressed as a single-path
    /// `PostProcessor`.
    CarryFrbCfgGates {
        /// File to scan for `#[cfg(...)]`-gated free functions (the FRB source crate's `lib.rs`).
        source_path: PathBuf,
        /// File to rewrite with the gates carried over (the generated `frb_generated.rs`).
        target_path: PathBuf,
    },
    /// Rewrite the `"name"` field of a wasm-pack-generated `package.json` to the
    /// alef-configured WASM npm package name.
    ///
    /// wasm-pack derives that file's own `name` from the crate's `Cargo.toml`, which alef
    /// does not (and should not) control — so after a fresh `wasm-pack build --target nodejs`,
    /// the generated `pkg/nodejs/package.json` disagrees with the name every e2e-generated
    /// `file:` dependency and `require()`/`import` specifier uses
    /// ([`ResolvedCrateConfig::wasm_package_name`]) unless something patches it. Neither
    /// `find`/`replace` in [`PostBuildStep::PatchFile`] can express this: the current name is
    /// unknown until wasm-pack writes it, so both the search and replacement text must be
    /// computed at build time rather than fixed at compile time. ~keep
    RewriteWasmPackageName {
        /// Path to the wasm-pack-generated `package.json`, relative to the workspace base
        /// dir (*not* the binding crate dir, unlike every other step above) — the wasm crate
        /// directory itself may come from `config.package_dir(Language::Wasm)`'s
        /// default-formula fallback rather than the language's `explicit_output`, so the
        /// caller resolves the full path once at construction time.
        package_json_path: PathBuf,
        /// The desired `"name"` field value (`config.wasm_package_name()`).
        package_name: String,
    },
    /// Verify that every free function declared in `facade_path` (the FRB source crate's
    /// `lib.rs`) has a matching function in `bridge_path` (the flutter_rust_bridge-generated
    /// `lib.dart`), and fail loudly if not.
    ///
    /// Placed immediately after the `RunCommand` step that invokes
    /// `flutter_rust_bridge_codegen`, before any `PostProcessFile` rewrite of `bridge_path` —
    /// see alef #135. That `RunCommand`'s runner treats a missing `flutter_rust_bridge_codegen`
    /// tool (or `ALEF_SKIP_COMMANDS`) as a non-fatal skip, falling back to whatever bridge
    /// source is already on disk — deliberate, so a host without the tool installed can still
    /// regenerate the facade. But the `PostProcessFile` steps that follow run unconditionally,
    /// patching whatever is on disk regardless of whether frb actually produced it this run. If
    /// the facade gained functions since the bridge was last regenerated and frb did not
    /// actually run this pass, those patches land on a stale bridge that looks freshly
    /// post-processed while silently missing the new functions. This step turns that silent,
    /// internally-inconsistent output into a loud build failure instead. ~keep
    VerifyFrbBridgeCoverage {
        /// The FRB source crate's `lib.rs`, relative to the binding crate dir.
        facade_path: PathBuf,
        /// The flutter_rust_bridge-generated `lib.dart`, relative to the binding crate dir.
        bridge_path: PathBuf,
        /// Facade functions expected to be absent from the bridge (stripped post-frb by
        /// `PostProcessor::FrbDartExcludeFunctions`) — never reported as a coverage gap.
        exclude_functions: Vec<String>,
    },
    /// Verify the `flutter_rust_bridge_codegen` binary on `PATH` reports `expected_version`
    /// before the `RunCommand` step right after this one invokes it.
    ///
    /// `flutter_rust_bridge_codegen` is not a pure function of its input: its generated
    /// `frb_generated.rs`/`frb_generated.dart` output (import ordering, wire dispatch
    /// structure, generated comments) is a function of *its own* version as well, so two
    /// developers -- or a developer and CI -- with different `flutter_rust_bridge_codegen`
    /// versions installed produce different committed bytes from identical Rust input. alef
    /// already carries a declared pin for this (`[crates.dart] frb_version`, defaulting to
    /// `template_versions::cargo::FLUTTER_RUST_BRIDGE`) because the generated crate's
    /// `Cargo.toml`/`pubspec.yaml` must depend on the exact `flutter_rust_bridge` runtime
    /// version the installed codegen binary was built against -- but nothing checked that
    /// pin against the binary actually on `PATH` before running it (alef #204).
    ///
    /// This step is deliberately not the thing that changes the installed binary's version --
    /// alef does not vendor `flutter_rust_bridge_codegen` and cannot force a specific one onto
    /// `PATH`. It fails loudly instead, before `generate` runs, so a version mismatch is a
    /// build error at the point it happens rather than a silent, ambient-machine-dependent diff
    /// discovered later in review or CI. A missing binary is not this step's concern: it
    /// resolves to `Ok(())` and lets the `RunCommand` step immediately after report the
    /// existing "not on PATH, falling back to committed output" skip the same way it always
    /// has. ~keep
    VerifyFrbCodegenVersion {
        /// The pinned `flutter_rust_bridge` version (`naming::dart_frb_version`) the installed
        /// `flutter_rust_bridge_codegen --version` output must match exactly.
        expected_version: String,
    },
}

/// Whether a generation run (`alef generate`, `alef all`) is allowed to invoke a compiler while
/// completing its generated artifacts.
///
/// Generation's documented contract is that it writes and post-processes source; compiling is
/// `alef build`'s job. Two steps on the generation path break that contract anyway, because the
/// bytes they need only exist as a side effect of a cargo invocation: Swift's post-build has to
/// run the swift-bridge crate's own `build.rs` before `MaterializeSwiftBridge` can copy the
/// `SwiftBridgeCore.swift`/`{crate}.swift`/`RustBridgeC.h` trio out of `OUT_DIR`, and the FFI
/// header gate has to build the `-ffi` crate before cbindgen has written a header to check. Both
/// are load-bearing, so [`Self::Allowed`] stays the default and no consumer relying on `alef all`
/// to produce them loses anything by upgrading. [`Self::Skipped`] is the opt-in escape for a
/// workflow that compiles in a separate task and only wants source written -- it drops exactly
/// those two steps, logs what it dropped and what will refresh them, and changes nothing else. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilePolicy {
    /// Run compiling post-build steps (the default for every generation command).
    Allowed,
    /// Skip them, leaving the artifacts they derive to a later `alef build`.
    Skipped,
}

impl PostBuildStep {
    /// Whether running this step invokes the Rust compiler on the consumer's crate graph.
    ///
    /// `cargo` is the only command any backend puts in a `RunCommand` that compiles anything --
    /// Dart's `flutter_rust_bridge_codegen` parses source and writes Dart, and every other
    /// variant is a pure in-process file rewrite, copy, or check. Swift's is the one that
    /// compiles: `cargo build --release` for `alef build`, and a `cargo check` on the generation
    /// path, which is cheaper but still walks the consumer's whole dependency graph (measured at
    /// over 17 minutes on a real workspace) inside a command contractually forbidden from
    /// compiling. Asking the step rather than matching on the language keeps the question with
    /// the thing that answers it, so a future backend that adds a cargo step is covered without
    /// touching the generation path. ~keep
    pub fn invokes_rust_compiler(&self) -> bool {
        matches!(self, PostBuildStep::RunCommand { cmd, .. } if *cmd == "cargo")
    }

    /// Paths this step writes directly to disk, outside the ownership-guarded writer
    /// (`cli::pipeline::generate::write::write_files_report`).
    ///
    /// A step earns an entry here only when it writes content a build tool -- not alef's
    /// own generator -- produced, so the file can never carry an alef marker and the
    /// ownership guard can never durably prove alef owns it (`MaterializeSwiftBridge`'s
    /// case: swift-bridge's own header/import conventions rule out `generated_header:
    /// true`, and the file changes on every build regardless of source input). Most
    /// variants return nothing: their output either flows through the normal
    /// `GeneratedFile`/`write_files_report` path already, or (like `StageDartNatives`
    /// copying prebuilt native libraries) is not something `alef generate`'s own run
    /// tracks as its output at all.
    ///
    /// Callers fold these into the same run's `generation_owned_paths` the generator's own
    /// `GeneratedFile`s populate, so the orphan sweep (`bin_cli::core_commands`) sees these
    /// paths as claimed on every run this step is configured to touch -- not only the runs
    /// where the corresponding generator call happened to find fresh content to emit. Without
    /// this, a path this step writes unguarded but the generator omits (because it was
    /// already up to date, or because build output wasn't available to read back without
    /// disagreeing with `normalize_content`) reads as "alef no longer generates this" on the
    /// very next run and gets deleted -- the alef #B incident
    /// (`packages/swift/Sources/RustBridgeC/RustBridgeC.h` removed from an otherwise
    /// unchanged tree). ~keep
    pub fn owned_paths(&self, base_dir: &std::path::Path) -> Vec<PathBuf> {
        match self {
            PostBuildStep::MaterializeSwiftBridge {
                binding_crate_name,
                package_root,
            } => {
                let package_root = base_dir.join(package_root);
                let sources_rust_bridge = package_root.join("Sources").join("RustBridge");
                let sources_rust_bridge_c = package_root.join("Sources").join("RustBridgeC");
                // `emit_swift_bridge_files` (the function this step actually calls) only
                // writes the full `SwiftBridgeCore.swift` / `{binding_crate_name}.swift` /
                // `RustBridgeC.h` trio once it finds a real swift-bridge build output
                // directory (or a header already carrying its marker from an earlier real
                // build); until then it writes the placeholder header alone. Predicting the
                // full trio unconditionally -- as this used to -- claims two files that were
                // never written on a project's first successful generation, so the ownership
                // manifest names paths `alef verify`/the orphan sweep can never find on disk.
                // Every caller of this method runs it after the post-build step it describes
                // has already executed (`bin_cli::core_commands`'s generate handler calls it
                // once `complete_generated_artifacts` returns; `alef verify` inspects a tree a
                // prior `alef generate` already built), so filtering to what is actually
                // present keeps both the alef #B protection (a real trio already on disk from
                // an earlier run stays claimed even when this run's build left it untouched)
                // and manifest accuracy (a path never written is never claimed). ~keep
                vec![
                    sources_rust_bridge_c.join("RustBridgeC.h"),
                    sources_rust_bridge.join("SwiftBridgeCore.swift"),
                    sources_rust_bridge.join(format!("{binding_crate_name}.swift")),
                ]
                .into_iter()
                .filter(|path| path.is_file())
                .collect()
            }
            PostBuildStep::PatchFile { .. }
            | PostBuildStep::RunCommand { .. }
            | PostBuildStep::PostProcessFile { .. }
            | PostBuildStep::StageDartNatives { .. }
            | PostBuildStep::StageFfiLibrary
            | PostBuildStep::CarryFrbCfgGates { .. }
            | PostBuildStep::RewriteWasmPackageName { .. }
            | PostBuildStep::VerifyFrbBridgeCoverage { .. }
            | PostBuildStep::VerifyFrbCodegenVersion { .. } => Vec::new(),
        }
    }
}

/// A generated file to write to disk.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    /// Path relative to the output root.
    pub path: PathBuf,
    /// File content.
    pub content: String,
    /// Whether to prepend a "DO NOT EDIT" header.
    pub generated_header: bool,
}

impl GeneratedFile {
    /// Whether the emitted file ends up carrying an alef header marker.
    ///
    /// Distinct from [`Self::generated_header`], which only says whether the
    /// writer prepends one: a backend may emit its own marker inside `content`
    /// and still set the flag to `false`. `alef verify` claims any file on disk
    /// carrying the marker, so the stamping pass must use this, not the flag —
    /// otherwise self-marked files are verified but never stamped. ~keep
    pub fn carries_alef_marker(&self) -> bool {
        self.generated_header || crate::core::hash::content_has_alef_marker(&self.content)
    }
}

/// One backend's rendered text for a single public function's parameter list and return
/// type, captured for the breaking-signature-change baseline
/// (`cli::breaking_changes::check_signature_breakage`).
///
/// Comparison against a prior run's baseline is textual, not a semantic parse of the
/// target language — a backend that reformats an otherwise-unchanged signature between
/// runs reads as changed. That trade favors over-reporting a false positive (a `WARN`) over
/// silently missing a real breaking change. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedSignature {
    /// The symbol name a hand-written caller would reference (the emitted function name).
    pub symbol: String,
    /// The emitted parameter list, in call order. Format is backend-defined; only equality
    /// across runs is load-bearing.
    pub params: String,
    /// The emitted return type, including any error-union/Result-like wrapper.
    pub return_type: String,
}

/// One trait-implementation registration entry a backend actually emits for a configured
/// `[[trait_bridges]]` entry — the API a host-language caller uses to register (and, where
/// emitted, unregister/clear) an implementation of a Rust trait.
///
/// Captured for trait-bridge reference-doc rendering
/// (`docs::language_pages::trait_bridge_render`). A backend reports only the symbols it
/// actually generates; it must never fabricate a name it does not emit — see
/// [`Backend::trait_bridge_registration_surface`]. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitBridgeRegistrationSurface {
    /// The Rust trait this registers a host-language implementation of.
    pub trait_name: String,
    /// The symbol/function a host calls to register an implementation, when this backend
    /// emits one.
    pub register_symbol: Option<String>,
    /// The symbol/function a host calls to unregister a previously registered
    /// implementation, when this backend emits one.
    pub unregister_symbol: Option<String>,
    /// The symbol/function a host calls to clear all registered implementations of this
    /// trait, when this backend emits one.
    pub clear_symbol: Option<String>,
}

/// Capabilities supported by a backend.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub supports_async: bool,
    pub supports_classes: bool,
    pub supports_enums: bool,
    pub supports_option: bool,
    pub supports_result: bool,
    pub supports_callbacks: bool,
    pub supports_streaming: bool,
    /// Whether this backend implements [`Backend::generate_service_api`].
    ///
    /// Backends that support service API generation set this to `true` and
    /// override `generate_service_api`.  When `false` and a crate has non-empty
    /// `services`, the generation pipeline emits a fatal readiness diagnostic.
    pub supports_service_api: bool,
}

/// Trait that all language backends implement.
pub trait Backend: Send + Sync {
    /// Backend identifier (e.g., "pyo3", "napi", "ffi").
    fn name(&self) -> &str;

    /// Target language.
    fn language(&self) -> Language;

    /// What this backend supports.
    fn capabilities(&self) -> Capabilities;

    /// Generate binding source code.
    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>>;

    /// Generate binding source code from a centrally validated API surface.
    fn generate_bindings_checked(
        &self,
        api: ValidatedApiSurface<'_>,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        self.generate_bindings(api.api(), config)
    }

    /// Currently-emitted public function signatures, for the breaking-signature-change
    /// baseline (`cli::breaking_changes::check_signature_breakage`). Optional — default
    /// returns empty, meaning this backend is not (yet) covered by that check: a signature
    /// only ever enters the baseline once a backend starts returning it here, so an
    /// uncovered backend silently detects nothing rather than erroring or fabricating
    /// signatures it did not actually render. ~keep
    fn public_function_signatures(&self, _api: &ApiSurface, _config: &ResolvedCrateConfig) -> Vec<EmittedSignature> {
        Vec::new()
    }

    /// Trait-implementation registration surface this backend actually emits for each active
    /// `[[trait_bridges]]` entry, for trait-bridge reference-doc rendering. Optional — default
    /// returns empty, meaning this backend is not (yet) covered by trait-bridge reference
    /// docs: an uncovered backend silently documents nothing rather than fabricating a
    /// registration name it does not actually emit. Docs must call this method rather than
    /// re-deriving the registration surface themselves. ~keep
    fn trait_bridge_registration_surface(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        Vec::new()
    }

    /// Generate type stubs (.pyi, .rbs, .d.ts). Optional — default returns empty.
    fn generate_type_stubs(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    /// Generate type stubs from a centrally validated API surface.
    fn generate_type_stubs_checked(
        &self,
        api: ValidatedApiSurface<'_>,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        self.generate_type_stubs(api.api(), config)
    }

    /// Generate package scaffolding. Optional — default returns empty.
    fn generate_scaffold(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    /// Generate language-native public API wrappers. Optional — default returns empty.
    fn generate_public_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    /// Generate public API wrappers from a centrally validated API surface.
    fn generate_public_api_checked(
        &self,
        api: ValidatedApiSurface<'_>,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        self.generate_public_api(api.api(), config)
    }

    /// Generate the idiomatic service/app object and async handler bridge for a
    /// backend that supports service API generation.
    ///
    /// Called **after** `generate_bindings` and **before** `generate_public_api`
    /// when `surface.services` is non-empty and `capabilities().supports_service_api`
    /// is `true`.  Backends that do not yet implement service API generation leave
    /// the default no-op in place; the pipeline emits a warning for crates that
    /// configure services against an unsupporting backend.
    fn generate_service_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
    }

    /// Generate service API wrappers from a centrally validated API surface.
    fn generate_service_api_checked(
        &self,
        api: ValidatedApiSurface<'_>,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        self.generate_service_api(api.api(), config)
    }

    /// Build configuration for this backend. Returns `None` if build is not supported.
    fn build_config(&self) -> Option<BuildConfig> {
        None
    }

    /// Build configuration for this backend with full access to the crate config.
    /// This allows backends to customize build steps based on configuration (e.g., exclude functions, styles).
    ///
    /// Default implementation calls `build_config()` (no config dependency).
    /// Backends that need config access (like Dart) can override this method.
    fn build_config_with_config(&self, _config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        self.build_config()
    }

    /// Post-build config for the contractually no-build `alef generate`/`alef all` paths
    /// (`bin_cli::helpers::complete_generated_artifacts`), as opposed to `build_config_with_config`,
    /// which is also used by `alef build`'s own dispatch and may legitimately invoke a real,
    /// expensive compile.
    ///
    /// Default implementation returns `build_config_with_config(config)` verbatim: most backends'
    /// post-build steps (`PatchFile`, `PostProcessFile`, frb's codegen `RunCommand`, ...) are
    /// already cheap and generate-appropriate, so there is nothing to downgrade. A backend whose
    /// post-build step genuinely compiles something (Swift's swift-bridge crate, task #541)
    /// overrides this to substitute a cheaper equivalent for the generate path while leaving
    /// `build_config_with_config` untouched for `alef build`. ~keep
    fn generate_post_build_config(&self, config: &ResolvedCrateConfig) -> Option<BuildConfig> {
        self.build_config_with_config(config)
    }
}

#[cfg(test)]
mod generated_file_tests {
    use super::GeneratedFile;
    use std::path::PathBuf;

    fn file(content: &str, generated_header: bool) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from("out.cs"),
            content: content.to_owned(),
            generated_header,
        }
    }

    #[test]
    fn should_claim_file_whose_body_carries_its_own_marker() {
        let emitted = file(
            "// This file is auto-generated by alef. DO NOT EDIT.\nclass X {}\n",
            false,
        );
        assert!(
            emitted.carries_alef_marker(),
            "a self-marked file is claimed by verify, so stamping must claim it too"
        );
    }

    #[test]
    fn should_claim_file_that_gets_a_prepended_header() {
        assert!(file("class X {}\n", true).carries_alef_marker());
    }

    #[test]
    fn should_not_claim_unmarked_handwritten_emission() {
        assert!(
            !file("class X {}\n", false).carries_alef_marker(),
            "scaffold-once files alef does not own must stay unclaimed"
        );
    }
}

#[cfg(test)]
mod invokes_rust_compiler_tests {
    use super::{PostBuildStep, PostProcessor};
    use std::path::PathBuf;

    /// The generation path drops exactly the steps this predicate claims, so a false negative
    /// here re-introduces a multi-minute compile into a command contractually forbidden from
    /// compiling, and a false positive silently stops a real post-build step from running.
    /// Table-driven over every variant that could plausibly shell out. ~keep
    #[test]
    fn only_a_cargo_run_command_counts_as_invoking_the_compiler() {
        let cases: Vec<(&str, PostBuildStep, bool)> = vec![
            (
                "swift's release build",
                PostBuildStep::RunCommand {
                    cmd: "cargo",
                    args: vec!["build", "--release"],
                },
                true,
            ),
            (
                "swift's generate-time check -- still a whole-graph compile",
                PostBuildStep::RunCommand {
                    cmd: "cargo",
                    args: vec!["check"],
                },
                true,
            ),
            (
                "dart's frb codegen parses source and writes dart",
                PostBuildStep::RunCommand {
                    cmd: "flutter_rust_bridge_codegen",
                    args: vec!["generate"],
                },
                false,
            ),
            ("copying a built cdylib", PostBuildStep::StageFfiLibrary, false),
            (
                "an in-process file rewrite",
                PostBuildStep::PostProcessFile {
                    path: PathBuf::from("lib.dart"),
                    processor: PostProcessor::DartStripTrailingWhitespace,
                },
                false,
            ),
            (
                "copying swift-bridge output out of OUT_DIR",
                PostBuildStep::MaterializeSwiftBridge {
                    binding_crate_name: "sample-lib-swift".to_owned(),
                    package_root: "packages/swift".to_owned(),
                },
                false,
            ),
        ];

        for (description, step, expected) in cases {
            assert_eq!(step.invokes_rust_compiler(), expected, "{description}: {step:?}");
        }
    }
}

#[cfg(test)]
mod post_build_step_owned_paths_tests {
    use super::PostBuildStep;
    use std::path::{Path, PathBuf};

    /// The regression this guards: the orphan sweep in `bin_cli::core_commands` claims a
    /// path as generation-owned via `generation_owned_paths`, built from *this run's*
    /// `owned_paths()` union across every configured post-build step. If
    /// `MaterializeSwiftBridge` ever stopped naming every path it actually left on disk,
    /// the missing one would read as "alef no longer generates this" on the very next run
    /// and get deleted -- the alef #B incident this whole mechanism exists to prevent.
    /// Every caller of `owned_paths` runs it after the post-build step already executed, so
    /// this test writes the real trio to a tempdir first to prove the claim is still made
    /// once the files genuinely exist. ~keep
    #[test]
    fn materialize_swift_bridge_claims_all_three_files_it_writes_unguarded() {
        let base = tempfile::tempdir().expect("tempdir");
        let base_dir = base.path();
        let step = PostBuildStep::MaterializeSwiftBridge {
            binding_crate_name: "sample-lib-swift".to_string(),
            package_root: "packages/swift".to_string(),
        };
        let sources_rust_bridge = base_dir.join("packages/swift/Sources/RustBridge");
        let sources_rust_bridge_c = base_dir.join("packages/swift/Sources/RustBridgeC");
        std::fs::create_dir_all(&sources_rust_bridge).expect("create RustBridge dir");
        std::fs::create_dir_all(&sources_rust_bridge_c).expect("create RustBridgeC dir");
        std::fs::write(sources_rust_bridge_c.join("RustBridgeC.h"), "// header\n").expect("write header");
        std::fs::write(sources_rust_bridge.join("SwiftBridgeCore.swift"), "// core\n").expect("write core");
        std::fs::write(sources_rust_bridge.join("sample-lib-swift.swift"), "// crate\n").expect("write crate swift");

        let mut owned = step.owned_paths(base_dir);
        owned.sort();

        let mut expected = vec![
            sources_rust_bridge_c.join("RustBridgeC.h"),
            sources_rust_bridge.join("SwiftBridgeCore.swift"),
            sources_rust_bridge.join("sample-lib-swift.swift"),
        ];
        expected.sort();
        assert_eq!(owned, expected);
    }

    /// The bug this guards: before this fix, `owned_paths` predicted the full swift-bridge
    /// trio unconditionally, even though `emit_swift_bridge_files` only writes
    /// `SwiftBridgeCore.swift`/`{binding_crate_name}.swift` once it finds real build output
    /// (or an already-materialized header) -- on a project's first successful generation,
    /// before any real `cargo build` output exists, it writes the placeholder header alone.
    /// That mismatch put two never-written paths in the generation ownership manifest, which
    /// `cli_generate_atomicity`'s `failed_swift_post_build_preserves_owned_files_and_finalizes_written_outputs`
    /// catches as "the first successful generation must leave every owned output on disk".
    /// Only the header that actually exists must be claimed; the two paths that were never
    /// written must not be. ~keep
    #[test]
    fn materialize_swift_bridge_does_not_claim_trio_members_it_never_wrote() {
        let base = tempfile::tempdir().expect("tempdir");
        let base_dir = base.path();
        let step = PostBuildStep::MaterializeSwiftBridge {
            binding_crate_name: "sample-lib-swift".to_string(),
            package_root: "packages/swift".to_string(),
        };
        let sources_rust_bridge_c = base_dir.join("packages/swift/Sources/RustBridgeC");
        std::fs::create_dir_all(&sources_rust_bridge_c).expect("create RustBridgeC dir");
        // Only the placeholder header exists -- as `emit_swift_bridge_files` leaves it before
        // any real swift-bridge build output has ever been found.
        std::fs::write(sources_rust_bridge_c.join("RustBridgeC.h"), "// placeholder header\n")
            .expect("write placeholder header");

        let owned = step.owned_paths(base_dir);

        assert_eq!(
            owned,
            vec![sources_rust_bridge_c.join("RustBridgeC.h")],
            "only the header that was actually written must be claimed; got: {owned:?}"
        );
    }

    #[test]
    fn steps_that_flow_through_the_normal_write_path_claim_nothing() {
        let base_dir = Path::new("/repo");
        let steps = [
            PostBuildStep::PatchFile {
                path: "lib.rs",
                find: "a",
                replace: "b",
            },
            PostBuildStep::RunCommand {
                cmd: "cargo",
                args: vec!["build"],
            },
            PostBuildStep::StageDartNatives {
                lib_stem: "sample_lib_dart".to_string(),
            },
            PostBuildStep::CarryFrbCfgGates {
                source_path: PathBuf::from("lib.rs"),
                target_path: PathBuf::from("frb_generated.rs"),
            },
            PostBuildStep::RewriteWasmPackageName {
                package_json_path: PathBuf::from("packages/wasm/pkg/package.json"),
                package_name: "@sample/lib".to_string(),
            },
            PostBuildStep::VerifyFrbBridgeCoverage {
                facade_path: PathBuf::from("packages/dart/rust/src/lib.rs"),
                bridge_path: PathBuf::from("packages/dart/lib/src/sample_bridge_generated/lib.dart"),
                exclude_functions: vec![],
            },
        ];
        for step in &steps {
            assert!(
                step.owned_paths(base_dir).is_empty(),
                "{step:?} flows through the ownership-guarded writer already and must not \
                 also claim paths here"
            );
        }
    }
}
