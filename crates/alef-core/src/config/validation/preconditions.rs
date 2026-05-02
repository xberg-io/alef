//! Pipeline override precondition enforcement.
//!
//! Custom `[lint|test|build_commands|setup|update|clean].<lang>` tables
//! that override a main command field must declare a `precondition` so the
//! step degrades gracefully when the underlying tool is missing. Tables that
//! only customize `before` (without overriding the main command) are exempt.

use std::collections::HashMap;

use crate::config::output::{BuildCommandConfig, CleanConfig, LintConfig, SetupConfig, TestConfig, UpdateConfig};
use crate::config::tools::ToolsConfig;
use crate::error::AlefError;

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
// "Which main fields are set?" helpers
// ---------------------------------------------------------------------------

pub(super) fn lint_main_fields(c: &LintConfig) -> Vec<&'static str> {
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

pub(super) fn test_main_fields(c: &TestConfig) -> Vec<&'static str> {
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

pub(super) fn build_main_fields(c: &BuildCommandConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.build.is_some() {
        v.push("build");
    }
    if c.build_release.is_some() {
        v.push("build_release");
    }
    v
}

pub(super) fn setup_main_fields(c: &SetupConfig) -> Vec<&'static str> {
    if c.install.is_some() {
        vec!["install"]
    } else {
        Vec::new()
    }
}

pub(super) fn update_main_fields(c: &UpdateConfig) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.update.is_some() {
        v.push("update");
    }
    if c.upgrade.is_some() {
        v.push("upgrade");
    }
    v
}

pub(super) fn clean_main_fields(c: &CleanConfig) -> Vec<&'static str> {
    if c.clean.is_some() { vec!["clean"] } else { Vec::new() }
}

// ---------------------------------------------------------------------------
// Tool-name well-formedness
//
// `[tools]` values name a single executable that is interpolated into a
// `command -v <tool>` precondition, so they should be short identifier-shaped
// strings. Rejecting non-identifier characters here catches typos up-front.
// ---------------------------------------------------------------------------

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
