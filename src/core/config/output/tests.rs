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
    let toml_str = r#"install = "uv sync --no-install-project --no-install-workspace""#;
    let cfg: SetupConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(
        cfg.install.unwrap().commands(),
        vec!["uv sync --no-install-project --no-install-workspace"]
    );
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
    let cfg: super::super::workspace::WorkspaceConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.lint.contains_key("python"));
    assert!(cfg.lint.contains_key("node"));

    let py_lint = cfg.lint.get("python").unwrap();
    assert_eq!(py_lint.format.as_ref().unwrap().commands(), vec!["ruff format ."]);

    let node_lint = cfg.lint.get("node").unwrap();
    assert_eq!(
        node_lint.format.as_ref().unwrap().commands(),
        vec!["npx oxfmt", "npx oxlint --fix"]
    );

    assert!(cfg.update.contains_key("python"));
    assert!(cfg.update.contains_key("node"));

    let node_update = cfg.update.get("node").unwrap();
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
command = "cd packages/python && uv run --no-sync pytest"

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
install = "cd packages/python && uv sync --no-install-project --no-install-workspace"

[clean.go]
before = "echo cleaning go"
clean = "cd packages/go && go clean -cache"
"#;
    let cfg: super::super::workspace::WorkspaceConfig = toml::from_str(toml_str).unwrap();

    let go_lint = cfg.lint.get("go").unwrap();
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

    let py_lint = cfg.lint.get("python").unwrap();
    assert!(
        py_lint.precondition.is_none(),
        "lint.python should have no precondition"
    );
    assert!(py_lint.before.is_none(), "lint.python should have no before");

    let go_test = cfg.test.get("go").unwrap();
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

    let go_build = cfg.build_commands.get("go").unwrap();
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

    let go_update = cfg.update.get("go").unwrap();
    assert_eq!(
        go_update.precondition.as_deref(),
        Some("test -d packages/go"),
        "update.go precondition should be preserved"
    );
    assert!(go_update.before.is_none(), "update.go before should be None");

    let py_setup = cfg.setup.get("python").unwrap();
    assert_eq!(
        py_setup.precondition.as_deref(),
        Some("which uv"),
        "setup.python precondition should be preserved"
    );
    assert!(py_setup.before.is_none(), "setup.python before should be None");

    let go_clean = cfg.clean.get("go").unwrap();
    assert!(go_clean.precondition.is_none(), "clean.go precondition should be None");
    assert_eq!(
        go_clean.before.as_ref().unwrap().commands(),
        vec!["echo cleaning go"],
        "clean.go before should be preserved"
    );
}

#[test]
fn output_template_resolves_explicit_entry() {
    let tmpl = OutputTemplate {
        python: Some("crates/{crate}-py/src/".to_string()),
        ..Default::default()
    };
    assert_eq!(
        tmpl.resolve("sample_router", "python", true),
        PathBuf::from("crates/sample_router-py/src/")
    );
}

#[test]
fn output_template_substitutes_lang_and_crate() {
    let tmpl = OutputTemplate {
        go: Some("packages/{lang}/{crate}/".to_string()),
        ..Default::default()
    };
    assert_eq!(
        tmpl.resolve("sample_router-runtime", "go", true),
        PathBuf::from("packages/go/sample_router-runtime/")
    );
}

#[test]
fn output_template_falls_back_to_multi_crate_default() {
    let tmpl = OutputTemplate::default();
    assert_eq!(
        tmpl.resolve("sample_router-runtime", "python", true),
        PathBuf::from("packages/python/sample_router-runtime")
    );
}

#[test]
fn output_template_falls_back_to_single_crate_historical_default() {
    let tmpl = OutputTemplate::default();
    assert_eq!(
        tmpl.resolve("sample_router", "python", false),
        PathBuf::from("packages/python")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "node", false),
        PathBuf::from("packages/node")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "ruby", false),
        PathBuf::from("packages/ruby")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "php", false),
        PathBuf::from("packages/php")
    );
    assert_eq!(
        tmpl.resolve("sample_router", "elixir", false),
        PathBuf::from("packages/elixir")
    );
}

#[test]
fn output_template_falls_back_to_lang_dir_for_unknown_languages() {
    let tmpl = OutputTemplate::default();
    assert_eq!(tmpl.resolve("sample_router", "go", false), PathBuf::from("packages/go"));
    assert_eq!(
        tmpl.resolve("sample_router", "swift", false),
        PathBuf::from("packages/swift")
    );
}

#[test]
fn output_template_deserializes_from_toml() {
    let toml_str = r#"
python = "packages/python/{crate}/"
go     = "packages/go/{crate}/"
"#;
    let tmpl: OutputTemplate = toml::from_str(toml_str).unwrap();
    assert_eq!(tmpl.python.as_deref(), Some("packages/python/{crate}/"));
    assert_eq!(tmpl.go.as_deref(), Some("packages/go/{crate}/"));
    assert!(tmpl.node.is_none());
}

#[test]
#[should_panic(expected = "path separators are not allowed")]
fn resolve_rejects_crate_name_with_path_separator() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("../foo", "python", false);
}

#[test]
#[should_panic(expected = "path separators are not allowed")]
fn resolve_rejects_crate_name_with_backslash() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("..\\foo", "python", false);
}

#[test]
#[should_panic(expected = "NUL byte is not allowed")]
fn resolve_rejects_crate_name_with_nul_byte() {
    let tmpl = OutputTemplate::default();
    tmpl.resolve("foo\0bar", "python", false);
}

#[test]
#[should_panic(expected = "would escape the project root")]
fn resolve_rejects_template_that_produces_parent_dir() {
    let tmpl = OutputTemplate {
        python: Some("../../etc/{crate}".to_string()),
        ..Default::default()
    };
    tmpl.resolve("mylib", "python", false);
}

#[test]
fn resolve_accepts_normal_crate_name() {
    let tmpl = OutputTemplate::default();
    let path = tmpl.resolve("my-lib", "python", false);
    assert_eq!(path, PathBuf::from("packages/python"));
}
