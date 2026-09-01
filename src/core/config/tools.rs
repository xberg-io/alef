//! Global tooling configuration.
//!
//! `[tools]` is a top-level section in `alef.toml` that selects per-language
//! package managers and dev-tool sets used by the default pipeline commands
//! (lint, test, build, setup, update, clean). Each field has a sensible default
//! so the section is fully optional; users only override what they need.
//!
//! One exception to "each field has a default": an explicitly set `python_package_manager` also
//! redirects the Python *build* through that manager's locked environment, while an unset one
//! leaves the build resolving maturin off `PATH`. [`super::python_build::python_tool_runner`]
//! therefore reads the raw field rather than [`ToolsConfig::python_pm`] — inventing a package
//! manager a repo never asked for would hand it an unrunnable build command. ~keep

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::extras::Language;

/// Default Rust dev tools installed by `alef setup rust`.
/// Mirrors the polyrepo's `task setup` so binding generators get a consistent
/// developer environment out of the box.
pub const DEFAULT_RUST_DEV_TOOLS: &[&str] = &[
    "cargo-edit",
    "cargo-sort",
    "cargo-machete",
    "cargo-deny",
    "cargo-llvm-cov",
];

const DEFAULT_PYTHON_PM: &str = "uv";
const DEFAULT_NODE_PM: &str = "pnpm";

/// Top-level `[tools]` config. Selects which package manager / tool variants
/// the default per-language pipeline commands target.
///
/// All fields are optional; getters return the documented default when unset.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolsConfig {
    /// Python package manager. One of: `"uv"`, `"pip"`, `"poetry"`. Default: `"uv"`.
    #[serde(default)]
    pub python_package_manager: Option<String>,

    /// Node package manager. One of: `"pnpm"`, `"npm"`, `"yarn"`. Default: `"pnpm"`.
    #[serde(default)]
    pub node_package_manager: Option<String>,

    /// Rust dev tools installed by the Rust `setup` default.
    /// Default: see [`DEFAULT_RUST_DEV_TOOLS`].
    #[serde(default)]
    pub rust_dev_tools: Option<Vec<String>>,
}

/// Per-language context passed to every `default_*_config` function.
///
/// Bundles the global `[tools]` selection plus three optional knobs that
/// reduce override boilerplate in consumer `alef.toml` files:
///
/// - `run_wrapper` — prefix every default tool invocation, e.g. wrap
///   `ruff format …` with `uv run --no-sync` so the lint step inherits the
///   project's package-manager environment without a full override.
/// - `extra_lint_paths` — append additional paths to the default lint
///   commands (`format`, `check`, `typecheck`).
/// - `project_file` — for languages whose tools target a project descriptor
///   (Java's `pom.xml`, C#'s `.csproj`/`.slnx`), use this file instead of
///   the package directory.
#[derive(Debug, Clone)]
pub struct LangContext<'a> {
    pub tools: &'a ToolsConfig,
    pub run_wrapper: Option<&'a str>,
    pub extra_lint_paths: &'a [String],
    pub project_file: Option<&'a str>,
}

impl<'a> LangContext<'a> {
    /// Create a context with all knobs unset (no wrapper, no extra paths,
    /// no project file). Useful in tests and call sites that only need the
    /// global tools selection.
    pub fn default(tools: &'a ToolsConfig) -> Self {
        Self {
            tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        }
    }
}

/// Wrap `cmd` with `wrapper` (e.g. `uv run --no-sync`) when set.
///
/// Used by per-language defaults so a single project-level knob can prefix
/// every default tool invocation without forcing a full command override.
pub fn wrap_command(cmd: String, wrapper: Option<&str>) -> String {
    match wrapper {
        Some(w) => format!("{w} {cmd}"),
        None => cmd,
    }
}

/// Append space-separated `paths` to `cmd`. No-op when `paths` is empty.
///
/// Path entries are inserted verbatim into a string that reaches `sh -c` (via the
/// `lint`/`build`/`test`/`update`/`setup` default builders that call this). This function
/// itself does no escaping -- the guarantee comes from upstream, not from this call site.
/// `super::validation::validate_extra_lint_paths` rejects any `extra_lint_paths` entry outside
/// `[A-Za-z0-9._/-]+` once, at config resolution, before any `Language` config carrying one
/// reaches a default builder. A previous version of this comment claimed that check existed
/// when it did not; re-derive this claim from `super::validation`'s actual contents rather
/// than trusting the comment, the same way that gap was found.
pub fn append_paths(cmd: String, paths: &[String]) -> String {
    if paths.is_empty() {
        cmd
    } else {
        format!("{} {}", cmd, paths.join(" "))
    }
}

