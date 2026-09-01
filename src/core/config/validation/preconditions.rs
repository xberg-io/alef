//! Pipeline override precondition enforcement.
//!
//! `test` is the only remaining per-command override table in `alef.toml` (0.82.0 removed
//! `lint`/`build_commands`/`setup`/`update`/`clean` -- alef now owns those commands end to end).
//! A custom `[test.<lang>]` table that overrides a main command field must declare a
//! `precondition` so the step degrades gracefully when the underlying tool is missing. A table
//! that only customizes `before` (without overriding the main command) is exempt.

use std::collections::HashMap;

use crate::core::config::output::TestConfig;
use crate::core::config::tools::ToolsConfig;
use crate::core::error::AlefError;

/// Validate that every entry in a pipeline section that sets a main command
/// also declares a `precondition`.
pub(super) fn validate_section<C, F, P>(
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
                "[{section}.{lang}] sets a main command ({fields}) without `precondition`. Add a POSIX \
                 check such as `precondition = \"command -v <tool> >/dev/null 2>&1\"`."
            )));
        }
    }
    Ok(())
}

/// Main fields gated by the block's top-level `precondition`.
///
/// `e2e` is deliberately excluded: it has its own `e2e_precondition` gate, checked separately by
/// [`validate_test_e2e_precondition`], so a block that sets only `e2e` is not forced to declare a
/// top-level `precondition` written for tooling `e2e` may not need.
pub(super) fn test_main_fields(c: &TestConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.command.is_some() {
        v.push("command");
    }
    if c.coverage.is_some() {
        v.push("coverage");
    }
    v
}

/// Reject a `[test.<lang>]` block that sets `e2e` without either `precondition` or
/// `e2e_precondition`.
///
/// Custom `e2e` commands are opaque to alef -- only the user knows what tooling they need. One of
/// the two precondition fields must degrade the run gracefully when that tooling is missing.
/// `e2e_precondition` exists precisely so this requirement does not force a block with no
/// `command` (only `before` + `e2e`, a common shape) into writing a `precondition` for tooling
/// the e2e command does not actually use.
pub(super) fn validate_test_e2e_precondition(table: &HashMap<String, TestConfig>) -> Result<(), AlefError> {
    for (lang, cfg) in table {
        if cfg.e2e.is_some() && cfg.precondition.is_none() && cfg.e2e_precondition.is_none() {
            return Err(AlefError::Config(format!(
                "[test.{lang}] sets `e2e` without `precondition` or `e2e_precondition`. Prefer \
                 `e2e_precondition`, scoped to what the `e2e` command itself needs: \
                 `e2e_precondition = \"command -v <tool> >/dev/null 2>&1\"`."
            )));
        }
    }
    Ok(())
}

/// Validate that all configured tool names are well-formed identifiers.
pub(super) fn validate_tools(tools: &ToolsConfig) -> Result<(), AlefError> {
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
