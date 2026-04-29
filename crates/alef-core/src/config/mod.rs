use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub mod build_defaults;
pub mod clean_defaults;
pub mod dto;
pub mod e2e;
pub mod extras;
pub mod languages;
pub mod lint_defaults;
pub mod output;
pub mod publish;
pub mod setup_defaults;
pub mod test_defaults;
pub mod tools;
pub mod trait_bridge;
pub mod update_defaults;
pub mod validation;

// Re-exports for backward compatibility — all types were previously flat in config.rs.
pub use dto::{
    CsharpDtoStyle, DtoConfig, ElixirDtoStyle, GoDtoStyle, JavaDtoStyle, NodeDtoStyle, PhpDtoStyle, PythonDtoStyle,
    RDtoStyle, RubyDtoStyle,
};
pub use e2e::E2eConfig;
pub use extras::{AdapterConfig, AdapterParam, AdapterPattern, Language};
pub use languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistration, CustomRegistrationsConfig, DartConfig, DartStyle,
    ElixirConfig, FfiConfig, GleamConfig, GoConfig, JavaConfig, KotlinConfig, KotlinTarget, NodeConfig, PhpConfig,
    PythonConfig, RConfig, RubyConfig, StubsConfig, SwiftConfig, WasmConfig, ZigConfig,
};
pub use output::{
    BuildCommandConfig, CleanConfig, ExcludeConfig, IncludeConfig, LintConfig, OutputConfig, ReadmeConfig,
    ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, TextReplacement, UpdateConfig,
};
pub use publish::{PublishConfig, PublishLanguageConfig, VendorMode};
pub use tools::{DEFAULT_RUST_DEV_TOOLS, LangContext, ToolsConfig, require_tool, require_tools};
pub use trait_bridge::TraitBridgeConfig;

/// Root configuration from alef.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlefConfig {
    /// Pinned alef CLI version (e.g. "0.7.7"). Used by install-alef to install
    /// the exact version this project expects.
    #[serde(default)]
    pub version: Option<String>,
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
    pub gleam: Option<GleamConfig>,
    #[serde(default)]
    pub go: Option<GoConfig>,
    #[serde(default)]
    pub java: Option<JavaConfig>,
    #[serde(default)]
    pub dart: Option<DartConfig>,
    #[serde(default)]
    pub kotlin: Option<KotlinConfig>,
    #[serde(default)]
    pub swift: Option<SwiftConfig>,
    #[serde(default)]
    pub csharp: Option<CSharpConfig>,
    #[serde(default)]
    pub r: Option<RConfig>,
    #[serde(default)]
    pub zig: Option<ZigConfig>,
    #[serde(default)]
    pub scaffold: Option<ScaffoldConfig>,
    #[serde(default)]
    pub readme: Option<ReadmeConfig>,
    #[serde(default)]
    pub lint: Option<HashMap<String, LintConfig>>,
    #[serde(default)]
    pub update: Option<HashMap<String, UpdateConfig>>,
    #[serde(default)]
    pub test: Option<HashMap<String, TestConfig>>,
    #[serde(default)]
    pub setup: Option<HashMap<String, SetupConfig>>,
    #[serde(default)]
    pub clean: Option<HashMap<String, CleanConfig>>,
    #[serde(default)]
    pub build_commands: Option<HashMap<String, BuildCommandConfig>>,
    /// Publish pipeline configuration (vendoring, packaging, cross-compilation).
    #[serde(default)]
    pub publish: Option<PublishConfig>,
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
    /// Post-generation formatting configuration (default: enabled for all languages).
    #[serde(default)]
    pub format: FormatConfig,
    /// Per-language formatting overrides (key = language name, e.g., "elixir").
    #[serde(default)]
    pub format_overrides: HashMap<String, FormatConfig>,
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
    /// Global tooling preferences — package managers and dev tools used by
    /// the default per-language pipeline commands. Sensible defaults apply
    /// when omitted.
    #[serde(default)]
    pub tools: ToolsConfig,
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
    /// The crate's error type name (e.g., `"KreuzbergError"`).
    /// Used in trait bridge generation for error wrapping.
    /// Defaults to `"Error"` if not set.
    #[serde(default)]
    pub error_type: Option<String>,
    /// Pattern for constructing error values from a String message in trait bridges.
    /// `{msg}` is replaced with the format!(...) expression.
    /// Example: `"KreuzbergError::Plugin { message: {msg}, plugin_name: name.to_string() }"`
    /// Defaults to `"{error_type}::from({msg})"` if not set.
    #[serde(default)]
    pub error_constructor: Option<String>,
    /// Cargo features that are enabled in binding crates.
    /// Fields gated by `#[cfg(feature = "...")]` matching these features
    /// are treated as always-present (cfg stripped from the IR).
    #[serde(default)]
    pub features: Vec<String>,
    /// Maps extracted rust_path prefixes to actual import paths in binding crates.
    /// Example: { "spikard" = "spikard_http" } rewrites "spikard::ServerConfig" to "spikard_http::ServerConfig"
    #[serde(default)]
    pub path_mappings: HashMap<String, String>,
    /// Additional Cargo dependencies added to ALL binding crate Cargo.tomls.
    /// Each entry is a crate name mapping to a TOML dependency spec
    /// (string for version-only, or inline table for path/features/etc.).
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// When true (default), automatically derive path_mappings from source file locations.
    /// For each source file matching `crates/{name}/src/`, adds a mapping from
    /// `{name}` to the configured `core_import`.
    #[serde(default = "default_true")]
    pub auto_path_mappings: bool,
    /// Multi-crate source groups for workspaces with types spread across crates.
    /// Each entry has a crate `name` and `sources` list. Types extracted from each
    /// group get `rust_path` reflecting the actual defining crate, not the facade.
    /// When non-empty, the top-level `sources` field is ignored.
    #[serde(default)]
    pub source_crates: Vec<SourceCrate>,
}

