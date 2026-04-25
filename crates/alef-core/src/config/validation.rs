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

use std::collections::HashMap;

use super::AlefConfig;
use super::output::{BuildCommandConfig, CleanConfig, LintConfig, SetupConfig, TestConfig, UpdateConfig};
use crate::error::AlefError;

/// Validate user-supplied pipeline overrides.
///
/// Returns the first error encountered (or `Ok(())` when every user-supplied
/// table either declares a precondition or only sets non-main fields).
pub fn validate(config: &AlefConfig) -> Result<(), AlefError> {
    validate_tools(&config.tools)?;
    if let Some(map) = &config.lint {
        validate_section("lint", map, lint_main_fields, |c| c.precondition.as_deref())?;
    }
    if let Some(map) = &config.test {
        validate_section("test", map, test_main_fields, |c| c.precondition.as_deref())?;
    }
    if let Some(map) = &config.build_commands {
        validate_section("build_commands", map, build_main_fields, |c| c.precondition.as_deref())?;
    }
    if let Some(map) = &config.setup {
        validate_section("setup", map, setup_main_fields, |c| c.precondition.as_deref())?;
    }
    if let Some(map) = &config.update {
        validate_section("update", map, update_main_fields, |c| c.precondition.as_deref())?;
    }
    if let Some(map) = &config.clean {
        validate_section("clean", map, clean_main_fields, |c| c.precondition.as_deref())?;
    }
    Ok(())
}

