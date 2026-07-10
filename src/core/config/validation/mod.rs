//! Validation of user-supplied pipeline overrides in `alef.toml`.
//!
//! When a user provides an explicit `[lint.<lang>]` / `[test.<lang>]` /
//! `[build_commands.<lang>]` / `[setup.<lang>]` / `[update.<lang>]` /
//! `[clean.<lang>]` table that **sets a main command field**, that table
//! must also declare a `precondition`. The rationale:
//!
//! - Built-in defaults all declare a `command -v <tool>` precondition so
//!   pipelines degrade gracefully when the underlying tool is missing.
//! - Custom commands are opaque to alef — only the user knows what the
//!   command requires. Forcing an explicit `precondition` keeps the
//!   warn-and-skip behavior intact on systems that can't run the command.
//!
//! Tables that only customize `before` (without overriding the main command)
//! are exempt: the default precondition still applies via the surrounding
//! defaults logic.

mod preconditions;

use super::resolved::ResolvedCrateConfig;
use crate::core::error::AlefError;
use preconditions::{
    build_main_fields, clean_main_fields, lint_main_fields, setup_main_fields, test_main_fields, update_main_fields,
    validate_section, validate_tools,
};

/// Validate user-supplied pipeline overrides in a resolved per-crate config.
///
/// Operates on the merged pipeline maps (already `HashMap` rather than
/// `Option<HashMap>`) that `ResolvedCrateConfig` carries after workspace
/// defaults are folded in.
pub fn validate_resolved(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    validate_tools(&config.tools)?;
    validate_package_metadata(config)?;
    validate_section("lint", &config.lint, lint_main_fields, |c| c.precondition.as_deref())?;
    validate_section("test", &config.test, test_main_fields, |c| c.precondition.as_deref())?;
    validate_section("build_commands", &config.build_commands, build_main_fields, |c| {
        c.precondition.as_deref()
    })?;
    validate_section("setup", &config.setup, setup_main_fields, |c| c.precondition.as_deref())?;
    validate_section("update", &config.update, update_main_fields, |c| {
        c.precondition.as_deref()
    })?;
    validate_section("clean", &config.clean, clean_main_fields, |c| c.precondition.as_deref())?;
    Ok(())
}