/// A source crate group for multi-crate extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCrate {
    /// Crate name (hyphens converted to underscores for rust_path).
    pub name: String,
    /// Source files belonging to this crate.
    pub sources: Vec<PathBuf>,
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

/// Post-generation formatting configuration.
/// After code generation, alef can automatically run language-native formatters
/// on the emitted package directories to ensure CI formatter checks pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConfig {
    /// Enable post-generation formatting (default: true).
    /// Set to false to skip formatting for all languages, or use per-language
    /// overrides in `[format.<lang>]` to disable specific formatters.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional custom command override. If set, this command is run instead
    /// of the language's default formatter. Must be a shell command string
    /// (e.g., "prettier --write .").
    #[serde(default)]
    pub command: Option<String>,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared config resolution helpers
// ---------------------------------------------------------------------------

impl AlefConfig {
    /// Resolve the binding field name for a given language, type, and field.
    ///
    /// Resolution order (highest to lowest priority):
    /// 1. Per-language `rename_fields` map for the key `"TypeName.field_name"`.
    /// 2. Automatic keyword escaping: if the field name is a reserved keyword in the target
    ///    language, append `_` (e.g. `class` → `class_`).
    /// 3. Original field name unchanged.
    ///
    /// Returns `Some(escaped_name)` when the field needs renaming, `None` when the original
    /// name can be used as-is.  Call sites that always need a `String` should use
    /// `resolve_field_name(...).unwrap_or_else(|| field_name.to_string())`.
    pub fn resolve_field_name(&self, lang: extras::Language, type_name: &str, field_name: &str) -> Option<String> {
        // 1. Explicit per-language rename_fields entry.
        let explicit_key = format!("{type_name}.{field_name}");
        let explicit = match lang {
            extras::Language::Python => self.python.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Node => self.node.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Ruby => self.ruby.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Php => self.php.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Elixir => self.elixir.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Wasm => self.wasm.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Ffi => self.ffi.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Gleam => self.gleam.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::R => self.r.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Zig => self.zig.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Dart => self.dart.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Swift => self.swift.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            extras::Language::Rust => None,
        };
        if let Some(renamed) = explicit {
            if renamed != field_name {
                return Some(renamed.clone());
            }
            return None;
        }

        // 2. Automatic keyword escaping.
        match lang {
            extras::Language::Python => crate::keywords::python_safe_name(field_name),
            // Java and C# use PascalCase for field names so `class` becomes `Class` — no conflict.
            // Go uses PascalCase for exported fields — no conflict.
            // JS/TS uses camelCase — `class` becomes `class` but is a JS keyword; Node backend
            // handles this via js_name attributes at the napi layer. For now only Python is wired.
            _ => None,
        }
    }

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
            extras::Language::Gleam => self.gleam.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::R => self.r.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Zig => self.zig.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Dart => self.dart.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Swift => self.swift.as_ref().and_then(|c| c.features.as_deref()),
            extras::Language::Rust => None, // Rust doesn't have binding-specific features
        };
        override_features.unwrap_or(&self.crate_config.features)
    }

    /// Get the merged extra dependencies for a specific language's binding crate.
    /// Merges crate-level `extra_dependencies` with per-language overrides.
    /// Language-specific entries override crate-level entries with the same key.
    pub fn extra_deps_for_language(&self, lang: extras::Language) -> HashMap<String, toml::Value> {
        let mut deps = self.crate_config.extra_dependencies.clone();
        let lang_deps = match lang {
            extras::Language::Python => self.python.as_ref().map(|c| &c.extra_dependencies),
            extras::Language::Node => self.node.as_ref().map(|c| &c.extra_dependencies),
            extras::Language::Ruby => self.ruby.as_ref().map(|c| &c.extra_dependencies),
            extras::Language::Php => self.php.as_ref().map(|c| &c.extra_dependencies),
            extras::Language::Elixir => self.elixir.as_ref().map(|c| &c.extra_dependencies),
            extras::Language::Wasm => self.wasm.as_ref().map(|c| &c.extra_dependencies),
            _ => None,
        };
        if let Some(lang_deps) = lang_deps {
            deps.extend(lang_deps.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        deps
    }

    /// Get the package output directory for a language.
    /// Uses `scaffold_output` from per-language config if set, otherwise defaults.
    ///
    /// Defaults: `packages/python`, `packages/node`, `packages/ruby`, `packages/php`, `packages/elixir`
    pub fn package_dir(&self, lang: extras::Language) -> String {
        let override_path = match lang {
            extras::Language::Python => self.python.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            extras::Language::Node => self.node.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            extras::Language::Ruby => self.ruby.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            extras::Language::Php => self.php.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            extras::Language::Elixir => self.elixir.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            _ => None,
        };
        if let Some(p) = override_path {
            p.to_string_lossy().to_string()
        } else {
            match lang {
                extras::Language::Python => "packages/python".to_string(),
                extras::Language::Node => "packages/node".to_string(),
                extras::Language::Ruby => "packages/ruby".to_string(),
                extras::Language::Php => "packages/php".to_string(),
                extras::Language::Elixir => "packages/elixir".to_string(),
                _ => format!("packages/{lang}"),
            }
        }
    }

    /// Validate user-supplied pipeline overrides.
    ///
    /// Custom `[lint|test|build_commands|setup|update|clean].<lang>` tables
    /// that override a main command field must declare a `precondition`
    /// so the step degrades gracefully when the underlying tool is missing
    /// on the user's system. See [`validation::validate`] for details.
    pub fn validate(&self) -> Result<(), crate::error::AlefError> {
        validation::validate(self)
    }

    /// Get the effective lint configuration for a language.
    ///
    /// Returns the explicit `[lint.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn lint_config_for_language(&self, lang: extras::Language) -> output::LintConfig {
        if let Some(lint_map) = &self.lint {
            let lang_str = lang.to_string();
            if let Some(explicit) = lint_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let extra_lint_paths = self.extra_lint_paths_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths,
            project_file,
        };
        lint_defaults::default_lint_config(lang, &output_dir, &ctx)
    }

    /// Get the effective update configuration for a language.
    ///
    /// Returns the explicit `[update.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn update_config_for_language(&self, lang: extras::Language) -> output::UpdateConfig {
        if let Some(update_map) = &self.update {
            let lang_str = lang.to_string();
            if let Some(explicit) = update_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        update_defaults::default_update_config(lang, &output_dir, &ctx)
    }

    /// Get the effective test configuration for a language.
    ///
    /// Returns the explicit `[test.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn test_config_for_language(&self, lang: extras::Language) -> output::TestConfig {
        if let Some(test_map) = &self.test {
            let lang_str = lang.to_string();
            if let Some(explicit) = test_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths: &[],
            project_file,
        };
        test_defaults::default_test_config(lang, &output_dir, &ctx)
    }

    /// Get the effective setup configuration for a language.
    ///
    /// Returns the explicit `[setup.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn setup_config_for_language(&self, lang: extras::Language) -> output::SetupConfig {
        if let Some(setup_map) = &self.setup {
            let lang_str = lang.to_string();
            if let Some(explicit) = setup_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        setup_defaults::default_setup_config(lang, &output_dir, &ctx)
    }

    /// Get the effective clean configuration for a language.
    ///
    /// Returns the explicit `[clean.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn clean_config_for_language(&self, lang: extras::Language) -> output::CleanConfig {
        if let Some(clean_map) = &self.clean {
            let lang_str = lang.to_string();
            if let Some(explicit) = clean_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        clean_defaults::default_clean_config(lang, &output_dir, &ctx)
    }

    /// Get the effective build command configuration for a language.
    ///
    /// Returns the explicit `[build_commands.<lang>]` config if present in alef.toml,
    /// otherwise falls back to sensible defaults for the language.
    pub fn build_command_config_for_language(&self, lang: extras::Language) -> output::BuildCommandConfig {
        if let Some(build_map) = &self.build_commands {
            let lang_str = lang.to_string();
            if let Some(explicit) = build_map.get(&lang_str) {
                return explicit.clone();
            }
        }
        let output_dir = self.package_dir(lang);
        let crate_name = &self.crate_config.name;
        let run_wrapper = self.run_wrapper_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths: &[],
            project_file,
        };
        build_defaults::default_build_config(lang, &output_dir, crate_name, &ctx)
    }

    /// Get the core crate import path (e.g., "liter_llm"). Used by codegen to call into the core crate.
    pub fn core_import(&self) -> String {
        self.crate_config
            .core_import
            .clone()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the crate error type name (e.g., "KreuzbergError"). Defaults to "Error".
    pub fn error_type(&self) -> String {
        self.crate_config
            .error_type
            .clone()
            .unwrap_or_else(|| "Error".to_string())
    }

    /// Get the error constructor pattern. `{msg}` is replaced with the message expression.
    /// Defaults to `"{core_import}::{error_type}::from({msg})"`.
    pub fn error_constructor(&self) -> String {
        self.crate_config
            .error_constructor
            .clone()
            .unwrap_or_else(|| format!("{}::{}::from({{msg}})", self.core_import(), self.error_type()))
    }

    /// Get the run_wrapper for a language, if set.
    /// Returns the wrapper string that prefixes default tool invocations.
    pub fn run_wrapper_for_language(&self, lang: extras::Language) -> Option<&str> {
        match lang {
            extras::Language::Python => self.python.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Node => self.node.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Ruby => self.ruby.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Php => self.php.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Elixir => self.elixir.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Wasm => self.wasm.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::R => self.r.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Dart => self.dart.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Swift => self.swift.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Gleam => self.gleam.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Zig => self.zig.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            extras::Language::Ffi | extras::Language::Rust => None,
        }
    }

    /// Get the extra_lint_paths for a language.
    /// Returns a slice of paths to append to default lint commands.
    pub fn extra_lint_paths_for_language(&self, lang: extras::Language) -> &[String] {
        match lang {
            extras::Language::Python => self
                .python
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::Node => self.node.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Ruby => self.ruby.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Php => self.php.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Elixir => self
                .elixir
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::Wasm => self.wasm.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Go => self.go.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Java => self.java.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Csharp => self
                .csharp
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::R => self.r.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Kotlin => self
                .kotlin
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::Dart => self.dart.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Swift => self
                .swift
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::Gleam => self
                .gleam
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            extras::Language::Zig => self.zig.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            extras::Language::Ffi | extras::Language::Rust => &[],
        }
    }

    /// Get the project_file for a language (Java or C# only).
    /// Returns the project file path that defaults use instead of output directory.
    pub fn project_file_for_language(&self, lang: extras::Language) -> Option<&str> {
        match lang {
            extras::Language::Java => self.java.as_ref().and_then(|c| c.project_file.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.project_file.as_deref()),
            _ => None,
        }
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

    /// Get the Dart bridging style (`frb` or `ffi`).
    pub fn dart_style(&self) -> languages::DartStyle {
        self.dart.as_ref().map(|d| d.style).unwrap_or_default()
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

    /// Get the GitHub repository URL, returning an error when no source has it set.
    ///
    /// Resolution order:
    /// 1. `[e2e.registry] github_repo`
    /// 2. `[scaffold] repository`
    ///
    /// Callers that can render meaningful output without a repository URL should
    /// use [`Self::github_repo`] instead, which falls back to a vendor-neutral
    /// placeholder (`https://example.invalid/{crate.name}`).
    pub fn try_github_repo(&self) -> Result<String, String> {
        if let Some(e2e) = &self.e2e {
            if let Some(url) = &e2e.registry.github_repo {
                return Ok(url.clone());
            }
        }
        if let Some(url) = self.scaffold.as_ref().and_then(|s| s.repository.as_ref()) {
            return Ok(url.clone());
        }
        Err(format!(
            "no repository URL configured — set `[scaffold] repository = \"...\"` (or `[e2e.registry] github_repo`) for crate `{}`",
            self.crate_config.name
        ))
    }

    /// Get the GitHub repository URL with a vendor-neutral placeholder fallback.
    ///
    /// Resolution order:
    /// 1. `[e2e.registry] github_repo`
    /// 2. `[scaffold] repository`
    /// 3. Placeholder: `https://example.invalid/{crate.name}` — surfaces missing
    ///    config in code review without smuggling another organization's URL.
    pub fn github_repo(&self) -> String {
        self.try_github_repo()
            .unwrap_or_else(|_| format!("https://example.invalid/{}", self.crate_config.name))
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

    /// Get the Kotlin package name.
    pub fn kotlin_package(&self) -> String {
        self.kotlin
            .as_ref()
            .and_then(|k| k.package.as_ref())
            .cloned()
            .unwrap_or_else(|| "dev.kreuzberg".to_string())
    }

    /// Get the Kotlin target platform.
    ///
    /// Returns `KotlinTarget::Jvm` (the default) when the `[kotlin]` section is absent or
    /// `target` is not set.
    pub fn kotlin_target(&self) -> KotlinTarget {
        self.kotlin.as_ref().map(|k| k.target).unwrap_or_default()
    }

    /// Get the Dart pubspec package name.
    ///
    /// Returns `[dart] pubspec_name` if set, otherwise derives a snake_case
    /// name from the crate name by replacing hyphens with underscores.
    pub fn dart_pubspec_name(&self) -> String {
        self.dart
            .as_ref()
            .and_then(|d| d.pubspec_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the resolved flutter_rust_bridge version, falling back to
    /// `template_versions::cargo::FLUTTER_RUST_BRIDGE`.
    pub fn dart_frb_version(&self) -> String {
        self.dart
            .as_ref()
            .and_then(|d| d.frb_version.as_ref())
            .cloned()
            .unwrap_or_else(|| crate::template_versions::cargo::FLUTTER_RUST_BRIDGE.to_string())
    }

    /// Get the Swift module name.
    ///
    /// Returns `[swift] module_name` if configured, otherwise derives a PascalCase
    /// name from the crate name (e.g. `"my-lib"` → `"MyLib"`).
    pub fn swift_module(&self) -> String {
        self.swift
            .as_ref()
            .and_then(|s| s.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                use heck::ToUpperCamelCase;
                self.crate_config.name.to_upper_camel_case()
            })
    }

    /// Get the resolved swift-bridge version, falling back to
    /// `template_versions::cargo::SWIFT_BRIDGE`.
    pub fn swift_bridge_version(&self) -> String {
        self.swift
            .as_ref()
            .and_then(|s| s.swift_bridge_version.as_ref())
            .cloned()
            .unwrap_or_else(|| crate::template_versions::cargo::SWIFT_BRIDGE.to_string())
    }

    /// Get the resolved minimum macOS deployment target.
    pub fn swift_min_macos(&self) -> String {
        self.swift
            .as_ref()
            .and_then(|s| s.min_macos_version.as_ref())
            .cloned()
            .unwrap_or_else(|| crate::template_versions::toolchain::SWIFT_MIN_MACOS.to_string())
    }

    /// Get the resolved minimum iOS deployment target.
    pub fn swift_min_ios(&self) -> String {
        self.swift
            .as_ref()
            .and_then(|s| s.min_ios_version.as_ref())
            .cloned()
            .unwrap_or_else(|| crate::template_versions::toolchain::SWIFT_MIN_IOS.to_string())
    }

    /// Get the Gleam app name.
    pub fn gleam_app_name(&self) -> String {
        self.gleam
            .as_ref()
            .and_then(|g| g.app_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
    }

    /// Get the Gleam NIF module name (Erlang atom for @external(erlang, "<nif>", ...) lookups).
    /// Defaults to "Elixir.<PascalCase>.Native" to match the atom registered by
    /// `rustler::init!` in the Rustler backend.
    pub fn gleam_nif_module(&self) -> String {
        use heck::ToUpperCamelCase;
        self.gleam
            .as_ref()
            .and_then(|g| g.nif_module.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                let pascal = self
                    .elixir
                    .as_ref()
                    .and_then(|e| e.app_name.as_deref())
                    .unwrap_or(&self.crate_config.name)
                    .to_upper_camel_case();
                format!("Elixir.{pascal}.Native")
            })
    }

    /// Get the Zig module name.
    pub fn zig_module_name(&self) -> String {
        self.zig
            .as_ref()
            .and_then(|z| z.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.crate_config.name.replace('-', "_"))
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
    ///    - snake_case: python, ruby, php, go, ffi, elixir, r, kotlin, gleam, zig
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
            extras::Language::Gleam => self.gleam.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Go => self.go.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Java => self.java.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Csharp => self.csharp.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::R => self.r.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Zig => self.zig.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Dart => self.dart.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            extras::Language::Swift => self.swift.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
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
            | extras::Language::Rust
            | extras::Language::Kotlin
            | extras::Language::Gleam
            | extras::Language::Zig
            | extras::Language::Swift
            | extras::Language::Dart => "snake_case".to_string(),
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

    /// Return the effective path mappings for this config.
    ///
    /// When `auto_path_mappings` is true, automatically derives a mapping from each source
    /// crate to the configured `core_import` facade.  For each source file whose path contains
    /// `crates/{crate-name}/src/`, a mapping `{crate_name}` → `{core_import}` is added
    /// (hyphens in the crate name are converted to underscores).  Source crates that already
    /// equal `core_import` are skipped.
    ///
    /// Explicit entries in `path_mappings` always override auto-derived ones.
    pub fn effective_path_mappings(&self) -> HashMap<String, String> {
        let mut mappings = HashMap::new();

        if self.crate_config.auto_path_mappings {
            let core_import = self.core_import();

            for source in &self.crate_config.sources {
                let source_str = source.to_string_lossy();
                // Match `crates/{name}/src/` pattern in the path.
                if let Some(after_crates) = find_after_crates_prefix(&source_str) {
                    // Extract the crate directory name (everything before the next `/`).
                    if let Some(slash_pos) = after_crates.find('/') {
                        let crate_dir = &after_crates[..slash_pos];
                        let crate_ident = crate_dir.replace('-', "_");
                        // Only add a mapping when the source crate differs from the facade.
                        if crate_ident != core_import && !mappings.contains_key(&crate_ident) {
                            mappings.insert(crate_ident, core_import.clone());
                        }
                    }
                }
            }
        }

        // Explicit path_mappings always win — insert last so they overwrite auto entries.
        for (from, to) in &self.crate_config.path_mappings {
            mappings.insert(from.clone(), to.clone());
        }

        mappings
    }
}

/// Find the path segment that comes after a `crates/` component.
///
/// Handles both absolute paths (e.g., `/workspace/repo/crates/foo/src/lib.rs`)
/// and relative paths (e.g., `crates/foo/src/lib.rs`).  Returns the slice
/// starting immediately after the `crates/` prefix, or `None` if the path
/// does not contain such a component.
fn find_after_crates_prefix(path: &str) -> Option<&str> {
    // Normalise to forward slashes for cross-platform matching.
    // We search for `/crates/` (with leading slash) first, then fall back to
    // a leading `crates/` for relative paths that start with that component.
    if let Some(pos) = path.find("/crates/") {
        return Some(&path[pos + "/crates/".len()..]);
    }
    if let Some(stripped) = path.strip_prefix("crates/") {
        return Some(stripped);
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config() -> AlefConfig {
        toml::from_str(
            r#"
languages = ["python", "node", "rust"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn lint_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.lint.is_none());

        let py = config.lint_config_for_language(Language::Python);
        assert!(py.format.is_some());
        assert!(py.check.is_some());
        assert!(py.typecheck.is_some());

        let node = config.lint_config_for_language(Language::Node);
        assert!(node.format.is_some());
        assert!(node.check.is_some());
    }

    #[test]
    fn lint_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[lint.python]
format = "custom-formatter"
check = "custom-checker"
"#,
        )
        .unwrap();

        let py = config.lint_config_for_language(Language::Python);
        assert_eq!(py.format.unwrap().commands(), vec!["custom-formatter"]);
        assert_eq!(py.check.unwrap().commands(), vec!["custom-checker"]);
        assert!(py.typecheck.is_none()); // explicit config had no typecheck
    }

    #[test]
    fn lint_config_partial_override_does_not_merge() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[lint.python]
format = "only-format"
"#,
        )
        .unwrap();

        let py = config.lint_config_for_language(Language::Python);
        assert_eq!(py.format.unwrap().commands(), vec!["only-format"]);
        // Explicit config replaces entirely, no fallback for missing fields
        assert!(py.check.is_none());
        assert!(py.typecheck.is_none());
    }

    #[test]
    fn lint_config_unconfigured_language_uses_defaults() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python", "node"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[lint.python]
format = "custom"
"#,
        )
        .unwrap();

        // Python uses explicit config
        let py = config.lint_config_for_language(Language::Python);
        assert_eq!(py.format.unwrap().commands(), vec!["custom"]);

        // Node falls back to defaults since not in [lint]
        let node = config.lint_config_for_language(Language::Node);
        let fmt = node.format.unwrap().commands().join(" ");
        assert!(fmt.contains("oxfmt"));
    }

    #[test]
    fn update_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.update.is_none());

        let py = config.update_config_for_language(Language::Python);
        assert!(py.update.is_some());
        assert!(py.upgrade.is_some());

        let rust = config.update_config_for_language(Language::Rust);
        let update = rust.update.unwrap().commands().join(" ");
        assert!(update.contains("cargo update"));
    }

    #[test]
    fn update_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["rust"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[update.rust]
update = "my-custom-update"
upgrade = ["step1", "step2"]
"#,
        )
        .unwrap();

        let rust = config.update_config_for_language(Language::Rust);
        assert_eq!(rust.update.unwrap().commands(), vec!["my-custom-update"]);
        assert_eq!(rust.upgrade.unwrap().commands(), vec!["step1", "step2"]);
    }

    #[test]
    fn test_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.test.is_none());

        let py = config.test_config_for_language(Language::Python);
        assert!(py.command.is_some());
        assert!(py.coverage.is_some());
        assert!(py.e2e.is_none());

        let rust = config.test_config_for_language(Language::Rust);
        let cmd = rust.command.unwrap().commands().join(" ");
        assert!(cmd.contains("cargo test"));
    }

    #[test]
    fn test_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[test.python]
