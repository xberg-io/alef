use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExcludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    /// Exclude specific methods: "TypeName.method_name"
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IncludeConfig {
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub csharp: Option<PathBuf>,
    pub r: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaffoldConfig {
    pub description: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// A value that can be either a single string or a list of strings.
///
/// Deserializes from both `"cmd"` and `["cmd1", "cmd2"]` in TOML/JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintConfig {
    pub format: Option<StringOrVec>,
    pub check: Option<StringOrVec>,
    pub typecheck: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Command(s) for safe dependency updates (compatible versions only).
    pub update: Option<StringOrVec>,
    /// Command(s) for aggressive updates (including incompatible/major bumps).
    pub upgrade: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestConfig {
    /// Command to run unit/integration tests for this language.
    pub command: Option<String>,
    /// Command to run e2e tests for this language.
    pub e2e: Option<String>,
}

/// A single text replacement rule for version sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextReplacement {
    /// Glob pattern for files to process.
    pub path: String,
    /// Regex pattern to search for (may contain `{version}` placeholder).
    pub search: String,
    /// Replacement string (may contain `{version}` placeholder).
    pub replace: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_or_vec_single_from_toml() {
        let toml_str = r#"format = "ruff format""#;
        #[derive(Deserialize)]
        struct T {
            format: StringOrVec,
        }
        let t: T = toml::from_str(toml_str).unwrap();
        assert_eq!(t.format.commands(), vec!["ruff format"]);
    }

    #[test]
    fn string_or_vec_multiple_from_toml() {
        let toml_str = r#"format = ["cmd1", "cmd2", "cmd3"]"#;
        #[derive(Deserialize)]
        struct T {
            format: StringOrVec,
        }
        let t: T = toml::from_str(toml_str).unwrap();
        assert_eq!(t.format.commands(), vec!["cmd1", "cmd2", "cmd3"]);
    }

    #[test]
    fn lint_config_backward_compat_string() {
        let toml_str = r#"
format = "ruff format ."
check = "ruff check ."
typecheck = "mypy ."
"#;
        let cfg: LintConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.format.unwrap().commands(), vec!["ruff format ."]);
        assert_eq!(cfg.check.unwrap().commands(), vec!["ruff check ."]);
        assert_eq!(cfg.typecheck.unwrap().commands(), vec!["mypy ."]);
    }

    #[test]
    fn lint_config_array_commands() {
        let toml_str = r#"
format = ["cmd1", "cmd2"]
check = "single-check"
"#;
        let cfg: LintConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.format.unwrap().commands(), vec!["cmd1", "cmd2"]);
        assert_eq!(cfg.check.unwrap().commands(), vec!["single-check"]);
        assert!(cfg.typecheck.is_none());
    }

    #[test]
    fn lint_config_all_optional() {
        let toml_str = "";
        let cfg: LintConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.format.is_none());
        assert!(cfg.check.is_none());
        assert!(cfg.typecheck.is_none());
    }

    #[test]
    fn update_config_from_toml() {
        let toml_str = r#"
update = "cargo update"
upgrade = ["cargo upgrade --incompatible", "cargo update"]
"#;
        let cfg: UpdateConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.update.unwrap().commands(), vec!["cargo update"]);
        assert_eq!(
            cfg.upgrade.unwrap().commands(),
            vec!["cargo upgrade --incompatible", "cargo update"]
        );
    }

    #[test]
    fn update_config_all_optional() {
        let toml_str = "";
        let cfg: UpdateConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.update.is_none());
        assert!(cfg.upgrade.is_none());
    }

    #[test]
    fn full_alef_toml_with_lint_and_update() {
        let toml_str = r#"
languages = ["python", "node"]

[crate]
name = "test"
sources = ["src/lib.rs"]

[lint.python]
format = "ruff format ."
check = "ruff check --fix ."

[lint.node]
format = ["npx oxfmt", "npx oxlint --fix"]

[update.python]
update = "uv sync --upgrade"
upgrade = "uv sync --all-packages --all-extras --upgrade"

[update.node]
update = "pnpm up -r"
upgrade = ["corepack up", "pnpm up --latest -r -w"]
"#;
        let cfg: super::super::AlefConfig = toml::from_str(toml_str).unwrap();
        let lint_map = cfg.lint.as_ref().unwrap();
        assert!(lint_map.contains_key("python"));
        assert!(lint_map.contains_key("node"));

        let py_lint = lint_map.get("python").unwrap();
        assert_eq!(py_lint.format.as_ref().unwrap().commands(), vec!["ruff format ."]);

        let node_lint = lint_map.get("node").unwrap();
        assert_eq!(
            node_lint.format.as_ref().unwrap().commands(),
            vec!["npx oxfmt", "npx oxlint --fix"]
        );

        let update_map = cfg.update.as_ref().unwrap();
        assert!(update_map.contains_key("python"));
        assert!(update_map.contains_key("node"));

        let node_update = update_map.get("node").unwrap();
        assert_eq!(node_update.update.as_ref().unwrap().commands(), vec!["pnpm up -r"]);
        assert_eq!(
            node_update.upgrade.as_ref().unwrap().commands(),
            vec!["corepack up", "pnpm up --latest -r -w"]
        );
    }
}

/// Configuration for the `sync-versions` command.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    /// Extra file paths to update version in (glob patterns).
    #[serde(default)]
    pub extra_paths: Vec<String>,
    /// Arbitrary text replacements applied during version sync.
    #[serde(default)]
    pub text_replacements: Vec<TextReplacement>,
}
