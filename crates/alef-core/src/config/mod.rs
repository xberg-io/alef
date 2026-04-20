use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub mod dto;
pub mod e2e;
pub mod extras;
pub mod languages;
pub mod output;
pub mod trait_bridge;

// Re-exports for backward compatibility — all types were previously flat in config.rs.
pub use dto::{
    CsharpDtoStyle, DtoConfig, ElixirDtoStyle, GoDtoStyle, JavaDtoStyle, NodeDtoStyle, PhpDtoStyle, PythonDtoStyle,
    RDtoStyle, RubyDtoStyle,
};
pub use e2e::E2eConfig;
pub use extras::{AdapterConfig, AdapterParam, AdapterPattern, Language};
pub use languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistration, CustomRegistrationsConfig, ElixirConfig, FfiConfig,
    GoConfig, JavaConfig, NodeConfig, PhpConfig, PythonConfig, RConfig, RubyConfig, StubsConfig, WasmConfig,
};
pub use output::{
    ExcludeConfig, IncludeConfig, LintConfig, OutputConfig, ReadmeConfig, ScaffoldConfig, SyncConfig, TestConfig,
    TextReplacement,
};
pub use trait_bridge::TraitBridgeConfig;

/// Root configuration from alef.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlefConfig {
    #[serde(rename = "crate")]
    pub crate_config: CrateConfig,
    pub languages: Vec<Language>,
    #[serde(default)]
    pub exclude: ExcludeConfig,
    #[serde(default)]
    pub include: IncludeConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub python: Option<PythonConfig>,
    #[serde(default)]
    pub node: Option<NodeConfig>,
    #[serde(default)]
    pub ruby: Option<RubyConfig>,
    #[serde(default)]
    pub php: Option<PhpConfig>,
    #[serde(default)]
    pub elixir: Option<ElixirConfig>,
    #[serde(default)]
    pub wasm: Option<WasmConfig>,
    #[serde(default)]
    pub ffi: Option<FfiConfig>,
    #[serde(default)]
    pub go: Option<GoConfig>,
    #[serde(default)]
    pub java: Option<JavaConfig>,
    #[serde(default)]
    pub csharp: Option<CSharpConfig>,
    #[serde(default)]
    pub r: Option<RConfig>,
    #[serde(default)]
    pub scaffold: Option<ScaffoldConfig>,
    #[serde(default)]
    pub readme: Option<ReadmeConfig>,
    #[serde(default)]
    pub lint: Option<HashMap<String, LintConfig>>,
    #[serde(default)]
    pub test: Option<HashMap<String, TestConfig>>,
    #[serde(default)]
    pub custom_files: Option<HashMap<String, Vec<PathBuf>>>,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    #[serde(default)]
    pub custom_modules: CustomModulesConfig,
    #[serde(default)]
    pub custom_registrations: CustomRegistrationsConfig,
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    /// Declare opaque types from external crates that alef can't extract.
    /// Map of type name → Rust path (e.g., "Tree" = "tree_sitter_language_pack::Tree").
    /// These get opaque wrapper structs in all backends.
    #[serde(default)]
    pub opaque_types: HashMap<String, String>,
    /// Controls which generation passes alef runs (all default to true).
    #[serde(default)]
    pub generate: GenerateConfig,
    /// Per-language overrides for generate flags (key = language name, e.g., "python").
    #[serde(default)]
    pub generate_overrides: HashMap<String, GenerateConfig>,
    /// Per-language DTO/type generation style (dataclass vs TypedDict, zod vs interface, etc.).
    #[serde(default)]
    pub dto: DtoConfig,
    /// E2E test generation configuration.
    #[serde(default)]
    pub e2e: Option<E2eConfig>,
    /// Trait bridge configurations — generate FFI bridge code that allows
    /// foreign language objects to implement Rust traits.
    #[serde(default)]
    pub trait_bridges: Vec<TraitBridgeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateConfig {
    pub name: String,
    pub sources: Vec<PathBuf>,
    #[serde(default = "default_version_from")]
    pub version_from: String,
    #[serde(default)]
    pub core_import: Option<String>,
    /// Optional workspace root path for resolving `pub use` re-exports from sibling crates.
    #[serde(default)]
    pub workspace_root: Option<PathBuf>,
    /// When true, skip adding `use {core_import};` to generated bindings.
    #[serde(default)]
    pub skip_core_import: bool,
    /// Cargo features that are enabled in binding crates.
    /// Fields gated by `#[cfg(feature = "...")]` matching these features
    /// are treated as always-present (cfg stripped from the IR).
    #[serde(default)]
    pub features: Vec<String>,
    /// Maps extracted rust_path prefixes to actual import paths in binding crates.
    /// Example: { "spikard" = "spikard_http" } rewrites "spikard::ServerConfig" to "spikard_http::ServerConfig"
    #[serde(default)]
    pub path_mappings: HashMap<String, String>,
}

