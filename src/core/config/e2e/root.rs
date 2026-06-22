use super::defaults::*;
use super::{CallConfig, DependencyMode, HarnessConfig, PackageRef, RegistryConfig};
use crate::core::config::manifest_extras::ManifestExtras;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Root e2e configuration from `[e2e]` section of alef.toml.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct E2eConfig {
    /// Directory containing fixture JSON files (default: "fixtures").
    #[serde(default = "default_fixtures_dir")]
    pub fixtures: String,
    /// Output directory for generated e2e test projects (default: "e2e").
    #[serde(default = "default_output_dir")]
    pub output: String,
    /// Repo-root-relative directory holding binary file fixtures referenced by
    /// `file_path` / `bytes` fixture args (default: "test_documents").
    ///
    /// Backends that emit chdir / setup hooks for file-based fixtures resolve
    /// the relative path from the test-emission directory via
    /// [`E2eConfig::test_documents_relative_from`]. The default matches the
    /// sample_core convention; downstream crates whose fixtures don't reference
    /// files (e.g. sample-llm, which uses pure mock-server fixtures) can leave
    /// the default in place — backends conditionally emit the setup only when
    /// fixtures actually need it.
    #[serde(default = "default_test_documents_dir")]
    pub test_documents_dir: String,
    /// Languages to generate e2e tests for. Defaults to top-level `languages` list.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Default function call configuration.
    pub call: CallConfig,
    /// Named additional call configurations for multi-function testing.
    /// Fixtures reference these via the `call` field, e.g. `"call": "embed"`.
    #[serde(default)]
    pub calls: HashMap<String, CallConfig>,
    /// Per-language package reference overrides.
    #[serde(default)]
    pub packages: HashMap<String, PackageRef>,
    /// Per-language extra dependencies to splice into the e2e harness's
    /// language-native manifest (`e2e/<lang>/package.json` for node/wasm,
    /// `e2e/python/pyproject.toml` for Python, etc.). Distinct from the
    /// Rust-binding `extra_dependencies` knob — this one targets the
    /// host-language test-harness manifest. Keys are canonical language
    /// names (`node`, `wasm`, `python`, …).
    #[serde(default)]
    pub harness_extras: HashMap<String, ManifestExtras>,
    /// Per-language extra system libraries to link into the generated e2e
    /// harness's native build alongside the FFI library.
    ///
    /// Keyed by canonical language name (`zig`, …); the value is the list of
    /// bare system-library names (no `lib` prefix, no extension) to link, e.g.
    /// `["heif"]`. Currently only the Zig e2e generator consumes this: when the
    /// linked FFI crate is built with feature sets that pull in additional
    /// native libraries (e.g. libheif via the `all` feature), the strict
    /// linker on some targets (notably aarch64) cannot resolve those undefined
    /// symbols unless the e2e build links them explicitly. The libraries must
    /// already be installed on the build host.
    ///
    /// Default is empty, so consumers that do not need extra links are
    /// unaffected.
    ///
    /// Example:
    /// ```toml
    /// [e2e.extra_system_libs]
    /// zig = ["heif"]
    /// ```
    #[serde(default)]
    pub extra_system_libs: HashMap<String, Vec<String>>,
    /// Per-language formatter commands.
    #[serde(default)]
    pub format: HashMap<String, String>,
    /// Field path aliases: maps fixture field paths to actual API struct paths.
    /// E.g., "metadata.title" -> "metadata.document.title"
    /// Supports struct access (`foo.bar`), map access (`foo[key]`), direct fields.
    #[serde(default)]
    pub fields: HashMap<String, String>,
    /// Fields that are Optional/nullable in the return type.
    /// Rust generators use .as_deref().unwrap_or("") for strings, .is_some() for structs.
    #[serde(default)]
    pub fields_optional: HashSet<String>,
    /// Fields that are arrays/Vecs on the result type.
    /// When a fixture path like `json_ld.name` traverses an array field, the
    /// accessor adds `[0]` (or language equivalent) to index into the first element.
    #[serde(default)]
    pub fields_array: HashSet<String>,
    /// Fields where the accessor is a method call (appends `()`) rather than a field access.
    /// Rust-specific: Java always uses `()`, Python/PHP use field access.
    /// Listed as the full resolved field path (after alias resolution).
    /// E.g., `"metadata.format.excel"` means `.excel` should be emitted as `.excel()`.
    #[serde(default)]
    pub fields_method_calls: HashSet<String>,
    /// Known top-level fields on the result type.
    ///
    /// When non-empty, assertions whose resolved field path starts with a
    /// segment that is NOT in this set are emitted as comments (skipped)
    /// instead of executable assertions.  This prevents broken assertions
    /// when fixtures reference fields from a different operation (e.g.,
    /// `batch.completed_count` on a `ScrapeResult`).
    #[serde(default)]
    pub result_fields: HashSet<String>,
    /// Fixture categories excluded from cross-language e2e codegen.
    ///
    /// Fixtures whose resolved category matches an entry in this set are
    /// skipped by every per-language e2e generator — no test is emitted at
    /// all (no skip directive, no commented-out body). The fixture files stay
    /// on disk and remain available to Rust integration tests inside the
    /// consumer crate's own `tests/` directory.
    ///
    /// Use this to keep fixtures that exercise internal middleware (cache,
    /// proxy, budget, hooks, etc.) out of bindings whose public surface does
    /// not expose those layers.
    ///
    /// Example:
    /// ```toml
    /// [e2e]
    /// exclude_categories = ["cache", "proxy", "budget", "hooks"]
    /// ```
    #[serde(default)]
    pub exclude_categories: HashSet<String>,
    /// C FFI accessor type chain: maps `"{parent_snake_type}.{field}"` to the
    /// PascalCase return type name (without prefix).
    ///
    /// Used by the C e2e generator to emit chained FFI accessor calls for
    /// nested field paths. The root type is always `conversion_result`.
    ///
    /// Example:
    /// ```toml
    /// [e2e.fields_c_types]
    /// "conversion_result.metadata" = "HtmlMetadata"
    /// "html_metadata.document" = "DocumentMetadata"
    /// ```
    #[serde(default)]
    pub fields_c_types: HashMap<String, String>,
    /// Fields whose resolved type is an enum in the generated bindings.
    ///
    /// When a `contains` / `contains_all` / etc. assertion targets one of these
    /// fields, language generators that cannot call `.contains()` directly on an
    /// enum (e.g., Java) will emit a string-conversion call first.  For Java,
    /// the generated assertion calls `.getValue()` on the enum — the `@JsonValue`
    /// method that all alef-generated Java enums expose — to obtain the lowercase
    /// serde string before performing the string comparison.
    ///
    /// Both the raw fixture field path (before alias resolution) and the resolved
    /// path (after alias resolution via `[e2e.fields]`) are accepted, so you can
    /// use either form:
    ///
    /// ```toml
    /// # Raw fixture field:
    /// fields_enum = ["links[].link_type", "assets[].category"]
    /// # …or the resolved (aliased) field name:
    /// fields_enum = ["links[].link_type", "assets[].asset_category"]
    /// ```
    #[serde(default)]
    pub fields_enum: HashSet<String>,
    /// Optional fields whose inner type carries a text accessor rather than
    /// being a plain `String`.
    ///
    /// When a `contains` / `equals` assertion targets one of these fields,
    /// language generators call the language-idiomatic text accessor:
    ///
    /// - Go: `field.Text()` instead of `string(*field)`
    /// - Java: `.map(v -> v.text()).orElse("")` instead of `Objects::toString`
    /// - C#: `field?.Text()?.Trim()` instead of `field?.ToString()?.Trim()`
    /// - PHP: `$result->getText()` instead of raw property access
    ///
    /// Use this for fields like `content` whose Rust type is `Option<RichTextContent>`
    /// (a multimodal union) rather than `Option<String>`. The inner type must
    /// expose a `text()` / `Text()` method that returns the textual representation.
    ///
    /// Example:
    /// ```toml
    /// [e2e]
    /// fields_display_as_text = ["content", "choices[0].message.content"]
    /// ```
    #[serde(default)]
    pub fields_display_as_text: HashSet<String>,
    /// Environment variables every generated e2e suite's setup must set
    /// before the binding's engine is constructed. Keyed by env-var name;
    /// values are passed through verbatim.
    ///
    /// Each per-language test-harness emitter consumes this map at
    /// suite-setup time (conftest.py for Python, spec_helper.rb for Ruby,
    /// TestMain for Go, bootstrap.php for PHP, test_helper.exs for Elixir,
    /// globalSetup.ts for WASM, assembly fixture for C#, XCTestCase
    /// classSetUp for Swift, etc.). The injection point sits next to where
    /// `MOCK_SERVER_URL` is exported so the binding's first call already
    /// sees the configured environment.
    ///
    /// Motivating use case: a binding may require an environment flag to
    /// allow loopback mock-server calls in e2e tests while keeping production
    /// URL validation strict by default. The map is intentionally generic so
    /// consumers can pass any binding-side env-var (feature flags,
    /// observability toggles, etc.) the same way.
    ///
    /// Example:
    /// ```toml
    /// [crates.e2e.env]
    /// ALLOW_PRIVATE_NETWORK = "true"
    /// ```
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Server-shaped e2e harness configuration for HTTP fixtures.
    /// Knobs for code generation that spawn the SUT app and register handlers.
    #[serde(default)]
    pub harness: HarnessConfig,
    /// Dependency mode: `Local` (default) or `Registry`.
    /// Set at runtime via `--registry` CLI flag; not serialized from TOML.
    #[serde(skip)]
    pub dep_mode: DependencyMode,
    /// Registry-mode configuration from `[e2e.registry]`.
    #[serde(default)]
    pub registry: RegistryConfig,
}

