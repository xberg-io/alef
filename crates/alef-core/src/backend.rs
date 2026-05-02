use crate::config::{Language, ResolvedCrateConfig};
use crate::ir::ApiSurface;
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

    /// Generate type stubs (.pyi, .rbs, .d.ts). Optional — default returns empty.
    fn generate_type_stubs(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        Ok(vec![])
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

    /// Build configuration for this backend. Returns `None` if build is not supported.
    fn build_config(&self) -> Option<BuildConfig> {
        None
    }
}