command = "my-custom-test"
"#,
        )
        .unwrap();

        let py = config.test_config_for_language(Language::Python);
        assert_eq!(py.command.unwrap().commands(), vec!["my-custom-test"]);
        assert!(py.coverage.is_none()); // explicit config had no coverage
    }

    #[test]
    fn setup_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.setup.is_none());

        let py = config.setup_config_for_language(Language::Python);
        assert!(py.install.is_some());
        let install = py.install.unwrap().commands().join(" ");
        assert!(install.contains("uv sync"));

        let rust = config.setup_config_for_language(Language::Rust);
        let install = rust.install.unwrap().commands().join(" ");
        assert!(install.contains("rustup update"));
    }

    #[test]
    fn setup_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[setup.python]
install = "my-custom-install"
"#,
        )
        .unwrap();

        let py = config.setup_config_for_language(Language::Python);
        assert_eq!(py.install.unwrap().commands(), vec!["my-custom-install"]);
    }

    #[test]
    fn clean_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.clean.is_none());

        let py = config.clean_config_for_language(Language::Python);
        assert!(py.clean.is_some());
        let clean = py.clean.unwrap().commands().join(" ");
        assert!(clean.contains("__pycache__"));

        let rust = config.clean_config_for_language(Language::Rust);
        let clean = rust.clean.unwrap().commands().join(" ");
        assert!(clean.contains("cargo clean"));
    }

    #[test]
    fn clean_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["rust"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[clean.rust]
clean = "my-custom-clean"
"#,
        )
        .unwrap();

        let rust = config.clean_config_for_language(Language::Rust);
        assert_eq!(rust.clean.unwrap().commands(), vec!["my-custom-clean"]);
    }

    #[test]
    fn build_command_config_falls_back_to_defaults() {
        let config = minimal_config();
        assert!(config.build_commands.is_none());

        let py = config.build_command_config_for_language(Language::Python);
        assert!(py.build.is_some());
        assert!(py.build_release.is_some());
        let build = py.build.unwrap().commands().join(" ");
        assert!(build.contains("maturin develop"));

        let rust = config.build_command_config_for_language(Language::Rust);
        let build = rust.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build --workspace"));
    }

    #[test]
    fn build_command_config_explicit_overrides_default() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["rust"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[build_commands.rust]
build = "my-custom-build"
build_release = "my-custom-build --release"
"#,
        )
        .unwrap();

        let rust = config.build_command_config_for_language(Language::Rust);
        assert_eq!(rust.build.unwrap().commands(), vec!["my-custom-build"]);
        assert_eq!(
            rust.build_release.unwrap().commands(),
            vec!["my-custom-build --release"]
        );
    }

    #[test]
    fn build_command_config_uses_crate_name() {
        let config = minimal_config();
        let py = config.build_command_config_for_language(Language::Python);
        let build = py.build.unwrap().commands().join(" ");
        assert!(
            build.contains("test-lib-py"),
            "Python build should reference crate name, got: {build}"
        );
    }

    #[test]
    fn package_dir_defaults_are_correct() {
        let config = minimal_config();
        assert_eq!(config.package_dir(Language::Python), "packages/python");
        assert_eq!(config.package_dir(Language::Node), "packages/node");
        assert_eq!(config.package_dir(Language::Ruby), "packages/ruby");
        assert_eq!(config.package_dir(Language::Go), "packages/go");
        assert_eq!(config.package_dir(Language::Java), "packages/java");
    }

    #[test]
    fn explicit_lint_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["go"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[lint.go]
precondition = "test -f target/release/libtest_ffi.so"
before = "cargo build --release -p test-ffi"
format = "gofmt -w packages/go"
check = "golangci-lint run ./..."
"#,
        )
        .unwrap();

        let lint = config.lint_config_for_language(Language::Go);
        assert_eq!(
            lint.precondition.as_deref(),
            Some("test -f target/release/libtest_ffi.so"),
            "precondition should be preserved from explicit config"
        );
        assert_eq!(
            lint.before.unwrap().commands(),
            vec!["cargo build --release -p test-ffi"],
            "before should be preserved from explicit config"
        );
    }

    #[test]
    fn explicit_lint_config_with_before_list_preserves_all_commands() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["go"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[lint.go]
before = ["cargo build --release -p test-ffi", "cp target/release/libtest_ffi.so packages/go/"]
check = "golangci-lint run ./..."
"#,
        )
        .unwrap();

        let lint = config.lint_config_for_language(Language::Go);
        assert!(lint.precondition.is_none(), "precondition should be None when not set");
        assert_eq!(
            lint.before.unwrap().commands(),
            vec![
                "cargo build --release -p test-ffi",
                "cp target/release/libtest_ffi.so packages/go/"
            ],
            "before list should be preserved from explicit config"
        );
    }

    #[test]
    fn default_lint_config_has_command_v_precondition() {
        let config = minimal_config();
        let py = config.lint_config_for_language(Language::Python);
        assert_eq!(py.precondition.as_deref(), Some("command -v ruff >/dev/null 2>&1"));
        assert!(py.before.is_none(), "default lint config should have no before");

        let go = config.lint_config_for_language(Language::Go);
        assert_eq!(go.precondition.as_deref(), Some("command -v gofmt >/dev/null 2>&1"));
        assert!(go.before.is_none(), "default Go lint config should have no before");
    }

    #[test]
    fn explicit_test_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[test.python]