fn validate_section<C, F, P>(
    section: &str,
    table: &HashMap<String, C>,
    main_fields: F,
    precondition: P,
) -> Result<(), AlefError>
where
    F: Fn(&C) -> Vec<&'static str>,
    P: Fn(&C) -> Option<&str>,
{
    for (lang, cfg) in table {
        let main = main_fields(cfg);
        if !main.is_empty() && precondition(cfg).is_none() {
            let fields = main.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join("/");
            return Err(AlefError::Config(format!(
                "[{section}.{lang}] sets a main command ({fields}) without `precondition`. \
                 Custom commands must declare a `precondition` so the step degrades gracefully \
                 when the tool is missing on the user's system. Use a POSIX check such as \
                 `precondition = \"command -v <tool> >/dev/null 2>&1\"`."
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-config "which main fields are set?" helpers.
//
// Each helper returns the names of the main fields that are actually `Some`
// on the user's override. Emptiness means the table only customizes
// ancillary fields (typically `before`), which doesn't require a
// precondition.
// ---------------------------------------------------------------------------

fn lint_main_fields(c: &LintConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.format.is_some() {
        v.push("format");
    }
    if c.check.is_some() {
        v.push("check");
    }
    if c.typecheck.is_some() {
        v.push("typecheck");
    }
    v
}

fn test_main_fields(c: &TestConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.command.is_some() {
        v.push("command");
    }
    if c.e2e.is_some() {
        v.push("e2e");
    }
    if c.coverage.is_some() {
        v.push("coverage");
    }
    v
}

fn build_main_fields(c: &BuildCommandConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.build.is_some() {
        v.push("build");
    }
    if c.build_release.is_some() {
        v.push("build_release");
    }
    v
}

fn setup_main_fields(c: &SetupConfig) -> Vec<&'static str> {
    if c.install.is_some() {
        vec!["install"]
    } else {
        Vec::new()
    }
}

fn update_main_fields(c: &UpdateConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.update.is_some() {
        v.push("update");
    }
    if c.upgrade.is_some() {
        v.push("upgrade");
    }
    v
}

fn clean_main_fields(c: &CleanConfig) -> Vec<&'static str> {
    if c.clean.is_some() { vec!["clean"] } else { Vec::new() }
}

// ---------------------------------------------------------------------------
// Tool-name well-formedness.
//
// `alef.toml` is trusted configuration: every shell-bound field
// (`precondition`, `before`, the main command fields) is passed verbatim
// to `sh -c`, by design — users author these commands and need full shell
// power (pipes, redirects, `&&`, etc.) to express real-world tooling.
//
// `[tools]` values are different. They name a single executable that is
// interpolated into a `command -v <tool>` precondition, so they should be
// short identifier-shaped strings — never multi-word commands or shell
// expressions. Rejecting non-identifier characters here catches typos
// (trailing space, accidental quote) up-front with a useful error, instead
// of failing later with a cryptic shell message. It is a well-formedness
// check, not a security boundary.
// ---------------------------------------------------------------------------

fn validate_tools(tools: &super::tools::ToolsConfig) -> Result<(), AlefError> {
    if let Some(pm) = tools.python_package_manager.as_deref() {
        ensure_well_formed_tool_name("tools.python_package_manager", pm)?;
    }
    if let Some(pm) = tools.node_package_manager.as_deref() {
        ensure_well_formed_tool_name("tools.node_package_manager", pm)?;
    }
    if let Some(list) = tools.rust_dev_tools.as_deref() {
        for tool in list {
            ensure_well_formed_tool_name("tools.rust_dev_tools[]", tool)?;
        }
    }
    Ok(())
}

fn is_well_formed_tool_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

fn ensure_well_formed_tool_name(field: &str, value: &str) -> Result<(), AlefError> {
    if value.is_empty() || !value.chars().all(is_well_formed_tool_char) {
        return Err(AlefError::Config(format!(
            "{field} = {value:?} is not a well-formed tool name. \
             Tool names must match `[A-Za-z0-9._-]+` (single executable, no spaces or shell metacharacters)."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> AlefConfig {
        toml::from_str(toml_str).expect("config should parse")
    }

    fn base_config() -> &'static str {
        r#"
languages = ["python"]
[crate]
name = "test-lib"
sources = ["src/lib.rs"]
"#
    }

    #[test]
    fn no_user_overrides_is_valid() {
        let config = parse(base_config());
        validate(&config).expect("default config should validate");
    }

    #[test]
    fn lint_override_with_main_cmd_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[lint.python]\nformat = \"black .\"\n",
            base = base_config()
        ));
        let err = validate(&config).expect_err("missing precondition should error");
        let msg = format!("{err}");
        assert!(msg.contains("[lint.python]"), "error should name the section: {msg}");
        assert!(msg.contains("precondition"), "error should mention precondition: {msg}");
    }

    #[test]
    fn lint_override_with_main_cmd_and_precondition_is_ok() {
        let config = parse(&format!(
            "{base}\n\n[lint.python]\nprecondition = \"command -v black\"\nformat = \"black .\"\n",
            base = base_config()
        ));
        validate(&config).expect("config with precondition should validate");
    }

    #[test]
    fn lint_override_with_only_before_no_precondition_is_ok() {
        // Adding `before` doesn't override the main command, so no precondition required.
        let config = parse(&format!(
            "{base}\n\n[lint.python]\nbefore = \"echo hi\"\n",
            base = base_config()
        ));
        validate(&config).expect("table with only `before` should validate");
    }

    #[test]
    fn test_override_with_main_cmd_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[test.python]\ncommand = \"pytest\"\n",
            base = base_config()
        ));
        let err = validate(&config).expect_err("missing precondition should error");
        assert!(format!("{err}").contains("[test.python]"));
    }

    #[test]
    fn test_override_with_only_e2e_requires_precondition() {
        let config = parse(&format!(
            "{base}\n\n[test.python]\ne2e = \"pytest tests/e2e\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("e2e without precondition should error");
    }

    #[test]
    fn build_override_with_main_cmd_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[build_commands.python]\nbuild = \"maturin develop\"\n",
            base = base_config()
        ));
        let err = validate(&config).expect_err("missing precondition should error");
        assert!(format!("{err}").contains("[build_commands.python]"));
    }

    #[test]
    fn setup_override_with_install_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[setup.python]\ninstall = \"uv sync\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("setup install without precondition should error");
    }

    #[test]
    fn update_override_with_main_cmd_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[update.python]\nupdate = \"uv sync --upgrade\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("update without precondition should error");
    }

    #[test]
    fn clean_override_with_main_cmd_no_precondition_errors() {
        let config = parse(&format!(
            "{base}\n\n[clean.python]\nclean = \"rm -rf dist\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("clean without precondition should error");
    }

    #[test]
    fn error_message_lists_only_actually_set_main_fields() {
        // User sets only `format` — error should name `format`, not the full triple.
        let config = parse(&format!(
            "{base}\n\n[lint.python]\nformat = \"black .\"\n",
            base = base_config()
        ));
        let msg = format!("{}", validate(&config).unwrap_err());
        assert!(msg.contains("`format`"), "expected `format`, got: {msg}");
        assert!(!msg.contains("`check`"), "should not mention unset `check`: {msg}");
        assert!(
            !msg.contains("`typecheck`"),
            "should not mention unset `typecheck`: {msg}"
        );
    }

    #[test]
    fn before_plus_main_cmd_without_precondition_still_errors() {
        // The "only-before" exemption must not leak into mixed cases.
        let config = parse(&format!(
            "{base}\n\n[lint.python]\nbefore = \"echo hi\"\nformat = \"black .\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("before + main without precondition must error");
    }

    #[test]
    fn malformed_python_package_manager_value_is_rejected() {
        let config = parse(&format!(
            "{base}\n\n[tools]\npython_package_manager = \"uv; rm -rf /\"\n",
            base = base_config()
        ));
        let err = validate(&config).expect_err("non-identifier tool name must be rejected");
        assert!(format!("{err}").contains("well-formed"));
    }

    #[test]
    fn malformed_node_package_manager_value_is_rejected() {
        let config = parse(&format!(
            "{base}\n\n[tools]\nnode_package_manager = \"pnpm$(echo bad)\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn malformed_rust_dev_tool_entry_is_rejected() {
        let config = parse(&format!(
            "{base}\n\n[tools]\nrust_dev_tools = [\"cargo-edit\", \"cargo`evil`\"]\n",
            base = base_config()
        ));
        validate(&config).expect_err("non-identifier tool name must be rejected");
    }

    #[test]
    fn whitespace_in_tool_name_is_rejected() {
        // Catches the common typo of a trailing space (`"uv "`).
        let config = parse(&format!(
            "{base}\n\n[tools]\npython_package_manager = \"uv \"\n",
            base = base_config()
        ));
        validate(&config).expect_err("trailing whitespace must be rejected");
    }

    #[test]
    fn empty_tool_name_is_rejected() {
        let config = parse(&format!(
            "{base}\n\n[tools]\npython_package_manager = \"\"\n",
            base = base_config()
        ));
        validate(&config).expect_err("empty tool name must be rejected");
    }

    #[test]
    fn safe_tool_names_are_accepted() {
        // Dot, hyphen, underscore, alphanumerics are all valid.
        let config = parse(&format!(
            "{base}\n\n[tools]\npython_package_manager = \"uv\"\n\
             node_package_manager = \"pnpm\"\n\
             rust_dev_tools = [\"cargo-edit\", \"cargo_sort\", \"tool.v2\"]\n",
            base = base_config()
        ));
        validate(&config).expect("normal tool names should validate");
    }

    #[test]
    fn override_with_main_cmd_and_precondition_validates_for_each_section() {
        let cases = [
            ("lint.python", "format", "command -v black"),
            ("test.python", "command", "command -v pytest"),
            ("build_commands.python", "build", "command -v maturin"),
            ("setup.python", "install", "command -v uv"),
            ("update.python", "update", "command -v uv"),
            ("clean.python", "clean", "command -v rm"),
        ];
        for (header, field, pre) in cases {
            let toml_str = format!(
                "{base}\n\n[{header}]\nprecondition = \"{pre}\"\n{field} = \"echo run\"\n",
                base = base_config()
            );
            let config = parse(&toml_str);
            validate(&config).unwrap_or_else(|_| panic!("[{header}] with precondition should validate"));
        }
    }
}
