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
    pub gleam: Option<PathBuf>,
    pub go: Option<PathBuf>,
    pub java: Option<PathBuf>,
    pub kotlin: Option<PathBuf>,
    pub dart: Option<PathBuf>,
    pub swift: Option<PathBuf>,
    pub csharp: Option<PathBuf>,
    pub r: Option<PathBuf>,
    pub zig: Option<PathBuf>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LintConfig {
    /// Shell command that must exit 0 for lint to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main lint commands; aborts on failure.
    pub before: Option<StringOrVec>,
    pub format: Option<StringOrVec>,
    pub check: Option<StringOrVec>,
    pub typecheck: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanConfig {
    /// Shell command that must exit 0 for clean to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main clean commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to clean build artifacts for this language.
    pub clean: Option<StringOrVec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

fn default_setup_timeout() -> u64 {
    600
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
    fn string_or_vec_empty_array_from_toml() {
        let toml_str = "format = []";
        #[derive(Deserialize)]
        struct T {
            format: StringOrVec,
        }
        let t: T = toml::from_str(toml_str).unwrap();
        assert!(matches!(t.format, StringOrVec::Multiple(_)));
        assert!(t.format.commands().is_empty());
    }

    #[test]
    fn string_or_vec_single_element_array_from_toml() {
        let toml_str = r#"format = ["cmd"]"#;
        #[derive(Deserialize)]
        struct T {
            format: StringOrVec,
        }
        let t: T = toml::from_str(toml_str).unwrap();
        assert_eq!(t.format.commands(), vec!["cmd"]);
    }

    #[test]
    fn setup_config_single_string() {
        let toml_str = r#"install = "uv sync""#;
        let cfg: SetupConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.install.unwrap().commands(), vec!["uv sync"]);
    }

    #[test]
    fn setup_config_array_commands() {
        let toml_str = r#"install = ["step1", "step2"]"#;
        let cfg: SetupConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.install.unwrap().commands(), vec!["step1", "step2"]);
    }

    #[test]
    fn setup_config_all_optional() {
        let toml_str = "";
        let cfg: SetupConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.install.is_none());
    }

    #[test]
    fn clean_config_single_string() {
        let toml_str = r#"clean = "rm -rf dist""#;
        let cfg: CleanConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.clean.unwrap().commands(), vec!["rm -rf dist"]);
    }

    #[test]
    fn clean_config_array_commands() {
        let toml_str = r#"clean = ["step1", "step2"]"#;
        let cfg: CleanConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.clean.unwrap().commands(), vec!["step1", "step2"]);
    }

    #[test]
    fn clean_config_all_optional() {
        let toml_str = "";
        let cfg: CleanConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.clean.is_none());
    }

    #[test]
    fn build_command_config_single_strings() {
        let toml_str = r#"
build = "cargo build"
build_release = "cargo build --release"
"#;
        let cfg: BuildCommandConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.build.unwrap().commands(), vec!["cargo build"]);
        assert_eq!(cfg.build_release.unwrap().commands(), vec!["cargo build --release"]);
    }

    #[test]
    fn build_command_config_array_commands() {
        let toml_str = r#"
build = ["step1", "step2"]
build_release = ["step1 --release", "step2 --release"]
"#;
        let cfg: BuildCommandConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.build.unwrap().commands(), vec!["step1", "step2"]);
        assert_eq!(
            cfg.build_release.unwrap().commands(),
            vec!["step1 --release", "step2 --release"]
        );
    }

    #[test]
    fn build_command_config_all_optional() {
        let toml_str = "";
        let cfg: BuildCommandConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.build.is_none());
        assert!(cfg.build_release.is_none());
    }

    #[test]
    fn test_config_backward_compat_string() {
        let toml_str = r#"command = "pytest""#;
        let cfg: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
        assert!(cfg.e2e.is_none());
        assert!(cfg.coverage.is_none());
    }

    #[test]
    fn test_config_array_command() {
        let toml_str = r#"command = ["cmd1", "cmd2"]"#;
        let cfg: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.command.unwrap().commands(), vec!["cmd1", "cmd2"]);
    }

    #[test]
    fn test_config_with_coverage() {
        let toml_str = r#"
command = "pytest"
coverage = "pytest --cov=. --cov-report=term-missing"
"#;
        let cfg: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
        assert_eq!(
            cfg.coverage.unwrap().commands(),
            vec!["pytest --cov=. --cov-report=term-missing"]
        );
        assert!(cfg.e2e.is_none());
    }

    #[test]
    fn test_config_all_optional() {
        let toml_str = "";
        let cfg: TestConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.command.is_none());
        assert!(cfg.e2e.is_none());
        assert!(cfg.coverage.is_none());
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

    #[test]
    fn lint_config_with_precondition_and_before() {
        let toml_str = r#"
precondition = "test -f target/release/libfoo.so"
before = "cargo build --release -p foo-ffi"
format = "gofmt -w packages/go"
check = "golangci-lint run ./..."
"#;
        let cfg: LintConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.precondition.as_deref(), Some("test -f target/release/libfoo.so"));
        assert_eq!(cfg.before.unwrap().commands(), vec!["cargo build --release -p foo-ffi"]);
        assert!(cfg.format.is_some());
        assert!(cfg.check.is_some());
    }

    #[test]
    fn test_config_with_before_list() {
        let toml_str = r#"
before = ["cd packages/python && maturin develop", "echo ready"]
command = "pytest"
"#;
        let cfg: TestConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.precondition.is_none());
        assert_eq!(
            cfg.before.unwrap().commands(),
            vec!["cd packages/python && maturin develop", "echo ready"]
        );
        assert_eq!(cfg.command.unwrap().commands(), vec!["pytest"]);
    }

    #[test]
    fn setup_config_with_precondition() {
        let toml_str = r#"
precondition = "which rustup"
install = "rustup update"
"#;
        let cfg: SetupConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.precondition.as_deref(), Some("which rustup"));
        assert!(cfg.before.is_none());
        assert!(cfg.install.is_some());
    }

    #[test]
    fn build_command_config_with_before() {
        let toml_str = r#"
before = "cargo build --release -p my-lib-ffi"
build = "cd packages/go && go build ./..."
"#;
        let cfg: BuildCommandConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.precondition.is_none());
        assert_eq!(
            cfg.before.unwrap().commands(),
            vec!["cargo build --release -p my-lib-ffi"]
        );
        assert!(cfg.build.is_some());
    }

    #[test]
    fn clean_config_precondition_and_before_optional() {
        let toml_str = r#"clean = "cargo clean""#;
        let cfg: CleanConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.precondition.is_none());
        assert!(cfg.before.is_none());
        assert!(cfg.clean.is_some());
    }

    #[test]
    fn update_config_with_precondition() {
        let toml_str = r#"
precondition = "test -f Cargo.lock"
update = "cargo update"
"#;
        let cfg: UpdateConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.precondition.as_deref(), Some("test -f Cargo.lock"));
        assert!(cfg.before.is_none());
        assert!(cfg.update.is_some());
    }

    #[test]
    fn full_alef_toml_with_precondition_and_before_across_sections() {
        let toml_str = r#"
languages = ["go", "python"]

[crate]
name = "mylib"
sources = ["src/lib.rs"]

[lint.go]
precondition = "test -f target/release/libmylib_ffi.so"
before = "cargo build --release -p mylib-ffi"
format = "gofmt -w packages/go"
check = "golangci-lint run ./..."

[lint.python]
format = "ruff format packages/python"
check = "ruff check --fix packages/python"

[test.go]
precondition = "test -f target/release/libmylib_ffi.so"
before = ["cargo build --release -p mylib-ffi", "cp target/release/libmylib_ffi.so packages/go/"]
command = "cd packages/go && go test ./..."

[test.python]
command = "cd packages/python && uv run pytest"

[build_commands.go]
precondition = "which go"
before = "cargo build --release -p mylib-ffi"
build = "cd packages/go && go build ./..."
build_release = "cd packages/go && go build -ldflags='-s -w' ./..."

[update.go]
precondition = "test -d packages/go"
update = "cd packages/go && go get -u ./..."

[setup.python]
precondition = "which uv"
install = "cd packages/python && uv sync"

[clean.go]
before = "echo cleaning go"
clean = "cd packages/go && go clean -cache"
"#;
        let cfg: super::super::AlefConfig = toml::from_str(toml_str).unwrap();

        // lint.go: precondition and before set
        let lint_map = cfg.lint.as_ref().unwrap();
        let go_lint = lint_map.get("go").unwrap();
        assert_eq!(
            go_lint.precondition.as_deref(),
            Some("test -f target/release/libmylib_ffi.so"),
            "lint.go precondition should be preserved"
        );
        assert_eq!(
            go_lint.before.as_ref().unwrap().commands(),
            vec!["cargo build --release -p mylib-ffi"],
            "lint.go before should be preserved"
        );
        assert!(go_lint.format.is_some());
        assert!(go_lint.check.is_some());

        // lint.python: no precondition or before
        let py_lint = lint_map.get("python").unwrap();
        assert!(
            py_lint.precondition.is_none(),
            "lint.python should have no precondition"
        );
        assert!(py_lint.before.is_none(), "lint.python should have no before");

        // test.go: precondition and multi-command before
        let test_map = cfg.test.as_ref().unwrap();
        let go_test = test_map.get("go").unwrap();
        assert_eq!(
            go_test.precondition.as_deref(),
            Some("test -f target/release/libmylib_ffi.so"),
            "test.go precondition should be preserved"
        );
        assert_eq!(
            go_test.before.as_ref().unwrap().commands(),
            vec![
                "cargo build --release -p mylib-ffi",
                "cp target/release/libmylib_ffi.so packages/go/"
            ],
            "test.go before list should be preserved"
        );

        // build_commands.go: precondition and before
        let build_map = cfg.build_commands.as_ref().unwrap();
        let go_build = build_map.get("go").unwrap();
        assert_eq!(
            go_build.precondition.as_deref(),
            Some("which go"),
            "build_commands.go precondition should be preserved"
        );
        assert_eq!(
            go_build.before.as_ref().unwrap().commands(),
            vec!["cargo build --release -p mylib-ffi"],
            "build_commands.go before should be preserved"
        );

        // update.go: precondition only, no before
        let update_map = cfg.update.as_ref().unwrap();
        let go_update = update_map.get("go").unwrap();
        assert_eq!(
            go_update.precondition.as_deref(),
            Some("test -d packages/go"),
            "update.go precondition should be preserved"
        );
        assert!(go_update.before.is_none(), "update.go before should be None");

        // setup.python: precondition only
        let setup_map = cfg.setup.as_ref().unwrap();
        let py_setup = setup_map.get("python").unwrap();
        assert_eq!(
            py_setup.precondition.as_deref(),
            Some("which uv"),
            "setup.python precondition should be preserved"
        );
        assert!(py_setup.before.is_none(), "setup.python before should be None");

        // clean.go: before only, no precondition
        let clean_map = cfg.clean.as_ref().unwrap();
        let go_clean = clean_map.get("go").unwrap();
        assert!(go_clean.precondition.is_none(), "clean.go precondition should be None");
        assert_eq!(
            go_clean.before.as_ref().unwrap().commands(),
            vec!["echo cleaning go"],
            "clean.go before should be preserved"
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