fn default_version_from() -> String {
    "Cargo.toml".to_string()
}

fn default_true() -> bool {
    true
}

/// Controls which generation passes alef runs.
/// All flags default to `true`; set to `false` to skip a pass.
/// Can be overridden per-language via `[generate_overrides.<lang>]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateConfig {
    /// Generate low-level struct wrappers, From impls, module init (default: true)
    #[serde(default = "default_true")]
    pub bindings: bool,
    /// Generate error type hierarchies from thiserror enums (default: true)
    #[serde(default = "default_true")]
    pub errors: bool,
    /// Generate config builder constructors from Default types (default: true)
    #[serde(default = "default_true")]
    pub configs: bool,
    /// Generate async/sync function pairs with runtime management (default: true)
    #[serde(default = "default_true")]
    pub async_wrappers: bool,
    /// Generate recursive type marshaling helpers (default: true)
    #[serde(default = "default_true")]
    pub type_conversions: bool,
    /// Generate package manifests (pyproject.toml, package.json, etc.) (default: true)
    #[serde(default = "default_true")]
    pub package_metadata: bool,
    /// Generate idiomatic public API wrappers (default: true)
    #[serde(default = "default_true")]
    pub public_api: bool,
    /// Generate `From<BindingType> for CoreType` reverse conversions (default: true).
    /// Set to false when the binding layer only returns core types and never accepts them.
    #[serde(default = "default_true")]
    pub reverse_conversions: bool,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            bindings: true,
            errors: true,
            configs: true,
            async_wrappers: true,
            type_conversions: true,
            package_metadata: true,
            public_api: true,
            reverse_conversions: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared config resolution helpers
// ---------------------------------------------------------------------------

impl AlefConfig {
    /// Get the features to use for a specific language's binding crate.
    /// Checks for a per-language override first, then falls back to `[crate] features`.
    pub fn features_for_language(&self, lang: extras::Language) -> &[String] {
        let override_features = match lang {
            extras::Language::Python => self.python.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Node => self.node.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Ruby => self.ruby.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Php => self.php.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Elixir => self.elixir.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Wasm => self.wasm.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Ffi => self.ffi.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::R => self.r.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Rust => None, // Rust doesn't have binding-specific features
        };
        override_features.unwrap_or(&self.crate_config.features)
    }

