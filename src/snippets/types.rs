use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// True for a lowercased fence-info token drawn from rustdoc's documented doctest
/// attribute vocabulary (<https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#attributes>).
/// None of these is ever a language tag on its own. `edition2015`/`2018`/`2021`/`2024`
/// are recognized by prefix rather than an exact list so a future edition needs no
/// change here.
fn is_rustdoc_test_attribute(token: &str) -> bool {
    matches!(token, "no_run" | "ignore" | "should_panic" | "compile_fail")
        || token
            .strip_prefix("edition")
            .is_some_and(|year| !year.is_empty() && year.bytes().all(|byte| byte.is_ascii_digit()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Bash,
    C,
    Csharp,
    Dart,
    Docker,
    Elixir,
    Go,
    Java,
    Json,
    Kotlin,
    Mermaid,
    Php,
    PowerShell,
    Python,
    R,
    Ruby,
    Rust,
    Swift,
    Text,
    Toml,
    TypeScript,
    Xml,
    Yaml,
    Zig,
    Unknown,
}

impl Language {
    #[must_use]
    pub fn from_fence_tag(tag: &str) -> Self {
        match tag.trim().to_lowercase().as_str() {
            "bash" | "sh" | "shell" | "zsh" | "console" => Self::Bash,
            "c" => Self::C,
            "csharp" | "c#" | "cs" => Self::Csharp,
            "dart" => Self::Dart,
            "docker" | "dockerfile" => Self::Docker,
            "elixir" | "ex" | "exs" => Self::Elixir,
            "go" | "golang" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kotlin" | "kt" | "kts" => Self::Kotlin,
            "mermaid" => Self::Mermaid,
            "php" => Self::Php,
            "powershell" | "ps" | "ps1" | "pwsh" => Self::PowerShell,
            "python" | "py" | "python3" => Self::Python,
            "r" | "rscript" => Self::R,
            "ruby" | "rb" => Self::Ruby,
            "rust" | "rs" => Self::Rust,
            "swift" => Self::Swift,
            "text" | "txt" | "plain" => Self::Text,
            "toml" => Self::Toml,
            "typescript" | "ts" | "javascript" | "js" => Self::TypeScript,
            "xml" => Self::Xml,
            "yaml" | "yml" => Self::Yaml,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }

    /// Parse a fenced code block's full info string -- everything after the opening
    /// backticks, e.g. `rust,no_run,should_panic` -- into the language it represents.
    ///
    /// Rustdoc's own doctest attributes (`no_run`, `ignore`, `should_panic`,
    /// `compile_fail`, `editionNNNN`) are meaningful only to rustdoc's harness and never
    /// denote a language by themselves: a fence carrying just one, several, or none of
    /// them alongside an explicit or implicit `rust` is still Rust. `from_fence_tag`
    /// alone cannot express this -- it treats the whole comma-joined string as one
    /// opaque tag, so `rust,no_run` and a bare `no_run` both miss every arm and resolve
    /// to `Unknown`. This is the single place that knows the rustdoc attribute
    /// vocabulary; callers that see a raw fence info string (docs generation's
    /// Rust-code-block detection, the snippet audit's fence check) must go through this
    /// rather than growing their own attribute list. ~keep
    #[must_use]
    pub fn from_fence_info(info: &str) -> Self {
        let tokens: Vec<&str> = info
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect();
        let language_tokens: Vec<&str> = tokens
            .into_iter()
            .filter(|token| !is_rustdoc_test_attribute(&token.to_lowercase()))
            .collect();
        match language_tokens.as_slice() {
            [] => Self::Rust,
            [only] if only.eq_ignore_ascii_case("rust") => Self::Rust,
            [only] => Self::from_fence_tag(only),
            _ => Self::Unknown,
        }
    }

    /// True for the variants that name a language alef actually generates bindings for, as
    /// opposed to a markup/data/prose language (`Json`, `Yaml`, `Bash`, `Docker`,
    /// `PowerShell`, `Text`, `Toml`, `Xml`, `Mermaid`) that a fence may legitimately use for
    /// illustration without alef ever targeting it as a binding language.
    ///
    /// The single authority a fence-tag audit asks to decide whether an unrecognized tag is
    /// a real target-language typo/leak worth flagging, or prose decoration (`astro`, `mdx`,
    /// `hcl`, ...) that must never fail validation just because nobody added it to a
    /// hand-maintained allowlist. ~keep
    #[must_use]
    pub fn is_binding_target(self) -> bool {
        matches!(
            self,
            Self::C
                | Self::Csharp
                | Self::Dart
                | Self::Elixir
                | Self::Go
                | Self::Java
                | Self::Kotlin
                | Self::Php
                | Self::Python
                | Self::R
                | Self::Ruby
                | Self::Rust
                | Self::Swift
                | Self::TypeScript
                | Self::Zig
        )
    }

    #[must_use]
    pub fn from_session_target(target: &str) -> Self {
        match Self::normalize_session_target(target).as_str() {
            "node" | "wasm" => Self::TypeScript,
            "kotlin_android" => Self::Kotlin,
            "core" | "rust_core" => Self::Rust,
            "c_ffi" | "ffi" => Self::C,
            other => Self::from_fence_tag(other),
        }
    }

    #[must_use]
    pub fn normalize_session_target(target: &str) -> String {
        target.trim().to_lowercase().replace('-', "_")
    }

    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "sh" | "bash" => Self::Bash,
            "c" | "h" => Self::C,
            "cs" => Self::Csharp,
            "dart" => Self::Dart,
            "dockerfile" => Self::Docker,
            "ex" | "exs" => Self::Elixir,
            "go" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kt" | "kts" => Self::Kotlin,
            "php" => Self::Php,
            "py" => Self::Python,
            "r" => Self::R,
            "rb" => Self::Ruby,
            "rs" => Self::Rust,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "ts" | "js" | "mts" | "mjs" => Self::TypeScript,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub fn from_dir_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "bash" | "shell" => Self::Bash,
            "c" => Self::C,
            "csharp" | "c-sharp" | "dotnet" => Self::Csharp,
            "dart" => Self::Dart,
            "docker" => Self::Docker,
            "elixir" => Self::Elixir,
            "go" | "golang" => Self::Go,
            "java" => Self::Java,
            "json" => Self::Json,
            "kotlin" | "kotlin_android" | "kotlin-android" => Self::Kotlin,
            "php" => Self::Php,
            "python" => Self::Python,
            "r" => Self::R,
            "ruby" => Self::Ruby,
            "rust" => Self::Rust,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "typescript" | "wasm" | "node" => Self::TypeScript,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bash => write!(f, "bash"),
            Self::C => write!(f, "c"),
            Self::Csharp => write!(f, "csharp"),
            Self::Dart => write!(f, "dart"),
            Self::Docker => write!(f, "docker"),
            Self::Elixir => write!(f, "elixir"),
            Self::Go => write!(f, "go"),
            Self::Java => write!(f, "java"),
            Self::Json => write!(f, "json"),
            Self::Kotlin => write!(f, "kotlin"),
            Self::Mermaid => write!(f, "mermaid"),
            Self::Php => write!(f, "php"),
            Self::PowerShell => write!(f, "powershell"),
            Self::Python => write!(f, "python"),
            Self::R => write!(f, "r"),
            Self::Ruby => write!(f, "ruby"),
            Self::Rust => write!(f, "rust"),
            Self::Swift => write!(f, "swift"),
            Self::Text => write!(f, "text"),
            Self::Toml => write!(f, "toml"),
            Self::TypeScript => write!(f, "typescript"),
            Self::Xml => write!(f, "xml"),
            Self::Yaml => write!(f, "yaml"),
            Self::Zig => write!(f, "zig"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Resolve one `required_languages` entry (the `[docs.snippets]`/`[crates.e2e.snippets]` config
/// key, or `alef snippets gaps --required-languages`) to a [`Language`].
///
/// Accepts a snippet fence tag (`python`, `kotlin`) OR a session target name (`node`, `wasm`,
/// `kotlin_android`, `kotlin-android`) -- the vocabulary a consumer's `alef.toml` already uses for
/// every other per-language surface.
///
/// ~keep This lives here, not beside either caller, because it previously existed only in
/// `cli::commands::snippets` while `docs::mod` parsed the SAME config key through `FromStr`
/// (fence tags only). One key, two vocabularies: `required_languages = ["node"]` was accepted by
/// `alef snippets gaps` and rejected by `alef docs`, so `alef all` failed on a config its own
/// sibling command had validated. Both callers must use this.
pub fn resolve_required_language(value: &str) -> Result<Language, String> {
    let language = Language::from_session_target(value);
    if language == Language::Unknown {
        Err(format!(
            "unknown language `{value}` (expected a snippet fence tag such as `python`/`go`/`kotlin`, or a \
             session target name such as `kotlin_android`/`node`/`wasm`)"
        ))
    } else {
        Ok(language)
    }
}

impl std::str::FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let language = Self::from_fence_tag(s);
        if language == Self::Unknown {
            Err(format!("unknown language: {s}"))
        } else {
            Ok(language)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationLevel {
    Syntax,
    Compile,
    /// Static type-checking without executing the code (e.g. `pyrefly check` for Python, `tsc` for
    /// TypeScript). Deeper than `Compile` for dynamically-typed languages whose compile step is
    /// only a bytecode/syntax pass; equivalent to `Compile` for languages whose compiler already
    /// type-checks. Ordered between `Compile` and `Run` so it is the strongest static guarantee
    /// short of execution. ~keep
    TypeCheck,
    Run,
}

impl fmt::Display for ValidationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => write!(f, "syntax"),
            Self::Compile => write!(f, "compile"),
            Self::TypeCheck => write!(f, "typecheck"),
            Self::Run => write!(f, "run"),
        }
    }
}

impl std::str::FromStr for ValidationLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "syntax" => Ok(Self::Syntax),
            "compile" => Ok(Self::Compile),
            "typecheck" | "type-check" => Ok(Self::TypeCheck),
            "run" => Ok(Self::Run),
            _ => Err(format!("unknown validation level: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetAnnotationKind {
    Skip,
    CompileOnly,
    SyntaxOnly,
    TypeCheckOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetAnnotation {
    pub kind: SnippetAnnotationKind,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetMetadata {
    pub id: Option<String>,
    pub language: Option<Language>,
    pub target: Option<String>,
    pub title: Option<String>,
    pub level: Option<ValidationLevel>,
    pub skip: bool,
    pub reason: Option<String>,
    #[serde(deserialize_with = "deserialize_nullable_metadata_list")]
    pub tags: Vec<String>,
    #[serde(deserialize_with = "deserialize_nullable_metadata_list")]
    pub requires: Vec<String>,
    pub side_effect: Option<SideEffectClass>,
}

// Preserve nullable YAML lists explicitly: a list error discards the whole frontmatter,
// including skip and side-effect safeguards, when the parser falls back to defaults. ~keep
fn deserialize_nullable_metadata_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideEffectClass {
    #[serde(alias = "none", alias = "local")]
    Safe,
    Network,
    Process,
    Install,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetStatus {
    Pass,
    Downgraded,
    Fail,
    Skip,
    Error,
    Unavailable,
}

impl fmt::Display for SnippetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "pass"),
            Self::Downgraded => write!(f, "downgraded"),
            Self::Fail => write!(f, "fail"),
            Self::Skip => write!(f, "skip"),
            Self::Error => write!(f, "error"),
            Self::Unavailable => write!(f, "unavailable"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: Option<String>,
    pub path: PathBuf,
    pub language: Language,
    pub title: Option<String>,
    pub code: String,
    pub start_line: usize,
    pub block_index: usize,
    pub annotation: Option<SnippetAnnotation>,
    pub metadata: SnippetMetadata,
    pub source_origin: SourceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceOrigin {
    pub path: PathBuf,
    pub line: usize,
    pub block_index: usize,
}

/// Why a result's effective level fell below the requested level, or why a `Pass` needed a
/// caveat at all. Distinct from `SnippetStatus`: a `capability_capped` `Pass` and a `Downgraded`
/// result can share a reason (`ValidatorCapability`), and two `Downgraded` results can differ
/// (`Annotation` vs `Environment`) — attribution needs the reason, not just the status, to tell a
/// consumer what to actually do about it. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DowngradeReason {
    /// A front-matter `level:` contract was requested and fully satisfied — reported for
    /// attribution even though the status is `Pass`, not a violation. ~keep
    Declared,
    /// A `<!-- snippet:*-only -->` suppression annotation lowered the ceiling below what was
    /// requested; the author's choice, so it still fails strict. ~keep
    Annotation,
    /// The validator can never reach the requested level for this language (`max_level`, or a
    /// structural `achievable_level` gap) — unsatisfiable in any environment.
    ValidatorCapability,
    /// This run's environment could not back the requested level, but a different environment
    /// could (e.g. a real type-checker binary happens to be missing).
    Environment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub snippet: Snippet,
    pub status: SnippetStatus,
    pub level: ValidationLevel,
    pub requested_level: ValidationLevel,
    pub effective_level: ValidationLevel,
    pub message: Option<String>,
    pub duration_ms: u64,
    /// True when the snippet passed below the requested level solely because its validator
    /// declares a lower `max_level`. That ceiling is a capability statement, not a quality
    /// signal, so strict mode must not treat it as a failure — otherwise requesting a level
    /// any validator caps below is structurally unsatisfiable. Downgrades from any other
    /// cause leave this false and still fail strict. ~keep
    #[serde(default)]
    pub capability_capped: bool,
    /// Populated whenever the effective level differs from the requested level for a reason
    /// worth naming — `None` for an ordinary, unqualified `Pass`, and equally for `Fail`, `Skip`,
    /// `Error`, or `Unavailable`, none of which have a reason in this taxonomy at all. `None` is
    /// deliberately a real "not applicable" here rather than a degraded default: the only writer
    /// that ever sets this to `Some` is `runner::finalize_result` (via `classify_result`), which
    /// is exhaustive over every path that produces `Downgraded` or a `capability_capped` `Pass`
    /// — see the `debug_assert!` there. ~keep
    #[serde(default)]
    pub downgrade_reason: Option<DowngradeReason>,
    /// True when this `Unavailable` result started as a validator `Fail` at `Compile`,
    /// `TypeCheck`, or `Run` whose message the validator's own `is_dependency_error` recognized
    /// as a missing import/package/symbol rather than a defect in the snippet. That shape is
    /// what a toolchain reports when the environment never built the artifact the snippet links
    /// or imports against — before this field existed, indistinguishable from a genuinely broken
    /// snippet, because both landed in `Fail`. `false` for every other result, including an
    /// ordinary toolchain-missing `Unavailable`, so it names one specific cause rather than
    /// standing in for the whole status. Set only by `runner::finalize_result`. ~keep
    #[serde(default)]
    pub unresolved_dependency: bool,
    /// True when this result's toolchain invocation was killed at `timeout_secs` instead of
    /// reporting a verdict on the snippet.
    ///
    /// A timeout is a stopwatch reading, not a judgement: the compiler never said anything about
    /// the code. It still lands in `SnippetStatus::Error` and still fails the run -- an
    /// unbounded toolchain is a real problem -- but a reader must be able to tell "N snippets are
    /// broken" from "N snippets ran out of clock", and before this flag existed they rendered
    /// identically as `Errors`. That mattered most in exactly the state this run is usually in
    /// when it happens: a batch validating against artifacts that were never built spends its
    /// whole budget getting nowhere, and the resulting count measured the budget, not the
    /// corpus. Set by `runner::finalize_result` from `ValidationOutcome::timed_out`. ~keep
    #[serde(default)]
    pub timed_out: bool,
    /// True when no validator process was ever spawned for this snippet because its session's
    /// required build artifacts were already known to be absent -- see
    /// `runner::artifact_preflight`.
    ///
    /// The status is `Unavailable` with `unresolved_dependency` set, identical to what the
    /// per-snippet path produces when it discovers the same missing artifact the expensive way,
    /// so every downstream verdict (`fully_verified`, `checked_nothing`, the strict-mode gates in
    /// `docs::enforce_snippet_summary`, the per-language rollup in `snippets::output`) is
    /// unchanged by detecting it early. This flag exists so the saving is *visible*: a skip that
    /// disappears into an existing bucket is indistinguishable from a check that ran, which is
    /// the failure mode this whole preflight has to avoid being. ~keep
    #[serde(default)]
    pub preflight_skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub schema_version: u32,
    pub total: usize,
    pub passed: usize,
    pub downgraded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub unavailable: usize,
    /// Passing snippets whose level was limited by their validator's declared ceiling.
    /// Reported so a strict run can say what it accepted rather than hiding it. ~keep
    #[serde(default)]
    pub capability_capped: usize,
    /// Passing snippets whose level was limited by their own front-matter `level:` contract
    /// (`DowngradeReason::Declared`) rather than by the validator's capability. Tracked
    /// separately from `capability_capped` for the same reason that one is tracked at all: a
    /// consumer who configured `docs.snippets.validation_level = "run"` and sees every result
    /// pass has no way to learn that some of them never actually ran, only typechecked, because a
    /// snippet's own declared `level:` clamped it first — that includes every fixture snippet
    /// `alef e2e generate` emits, which stamps `level: typecheck` unconditionally. ~keep
    #[serde(default)]
    pub declared_capped: usize,
    /// Subset of `unavailable`: results reclassified from `Fail` to `Unavailable` because their
    /// message was dependency-shaped at a level above `Syntax` — see
    /// `ValidationResult::unresolved_dependency`. Never counted in `failed`, `errors`, or any
    /// other bucket; always `<= unavailable`. Reported separately from a plain toolchain-missing
    /// `Unavailable` because the remediation differs: install a toolchain vs. run `alef build`. ~keep
    #[serde(default)]
    pub unresolved_dependency: usize,
    /// `Pass` results that reached the requested level with nothing to caveat at all -- no
    /// `capability_capped`, no `declared_capped`, no downgrade of any kind (`downgrade_reason`
    /// is `None`). This is the "actually checked at the level you asked for" count task #488
    /// exists for: `passed` alone cannot answer that question, because it also includes every
    /// `capability_capped`/`declared_capped` `Pass` -- a `Pass` that never ran at the requested
    /// level at all. A run where `total` is large and `fully_verified` is small is exactly the
    /// shape that let a consumer see "1482 passed" and believe the corpus was checked, when 684
    /// of 1985 results (`unavailable` + `capability_capped`) never validated at the level their
    /// own front matter requested. Reported prominently in `output::print_summary` rather than
    /// left for a reader to reconstruct from the other counts. ~keep
    #[serde(default)]
    pub fully_verified: usize,
    /// Results whose toolchain was killed at the timeout rather than reporting on the snippet --
    /// see [`ValidationResult::timed_out`]. A subset of `errors`, never a separate bucket:
    /// downgrading a timeout out of the failing counts would hide an unbounded toolchain, which
    /// is the opposite of the problem. Reported alongside `errors` so "32 errors" can no longer
    /// be read as "32 broken snippets" when it is really a stopwatch reading. ~keep
    #[serde(default)]
    pub timed_out: usize,
    /// Results that never spawned a validator because the preflight already knew their session's
    /// build artifacts were missing -- see [`ValidationResult::preflight_skipped`]. A subset of
    /// `unresolved_dependency` (and so of `unavailable`), counted separately so the summary can
    /// state how many snippets were skipped without being checked, rather than letting the saving
    /// read as a pass. ~keep
    #[serde(default)]
    pub preflight_skipped: usize,
    pub results: Vec<ValidationResult>,
}

impl RunSummary {
    #[must_use]
    pub fn from_results(results: Vec<ValidationResult>) -> Self {
        let mut summary = Self {
            schema_version: 1,
            total: results.len(),
            passed: 0,
            downgraded: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            unavailable: 0,
            capability_capped: 0,
            declared_capped: 0,
            unresolved_dependency: 0,
            fully_verified: 0,
            timed_out: 0,
            preflight_skipped: 0,
            results,
        };

        for result in &summary.results {
            if result.timed_out {
                summary.timed_out += 1;
            }
            if result.preflight_skipped {
                summary.preflight_skipped += 1;
            }
            if result.capability_capped {
                summary.capability_capped += 1;
            }
            if result.downgrade_reason == Some(DowngradeReason::Declared) {
                summary.declared_capped += 1;
            }
            if result.unresolved_dependency {
                summary.unresolved_dependency += 1;
            }
            if result.status == SnippetStatus::Pass && result.downgrade_reason.is_none() {
                summary.fully_verified += 1;
            }
            match result.status {
                SnippetStatus::Pass => summary.passed += 1,
                SnippetStatus::Downgraded => summary.downgraded += 1,
                SnippetStatus::Fail => summary.failed += 1,
                SnippetStatus::Skip => summary.skipped += 1,
                SnippetStatus::Error => summary.errors += 1,
                SnippetStatus::Unavailable => summary.unavailable += 1,
            }
        }

        summary
    }

    #[must_use]
    pub const fn has_failures(&self) -> bool {
        self.failed > 0 || self.errors > 0
    }

    /// True when this run checked *nothing* at its requested level: every result was a failure,
    /// an error, a skip, an unavailable environment gap, or a `Pass` capped below what was
    /// requested. Distinct from `has_failures`: a run can have zero failures and still be this,
    /// when the entire corpus fell into an exempted or unavailable bucket (task #488) -- exactly
    /// the shape that let a run report overall success while validating almost nothing at the
    /// level it claimed to check. Deliberately unconditional, not gated on `--strict`: a single
    /// `capability_capped`/`unavailable` result can be a legitimate, unsatisfiable-by-design
    /// outcome for one language, but a run where NOT ONE result reached its requested level is
    /// never a legitimate mixed outcome to accept silently by default. `total > 0` guards an
    /// empty run (nothing discovered) from reading as "checked nothing" -- that is a discovery
    /// problem the caller already reports separately. ~keep
    #[must_use]
    pub const fn checked_nothing(&self) -> bool {
        self.total > 0 && self.fully_verified == 0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DowngradeReason, Language, RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetMetadata,
        SnippetStatus, SourceOrigin, ValidationLevel, ValidationResult,
    };

    fn result(status: SnippetStatus, unresolved_dependency: bool) -> ValidationResult {
        ValidationResult {
            snippet: Snippet {
                id: None,
                path: "example.md".into(),
                language: Language::Go,
                title: None,
                code: "package main".into(),
                start_line: 1,
                block_index: 0,
                annotation: None,
                metadata: SnippetMetadata::default(),
                source_origin: SourceOrigin {
                    path: "example.md".into(),
                    line: 1,
                    block_index: 0,
                },
            },
            status,
            level: ValidationLevel::Compile,
            requested_level: ValidationLevel::Compile,
            effective_level: ValidationLevel::Compile,
            message: None,
            duration_ms: 0,
            capability_capped: false,
            downgrade_reason: None,
            unresolved_dependency,
            timed_out: false,
            preflight_skipped: false,
        }
    }

    /// The reconciliation the fix promises: `unresolved_dependency` is always a subset of
    /// `unavailable`, never overlaps `failed`/`errors`, and every top-level bucket still sums to
    /// `total` — so a reader never has to trust the count, only add it up. ~keep
    #[test]
    fn unresolved_dependency_is_a_reconcilable_subset_of_unavailable() {
        let summary = RunSummary::from_results(vec![
            result(SnippetStatus::Unavailable, true),
            result(SnippetStatus::Unavailable, false),
            result(SnippetStatus::Fail, false),
            result(SnippetStatus::Pass, false),
        ]);

        assert_eq!(summary.total, 4);
        assert_eq!(summary.unavailable, 2);
        assert_eq!(summary.unresolved_dependency, 1);
        assert!(summary.unresolved_dependency <= summary.unavailable);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.passed, 1);
        assert_eq!(
            summary.total,
            summary.passed
                + summary.downgraded
                + summary.failed
                + summary.skipped
                + summary.errors
                + summary.unavailable
        );
        assert!(summary.has_failures());
    }

    /// A `capability_capped` `Pass` is still a `Pass` in the `passed` bucket, but must not count
    /// as `fully_verified` -- it never reached the requested level at all. Task #488's whole
    /// point: `passed` alone cannot tell a reader how much of the corpus was actually checked at
    /// the level it claims. ~keep
    #[test]
    fn fully_verified_excludes_capability_capped_and_declared_capped_passes() {
        let mut capability_capped = result(SnippetStatus::Pass, false);
        capability_capped.capability_capped = true;
        capability_capped.downgrade_reason = Some(DowngradeReason::ValidatorCapability);
        let mut declared_capped = result(SnippetStatus::Pass, false);
        declared_capped.downgrade_reason = Some(DowngradeReason::Declared);
        let clean_pass = result(SnippetStatus::Pass, false);

        let summary = RunSummary::from_results(vec![capability_capped, declared_capped, clean_pass]);

        assert_eq!(summary.passed, 3, "all three are still Pass results");
        assert_eq!(
            summary.fully_verified, 1,
            "only the uncapped Pass reached the requested level"
        );
    }

    /// Negative control: a healthy run with real passes must never report `checked_nothing`, even
    /// with unrelated failures and unavailable results mixed in. ~keep
    #[test]
    fn checked_nothing_is_false_when_anything_was_fully_verified() {
        let summary = RunSummary::from_results(vec![
            result(SnippetStatus::Pass, false),
            result(SnippetStatus::Fail, false),
            result(SnippetStatus::Unavailable, true),
        ]);

        assert!(!summary.checked_nothing());
    }

    /// The gate this whole field exists for: every result exempted or unavailable, and not one
    /// that actually reached the requested level, must be visible as "checked nothing" even
    /// though `has_failures()` alone reports a clean run. ~keep
    #[test]
    fn checked_nothing_is_true_when_the_whole_corpus_is_capped_or_unavailable() {
        let mut capability_capped = result(SnippetStatus::Pass, false);
        capability_capped.capability_capped = true;
        capability_capped.downgrade_reason = Some(DowngradeReason::ValidatorCapability);

        let summary = RunSummary::from_results(vec![capability_capped, result(SnippetStatus::Unavailable, true)]);

        assert!(!summary.has_failures(), "sanity: nothing here is a Fail or an Error");
        assert!(summary.checked_nothing());
    }

    /// An empty run (nothing discovered) is a discovery problem, not a "checked nothing" one --
    /// `checked_nothing` must not fire on `total == 0`.
    #[test]
    fn checked_nothing_is_false_on_an_empty_run() {
        let summary = RunSummary::from_results(vec![]);

        assert!(!summary.checked_nothing());
    }

    /// Table-driven coverage for every rustdoc fence-info shape task #370 named, plus a
    /// genuinely unknown language that must still be rejected -- accepting everything
    /// would fix the false positive by making the check vacuous in the other direction.
    #[test]
    fn from_fence_info_parses_rustdoc_attribute_combinations() {
        let cases = [
            ("rust", Language::Rust),
            ("", Language::Rust),
            ("no_run", Language::Rust),
            ("ignore", Language::Rust),
            ("should_panic", Language::Rust),
            ("compile_fail", Language::Rust),
            ("rust,no_run", Language::Rust),
            ("rust,ignore", Language::Rust),
            ("rust,no_run,should_panic", Language::Rust),
            ("rust,edition2021", Language::Rust),
            ("python", Language::Python),
            ("some_unknown_language", Language::Unknown),
        ];
        for (fence_info, expected) in cases {
            assert_eq!(
                Language::from_fence_info(fence_info),
                expected,
                "fence info `{fence_info}` should resolve to {expected:?}"
            );
        }
    }

    #[test]
    fn validation_level_parses_typecheck_aliases() {
        assert_eq!("typecheck".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!("type-check".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!("TypeCheck".parse::<ValidationLevel>(), Ok(ValidationLevel::TypeCheck));
        assert_eq!(ValidationLevel::TypeCheck.to_string(), "typecheck");
    }

    #[test]
    fn typecheck_orders_between_compile_and_run() {
        assert!(ValidationLevel::Compile < ValidationLevel::TypeCheck);
        assert!(ValidationLevel::TypeCheck < ValidationLevel::Run);
    }

    #[test]
    fn typecheck_only_annotation_kind_is_distinct() {
        assert_ne!(SnippetAnnotationKind::TypeCheckOnly, SnippetAnnotationKind::CompileOnly);
    }

    #[test]
    fn side_effects_round_trip_and_accept_legacy_safe_aliases() {
        for class in [
            SideEffectClass::Safe,
            SideEffectClass::Network,
            SideEffectClass::Process,
            SideEffectClass::Install,
            SideEffectClass::Server,
        ] {
            let encoded = serde_json::to_string(&class).unwrap();
            assert_eq!(serde_json::from_str::<SideEffectClass>(&encoded).unwrap(), class);
        }
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""none""#).unwrap(),
            SideEffectClass::Safe
        );
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""local""#).unwrap(),
            SideEffectClass::Safe
        );
        assert!(serde_json::from_str::<SideEffectClass>(r#""external_mutation""#).is_err());
    }
}
