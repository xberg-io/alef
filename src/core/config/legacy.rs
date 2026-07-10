//! Detection of legacy `alef.toml` top-level keys.
//!
//! The pre-Phase-2 schema put everything at the top level of `alef.toml`:
//! `[crate]`, `languages`, `[python]`, `[lint.python]`, etc.  The new schema
//! groups things under `[workspace]` and `[[crates]]`.  This module scans raw
//! TOML text and reports every top-level key that belongs to the old layout,
//! with a human-readable suggestion for where to move it.
//!
//! Span detection: we do a best-effort line scan for each banned key rather
//! than requiring toml span support (which is version-dependent).  The line
//! number is 1-based; we don't track column because the line scan only
//! recognises top-level forms (`[key]`, `[[key]]`, `key = …`) where the key
//! always starts at column 1 anyway.

use std::collections::HashMap;
use std::sync::OnceLock;

/// A single legacy key detected in raw TOML.
#[derive(Debug, Clone)]
pub struct LegacyKey {
    /// The top-level TOML key that is no longer valid.
    pub key: String,
    /// 1-based line number of the first occurrence of the key. Best-effort —
    /// derived from a line scan over the raw TOML rather than the parser's
    /// span info, which would couple us to the toml crate version.
    pub line: usize,
    /// Human-readable migration suggestion.
    pub suggestion: String,
}

/// Error returned by [`detect_legacy_keys`] when legacy keys are found.
///
/// The detected keys are accessed via [`LegacyConfigError::keys`]; the field is
/// private so callers cannot truncate or reorder the list before formatting.
#[derive(Debug, thiserror::Error)]
#[error(
    "legacy alef.toml schema detected: {} key(s) must be moved. Run `alef migrate` to update automatically.\n{}",
    keys.len(),
    format_keys(keys)
)]
pub struct LegacyConfigError {
    keys: Vec<LegacyKey>,
}

impl LegacyConfigError {
    /// All legacy keys discovered, in detection order.
    pub fn keys(&self) -> &[LegacyKey] {
        &self.keys
    }
}

fn format_keys(keys: &[LegacyKey]) -> String {
    keys.iter()
        .map(|k| format!("  line {}: `{}` — {}", k.line, k.key, k.suggestion))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Scan `raw_toml` for top-level keys that belong to the old single-crate schema.
///
/// Returns `Ok(())` when no legacy keys are found, or a [`LegacyConfigError`]
/// listing every banned key with its line number and migration suggestion.
///
/// The check is intentionally conservative: it will not fire on `[[crates]]`
/// entries that happen to contain a field with the same name as a banned
/// top-level key — only genuine top-level bare assignments or section headers
/// trigger it.
pub fn detect_legacy_keys(raw_toml: &str) -> Result<(), LegacyConfigError> {
    let suggestions = banned_key_suggestions();

    let table: toml::Table = match toml::from_str(raw_toml) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };

    let mut found: Vec<(String, &str)> = table
        .keys()
        .filter_map(|k| suggestions.get(k.as_str()).map(|s| (k.clone(), *s)))
        .collect();

    if found.is_empty() {
        return Ok(());
    }

    found.sort_by(|a, b| a.0.cmp(&b.0));

    let line_map = first_occurrence_lines(raw_toml, found.iter().map(|(k, _)| k.as_str()));

    let keys: Vec<LegacyKey> = found
        .into_iter()
        .map(|(key, suggestion)| {
            let line = line_map.get(key.as_str()).copied().unwrap_or(1);
            LegacyKey {
                key,
                line,
                suggestion: suggestion.to_string(),
            }
        })
        .collect();

    Err(LegacyConfigError { keys })
}

/// Strip deprecated `format` and `format_overrides` keys from a parsed TOML value
/// before deserialization, emitting a warning string for each removed key.
///
/// This provides backward compatibility: old `alef.toml` files that still have
/// `[workspace.format]` / `[workspace.format_overrides]` / `[[crates]] format` /
/// `[[crates]] format_overrides` will parse cleanly instead of failing with
/// `deny_unknown_fields`.
pub fn strip_deprecated_keys(value: &mut toml::Value) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(root) = value.as_table_mut() else {
        return warnings;
    };
    if let Some(ws) = root.get_mut("workspace").and_then(|v| v.as_table_mut()) {
        if ws.remove("format").is_some() {
            warnings.push(
                "[workspace.format] is deprecated and ignored — alef always delegates formatting to poly".to_string(),
            );
        }
        if ws.remove("format_overrides").is_some() {
            warnings.push(
                "[workspace.format_overrides] is deprecated and ignored — alef always delegates formatting to poly"
                    .to_string(),
            );
        }
    }
    if let Some(crates) = root.get_mut("crates").and_then(|v| v.as_array_mut()) {
        for entry in crates.iter_mut() {
            if let Some(tbl) = entry.as_table_mut() {
                if tbl.remove("format").is_some() {
                    warnings.push(
                        "[[crates]] format is deprecated and ignored — alef always delegates formatting to poly"
                            .to_string(),
                    );
                }
                if tbl.remove("format_overrides").is_some() {
                    warnings.push(
                        "[[crates]] format_overrides is deprecated and ignored — alef always delegates formatting to poly"
                            .to_string(),
                    );
                }
            }
        }
    }
    warnings
}

