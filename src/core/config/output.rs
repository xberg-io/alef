use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

mod argv;
mod citation;
mod command_defaults;
mod sync;

pub use argv::{ArgvRunConfig, ArgvStep};
pub use citation::{CitationAuthor, CitationConfig};
pub use command_defaults::{BuildCommandConfig, CleanConfig, LintConfig, SetupConfig, UpdateConfig};
pub use sync::{SyncConfig, TextReplacement};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExcludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    /// Exclude specific methods: "TypeName.method_name"
    #[serde(default)]
    pub methods: Vec<String>,
    /// Exclude specific struct fields from ALL bindings: "TypeName.field_name".
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IncludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    /// Opt-in workspace `.cargo/config.toml` management. When present, alef writes
    /// the full file with hash-based drift detection. Absent = legacy behavior
    /// (wasm32 block only, create-if-missing, unmanaged).
    pub cargo: Option<ScaffoldCargo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
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

/// Opt-in management of workspace-level `.cargo/config.toml`.
///
/// All fields default to canonical values that produce the same `.cargo/config.toml`
/// across polyglot repos. Override individual targets via `targets`, or inject
/// repo-specific `[env]` entries via `env`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

/// How a generated reference page links to another generated reference page in the same
/// `reference_output` directory.
///
/// Different docs-site generators resolve a sibling page's URL differently: plain
/// GitHub-rendered Markdown and most static-site generators serve the file as written, so
/// the link must keep the `.md` suffix, while content-collection site generators (for
/// example Astro Starlight) resolve routes from a page's slug and treat a `.md`-suffixed
/// link as a broken route. Alef cannot infer which is true for a given consumer, so this is
/// configuration rather than a guess -- see `docs::shared_pages::reference_page_link`,
/// the single place that reads it. ~keep
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocsReferenceLinkStyle {
    /// Keep the `.md` suffix: `configuration.md`. Correct for GitHub-rendered Markdown and
    /// most static-site generators that serve the file as-is.
    #[default]
    Suffixed,
    /// Drop the file extension and use a directory-style route: `./configuration/`.
    /// Matches content-collection site generators that resolve routes from slugs.
    Extensionless,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    pub mcp: Option<DocsMcpConfig>,
    /// Template-rendered llms.txt output.
    #[serde(default)]
    pub llms: Option<DocsLlmsConfig>,
    /// Template-rendered agent skill outputs.
    #[serde(default)]
    pub skills: Option<DocsSkillsConfig>,
    /// Snippet discovery and validation config used by docs templates.
    #[serde(default)]
    pub snippets: Option<DocsSnippetsConfig>,
    /// How generated reference pages link to each other. Defaults to
    /// [`DocsReferenceLinkStyle::Suffixed`].
    #[serde(default)]
    pub reference_link_style: DocsReferenceLinkStyle,
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
            mcp: DocsMcpConfig::merge(
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
            reference_link_style: krate
                .map(|cfg| cfg.reference_link_style)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.reference_link_style).unwrap_or_default()),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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

