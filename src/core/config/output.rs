use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExcludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    /// Exclude specific methods: "TypeName.method_name"
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct IncludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct OutputConfig {
    pub python: Option<PathBuf>,
    pub node: Option<PathBuf>,
    pub ruby: Option<PathBuf>,
    pub php: Option<PathBuf>,
    pub elixir: Option<PathBuf>,
    pub wasm: Option<PathBuf>,
    pub ffi: Option<PathBuf>,
    pub go: Option<PathBuf>,
    pub java: Option<PathBuf>,
    pub kotlin: Option<PathBuf>,
    pub kotlin_android: Option<PathBuf>,
    pub dart: Option<PathBuf>,
    pub swift: Option<PathBuf>,
    pub gleam: Option<PathBuf>,
    pub csharp: Option<PathBuf>,
    pub r: Option<PathBuf>,
    pub zig: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScaffoldConfig {
    pub description: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Generated-file header text overrides.
    #[serde(default)]
    pub generated_header: Option<GeneratedHeaderConfig>,
    /// Pre-commit scaffold overrides.
    #[serde(default)]
    pub precommit: Option<PrecommitConfig>,
    /// Opt-in workspace `.cargo/config.toml` management. When present, alef writes
    /// the full file with hash-based drift detection. Absent = legacy behavior
    /// (wasm32 block only, create-if-missing, unmanaged).
    pub cargo: Option<ScaffoldCargo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct GeneratedHeaderConfig {
    /// URL shown in generated-file headers for issue reporting and docs.
    #[serde(default)]
    pub issues_url: Option<String>,
    /// Regeneration command shown in generated-file headers.
    #[serde(default)]
    pub regenerate_command: Option<String>,
    /// Freshness verification command shown in generated-file headers.
    #[serde(default)]
    pub verify_command: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PrecommitConfig {
    /// Whether to include the shared shell/Docker/docs hooks block.
    #[serde(default)]
    pub include_shared_hooks: Option<bool>,
    /// Repository URL for the shared hooks block.
    #[serde(default)]
    pub shared_hooks_repo: Option<String>,
    /// Revision for the shared hooks block.
    #[serde(default)]
    pub shared_hooks_rev: Option<String>,
    /// Whether to include the alef hook block.
    #[serde(default)]
    pub include_alef_hooks: Option<bool>,
    /// Repository URL for the alef hook block.
    #[serde(default)]
    pub alef_hooks_repo: Option<String>,
    /// Revision for the alef hook block.
    #[serde(default)]
    pub alef_hooks_rev: Option<String>,
}

/// Opt-in management of workspace-level `.cargo/config.toml`.
///
/// All fields default to canonical values that produce the same `.cargo/config.toml`
/// across polyglot repos. Override individual targets via `targets`, or inject
/// repo-specific `[env]` entries via `env`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScaffoldCargo {
    /// Per-target cross-compile / rustflags overrides. Defaults emit the canonical
    /// 6-target template (macOS dynamic_lookup, Windows MSVC rust-lld x64+i686,
    /// aarch64-linux-gnu cross-gcc, x86_64-linux-musl, wasm32 bulk-memory).
    #[serde(default)]
    pub targets: ScaffoldCargoTargets,
    /// Limit concurrent rustc jobs to prevent OOM during large builds.
    /// Defaults to 4 (safe for 16 GB dev machines). Set to 0 to disable.
    #[serde(default = "default_build_jobs")]
    pub build_jobs: u32,
    /// Optional cargo rustc wrapper command, for example `.cargo/rustc-wrapper.sh`.
    #[serde(default)]
    pub rustc_wrapper: Option<String>,
    /// Free-form `[env]` entries copied verbatim into the generated file.
    /// Values can be a plain string or `{ value, relative }`. Empty by default.
    #[serde(default)]
    pub env: HashMap<String, ScaffoldCargoEnvValue>,
}

impl Default for ScaffoldCargo {
    fn default() -> Self {
        Self {
            targets: ScaffoldCargoTargets::default(),
            build_jobs: default_build_jobs(),
            rustc_wrapper: None,
            env: HashMap::new(),
        }
    }
}

/// Per-target opt-out flags. All default to `true`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScaffoldCargoTargets {
    #[serde(default = "default_true")]
    pub macos_dynamic_lookup: bool,
    #[serde(default = "default_true")]
    pub x86_64_pc_windows_msvc: bool,
    #[serde(default = "default_true")]
    pub i686_pc_windows_msvc: bool,
    #[serde(default = "default_true")]
    pub aarch64_unknown_linux_gnu: bool,
    #[serde(default = "default_true")]
    pub x86_64_unknown_linux_musl: bool,
    #[serde(default = "default_true")]
    pub wasm32_unknown_unknown: bool,
}

impl Default for ScaffoldCargoTargets {
    fn default() -> Self {
        Self {
            macos_dynamic_lookup: true,
            x86_64_pc_windows_msvc: true,
            i686_pc_windows_msvc: true,
            aarch64_unknown_linux_gnu: true,
            x86_64_unknown_linux_musl: true,
            wasm32_unknown_unknown: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_build_jobs() -> u32 {
    4
}

/// Value for a `[scaffold.cargo.env]` entry. Either a bare string (renders as
/// `KEY = "value"`) or a structured form with `value` + optional `relative`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScaffoldCargoEnvValue {
    Plain(String),
    Structured {
        value: String,
        #[serde(default)]
        relative: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadmeConfig {
    pub template_dir: Option<PathBuf>,
    pub snippets_dir: Option<PathBuf>,
    /// Deprecated: path to an external YAML config file. Prefer inline fields below.
    pub config: Option<PathBuf>,
    pub output_pattern: Option<String>,
    /// Discord invite URL used in README templates.
    pub discord_url: Option<String>,
    /// Banner image URL used in README templates.
    pub banner_url: Option<String>,
    /// Per-language README configuration, keyed by language code
    /// (e.g. "python", "typescript", "ruby"). Values are flexible JSON objects
    /// that map directly to minijinja template context variables.
    #[serde(default)]
    pub languages: HashMap<String, JsonValue>,
    /// Non-language README targets, keyed by target name
    /// (e.g. "root", "cli"). Targets must declare `output_path` or `output`.
    #[serde(default)]
    pub targets: HashMap<String, JsonValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsConfig {
    /// Directory for generated API/CLI/MCP reference markdown. Defaults to
    /// `docs/reference` when unset.
    #[serde(default)]
    pub reference_output: Option<PathBuf>,
    /// Static extraction config for Clap-based CLI reference docs.
    #[serde(default)]
    pub cli: Option<DocsSourceConfig>,
    /// Static extraction config for rmcp-style MCP reference docs.
    #[serde(default)]
    pub mcp: Option<DocsSourceConfig>,
    /// Template-rendered llms.txt output.
    #[serde(default)]
    pub llms: Option<DocsLlmsConfig>,
    /// Template-rendered agent skill outputs.
    #[serde(default)]
    pub skills: Option<DocsSkillsConfig>,
    /// Snippet discovery and validation config used by docs templates.
    #[serde(default)]
    pub snippets: Option<DocsSnippetsConfig>,
}

impl DocsConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        Some(Self {
            reference_output: krate
                .and_then(|cfg| cfg.reference_output.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.reference_output.clone())),
            cli: DocsSourceConfig::merge(
                workspace.and_then(|cfg| cfg.cli.as_ref()),
                krate.and_then(|cfg| cfg.cli.as_ref()),
            ),
            mcp: DocsSourceConfig::merge(
                workspace.and_then(|cfg| cfg.mcp.as_ref()),
                krate.and_then(|cfg| cfg.mcp.as_ref()),
            ),
            llms: DocsLlmsConfig::merge(
                workspace.and_then(|cfg| cfg.llms.as_ref()),
                krate.and_then(|cfg| cfg.llms.as_ref()),
            ),
            skills: DocsSkillsConfig::merge(
                workspace.and_then(|cfg| cfg.skills.as_ref()),
                krate.and_then(|cfg| cfg.skills.as_ref()),
            ),
            snippets: DocsSnippetsConfig::merge(
                workspace.and_then(|cfg| cfg.snippets.as_ref()),
                krate.and_then(|cfg| cfg.snippets.as_ref()),
            ),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsSourceConfig {
    /// Enable this reference extractor. Defaults to true when the table exists.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Rust source files to parse for this reference surface. When empty, Alef
    /// falls back to the crate source list.
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    /// Output markdown file. Relative paths are resolved from the repository root.
    /// When unset, Alef writes into `reference_output`.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Allow the first render to replace an existing unmanaged output file.
    /// Defaults to false to avoid clobbering hand-authored CLI/MCP docs.
    #[serde(default)]
    pub adopt_existing: bool,
}

impl DocsSourceConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        let sources = krate
            .filter(|cfg| !cfg.sources.is_empty())
            .map(|cfg| cfg.sources.clone())
            .or_else(|| {
                workspace
                    .filter(|cfg| !cfg.sources.is_empty())
                    .map(|cfg| cfg.sources.clone())
            })
            .unwrap_or_default();
        Some(Self {
            enabled: krate
                .and_then(|cfg| cfg.enabled)
                .or_else(|| workspace.and_then(|cfg| cfg.enabled)),
            sources,
            output: krate
                .and_then(|cfg| cfg.output.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.output.clone())),
            adopt_existing: krate
                .map(|cfg| cfg.adopt_existing)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.adopt_existing).unwrap_or(false)),
        })
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsLlmsConfig {
    /// Minijinja template path for llms.txt. Required when this table is present.
    #[serde(default)]
    pub template: Option<PathBuf>,
    /// Output path. Defaults to `docs/llms.txt`.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Allow the first render to replace an existing unmanaged output file.
    /// Defaults to false to avoid clobbering hand-authored llms.txt files.
    #[serde(default)]
    pub adopt_existing: bool,
}

impl DocsLlmsConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        Some(Self {
            template: krate
                .and_then(|cfg| cfg.template.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.template.clone())),
            output: krate
                .and_then(|cfg| cfg.output.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.output.clone())),
            adopt_existing: krate
                .map(|cfg| cfg.adopt_existing)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.adopt_existing).unwrap_or(false)),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsSkillsConfig {
    /// Base directory for skill templates. When `templates` is empty, Alef expects
    /// `api/SKILL.md.jinja`, `cli/SKILL.md.jinja`, and `mcp/SKILL.md.jinja`
    /// below this directory.
    #[serde(default)]
    pub template_dir: Option<PathBuf>,
    /// Agent skill roots to write into, for example `.codex/skills`.
    #[serde(default)]
    pub outputs: Vec<PathBuf>,
    /// Explicit skill templates keyed by skill group.
    #[serde(default)]
    pub templates: HashMap<String, DocsSkillTemplateConfig>,
    /// Allow the first render to replace existing unmanaged skill files.
    /// Defaults to false to avoid clobbering hand-authored skills.
    #[serde(default)]
    pub adopt_existing: bool,
}

impl DocsSkillsConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        let outputs = krate
            .filter(|cfg| !cfg.outputs.is_empty())
            .map(|cfg| cfg.outputs.clone())
            .or_else(|| {
                workspace
                    .filter(|cfg| !cfg.outputs.is_empty())
                    .map(|cfg| cfg.outputs.clone())
            })
            .unwrap_or_default();
        let mut templates = workspace.map(|cfg| cfg.templates.clone()).unwrap_or_default();
        if let Some(krate) = krate {
            templates.extend(krate.templates.clone());
        }
        Some(Self {
            template_dir: krate
                .and_then(|cfg| cfg.template_dir.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.template_dir.clone())),
            outputs,
            templates,
            adopt_existing: krate
                .map(|cfg| cfg.adopt_existing)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.adopt_existing).unwrap_or(false)),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsSkillTemplateConfig {
    /// Template path. Relative paths are resolved against `skills.template_dir`
    /// when set, otherwise the repository root.
    #[serde(default)]
    pub template: Option<PathBuf>,
    /// Output path below every configured `skills.outputs` root. Defaults to
    /// `{group}/SKILL.md`.
    #[serde(default)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsSnippetsConfig {
    /// Snippet roots to discover.
    #[serde(default)]
    pub dirs: Vec<PathBuf>,
    /// Documentation/template roots to scan for MkDocs snippet includes.
    #[serde(default)]
    pub docs_dirs: Vec<PathBuf>,
    /// Required language variants for every language-grouped snippet.
    #[serde(default)]
    pub required_languages: Vec<String>,
    /// Additional base paths used when resolving MkDocs `--8<--` includes.
    #[serde(default)]
    pub include_base_paths: Vec<PathBuf>,
    /// Require YAML frontmatter in snippet markdown files.
    #[serde(default)]
    pub require_frontmatter: bool,
    /// Optional validation level: `syntax`, `compile`, `typecheck`, or `run`.
    #[serde(default)]
    pub validation_level: Option<String>,
    /// Snippet validation timeout in seconds. Defaults to the snippet runner default.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Stop snippet validation on the first failure.
    #[serde(default)]
    pub fail_fast: bool,
}

impl DocsSnippetsConfig {
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        Some(Self {
            dirs: merge_vec(workspace.map(|cfg| &cfg.dirs), krate.map(|cfg| &cfg.dirs)),
            docs_dirs: merge_vec(workspace.map(|cfg| &cfg.docs_dirs), krate.map(|cfg| &cfg.docs_dirs)),
            required_languages: merge_vec(
                workspace.map(|cfg| &cfg.required_languages),
                krate.map(|cfg| &cfg.required_languages),
            ),
            include_base_paths: merge_vec(
                workspace.map(|cfg| &cfg.include_base_paths),
                krate.map(|cfg| &cfg.include_base_paths),
            ),
            require_frontmatter: krate
                .map(|cfg| cfg.require_frontmatter)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.require_frontmatter).unwrap_or(false)),
            validation_level: krate
                .and_then(|cfg| cfg.validation_level.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.validation_level.clone())),
            timeout_secs: krate
                .and_then(|cfg| cfg.timeout_secs)
                .or_else(|| workspace.and_then(|cfg| cfg.timeout_secs)),
            fail_fast: krate
                .map(|cfg| cfg.fail_fast)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.fail_fast).unwrap_or(false)),
        })
    }
}