impl E2eConfig {
    /// Resolve the call config for a fixture. Uses the named call if specified,
    /// otherwise falls back to the default `[e2e.call]`.
    pub fn resolve_call(&self, call_name: Option<&str>) -> &CallConfig {
        match call_name {
            Some(name) => self.calls.get(name).unwrap_or(&self.call),
            None => &self.call,
        }
    }

    /// Resolve the call config for a fixture, applying `select_when` auto-routing.
    ///
    /// When the fixture has an explicit `call` name, that named config is returned
    /// (same as [`Self::resolve_call`]).  When the fixture has no explicit call, the method
    /// scans named calls for a [`super::selection::SelectWhen`] condition that matches the fixture's
    /// shape (id, category, tags, input) and returns the first match.  If no condition
    /// matches, it falls back to the default `[e2e.call]`.
    ///
    /// All non-`None` discriminators on a `SelectWhen` must match (logical AND) for
    /// the condition to fire. A `SelectWhen` with every field `None` never matches —
    /// at least one discriminator must be set.
    pub fn resolve_call_for_fixture(
        &self,
        call_name: Option<&str>,
        fixture_id: &str,
        fixture_category: &str,
        fixture_tags: &[String],
        fixture_input: &serde_json::Value,
    ) -> &CallConfig {
        if let Some(name) = call_name {
            return self.calls.get(name).unwrap_or(&self.call);
        }
        // Auto-route by select_when condition. Deterministic order: sort by call name.
        let mut names: Vec<&String> = self.calls.keys().collect();
        names.sort();
        for name in names {
            let call_config = &self.calls[name];
            if let Some(sel) = &call_config.select_when {
                if sel.matches(fixture_id, fixture_category, fixture_tags, fixture_input) {
                    return call_config;
                }
            }
        }
        &self.call
    }