/// Build a POSIX precondition that checks whether `tool` is on `PATH`.
///
/// The resulting command exits 0 when the tool is available and non-zero otherwise. Used by
/// per-language defaults to gate a single command step (e.g. Elixir's `mix deps.get` dependency
/// check) via `cli::pipeline::helpers::check_precondition`, which still treats a failing
/// precondition as that one step's own declared skip switch.
///
/// This precondition string is no longer the enforcement point for *toolchain* presence: a
/// missing tool used to make `check_precondition` skip the whole language with a warning,
/// reporting success for a step that ran nothing (the defect `enforce_required_toolchains`
/// fixes). For every language named in [`required_tools_for_language`], the toolchain check now
/// runs earlier, as a hard `anyhow::bail!` in `cli::pipeline::toolchains`, before any
/// `check_precondition` call keyed on one of these strings is ever reached. This function is
/// kept for the small number of remaining single-step gates that are not full toolchain
/// requirements. ~keep
pub fn require_tool(tool: &str) -> String {
    format!("command -v {tool} >/dev/null 2>&1")
}

/// Build a POSIX precondition requiring multiple tools to be on `PATH`.
///
/// Joins individual `command -v` checks with `&&` so the precondition only
/// passes when every listed tool is present. See [`require_tool`]'s doc for why this no longer
/// carries the toolchain-enforcement contract it used to.
pub fn require_tools(tools: &[&str]) -> String {
    tools.iter().map(|t| require_tool(t)).collect::<Vec<_>>().join(" && ")
}

/// The toolchain probe names a `Language`, once enabled for a crate, cannot do anything
/// without -- its package manager or primary build tool.
///
/// Deliberately narrower than every `require_tool` precondition scattered across
/// `*_defaults.rs`: those also gate optional per-step tools (`ruff`, `ktfmt`, `gofmt`,
/// `clang-format`, the mix/venv dependency checks) that a skipped step can tolerate without the
/// whole language silently doing nothing. This table is the input to
/// `cli::pipeline::toolchains::enforce_required_toolchains`, which hard-fails an enabled
/// language whose toolchain is missing instead of warning and skipping it. `Language::C` has no
/// entry: it is an FFI consumer test target, not a language alef generates or builds anything
/// for. ~keep
#[must_use]
pub fn required_tools_for_language(lang: Language, tools: &ToolsConfig) -> Vec<String> {
    match lang {
        // ~keep `cargo-upgrade` is cargo-edit's `cargo upgrade` subcommand. It cannot be probed
        // as `cargo-upgrade --version` (that binary expects to be invoked as a cargo subcommand
        // and rejects the bare flag); `command -v cargo-upgrade` -- what `is_tool_available`
        // does -- is the check that actually works, since cargo subcommand binaries are always
        // plain executables on PATH named `cargo-<name>`.
        Language::Rust => vec!["cargo".to_string(), "cargo-upgrade".to_string()],
        Language::Python => vec![tools.python_pm().to_string()],
        Language::Node => vec![tools.node_pm().to_string()],
        Language::Wasm => vec!["wasm-pack".to_string()],
        // ~keep Both, not just `ruby`. A bare `ruby` probe is very nearly vacuous -- system Ruby
        // is present on every macOS and most Linux images -- while what the Ruby steps actually
        // invoke is `ruby -S bundle` (see `ruby_bundle`), and `require_ruby_bundler`'s
        // precondition SKIPS setup silently when that resolves nothing. Probing `bundle` on PATH
        // is a slightly stronger condition than `ruby -S bundle` succeeding: a Ruby install that
        // ships bundler as a default gem without a PATH shim would pass the command and fail this
        // probe. That gap has not been observed on any of the five repos' images (rbenv, asdf and
        // the GitHub runners all shim `bundle`), and a false hard failure that names the missing
        // tool is a far better outcome than a Ruby stage that reports success having installed
        // nothing.
        Language::Ruby => vec!["ruby".to_string(), "bundle".to_string()],
        Language::Php => vec!["composer".to_string()],
        Language::Elixir => vec!["mix".to_string()],
        Language::Go => vec!["go".to_string()],
        Language::Java => vec!["mvn".to_string()],
        Language::Csharp => vec!["dotnet".to_string()],
        Language::R => vec!["Rscript".to_string()],
        Language::Kotlin | Language::KotlinAndroid => vec!["gradle".to_string()],
        Language::Swift => vec!["swift".to_string()],
        Language::Dart => vec!["dart".to_string()],
        Language::Gleam => vec!["gleam".to_string()],
        Language::Zig => vec!["zig".to_string()],
        // ~keep FFI and JNI ship no host-language toolchain of their own -- both are Rust crates
        // (a cdylib and a JNI shim, respectively) built exclusively through cargo.
        Language::Ffi | Language::Jni => vec!["cargo".to_string()],
        Language::C => vec![],
    }
}