/// Return a map from banned top-level key → migration suggestion string.
///
/// Built once on first call and cached in a [`OnceLock`] so repeated calls to
/// [`detect_legacy_keys`] don't re-allocate the table.
fn banned_key_suggestions() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(build_banned_key_suggestions)
}

/// Construct the banned-key suggestion map from scratch. Only called once via
/// [`OnceLock`].
fn build_banned_key_suggestions() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();

    m.insert("crate", "move under `[[crates]]` (array of tables)");

    m.insert("version", "rename to `[workspace] alef_version`");

    for lang in [
        "python", "node", "ruby", "php", "elixir", "wasm", "ffi", "gleam", "go", "java", "dart", "kotlin", "swift",
        "csharp", "r", "zig",
    ] {
        m.insert(lang, "move under `[[crates]]` for the relevant crate");
    }

    for key in [
        "output",
        "exclude",
        "include",
        "lint",
        "test",
        "setup",
        "update",
        "clean",
        "build_commands",
        "publish",
        "e2e",
        "scaffold",
        "readme",
        "custom_files",
        "custom_modules",
        "custom_registrations",
        "adapters",
        "trait_bridges",
    ] {
        m.insert(key, "move under `[[crates]]` for the relevant crate");
    }

    m.insert("languages", "move to `[workspace] languages`");

    for key in [
        "tools",
        "dto",
        "format",
        "format_overrides",
        "generate",
        "generate_overrides",
        "opaque_types",
        "sync",
    ] {
        m.insert(key, "move under `[workspace.<key>]`");
    }

    for key in [
        "path_mappings",
        "auto_path_mappings",
        "source_crates",
        "extra_dependencies",
    ] {
        m.insert(key, "move under `[[crates]] <key>`");
    }

    m
}

/// For each key in `keys`, scan `raw_toml` line by line and return the
/// 1-based line number of the first occurrence of that key as a top-level
/// TOML key (bare assignment or section header).
fn first_occurrence_lines<'k>(raw_toml: &str, keys: impl Iterator<Item = &'k str>) -> HashMap<String, usize> {
    let keys_vec: Vec<&str> = keys.collect();
    let mut result: HashMap<String, usize> = HashMap::new();

    for (idx, line) in raw_toml.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim_start();

        for &key in &keys_vec {
            if result.contains_key(key) {
                continue;
            }
            if is_top_level_key_line(trimmed, key) {
                result.insert(key.to_string(), line_no);
            }
        }

        if result.len() == keys_vec.len() {
            break;
        }
    }

    result
}