    /// Resolve the effective package reference for a language.
    ///
    /// In registry mode, entries from `[e2e.registry.packages]` are merged on
    /// top of the base `[e2e.packages]` — registry overrides win for any field
    /// that is `Some`.
    pub fn resolve_package(&self, lang: &str) -> Option<PackageRef> {
        let base = self.packages.get(lang);
        if self.dep_mode == DependencyMode::Registry {
            let reg = self.registry.packages.get(lang);
            match (base, reg) {
                (Some(b), Some(r)) => Some(PackageRef {
                    name: r.name.clone().or_else(|| b.name.clone()),
                    path: r.path.clone().or_else(|| b.path.clone()),
                    module: r.module.clone().or_else(|| b.module.clone()),
                    version: r.version.clone().or_else(|| b.version.clone()),
                    hash: r.hash.clone().or_else(|| b.hash.clone()),
                    platform_hashes: if r.platform_hashes.is_empty() {
                        b.platform_hashes.clone()
                    } else {
                        r.platform_hashes.clone()
                    },
                    tap: r.tap.clone().or_else(|| b.tap.clone()),
                    cli_formula: r.cli_formula.clone().or_else(|| b.cli_formula.clone()),
                    ffi_formula: r.ffi_formula.clone().or_else(|| b.ffi_formula.clone()),
                    // Registry cli_tests win; fall back to base when registry has none.
                    cli_tests: if r.cli_tests.is_empty() {
                        b.cli_tests.clone()
                    } else {
                        r.cli_tests.clone()
                    },
                }),
                (None, Some(r)) => Some(r.clone()),
                (Some(b), None) => Some(b.clone()),
                (None, None) => None,
            }
        } else {
            base.cloned()
        }
    }