mod mcp;
pub use mcp::{DeclaredMcpItem, DeclaredMcpKind, DocsMcpConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct DocsSnippetsConfig {
    /// Snippet roots to discover.
    #[serde(default)]
    pub dirs: Vec<PathBuf>,
    /// Documentation/template roots to scan for MkDocs snippet includes.
    #[serde(default)]
    pub docs_dirs: Vec<PathBuf>,
    /// Astro content collection names mapped to the snippet root they load. ~keep
    #[serde(default)]
    pub content_collections: BTreeMap<String, PathBuf>,
    /// Documentation roots whose fenced code blocks are validated as snippets.
    #[serde(default)]
    pub inline_dirs: Vec<PathBuf>,
    /// Root-relative path prefixes excluded from snippet discovery and coverage.
    #[serde(default)]
    pub exclude: Vec<PathBuf>,
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
    /// Timeout in seconds for a session's `before` hook. Defaults to `timeout_secs`.
    #[serde(default)]
    pub before_timeout_secs: Option<u64>,
    /// Stop snippet validation on the first failure.
    #[serde(default)]
    pub fail_fast: bool,
    /// Treat coverage gaps and unavailable or downgraded checks as errors.
    ///
    /// Equivalent to always passing `--strict` to `alef snippets check`: this and the CLI flag
    /// are unioned (either one enables strict behavior), and both enable the identical set of
    /// checks, including [`Self::deny_unclassified`] -- setting this alone is enough to reject
    /// unclassified snippets, without also passing `--strict` on every invocation.
    #[serde(default)]
    pub strict: bool,
    /// Reject snippets whose side-effect classification is missing.
    ///
    /// Always enabled while [`Self::strict`] is set (or `--strict` is passed), the same as
    /// every other strict-gated check in this command. Set this independently of `strict` for a
    /// narrower opt-in: denying unclassified snippets without also failing on coverage gaps or
    /// unavailable/downgraded checks.
    #[serde(default)]
    pub deny_unclassified: bool,
    /// Permitted side-effect classes. Empty permits only `safe` snippets.
    #[serde(default)]
    pub allowed_side_effects: Vec<String>,
    /// Persistent validation cache directory. Defaults to `.alef/snippets`.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    /// Optional path for the machine-readable validation report.
    #[serde(default)]
    pub report_output: Option<PathBuf>,
    /// Binding-aware validation sessions keyed by language name.
    #[serde(default)]
    pub sessions: BTreeMap<String, DocsSnippetSessionConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocsSnippetSessionConfig {
    /// Working directory used to resolve the generated local package.
    pub cwd: PathBuf,
    /// Optional local package manifest, resolved relative to the repository root.
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// Setup commands run once before validating this language.
    #[serde(default)]
    pub before: Vec<String>,
    /// Environment variables applied to setup and validation commands.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Native include directories, resolved relative to the repository root.
    #[serde(default)]
    pub include_paths: Vec<PathBuf>,
    /// Cargo features enabled on the local Rust package dependency.
    #[serde(default)]
    pub rust_features: Vec<String>,
    /// Additional Cargo dependencies available to Rust snippets.
    #[serde(default)]
    pub rust_dependencies: BTreeMap<String, DocsSnippetRustDependencyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocsSnippetRustDependencyConfig {
    /// Cargo package version requirement.
    pub version: String,
    /// Cargo features enabled for the dependency.
    #[serde(default)]
    pub features: Vec<String>,
    /// Whether Cargo enables the dependency's default features.
    #[serde(default = "default_true")]
    pub default_features: bool,
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
            content_collections: merge_btree_map(
                workspace.map(|cfg| &cfg.content_collections),
                krate.map(|cfg| &cfg.content_collections),
            ),
            inline_dirs: merge_vec(workspace.map(|cfg| &cfg.inline_dirs), krate.map(|cfg| &cfg.inline_dirs)),
            exclude: merge_vec(workspace.map(|cfg| &cfg.exclude), krate.map(|cfg| &cfg.exclude)),
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
            before_timeout_secs: krate
                .and_then(|cfg| cfg.before_timeout_secs)
                .or_else(|| workspace.and_then(|cfg| cfg.before_timeout_secs)),
            fail_fast: krate
                .map(|cfg| cfg.fail_fast)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.fail_fast).unwrap_or(false)),
            strict: krate
                .map(|cfg| cfg.strict)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.strict).unwrap_or(false)),
            deny_unclassified: krate
                .map(|cfg| cfg.deny_unclassified)
                .unwrap_or_else(|| workspace.map(|cfg| cfg.deny_unclassified).unwrap_or(false)),
            allowed_side_effects: merge_vec(
                workspace.map(|cfg| &cfg.allowed_side_effects),
                krate.map(|cfg| &cfg.allowed_side_effects),
            ),
            cache_dir: krate
                .and_then(|cfg| cfg.cache_dir.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.cache_dir.clone())),
            report_output: krate
                .and_then(|cfg| cfg.report_output.clone())
                .or_else(|| workspace.and_then(|cfg| cfg.report_output.clone())),
            sessions: merge_btree_map(workspace.map(|cfg| &cfg.sessions), krate.map(|cfg| &cfg.sessions)),
        })
    }

    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(".alef/snippets"))
    }
}