/// Require the selected Ruby interpreter to resolve its own Bundler executable. ~keep
pub fn require_ruby_bundler() -> String {
    format!(
        "{} && {} >/dev/null 2>&1",
        require_tool("ruby"),
        ruby_bundle("--version")
    )
}

/// Run Bundler with a project-local gem path separated by Ruby ABI. ~keep
pub(crate) fn ruby_bundle(arguments: &str) -> String {
    format!("BUNDLE_PATH=vendor/bundle ruby -S bundle {arguments}")
}

/// Run a bundled Ruby gem executable through the active Ruby interpreter. ~keep
pub(crate) fn ruby_bundle_exec(command: &str) -> String {
    ruby_bundle(&format!("exec ruby -S {command}"))
}

impl ToolsConfig {
    /// Resolved Python package manager (defaults to `uv` when unset).
    pub fn python_pm(&self) -> &str {
        self.python_package_manager.as_deref().unwrap_or(DEFAULT_PYTHON_PM)
    }

    /// Resolved Node package manager (defaults to `pnpm` when unset).
    pub fn node_pm(&self) -> &str {
        self.node_package_manager.as_deref().unwrap_or(DEFAULT_NODE_PM)
    }

    /// Resolved Rust dev tools (defaults to [`DEFAULT_RUST_DEV_TOOLS`] when unset).
    pub fn rust_tools(&self) -> Vec<&str> {
        match self.rust_dev_tools.as_deref() {
            Some(list) => list.iter().map(String::as_str).collect(),
            None => DEFAULT_RUST_DEV_TOOLS.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let cfg = ToolsConfig::default();
        assert_eq!(cfg.python_pm(), "uv");
        assert_eq!(cfg.node_pm(), "pnpm");
        assert_eq!(
            cfg.rust_tools(),
            vec![
                "cargo-edit",
                "cargo-sort",
                "cargo-machete",
                "cargo-deny",
                "cargo-llvm-cov"
            ]
        );
    }

    #[test]
    fn getters_return_user_value_when_set() {
        let cfg = ToolsConfig {
            python_package_manager: Some("pip".to_string()),
            node_package_manager: Some("yarn".to_string()),
            rust_dev_tools: Some(vec!["cargo-foo".to_string(), "cargo-bar".to_string()]),
        };
        assert_eq!(cfg.python_pm(), "pip");
        assert_eq!(cfg.node_pm(), "yarn");
        assert_eq!(cfg.rust_tools(), vec!["cargo-foo", "cargo-bar"]);
    }

    #[test]
    fn empty_rust_dev_tools_is_respected() {
        let cfg = ToolsConfig {
            rust_dev_tools: Some(vec![]),
            ..Default::default()
        };
        assert!(cfg.rust_tools().is_empty());
    }

    #[test]
    fn deserializes_from_toml() {
        let toml_str = r#"
            python_package_manager = "poetry"
            node_package_manager = "npm"
            rust_dev_tools = ["cargo-edit"]
        "#;
        let cfg: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.python_pm(), "poetry");
        assert_eq!(cfg.node_pm(), "npm");
        assert_eq!(cfg.rust_tools(), vec!["cargo-edit"]);
    }