fn validate_package_metadata(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    const CRATES_IO_LIST_LIMIT: usize = 5;
    let Some(meta) = &config.package_metadata else {
        return Ok(());
    };
    if !meta.truncate_registry_lists {
        if meta.keywords.len() > CRATES_IO_LIST_LIMIT {
            return Err(AlefError::Config(format!(
                "crate `{}` package_metadata.keywords has {} entries; crates.io supports at most {CRATES_IO_LIST_LIMIT}. \
                 Reduce the list or set package_metadata.truncate_registry_lists = true.",
                config.name,
                meta.keywords.len()
            )));
        }
        if meta.categories.len() > CRATES_IO_LIST_LIMIT {
            return Err(AlefError::Config(format!(
                "crate `{}` package_metadata.categories has {} entries; crates.io supports at most {CRATES_IO_LIST_LIMIT}. \
                 Reduce the list or set package_metadata.truncate_registry_lists = true.",
                config.name,
                meta.categories.len()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;
    use tracing_test::traced_test;

    /// Parse a new-schema alef.toml and return the first resolved crate.
    fn resolve_first(toml_str: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_str).expect("config should parse");
        cfg.resolve().expect("config should resolve").remove(0)
    }

    fn base_config() -> &'static str {
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#
    }

    #[test]
    fn no_user_overrides_is_valid() {
        let config = resolve_first(base_config());
        validate_resolved(&config).expect("default config should validate");
    }

    #[test]
    fn lint_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nformat = \"black .\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("missing precondition should error");
        let msg = format!("{err}");
        assert!(msg.contains("[lint.python]"), "error should name the section: {msg}");
        assert!(msg.contains("precondition"), "error should mention precondition: {msg}");
    }

    #[test]
    fn lint_override_with_main_cmd_and_precondition_is_ok() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nprecondition = \"command -v black\"\nformat = \"black .\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("config with precondition should validate");
    }

    #[test]
    fn lint_override_with_only_before_no_precondition_is_ok() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nbefore = \"echo hi\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("table with only `before` should validate");
    }

    #[test]
    fn test_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.test.python]\ncommand = \"pytest\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("missing precondition should error");
        assert!(format!("{err}").contains("[test.python]"));
    }

    #[test]
    fn test_override_with_only_e2e_requires_precondition() {
        let toml = format!(
            "{base}\n[crates.test.python]\ne2e = \"pytest tests/e2e\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("e2e without precondition should error");
    }

    #[test]
    fn build_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.build_commands.python]\nbuild = \"maturin develop\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("missing precondition should error");
        assert!(format!("{err}").contains("[build_commands.python]"));
    }

    #[test]
    fn setup_override_with_install_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.setup.python]\ninstall = \"uv sync --no-install-project --no-install-workspace\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("setup install without precondition should error");
    }

    #[test]
    fn update_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.update.python]\nupdate = \"uv sync --upgrade\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("update without precondition should error");
    }

    #[test]
    fn clean_override_with_main_cmd_no_precondition_errors() {
        let toml = format!(
            "{base}\n[crates.clean.python]\nclean = \"rm -rf dist\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("clean without precondition should error");
    }

    #[test]
    fn error_message_lists_only_actually_set_main_fields() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nformat = \"black .\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let msg = format!("{}", validate_resolved(&config).unwrap_err());
        assert!(msg.contains("`format`"), "expected `format`, got: {msg}");
        assert!(!msg.contains("`check`"), "should not mention unset `check`: {msg}");
        assert!(
            !msg.contains("`typecheck`"),
            "should not mention unset `typecheck`: {msg}"
        );
    }

    #[test]
    fn before_plus_main_cmd_without_precondition_still_errors() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nbefore = \"echo hi\"\nformat = \"black .\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("before + main without precondition must error");
    }

    #[test]
    fn malformed_python_package_manager_value_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv; rm -rf /\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
        assert!(format!("{err}").contains("well-formed"));
    }

    #[test]
    fn malformed_node_package_manager_value_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\nnode_package_manager = \"pnpm$(echo bad)\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn malformed_rust_dev_tool_entry_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\nrust_dev_tools = [\"cargo-edit\", \"cargo`evil`\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn whitespace_in_tool_name_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv \"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("trailing whitespace must be rejected");
    }

    #[test]
    fn empty_tool_name_is_rejected() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect_err("empty tool name must be rejected");
    }

    #[test]
    fn safe_tool_names_are_accepted() {
        let toml = format!(
            "{base}\n[workspace.tools]\npython_package_manager = \"uv\"\n\
             node_package_manager = \"pnpm\"\n\
             rust_dev_tools = [\"cargo-edit\", \"cargo_sort\", \"tool.v2\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("normal tool names should validate");
    }

    #[test]
    fn package_metadata_keywords_over_crates_io_limit_errors() {
        let toml = format!(
            "{base}\n[crates.package_metadata]\nkeywords = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let err = validate_resolved(&config).expect_err("too many crates.io keywords should error");
        let msg = format!("{err}");
        assert!(msg.contains("package_metadata.keywords"), "got: {msg}");
        assert!(msg.contains("at most 5"), "got: {msg}");
    }

    #[test]
    fn package_metadata_can_opt_into_registry_list_truncation() {
        let toml = format!(
            "{base}\n[crates.package_metadata]\n\
             truncate_registry_lists = true\n\
             keywords = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n\
             categories = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("explicit truncation opt-in should validate");
    }

    #[test]
    fn override_with_main_cmd_and_precondition_validates_for_each_section() {
        for (section, field, lang) in [
            ("lint", "format", "python"),
            ("test", "command", "python"),
            ("build_commands", "build", "python"),
            ("setup", "install", "python"),
            ("update", "update", "python"),
            ("clean", "clean", "python"),
        ] {
            let toml = format!(
                "{base}\n[crates.{section}.{lang}]\nprecondition = \"command -v tool\"\n{field} = \"tool run\"\n",
                base = base_config()
            );
            let config = resolve_first(&toml);
            validate_resolved(&config).unwrap_or_else(|e| panic!("[{section}] with precondition should validate: {e}"));
        }
    }

    #[traced_test]
    #[test]
    fn lint_verbatim_default_emits_warning() {
        use crate::core::config::extras::Language;
        use crate::core::config::lint_defaults;
        use crate::core::config::tools::LangContext;
        let config = resolve_first(base_config());
        let ctx = LangContext::default(&config.tools);
        let default = lint_defaults::default_lint_config(Language::Python, "packages/python", &ctx);
        let Some(fmt_cmd) = default.format.as_ref().map(|c| c.commands().join(" ")) else {
            return;
        };
        let toml = format!(
            "{base}\n[crates.lint.python]\nformat = {fmt_cmd:?}\n",
            base = base_config()
        );
        let _resolved = resolve_first(&toml);
    }

    #[traced_test]
    #[test]
    fn lint_all_custom_emits_no_warning() {
        let toml = format!(
            "{base}\n[crates.lint.python]\nprecondition = \"command -v custom\"\nformat = \"custom-fmt\"\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        validate_resolved(&config).expect("custom lint with precondition must validate");
        assert!(!logs_contain("matches the built-in default"));
    }

    #[traced_test]
    #[test]
    fn node_custom_value_no_warning() {
        let toml_str = r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.lint.node]
precondition = "command -v custom-linter"
check = "custom-linter src/"
"#;
        let config = resolve_first(toml_str);
        validate_resolved(&config).expect("custom node lint must validate");
        assert!(!logs_contain("matches the built-in default"));
    }
}