fn merge_vec<T: Clone>(workspace: Option<&Vec<T>>, krate: Option<&Vec<T>>) -> Vec<T> {
    krate
        .filter(|items| !items.is_empty())
        .cloned()
        .or_else(|| workspace.filter(|items| !items.is_empty()).cloned())
        .unwrap_or_default()
}

fn merge_btree_map<K: Clone + Ord, V: Clone>(
    workspace: Option<&BTreeMap<K, V>>,
    krate: Option<&BTreeMap<K, V>>,
) -> BTreeMap<K, V> {
    let mut merged = workspace.cloned().unwrap_or_default();
    if let Some(values) = krate {
        merged.extend(values.clone());
    }
    merged
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
#[serde(deny_unknown_fields)]
pub struct TestAppRunConfig {
    /// Shell command that must exit 0 for the test-app run to proceed; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main run commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) that install the published package into the registry-mode test
    /// app and exercise it (e.g. `cd test_apps/ruby && BUNDLE_PATH=vendor/bundle ruby -S bundle install &&
    /// BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rspec`).
    ///
    /// Mutually exclusive with `argv_run` in practice: a default that needs to embed a
    /// config-supplied value as a literal argument sets `argv_run` and leaves this `None`.
    /// `alef.toml` overrides (`[crates.e2e.registry.run.<lang>]`) always set this field --
    /// hand-written shell syntax is exactly what a user authoring an override means to run.
    pub run: Option<StringOrVec>,
    /// Argv-only alternative to `run`, for generated defaults that must not risk a shell
    /// re-interpreting a config-supplied value. See [`ArgvRunConfig`]. When both `run` and
    /// `argv_run` are set, the caller runs `argv_run` and ignores `run`.
    #[serde(default)]
    pub argv_run: Option<ArgvRunConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestConfig {
    /// Shell command that must exit 0 for the `command`/`coverage` phase to run; skip with
    /// warning on failure. Written for what `command`/`before` need -- it does not gate `e2e`.
    pub precondition: Option<String>,
    /// Command(s) to run before the main test commands; aborts on failure. Also runs ahead of
    /// `e2e` (e.g. building the native library the e2e suite loads), since `before` commonly
    /// sets up state both phases depend on.
    pub before: Option<StringOrVec>,
    /// Command to run unit/integration tests for this language.
    pub command: Option<StringOrVec>,
    /// Command to run e2e tests for this language.
    pub e2e: Option<StringOrVec>,
    /// Shell command that must exit 0 for the `e2e` phase to run; skip with warning on failure.
    /// Scopes the e2e tooling check separately from `precondition`, which is written for
    /// `command`/`before` and is often unrelated to what `e2e` needs (e.g. a linter required by
    /// `command` but not by the e2e suite). When unset, `e2e` runs without a precondition gate
    /// rather than inheriting `precondition` -- see `check_e2e_precondition` in
    /// `cli::pipeline::commands::test` for the rationale.
    pub e2e_precondition: Option<String>,
    /// Command to run tests with coverage for this language.
    pub coverage: Option<StringOrVec>,
}

/// Per-language output path templates for multi-crate workspaces.
///
/// Each entry is a path string that may contain `{crate}` and `{lang}` placeholders.
/// Resolved by [`OutputTemplate::resolve`] to produce a concrete path for one
/// `(crate, language)` pair.
///
/// Defaults (when a language entry is absent and no per-crate explicit override is set):
/// - Single-crate workspaces resolve to `crates/{crate}-<suffix>/src` for languages with a
///   dedicated binding crate (Python, Node, PHP, FFI, wasm), else `packages/{lang}/`.
/// - Multi-crate workspaces resolve to `packages/{lang}/{crate}/`.
///
/// Per-crate explicit paths in [`OutputConfig`] always win over a workspace template.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    /// 2. Default fallback: `packages/{lang}/{crate}/` if `multi_crate`, else, for
    ///    languages with a dedicated binding crate (see
    ///    [`default_binding_crate_root`](super::resolve_helpers::default_binding_crate_root)),
    ///    `crates/{crate}-<suffix>/src`; `packages/{lang}` for every other language.
    ///
    /// # Panics
    ///
    /// Panics if `crate_name` contains a NUL byte, path separator (`/`, `\`),
    /// or is a bare relative reference (`..`), and if the resolved path would
    /// escape the project root via `..` components or an absolute root.
    ///
    /// `crate_name` and `lang` are expected to have already been validated at the
    /// config-resolution boundary (see [`crate::core::config::new_config::NewAlefConfig::resolve`]),
    /// which surfaces the same underlying checks as a contextual [`crate::core::config::new_config::ResolveError`]
    /// instead of a panic. This call site keeps panicking so every other caller of
    /// `resolve` (tests, ad hoc tooling) is not forced to thread a `Result` through —
    /// by the time `resolve` runs on a real config, the value has already been
    /// accepted or rejected upstream.
    pub fn resolve(&self, crate_name: &str, lang: &str, multi_crate: bool) -> PathBuf {
        self.try_resolve(crate_name, lang, multi_crate)
            .unwrap_or_else(|message| panic!("{message}"))
    }

    /// Fallible form of [`Self::resolve`] used at the config boundary.
    pub(crate) fn try_resolve(&self, crate_name: &str, lang: &str, multi_crate: bool) -> Result<PathBuf, String> {
        validate_output_segment(crate_name, "crate_name")?;
        validate_output_segment(lang, "lang")?;

        let path = if let Some(template) = self.entry(lang) {
            PathBuf::from(template.replace("{crate}", crate_name).replace("{lang}", lang))
        } else if multi_crate {
            PathBuf::from(format!(
                "{}/{crate_name}",
                super::resolve_helpers::default_package_root(lang)
            ))
        } else if let Some(root) = super::resolve_helpers::default_binding_crate_root(crate_name, lang) {
            PathBuf::from(format!("{root}/src"))
        } else {
            PathBuf::from(super::resolve_helpers::default_package_root(lang))
        };

        validate_output_path(&path)?;
        Ok(path)
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

/// Validate that a user-supplied path segment (crate name, language code, or any other
/// config value later spliced into an output path as a single component) does not contain
/// characters that could enable path traversal or an absolute-path override.
///
/// Returns `Err` with a human-readable message instead of panicking, so config-resolution
/// call sites (see [`crate::core::config::new_config::NewAlefConfig::resolve`]) can surface
/// the failure as a contextual [`crate::core::config::new_config::ResolveError`]. The one
/// existing panicking call site ([`OutputTemplate::resolve`]) unwraps this itself to keep
/// its long-standing panic-on-invalid-input contract intact for callers that predate
/// fallible config resolution.
pub(crate) fn validate_output_segment(segment: &str, label: &str) -> Result<(), String> {
    if segment.contains('\0') {
        return Err(format!(
            "invalid {label}: NUL byte is not allowed in output path segments (got {segment:?})"
        ));
    }
    if segment.contains('/') || segment.contains('\\') {
        return Err(format!(
            "invalid {label}: path separators are not allowed in output path segments (got {segment:?})"
        ));
    }
    Ok(())
}

/// Validate that a `Path` does not escape the project root: no `..` component, and not
/// absolute.
///
/// Returns `Err` with a human-readable message instead of panicking; see
/// [`validate_output_segment`] for why both a fallible and a panicking call site exist.
pub(crate) fn validate_output_path(path: &std::path::Path) -> Result<(), String> {
    let rendered = path.to_string_lossy();
    let bytes = rendered.as_bytes();
    let has_drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if rendered.starts_with('/') || rendered.starts_with('\\') || has_drive_prefix {
        return Err(format!(
            "resolved output path `{}` is absolute and would escape the project root",
            path.display()
        ));
    }
    if rendered.split(['/', '\\']).any(|component| component == "..") {
        return Err(format!(
            "resolved output path `{}` contains `..` and would escape the project root",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