fn merge_vec<T: Clone>(workspace: Option<&Vec<T>>, krate: Option<&Vec<T>>) -> Vec<T> {
    krate
        .filter(|items| !items.is_empty())
        .cloned()
        .or_else(|| workspace.filter(|items| !items.is_empty()).cloned())
        .unwrap_or_default()
}

/// A value that can be either a single string or a list of strings.
///
/// Deserializes from both `"cmd"` and `["cmd1", "cmd2"]` in TOML/JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    /// Return all commands as a slice-like iterator.
    pub fn commands(&self) -> Vec<&str> {
        match self {
            StringOrVec::Single(s) => vec![s.as_str()],
            StringOrVec::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LintConfig {
    /// Shell command that must exit 0 for lint to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main lint commands; aborts on failure.
    pub before: Option<StringOrVec>,
    pub format: Option<StringOrVec>,
    pub check: Option<StringOrVec>,
    pub typecheck: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UpdateConfig {
    /// Shell command that must exit 0 for update to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main update commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) for safe dependency updates (compatible versions only).
    pub update: Option<StringOrVec>,
    /// Command(s) for aggressive updates (including incompatible/major bumps).
    pub upgrade: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TestAppRunConfig {
    /// Shell command that must exit 0 for the test-app run to proceed; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main run commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) that install the published package into the registry-mode test
    /// app and exercise it (e.g. `cd test_apps/ruby && bundle install && bundle exec rspec`).
    pub run: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
pub struct TestConfig {
    /// Shell command that must exit 0 for test to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main test commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command to run unit/integration tests for this language.
    pub command: Option<StringOrVec>,
    /// Command to run e2e tests for this language.
    pub e2e: Option<StringOrVec>,
    /// Command to run tests with coverage for this language.
    pub coverage: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct SetupConfig {
    /// Shell command that must exit 0 for setup to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main setup commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to install dependencies for this language.
    pub install: Option<StringOrVec>,
    /// Timeout in seconds for the complete setup (precondition + before + install).
    #[serde(default = "default_setup_timeout")]
    pub timeout_seconds: u64,
    /// Optional working directory (relative to repo root) for setup commands.
    ///
    /// When set, install commands run from `base_dir.join(workdir)` instead of
    /// `base_dir`. Required for languages whose manifest does not live at the
    /// workspace root (Swift's `Package.swift`, Kotlin-Android's `gradlew`,
    /// Dart's `pubspec.yaml`, Zig's `build.zig`). Defaults to `None` (run from
    /// repo root).
    #[serde(default)]
    pub workdir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct CleanConfig {
    /// Shell command that must exit 0 for clean to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main clean commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to clean build artifacts for this language.
    pub clean: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BuildCommandConfig {
    /// Shell command that must exit 0 for build to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main build commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to build in debug mode.
    pub build: Option<StringOrVec>,
    /// Command(s) to build in release mode.
    pub build_release: Option<StringOrVec>,
}

impl BuildCommandConfig {
    /// Overlay `other` onto this config field-by-field.
    ///
    /// Used for build command defaults where built-ins, workspace defaults, and
    /// crate overrides should compose without forcing callers to restate every
    /// command field.
    pub fn merge_overlay(mut self, other: &Self) -> Self {
        if other.precondition.is_some() {
            self.precondition = other.precondition.clone();
        }
        if other.before.is_some() {
            self.before = other.before.clone();
        }
        if other.build.is_some() {
            self.build = other.build.clone();
        }
        if other.build_release.is_some() {
            self.build_release = other.build_release.clone();
        }
        self
    }
}

fn default_setup_timeout() -> u64 {
    1800
}

/// Per-language output path templates for multi-crate workspaces.
///
/// Each entry is a path string that may contain `{crate}` and `{lang}` placeholders.
/// Resolved by [`OutputTemplate::resolve`] to produce a concrete path for one
/// `(crate, language)` pair.
///
/// Defaults (when a language entry is absent and no per-crate explicit override is set):
/// - Single-crate workspaces resolve to `packages/{lang}/`.
/// - Multi-crate workspaces resolve to `packages/{lang}/{crate}/`.
///
/// Per-crate explicit paths in [`OutputConfig`] always win over a workspace template.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct OutputTemplate {
    pub python: Option<String>,
    pub node: Option<String>,
    pub ruby: Option<String>,
    pub php: Option<String>,
    pub elixir: Option<String>,
    pub wasm: Option<String>,
    pub ffi: Option<String>,
    pub go: Option<String>,
    pub java: Option<String>,
    pub kotlin: Option<String>,
    pub kotlin_android: Option<String>,
    pub dart: Option<String>,
    pub swift: Option<String>,
    pub gleam: Option<String>,
    pub csharp: Option<String>,
    pub r: Option<String>,
    pub zig: Option<String>,
}

impl OutputTemplate {
    /// Resolve a `(crate, language)` pair to a concrete output path.
    ///
    /// Resolution order (highest priority first):
    /// 1. Per-language template entry on `self`, if set, with `{crate}` and `{lang}`
    ///    placeholders substituted.
    /// 2. Default fallback: `packages/{lang}/{crate}/` if `multi_crate`, else
    ///    language-specific historical defaults (`packages/python`, `packages/node`,
    ///    `packages/ruby`, `packages/php`, `packages/elixir`) or `packages/{lang}` for
    ///    languages without a historical default.
    ///
    /// # Panics
    ///
    /// Panics if `crate_name` contains a NUL byte, path separator (`/`, `\`),
    /// or is a bare relative reference (`..`), and if the resolved path would
    /// escape the project root via `..` components or an absolute root.
    pub fn resolve(&self, crate_name: &str, lang: &str, multi_crate: bool) -> PathBuf {
        validate_output_segment(crate_name, "crate_name");
        validate_output_segment(lang, "lang");

        let path = if let Some(template) = self.entry(lang) {
            PathBuf::from(template.replace("{crate}", crate_name).replace("{lang}", lang))
        } else if multi_crate {
            PathBuf::from(format!("packages/{lang}/{crate_name}"))
        } else {
            match lang {
                "python" => PathBuf::from("packages/python"),
                "node" => PathBuf::from("packages/node"),
                "ruby" => PathBuf::from("packages/ruby"),
                "php" => PathBuf::from("packages/php"),
                "elixir" => PathBuf::from("packages/elixir"),
                other => PathBuf::from(format!("packages/{other}")),
            }
        };

        validate_output_path(&path);
        path
    }

    /// Return the raw template string for a language code, if set.
    pub fn entry(&self, lang: &str) -> Option<&str> {
        match lang {
            "python" => self.python.as_deref(),
            "node" => self.node.as_deref(),
            "ruby" => self.ruby.as_deref(),
            "php" => self.php.as_deref(),
            "elixir" => self.elixir.as_deref(),
            "wasm" => self.wasm.as_deref(),
            "ffi" => self.ffi.as_deref(),
            "go" => self.go.as_deref(),
            "java" => self.java.as_deref(),
            "kotlin" => self.kotlin.as_deref(),
            "kotlin_android" => self.kotlin_android.as_deref(),
            "dart" => self.dart.as_deref(),
            "swift" => self.swift.as_deref(),
            "gleam" => self.gleam.as_deref(),
            "csharp" => self.csharp.as_deref(),
            "r" => self.r.as_deref(),
            "zig" => self.zig.as_deref(),
            _ => None,
        }
    }
}

/// Validate that a user-supplied path segment (crate name or language code) does not
/// contain characters that could enable path traversal.
///
/// # Panics
///
/// Panics if the segment contains a NUL byte, a forward slash, or a backslash.
fn validate_output_segment(segment: &str, label: &str) {
    if segment.contains('\0') {
        panic!("invalid {label}: NUL byte is not allowed in output path segments (got {segment:?})");
    }
    if segment.contains('/') || segment.contains('\\') {
        panic!("invalid {label}: path separators are not allowed in output path segments (got {segment:?})");
    }
}

/// Validate that a resolved output `PathBuf` does not escape the project root.
///
/// # Panics
///
/// Panics if the path contains a `..` component or is absolute.
fn validate_output_path(path: &std::path::Path) {
    use std::path::Component;
    for component in path.components() {
        match component {
            Component::ParentDir => {
                panic!(
                    "resolved output path `{}` contains `..` and would escape the project root",
                    path.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                panic!(
                    "resolved output path `{}` is absolute and would escape the project root",
                    path.display()
                );
            }
            _ => {}
        }
    }
}

/// A single text replacement rule for version sync.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TextReplacement {
    /// Glob pattern for files to process.
    pub path: String,
    /// Regex pattern to search for (may contain `{version}` placeholder).
    pub search: String,
    /// Replacement string (may contain `{version}` placeholder).
    pub replace: String,
}

#[cfg(test)]
mod tests;

/// Configuration for the `sync-versions` command.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SyncConfig {
    /// Extra file paths to update version in (glob patterns).
    #[serde(default)]
    pub extra_paths: Vec<String>,
    /// Arbitrary text replacements applied during version sync.
    #[serde(default)]
    pub text_replacements: Vec<TextReplacement>,
}

/// A single author entry in a `CITATION.cff` file. Per the Citation File Format
/// schema, each entry is either a person (uses `family_names` + `given_names`)
/// or a legal entity (uses `name`). Validation lives in the renderer rather
/// than in serde because the choice is mutually exclusive.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CitationAuthor {
    /// Person author: family name(s).
    #[serde(default, alias = "family-names")]
    pub family_names: Option<String>,
    /// Person author: given name(s).
    #[serde(default, alias = "given-names")]
    pub given_names: Option<String>,
    /// Entity author: organisation or legal-entity name.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional contact email (applies to either person or entity).
    #[serde(default)]
    pub email: Option<String>,
    /// Optional ORCID iD URL (`https://orcid.org/0000-0000-0000-0000`).
    #[serde(default)]
    pub orcid: Option<String>,
}

/// Configuration for the alef-generated `CITATION.cff` file at the repo root.
///
/// When this section is present in `alef.toml`, `alef sync-versions` writes a
/// fully-rendered Citation File Format YAML using these fields plus the current
/// workspace version (read from `Cargo.toml`). When absent, alef falls back to
/// updating the `version:` line of a hand-authored CITATION.cff in place.
///
/// All field names follow Rust convention; the renderer emits the canonical
/// CFF kebab-case keys (`cff-version`, `repository-code`, `date-released`,
/// `family-names`, `given-names`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CitationConfig {
    /// Software title (`title:`). Required.
    pub title: String,
    /// One-paragraph summary (`abstract:`). Required.
    #[serde(rename = "abstract")]
    pub abstract_: String,
    /// Authors list — at least one entry required. Persons and legal entities
    /// can be mixed (e.g. `Na'aman Hirschfeld` + `SampleCrate, Inc.`).
    pub authors: Vec<CitationAuthor>,
    /// Canonical citation message shown to consumers (`message:`).
    #[serde(default = "default_citation_message")]
    pub message: String,
    /// Source-code repository URL (`repository-code:`). Required.
    #[serde(rename = "repository-code", alias = "repository_code")]
    pub repository_code: String,
    /// Project landing-page URL (`url:`). Optional.
    #[serde(default)]
    pub url: Option<String>,
    /// SPDX license identifier (`license:`). When omitted, the renderer falls
    /// back to `Cargo.toml [workspace.package].license`.
    #[serde(default)]
    pub license: Option<String>,
    /// Release date in `YYYY-MM-DD` form (`date-released:`). Optional override.
    ///
    /// When omitted (the recommended default), `alef sync-versions` stamps the
    /// current system date on every regen so consumers do not need to hand-edit
    /// alef.toml per release. Set this explicitly only when you need to replay
    /// a historical release date (e.g. backports, CFF reproducibility audits).
    #[serde(default, rename = "date-released", alias = "date_released")]
    pub date_released: Option<String>,
    /// Persistent DOI for the cited release (`doi:`). Optional.
    #[serde(default)]
    pub doi: Option<String>,
}

fn default_citation_message() -> String {
    "If you use this software, please cite it using the metadata below.".to_string()
}