    /// Return the effective `result_fields` for `call`.
    ///
    /// Returns `call.result_fields` when non-empty, otherwise the global
    /// `self.result_fields`.
    pub fn effective_result_fields<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.result_fields.is_empty() {
            &call.result_fields
        } else {
            &self.result_fields
        }
    }

    /// Return the effective `fields` alias map for `call`.
    pub fn effective_fields<'a>(&'a self, call: &'a CallConfig) -> &'a HashMap<String, String> {
        if !call.fields.is_empty() {
            &call.fields
        } else {
            &self.fields
        }
    }

    /// Return the effective `fields_optional` for `call`.
    pub fn effective_fields_optional<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.fields_optional.is_empty() {
            &call.fields_optional
        } else {
            &self.fields_optional
        }
    }

    /// Return the effective `fields_array` for `call`.
    pub fn effective_fields_array<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.fields_array.is_empty() {
            &call.fields_array
        } else {
            &self.fields_array
        }
    }

    /// Return the effective `fields_method_calls` for `call`.
    pub fn effective_fields_method_calls<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.fields_method_calls.is_empty() {
            &call.fields_method_calls
        } else {
            &self.fields_method_calls
        }
    }

    /// Return the effective `fields_enum` for `call`.
    pub fn effective_fields_enum<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.fields_enum.is_empty() {
            &call.fields_enum
        } else {
            &self.fields_enum
        }
    }

    /// Return the effective `fields_display_as_text` for `call`.
    pub fn effective_fields_display_as_text<'a>(&'a self, call: &'a CallConfig) -> &'a HashSet<String> {
        if !call.fields_display_as_text.is_empty() {
            &call.fields_display_as_text
        } else {
            &self.fields_display_as_text
        }
    }

    /// Return the effective `fields_c_types` for `call`.
    pub fn effective_fields_c_types<'a>(&'a self, call: &'a CallConfig) -> &'a HashMap<String, String> {
        if !call.fields_c_types.is_empty() {
            &call.fields_c_types
        } else {
            &self.fields_c_types
        }
    }

    /// Return the effective output directory: `registry.output` in registry
    /// mode, `output` otherwise.
    pub fn effective_output(&self) -> &str {
        if self.dep_mode == DependencyMode::Registry {
            &self.registry.output
        } else {
            &self.output
        }
    }

    /// Extra system libraries to link for a given language's e2e harness build.
    ///
    /// Returns an empty slice when none are configured for `lang`.
    pub fn extra_system_libs_for(&self, lang: &str) -> &[String] {
        self.extra_system_libs.get(lang).map_or(&[], Vec::as_slice)
    }

    /// Relative path from a backend's emission directory to the
    /// `test_documents_dir` at the repo root.
    ///
    /// `emission_depth` counts the number of additional `../` segments needed
    /// to reach `<output>/<lang>/` from where the file is being emitted:
    ///
    /// * `0` — emitted directly at `e2e/<lang>/` (e.g. dart, zig `build.zig`)
    /// * `1` — emitted at `e2e/<lang>/<sub>/` (e.g. ruby `spec/`, R `tests/`)
    /// * `2` — emitted at `e2e/<lang>/<sub1>/<sub2>/`
    ///
    /// The base prefix is two segments above `<output>/<lang>/` (i.e.
    /// `../../`), matching the canonical layout where `<output>` (default
    /// `"e2e"`) sits at the repo root next to the configured
    /// `test_documents_dir`.
    pub fn test_documents_relative_from(&self, emission_depth: usize) -> String {
        let mut up = String::from("../../");
        for _ in 0..emission_depth {
            up.push_str("../");
        }
        format!("{up}{}", self.test_documents_dir)
    }
}

impl Default for E2eConfig {
    fn default() -> Self {
        Self {
            fixtures: default_fixtures_dir(),
            output: default_output_dir(),
            test_documents_dir: default_test_documents_dir(),
            languages: Vec::new(),
            call: CallConfig::default(),
            calls: HashMap::new(),
            packages: HashMap::new(),
            harness_extras: HashMap::new(),
            extra_system_libs: HashMap::new(),
            format: HashMap::new(),
            fields: HashMap::new(),
            fields_optional: HashSet::new(),
            fields_array: HashSet::new(),
            fields_method_calls: HashSet::new(),
            result_fields: HashSet::new(),
            exclude_categories: HashSet::new(),
            fields_c_types: HashMap::new(),
            fields_enum: HashSet::new(),
            fields_display_as_text: HashSet::new(),
            env: HashMap::new(),
            harness: HarnessConfig::default(),
            dep_mode: DependencyMode::default(),
            registry: RegistryConfig::default(),
        }
    }
}
