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
}