    #[test]
    fn require_tool_emits_command_v() {
        assert_eq!(require_tool("ruff"), "command -v ruff >/dev/null 2>&1");
    }

    #[test]
    fn ruby_bundler_precondition_checks_the_active_interpreter() {
        assert_eq!(
            require_ruby_bundler(),
            "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
        );
    }

    #[test]
    fn ruby_bundle_exec_forces_bundler_and_gem_tool_through_active_interpreter() {
        assert_eq!(
            ruby_bundle_exec("rubocop -A ."),
            "BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop -A ."
        );
    }

    #[cfg(unix)]
    fn write_executable(path: &std::path::Path, content: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::write(path, content).expect("write executable");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod executable");
    }

    #[cfg(unix)]
    #[test]
    fn ruby_bundle_exec_survives_foreign_tool_shebangs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ruby = temp.path().join("ruby");
        let bundle = temp.path().join("bundle");
        let rubocop = temp.path().join("rubocop");
        let marker = temp.path().join("marker");
        write_executable(
            &ruby,
            "#!/bin/sh\n[ \"$1\" = -S ] || exit 91\nshift\nscript=$(command -v \"$1\") || exit 92\nshift\nexec /bin/sh \"$script\" \"$@\"\n",
        );
        write_executable(
            &bundle,
            "#!/missing/foreign/ruby\n[ \"$BUNDLE_PATH\" = vendor/bundle ] || exit 94\n[ \"$1\" = exec ] || exit 93\nshift\nexec \"$@\"\n",
        );
        write_executable(
            &rubocop,
            "#!/missing/foreign/ruby\nprintf '%s\\n' active > \"$ABI_PROBE\"\n",
        );
        let path = format!(
            "{}:{}",
            temp.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |command: &str| {
            std::process::Command::new("/bin/sh")
                .args(["-c", command])
                .env("PATH", &path)
                .env("ABI_PROBE", &marker)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("run ABI probe")
        };

        assert!(!run("BUNDLE_PATH=vendor/bundle bundle exec ruby -S rubocop").success());
        assert!(!run("ruby -S bundle exec ruby -S rubocop").success());
        assert!(!run("BUNDLE_PATH=vendor/bundle ruby -S bundle exec rubocop").success());
        assert!(!marker.exists());
        assert!(run(&ruby_bundle_exec("rubocop")).success());
        assert_eq!(std::fs::read_to_string(marker).expect("read marker"), "active\n");
    }

    #[test]
    fn require_tools_joins_with_and() {
        assert_eq!(
            require_tools(&["go", "gofmt"]),
            "command -v go >/dev/null 2>&1 && command -v gofmt >/dev/null 2>&1"
        );
    }

    #[test]
    fn empty_toml_uses_defaults() {
        let cfg: ToolsConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.python_pm(), "uv");
        assert_eq!(cfg.node_pm(), "pnpm");
    }

    #[test]
    fn rust_required_tools_include_cargo_edit_alongside_cargo() {
        let tools = ToolsConfig::default();
        assert_eq!(
            required_tools_for_language(Language::Rust, &tools),
            vec!["cargo".to_string(), "cargo-upgrade".to_string()]
        );
    }

    #[test]
    fn python_and_node_required_tools_follow_the_configured_package_manager() {
        let tools = ToolsConfig {
            python_package_manager: Some("poetry".to_string()),
            node_package_manager: Some("yarn".to_string()),
            ..Default::default()
        };
        assert_eq!(
            required_tools_for_language(Language::Python, &tools),
            vec!["poetry".to_string()]
        );
        assert_eq!(
            required_tools_for_language(Language::Node, &tools),
            vec!["yarn".to_string()]
        );
    }

    #[test]
    fn c_has_no_required_toolchain() {
        assert!(required_tools_for_language(Language::C, &ToolsConfig::default()).is_empty());
    }

    #[test]
    fn ffi_and_jni_require_only_cargo() {
        let tools = ToolsConfig::default();
        assert_eq!(
            required_tools_for_language(Language::Ffi, &tools),
            vec!["cargo".to_string()]
        );
        assert_eq!(
            required_tools_for_language(Language::Jni, &tools),
            vec!["cargo".to_string()]
        );
    }
}