/// Return true when `line` is a TOML line that introduces `key` as a
/// top-level key (not nested inside another table header).
fn is_top_level_key_line(line: &str, key: &str) -> bool {
    if let Some(inner) = line.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        let first_segment = inner.split('.').next().unwrap_or("").trim();
        if first_segment == key {
            return true;
        }
    }
    if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if !inner.starts_with('[') {
            let first_segment = inner.split('.').next().unwrap_or("").trim();
            if first_segment == key {
                return true;
            }
        }
    }
    if let Some(rest) = line.strip_prefix(key) {
        let next = rest.chars().next();
        let is_word_boundary = match next {
            Some(c) => !(c.is_alphanumeric() || c == '_' || c == '-'),
            None => true,
        };
        if is_word_boundary {
            let trimmed = rest.trim_start();
            if trimmed.starts_with('=') {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_legacy_keys_returns_ok_for_new_schema() {
        let toml_str = r#"
[workspace]
alef_version = "0.13.0"
languages = ["python", "node"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.lint.python]
check = "ruff check ."
"#;
        assert!(detect_legacy_keys(toml_str).is_ok());
    }

    #[test]
    fn detect_legacy_keys_catches_bare_crate_table() {
        let toml_str = r#"
languages = ["python"]

[crate]
name = "sample_router"
sources = ["src/lib.rs"]
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        assert!(keys.contains(&"crate"), "expected `crate` in banned keys: {keys:?}");
        assert!(
            keys.contains(&"languages"),
            "expected `languages` in banned keys: {keys:?}"
        );
    }

    #[test]
    fn detect_legacy_keys_catches_bare_version() {
        let toml_str = r#"
version = "0.7.7"
languages = ["go"]

[crate]
name = "foo"
sources = []
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        assert!(keys.contains(&"version"), "`version` should be banned: {keys:?}");
    }

    #[test]
    fn detect_legacy_keys_catches_bare_languages() {
        let toml_str = r#"
languages = ["python", "go"]

[crate]
name = "sample_router"
sources = []
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        assert!(keys.contains(&"languages"), "`languages` should be banned: {keys:?}");
    }

    #[test]
    fn detect_legacy_keys_catches_language_sections() {
        for lang in [
            "python", "node", "ruby", "go", "java", "csharp", "wasm", "ffi", "elixir", "gleam", "zig",
        ] {
            let toml_str = format!(
                "languages = [\"{lang}\"]\n\n[crate]\nname = \"foo\"\nsources = []\n\n[{lang}]\nmodule_name = \"foo\"\n"
            );
            let err = detect_legacy_keys(&toml_str).unwrap_err();
            let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
            assert!(keys.contains(&lang), "`{lang}` should be detected as legacy: {keys:?}");
        }
    }

    #[test]
    fn detect_legacy_keys_catches_workspace_level_pipeline_keys() {
        let toml_str = r#"
languages = ["python"]

[crate]
name = "foo"
sources = []

[tools]
python_package_manager = "uv"

[dto]
python = "dataclass"

[format]
enabled = true

[generate]
bindings = true

[opaque_types]
Tree = "sample_language::Tree"
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        for expected in ["tools", "dto", "format", "generate", "opaque_types"] {
            assert!(
                keys.contains(&expected),
                "`{expected}` should be detected as legacy: {keys:?}"
            );
        }
    }

    #[test]
    fn detect_legacy_keys_catches_per_crate_source_keys() {
        let toml_str = r#"
languages = ["python"]
auto_path_mappings = true

[crate]
name = "foo"
sources = []

[path_mappings]
foo = "foo_core"

[extra_dependencies]
pyo3 = "0.22"
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        for expected in ["path_mappings", "auto_path_mappings", "extra_dependencies"] {
            assert!(
                keys.contains(&expected),
                "`{expected}` should be detected as legacy: {keys:?}"
            );
        }
    }

    #[test]
    fn detect_legacy_keys_catches_pipeline_table_keys() {
        let toml_str = r#"
languages = ["python"]

[crate]
name = "foo"
sources = []

[lint.python]
check = "ruff check ."

[test.python]
command = "pytest"

[build_commands.go]
build = "go build ./..."

[publish]
vendored = true

[e2e]
fixtures_dir = "e2e/fixtures"

[scaffold]
description = "My lib"

[readme]
template_dir = "docs/templates"
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        let keys: Vec<&str> = err.keys().iter().map(|k| k.key.as_str()).collect();
        for expected in ["lint", "test", "build_commands", "publish", "e2e", "scaffold", "readme"] {
            assert!(
                keys.contains(&expected),
                "`{expected}` should be detected as legacy: {keys:?}"
            );
        }
    }

    #[test]
    fn detect_legacy_keys_line_numbers_are_positive() {
        let toml_str = r#"
languages = ["python"]

[crate]
name = "foo"
sources = []
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        for k in err.keys() {
            assert!(k.line > 0, "line number must be positive for key `{}`", k.key);
        }
    }

    #[test]
    fn detect_legacy_keys_suggestions_are_non_empty() {
        let toml_str = r#"
languages = ["python"]

[crate]
name = "foo"
sources = []

[lint.python]
check = "ruff check ."
"#;
        let err = detect_legacy_keys(toml_str).unwrap_err();
        for k in err.keys() {
            assert!(
                !k.suggestion.is_empty(),
                "suggestion must be non-empty for key `{}`",
                k.key
            );
        }
    }

    #[test]
    fn detect_legacy_keys_invalid_toml_returns_ok() {
        let bad = "[[[ not valid toml";
        assert!(detect_legacy_keys(bad).is_ok());
    }

    #[test]
    fn is_top_level_key_line_respects_word_boundary_on_bare_assignment() {
        assert!(!is_top_level_key_line("rust = true", "r"));
        assert!(!is_top_level_key_line("ruby_extras = []", "ruby"));
        assert!(is_top_level_key_line("r = { something = true }", "r"));
        assert!(is_top_level_key_line("ruby = {}", "ruby"));
        assert!(is_top_level_key_line("ruby={}", "ruby"));
    }

    #[test]
    fn strip_deprecated_keys_removes_workspace_format() {
        let toml_str = r#"
[workspace]
languages = ["python"]

[workspace.format]
enabled = false

[workspace.format_overrides.python]
command = "ruff format ."

[[crates]]
name = "foo"
sources = ["src/lib.rs"]

[crates.format]
enabled = true

[crates.format_overrides.node]
enabled = false
"#;
        let mut value: toml::Value = toml::from_str(toml_str).unwrap();
        let warnings = strip_deprecated_keys(&mut value);
        assert!(!warnings.is_empty(), "expected deprecation warnings");
        let cfg: crate::core::config::NewAlefConfig = value.try_into().unwrap();
        assert_eq!(cfg.workspace.languages.len(), 1);
    }

    #[test]
    fn strip_deprecated_keys_returns_empty_for_clean_config() {
        let toml_str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "foo"
sources = ["src/lib.rs"]
"#;
        let mut value: toml::Value = toml::from_str(toml_str).unwrap();
        let warnings = strip_deprecated_keys(&mut value);
        assert!(warnings.is_empty(), "expected no warnings for clean config");
    }
}
