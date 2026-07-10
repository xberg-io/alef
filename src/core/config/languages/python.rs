use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::{FfiTargetDepOverride, StubsConfig};

/// Configuration for a single capsule type entry in `PythonConfig::capsule_types`.
///
/// Supports two TOML forms via `#[serde(untagged)]`:
///
/// - String: `Language = "sample_language.Language"` → capsule round-trip via `into_raw()`
/// - Struct: `Parser = { python_type = "sample_language.Parser", construct_from = "Language" }` → Python-side construction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum CapsuleTypeConfig {
    /// Capsule round-trip: the Rust type exposes `into_raw()` returning a raw pointer.
    /// The generated code calls `PyCapsule_New(value.into_raw(), capsule_name, None)` on return,
    /// and `PyCapsule_GetPointer` + `from_raw()` on input.
    ///
    /// Value is the fully-qualified Python capsule name (e.g. `"sample_language.Language"`).
    Capsule(String),
    /// Python-side construction: the type does not have a direct `into_raw()`.
    /// Instead, the generated code constructs the Python type by calling a Python factory
    /// (e.g. `sample_language.Parser(language)`) where `language` is a bound capsule argument.
    ConstructFrom {
        /// The fully-qualified Python type to import and call (e.g. `"sample_language.Parser"`).
        python_type: String,
        /// The capsule-type argument name to pass to the Python constructor.
        /// Must be one of the other capsule-type entries (e.g. `"Language"`).
        construct_from: String,
    },
}

impl CapsuleTypeConfig {
    /// Returns the Python type string (dotted path) for this config entry.
    pub fn python_type(&self) -> &str {
        match self {
            Self::Capsule(name) => name,
            Self::ConstructFrom { python_type, .. } => python_type,
        }
    }

    /// Returns the `construct_from` dependency type name, if this is a `ConstructFrom` entry.
    pub fn construct_from(&self) -> Option<&str> {
        match self {
            Self::ConstructFrom { construct_from, .. } => Some(construct_from.as_str()),
            Self::Capsule(_) => None,
        }
    }

    /// Returns true when this entry represents a raw capsule round-trip (not Python-side construction).
    pub fn is_capsule_roundtrip(&self) -> bool {
        matches!(self, Self::Capsule(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PythonConfig {
    pub module_name: Option<String>,
    pub async_runtime: Option<String>,
    pub stubs: Option<StubsConfig>,
    /// PyPI package name (e.g. `"sample-markdown"`). Used as the `[project] name` in
    /// `pyproject.toml` and to derive the `python-packages` list for maturin.
    /// Defaults to the crate name.
    #[serde(default)]
    pub pip_name: Option<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Map of type name -> capsule config for PyCapsule pass-through.
    /// Types listed here are emitted as PyCapsule_New / PyCapsule_GetPointer instead of
    /// opaque `#[pyclass]` wrappers. Use `CapsuleTypeConfig::Capsule` for raw capsule
    /// round-trips and `CapsuleTypeConfig::ConstructFrom` for Python-side construction.
    #[serde(default)]
    pub capsule_types: HashMap<String, CapsuleTypeConfig>,
    /// When true, wrap blocking function bodies in py.allow_threads() to release the GIL.
    #[serde(default)]
    pub release_gil: bool,
    /// Functions to exclude from Python binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Python binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    #[schemars(with = "HashMap<String, serde_json::Value>")]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Runtime Python (PyPI) dependencies emitted into `[project] dependencies`
    /// of the scaffold-generated `pyproject.toml`. Entries are PEP 508 strings
    /// such as `"sample_language>=0.23"` and pass through verbatim. Empty by default.
    #[serde(default)]
    pub pip_dependencies: Vec<String>,
    /// Extra paths to include in the maturin source distribution (sdist), emitted
    /// as `sdist-include` under `[tool.maturin]` in the scaffold-generated
    /// `pyproject.toml`. Entries are workspace-relative glob patterns such as
    /// `"../../crates/sample-markdown/**/*"` and pass through verbatim. Use this
    /// to bundle path-dependent workspace crates into the sdist so source builds
    /// (e.g. Alpine/musl PyPI installs) can compile from the published archive.
    /// Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sdist_include: Vec<String>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name` (e.g.
    /// `"LayoutDetection.class"`), value is the desired binding field name. Applied after
    /// automatic keyword escaping, so an explicit entry takes priority.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    /// E.g., `run_wrapper = "uv run --no-sync"` turns `ruff format packages/python` into
    /// `uv run --no-sync ruff format packages/python`.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    /// Space-separated paths are appended to the command.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Additional `from <module> import <symbol>` lines to emit in the generated `__init__.py`.
    /// Key is the relative or absolute Python module path (e.g. `"._supported_languages"`),
    /// value is the list of symbols to import. The symbols are also added to `__all__`.
    ///
    /// Use this to re-export hand-written sibling modules (e.g. generated by a project's own
    /// build script) without alef's cleanup culling them. The hand-written file must NOT contain
    /// the substrings `"DO NOT EDIT"`, `"auto-generated by alef"`, or `"AUTO-GENERATED by alef"`
    /// in its first 5 lines, or alef's cleanup pipeline will treat it as a stale alef artifact.
    #[serde(default)]
    pub extra_init_imports: std::collections::BTreeMap<String, Vec<String>>,
    /// Type names to skip `_rust.` qualification in function return-type annotations.
    /// List type names that are re-exported in the public `__init__.py` to avoid
    /// annotating them with `_rust.TypeName` (which causes type-checker confusion when
    /// the type is also imported as a bare name in the public API).
    /// Example: `["ExtractionResult", "ExtractionDiff"]` makes function returns bare
    /// `ExtractionResult` instead of `_rust.ExtractionResult`.
    #[serde(default)]
    pub reexported_types: Vec<String>,
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated `Cargo.toml`. Mirrors [`super::FfiConfig::target_dep_overrides`]:
    /// when non-empty, the core dependency is wrapped in
    /// `[target.'cfg(not(<any-cfg>))'.dependencies]` plus one
    /// `[target.'cfg(<cfg>)'.dependencies]` block per override, so a specific
    /// target can swap the core crate's feature set.
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
}