precondition = "test -f target/release/libtest.so"
before = "maturin develop"
command = "pytest"
"#,
        )
        .unwrap();

        let test = config.test_config_for_language(Language::Python);
        assert_eq!(
            test.precondition.as_deref(),
            Some("test -f target/release/libtest.so"),
            "test precondition should be preserved"
        );
        assert_eq!(
            test.before.unwrap().commands(),
            vec!["maturin develop"],
            "test before should be preserved"
        );
    }

    #[test]
    fn default_test_config_has_command_v_precondition() {
        let config = minimal_config();
        let py = config.test_config_for_language(Language::Python);
        assert_eq!(py.precondition.as_deref(), Some("command -v uv >/dev/null 2>&1"));
        assert!(py.before.is_none(), "default test config should have no before");
    }

    #[test]
    fn explicit_setup_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[setup.python]
precondition = "which uv"
before = "pip install uv"
install = "uv sync"
"#,
        )
        .unwrap();

        let setup = config.setup_config_for_language(Language::Python);
        assert_eq!(
            setup.precondition.as_deref(),
            Some("which uv"),
            "setup precondition should be preserved"
        );
        assert_eq!(
            setup.before.unwrap().commands(),
            vec!["pip install uv"],
            "setup before should be preserved"
        );
    }

    #[test]
    fn default_setup_config_has_command_v_precondition() {
        let config = minimal_config();
        let py = config.setup_config_for_language(Language::Python);
        assert_eq!(py.precondition.as_deref(), Some("command -v uv >/dev/null 2>&1"));
        assert!(py.before.is_none(), "default setup config should have no before");
    }

    #[test]
    fn explicit_update_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["rust"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[update.rust]
precondition = "test -f Cargo.lock"
before = "cargo fetch"
update = "cargo update"
"#,
        )
        .unwrap();

        let update = config.update_config_for_language(Language::Rust);
        assert_eq!(
            update.precondition.as_deref(),
            Some("test -f Cargo.lock"),
            "update precondition should be preserved"
        );
        assert_eq!(
            update.before.unwrap().commands(),
            vec!["cargo fetch"],
            "update before should be preserved"
        );
    }

    #[test]
    fn default_update_config_has_command_v_precondition() {
        let config = minimal_config();
        let rust = config.update_config_for_language(Language::Rust);
        assert_eq!(rust.precondition.as_deref(), Some("command -v cargo >/dev/null 2>&1"));
        assert!(rust.before.is_none(), "default update config should have no before");
    }

    #[test]
    fn explicit_clean_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["rust"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[clean.rust]
precondition = "test -d target"
before = "echo cleaning"
clean = "cargo clean"
"#,
        )
        .unwrap();

        let clean = config.clean_config_for_language(Language::Rust);
        assert_eq!(
            clean.precondition.as_deref(),
            Some("test -d target"),
            "clean precondition should be preserved"
        );
        assert_eq!(
            clean.before.unwrap().commands(),
            vec!["echo cleaning"],
            "clean before should be preserved"
        );
    }

    #[test]
    fn default_clean_config_precondition_matches_toolchain_use() {
        let config = minimal_config();
        // Rust clean uses `cargo clean` → precondition guards on cargo.
        let rust = config.clean_config_for_language(Language::Rust);
        assert_eq!(rust.precondition.as_deref(), Some("command -v cargo >/dev/null 2>&1"));
        assert!(rust.before.is_none(), "default clean config should have no before");

        // Python clean is pure shell `rm -rf …` → no precondition needed.
        let py = config.clean_config_for_language(Language::Python);
        assert!(
            py.precondition.is_none(),
            "pure-shell clean should not have a precondition"
        );
    }

    #[test]
    fn explicit_build_command_config_preserves_precondition_and_before() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["go"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[build_commands.go]
precondition = "which go"
before = "cargo build --release -p test-ffi"
build = "cd packages/go && go build ./..."
build_release = "cd packages/go && go build -ldflags='-s -w' ./..."
"#,
        )
        .unwrap();

        let build = config.build_command_config_for_language(Language::Go);
        assert_eq!(
            build.precondition.as_deref(),
            Some("which go"),
            "build precondition should be preserved"
        );
        assert_eq!(
            build.before.unwrap().commands(),
            vec!["cargo build --release -p test-ffi"],
            "build before should be preserved"
        );
    }

    #[test]
    fn default_build_command_config_has_command_v_precondition() {
        let config = minimal_config();
        let rust = config.build_command_config_for_language(Language::Rust);
        assert_eq!(rust.precondition.as_deref(), Some("command -v cargo >/dev/null 2>&1"));
        assert!(
            rust.before.is_none(),
            "default build command config should have no before"
        );
    }

    #[test]
    fn version_defaults_to_none_when_omitted() {
        let config = minimal_config();
        assert!(config.version.is_none());
    }

    #[test]
    fn version_parses_from_top_level_key() {
        let config: AlefConfig = toml::from_str(
            r#"
version = "0.7.7"
languages = ["python"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        assert_eq!(config.version.as_deref(), Some("0.7.7"));
    }
}
