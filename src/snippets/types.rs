use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

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
    /// Static type-checking without executing the code (e.g. `mypy` for Python, `tsc` for
    /// TypeScript). Deeper than `Compile` for dynamically-typed languages whose compile step is
    /// only a bytecode/syntax pass; equivalent to `Compile` for languages whose compiler already
    /// type-checks. Ordered between `Compile` and `Run` so it is the strongest static guarantee
    /// short of execution.
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
    pub title: Option<String>,
    pub level: Option<ValidationLevel>,
    pub skip: bool,
    pub reason: Option<String>,
    pub tags: Vec<String>,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetStatus {
    Pass,
    Fail,
    Skip,
    Error,
    Unavailable,
}

impl fmt::Display for SnippetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "pass"),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub snippet: Snippet,
    pub status: SnippetStatus,
    pub level: ValidationLevel,
    pub message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub unavailable: usize,
    pub results: Vec<ValidationResult>,
}

impl RunSummary {
    #[must_use]
    pub fn from_results(results: Vec<ValidationResult>) -> Self {
        let mut summary = Self {
            total: results.len(),
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            unavailable: 0,
            results,
        };

        for result in &summary.results {
            match result.status {
                SnippetStatus::Pass => summary.passed += 1,
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
}

#[cfg(test)]
mod tests {
    use super::{SnippetAnnotationKind, ValidationLevel};

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
}
