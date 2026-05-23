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

    // Parse the TOML to get the top-level key set, then find their line numbers
    // via a line scan.  We parse first so we only flag keys that actually exist
    // in the document rather than doing a purely textual match.
    let table: toml::Table = match toml::from_str(raw_toml) {
        Ok(t) => t,
        // If the document is not valid TOML we can't do meaningful detection;
        // let the caller's real deserializer surface the parse error.
        Err(_) => return Ok(()),
    };

    // Collect top-level keys that are in the banned set.
    let mut found: Vec<(String, &str)> = table
        .keys()
        .filter_map(|k| suggestions.get(k.as_str()).map(|s| (k.clone(), *s)))
        .collect();

    if found.is_empty() {
        return Ok(());
    }

    // Stable order: sort by key name so output is deterministic.
    found.sort_by(|a, b| a.0.cmp(&b.0));

    // Find line numbers with a best-effort line scan.
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

    // Singular [crate] table → [[crates]] array of tables
    m.insert("crate", "move under `[[crates]]` (array of tables)");

    // Bare `version` scalar → [workspace] alef_version
    m.insert("version", "rename to `[workspace] alef_version`");

    // Per-language config → [[crates]] sub-table
    for lang in [
        "python", "node", "ruby", "php", "elixir", "wasm", "ffi", "gleam", "go", "java", "dart", "kotlin", "swift",
        "csharp", "r", "zig",
    ] {
        m.insert(lang, "move under `[[crates]]` for the relevant crate");
    }

    // Pipeline maps → [[crates]] sub-tables
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

    // Bare `languages` → [workspace] languages
    m.insert("languages", "move to `[workspace] languages`");

    // Workspace-level generation/format flags
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

    // Per-crate source/dep config
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
            // Match: `[key]`, `[[key]]`, or `key =` / `key=` at line start.
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
    // Section header: `[key]` or `[key.something]`
    // Array-of-tables header: `[[key]]` or `[[key.something]]`
    if let Some(inner) = line.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        let first_segment = inner.split('.').next().unwrap_or("").trim();
        if first_segment == key {
            return true;
        }
    }
    if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        // Exclude `[[…]]` — already handled above; a bare `[` line won't have
        // already been matched.
        if !inner.starts_with('[') {
            let first_segment = inner.split('.').next().unwrap_or("").trim();
            if first_segment == key {
                return true;
            }
        }
    }
    // Bare assignment: `key =` or `key=` — guard with a word boundary so that
    // a banned key `r` does not match a longer key like `rust = "x"`.
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
name = "spikard"
sources = ["src/lib.rs"]

[crates.lint.python]
check = "ruff check ."
"#;
        assert!(detect_legacy_keys(toml_str).is_ok());
    }

    #[test]
    fn detect_legacy_keys_catches_bare_crate_table() {
        // In the legacy schema, `languages` is a top-level key — it MUST appear before
        // any section header, or after ALL section headers.  Here we put both at the top
        // level so the TOML parser assigns them to the document root.
        let toml_str = r#"
languages = ["python"]

[crate]
name = "spikard"
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
name = "spikard"
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
            // languages must be top-level (before any section header)
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
        // languages and crate must be top-level; section headers below belong to root.
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
Tree = "tree_sitter::Tree"
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
        // Top-level scalars and tables must appear before any section header or after
        // the last one — put them all at the top so TOML assigns them to the document root.
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
        // Invalid TOML should not panic — return Ok and let the real parser
        // surface the error.
        let bad = "[[[ not valid toml";
        assert!(detect_legacy_keys(bad).is_ok());
    }

    #[test]
    fn is_top_level_key_line_respects_word_boundary_on_bare_assignment() {
        // The banned key `r` must not match `rust = ...` (different identifier
        // that happens to start with `r`).
        assert!(!is_top_level_key_line("rust = true", "r"));
        assert!(!is_top_level_key_line("ruby_extras = []", "ruby"));
        // But it must still match the genuine assignment forms.
        assert!(is_top_level_key_line("r = { something = true }", "r"));
        assert!(is_top_level_key_line("ruby = {}", "ruby"));
        assert!(is_top_level_key_line("ruby={}", "ruby"));
    }
}