    /// Get the core crate import path (e.g., "liter_llm"). Used by codegen to call into the core crate.
    pub fn core_import(&self) -> String {
        self.crate_config
            .core_import
            .clone()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the FFI prefix (e.g., "kreuzberg"). Used by FFI, Go, Java, C# backends.
    pub fn ffi_prefix(&self) -> String {
        self.ffi
            .as_ref()
            .and_then(|f| f.prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the FFI native library name (for Go cgo, Java Panama, C# P/Invoke).
    ///
    /// Resolution order:
    /// 1. `[ffi] lib_name` explicit override
    /// 2. Directory name of `output.ffi` path with hyphens replaced by underscores
    ///    (e.g. `crates/html-to-markdown-ffi/src/` → `html_to_markdown_ffi`)
    /// 3. `{ffi_prefix}_ffi` fallback
    pub fn ffi_lib_name(&self) -> String {
        // 1. Explicit override in [ffi] section.
        if let Some(name) = self.ffi.as_ref().and_then(|f| f.lib_name.as_ref()) {
            return name.clone();
        }

        // 2. Derive from output.ffi path: take the last meaningful directory component
        //    (skip trailing "src" or similar), then replace hyphens with underscores.
        if let Some(ffi_path) = self.output.ffi.as_ref() {
            let path = std::path::Path::new(ffi_path);
            // Walk components from the end to find the crate directory name.
            // Skip components like "src" that are inside the crate dir.
            let components: Vec<_> = path
                .components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(s) = c {
                        s.to_str()
                    } else {
                        None
                    }
                })
                .collect();
            // The crate name is typically the last component that looks like a crate dir
            // (i.e. not "src", "lib", or similar). Search from the end.
            let crate_dir = components
                .iter()
                .rev()
                .find(|&&s| s != "src" && s != "lib" && s != "include")
                .copied();
            if let Some(dir) = crate_dir {
                return dir.replace('-', "_");
            }
        }

        // 3. Default fallback.
        format!("{}_ffi", self.ffi_prefix())
    }

    /// Get the FFI header name.
    pub fn ffi_header_name(&self) -> String {
        self.ffi
            .as_ref()
            .and_then(|f| f.header_name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("{}.h", self.ffi_prefix()))
    }

    /// Get the Python module name.
    pub fn python_module_name(&self) -> String {
        self.python
            .as_ref()
            .and_then(|p| p.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("_{}", self.crate_config.name.replace('-', "_")))
    }

    /// Get the PyPI package name used as `[project] name` in `pyproject.toml`.
    ///
    /// Returns `[python] pip_name` if set, otherwise falls back to the crate name.
    pub fn python_pip_name(&self) -> String {
        self.python
            .as_ref()
            .and_then(|p| p.pip_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.clone())
    }

    /// Get the PHP Composer autoload namespace derived from the extension name.
    ///
    /// Converts the extension name (e.g. `html_to_markdown_rs`) into a
    /// PSR-4 namespace string (e.g. `Html\\To\\Markdown\\Rs`).
    pub fn php_autoload_namespace(&self) -> String {
        use heck::ToPascalCase;
        let ext = self.php_extension_name();
        if ext.contains('_') {
            ext.split('_')
                .map(|p| p.to_pascal_case())
                .collect::<Vec<_>>()
                .join("\\")
        } else {
            ext.to_pascal_case()
        }
    }

    /// Get the Node package name.
    pub fn node_package_name(&self) -> String {
        self.node
            .as_ref()
            .and_then(|n| n.package_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.clone())
    }

    /// Get the Ruby gem name.
    pub fn ruby_gem_name(&self) -> String {
        self.ruby
            .as_ref()
            .and_then(|r| r.gem_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the PHP extension name.
    pub fn php_extension_name(&self) -> String {
        self.php
            .as_ref()
            .and_then(|p| p.extension_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the Elixir app name.
    pub fn elixir_app_name(&self) -> String {
        self.elixir
            .as_ref()
            .and_then(|e| e.app_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the Go module path.
    pub fn go_module(&self) -> String {
        self.go
            .as_ref()
            .and_then(|g| g.module.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("github.com/kreuzberg-dev/{}", self.crate_config.name))
    }

    /// Get the GitHub repository URL.
    ///
    /// Resolution order:
    /// 1. `[e2e.registry] github_repo`
    /// 2. `[scaffold] repository`
    /// 3. Default: `https://github.com/kreuzberg-dev/{crate.name}`
    pub fn github_repo(&self) -> String {
        if let Some(e2e) = &self.e2e {
            if let Some(url) = &e2e.registry.github_repo {
                return url.clone();
            }
        }
        self.scaffold
            .as_ref()
            .and_then(|s| s.repository.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("https://github.com/kreuzberg-dev/{}", self.crate_config.name))
    }

    /// Get the Java package name.
    pub fn java_package(&self) -> String {
        self.java
            .as_ref()
            .and_then(|j| j.package.as_ref())
            .cloned()
            .unwrap_or_else(|| "dev.kreuzberg".to_string())
    }

    /// Get the Java Maven groupId.
    ///
    /// Uses the full Java package as the groupId, matching Maven convention
    /// where groupId equals the package declaration.
    pub fn java_group_id(&self) -> String {
        self.java_package()
    }

    /// Get the C# namespace.
    pub fn csharp_namespace(&self) -> String {
        self.csharp
            .as_ref()
            .and_then(|c| c.namespace.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                use heck::ToPascalCase;
                self.crate_config.name.to_pascal_case()
            })
    }

    /// Get the directory name of the core crate (derived from sources or falling back to name).
    ///
    /// For example, if `sources` contains `"crates/html-to-markdown/src/lib.rs"`, this returns
    /// `"html-to-markdown"`.  Used by the scaffold to generate correct `path = "../../crates/…"`
    /// references in binding-crate `Cargo.toml` files.
    pub fn core_crate_dir(&self) -> String {
        // Try to derive from first source path: "crates/foo/src/types/config.rs" → "foo"
        // Walk up from the file until we find the "src" directory, then take its parent.
        if let Some(first_source) = self.crate_config.sources.first() {
            let path = std::path::Path::new(first_source);
            let mut current = path.parent();
            while let Some(dir) = current {
                if dir.file_name().is_some_and(|n| n == "src") {
                    if let Some(crate_dir) = dir.parent() {
                        if let Some(dir_name) = crate_dir.file_name() {
                            return dir_name.to_string_lossy().into_owned();
                        }
                    }
                    break;
                }
                current = dir.parent();
            }
        }
        self.crate_config.name.clone()
    }

    /// Get the WASM type name prefix (e.g. "Wasm" produces `WasmConversionOptions`).
    /// Defaults to `"Wasm"`.
    pub fn wasm_type_prefix(&self) -> String {
        self.wasm
            .as_ref()
            .and_then(|w| w.type_prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| "Wasm".to_string())
    }

    /// Get the Node/NAPI type name prefix (e.g. "Js" produces `JsConversionOptions`).
    /// Defaults to `"Js"`.
    pub fn node_type_prefix(&self) -> String {
        self.node
            .as_ref()
            .and_then(|n| n.type_prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| "Js".to_string())
    }

    /// Get the R package name.
    pub fn r_package_name(&self) -> String {
        self.r
            .as_ref()
            .and_then(|r| r.package_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.clone())
    }

    /// Attempt to read the resolved version string from the configured `version_from` file.
    /// Returns `None` if the file cannot be read or the version cannot be found.
    pub fn resolved_version(&self) -> Option<String> {
        let content = std::fs::read_to_string(&self.crate_config.version_from).ok()?;
        let value: toml::Value = toml::from_str(&content).ok()?;
        if let Some(v) = value
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
        {
            return Some(v.to_string());
        }
        value
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
    }

    /// Get the effective serde rename_all strategy for a given language.
    ///
    /// Resolution order:
    /// 1. Per-language config override (`[python] serde_rename_all = "..."`)
    /// 2. Language default:
    ///    - camelCase: node, wasm, java, csharp
    ///    - snake_case: python, ruby, php, go, ffi, elixir, r
    pub fn serde_rename_all_for_language(&self, lang: extras::Language) -> String {
        // 1. Check per-language config override.
        let override_val = match lang {
            extras::Language::Python => self.python.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Node => self.node.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Ruby => self.ruby.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Php => self.php.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Elixir => self.elixir.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Wasm => self.wasm.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Ffi => self.ffi.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::R => self.r.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Rust => None, // Rust uses native naming (snake_case)
        };

        if let Some(val) = override_val {
            return val.to_string();
        }

        // 2. Language defaults.
        match lang {
            extras::Language::Node | extras::Language::Wasm | extras::Language::Java | extras::Language::Csharp => {
                "camelCase".to_string()
            }
            extras::Language::Python
            | extras::Language::Ruby
            | extras::Language::Php
            | extras::Language::Go
            | extras::Language::Ffi
            | extras::Language::Elixir
            | extras::Language::R
            | extras::Language::Rust => "snake_case".to_string(),
        }
    }

    /// Rewrite a rust_path using path_mappings.
    /// Matches the longest prefix first.
    pub fn rewrite_path(&self, rust_path: &str) -> String {
        // Sort mappings by key length descending (longest prefix first)
        let mut mappings: Vec<_> = self.crate_config.path_mappings.iter().collect();
        mappings.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

        for (from, to) in &mappings {
            if rust_path.starts_with(from.as_str()) {
                return format!("{}{}", to, &rust_path[from.len()..]);
            }
        }
        rust_path.to_string()
    }
}

/// Helper function to resolve output directory path from config.
/// Replaces {name} placeholder with the crate name.
pub fn resolve_output_dir(config_path: Option<&PathBuf>, crate_name: &str, default: &str) -> String {
    config_path
        .map(|p| p.to_string_lossy().replace("{name}", crate_name))
        .unwrap_or_else(|| default.replace("{name}", crate_name))
}

/// Detect whether `serde` and `serde_json` are available in a binding crate's Cargo.toml.
///
/// `output_dir` is the generated source directory (e.g., `crates/spikard-py/src/`).
/// The function walks up to find the crate's Cargo.toml and checks its `[dependencies]`
/// for both `serde` and `serde_json`.
pub fn detect_serde_available(output_dir: &str) -> bool {
    let src_path = std::path::Path::new(output_dir);
    // Walk up from the output dir to find Cargo.toml (usually output_dir is `crates/foo/src/`)
    let mut dir = src_path;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return cargo_toml_has_serde(&cargo_toml);
        }
        match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => dir = parent,
            _ => break,
        }
    }
    false
}

/// Check if a Cargo.toml has both `serde` (with derive feature) and `serde_json` in its dependencies.
///
/// The `serde::Serialize` derive macro requires `serde` as a direct dependency with the `derive`
/// feature enabled. Having only `serde_json` is not sufficient since it only pulls in `serde`
/// transitively without the derive proc-macro.
fn cargo_toml_has_serde(path: &std::path::Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let has_serde_json = content.contains("serde_json");
    // Check for `serde` as a direct dependency (not just serde_json).
    // Must match "serde" as a TOML key, not as a substring of "serde_json".
    // Valid patterns: `serde = `, `serde.`, `[dependencies.serde]`
    let has_serde_dep = content.lines().any(|line| {
        let trimmed = line.trim();
        // Match `serde = ...` or `serde.workspace = true` etc., but not `serde_json`
        trimmed.starts_with("serde ")
            || trimmed.starts_with("serde=")
            || trimmed.starts_with("serde.")
            || trimmed == "[dependencies.serde]"
    });

    has_serde_json && has_serde_dep
}
