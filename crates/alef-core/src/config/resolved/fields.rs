//! Field name resolution, serde strategy, path rewriting, and version methods.

use std::cmp::Reverse;

use super::ResolvedCrateConfig;
use crate::config::extras::Language;

impl ResolvedCrateConfig {
    /// Resolve the binding field name for a given language, type, and field.
    ///
    /// Resolution order (highest to lowest priority):
    /// 1. Per-language `rename_fields` map for the key `"TypeName.field_name"`.
    /// 2. Automatic keyword escaping: if the field name is a reserved keyword in the target
    ///    language, append `_` (e.g. `class` → `class_`).
    /// 3. Original field name unchanged.
    ///
    /// Returns `Some(escaped_name)` when the field needs renaming, `None` when the original
    /// name can be used as-is. Call sites that always need a `String` should use
    /// `resolve_field_name(...).unwrap_or_else(|| field_name.to_string())`.
    pub fn resolve_field_name(&self, lang: Language, type_name: &str, field_name: &str) -> Option<String> {
        // 1. Explicit per-language rename_fields entry.
        let explicit_key = format!("{type_name}.{field_name}");
        let explicit = match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Node => self.node.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Php => self.php.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Ffi => self.ffi.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Go => self.go.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Java => self.java.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::R => self.r.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Zig => self.zig.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Dart => self.dart.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Swift => self.swift.as_ref().and_then(|c| c.rename_fields.get(&explicit_key)),
            Language::Rust | Language::C => None,
        };
        if let Some(renamed) = explicit {
            if renamed != field_name {
                return Some(renamed.clone());
            }
            return None;
        }

        // 2. Automatic keyword escaping.
        match lang {
            Language::Python => crate::keywords::python_safe_name(field_name),
            // Java and C# use PascalCase for field names — no conflict.
            // Go uses PascalCase for exported fields — no conflict.
            // JS/TS handles keyword escaping at the napi layer via js_name attributes.
            _ => None,
        }
    }

    /// Get the effective serde rename_all strategy for a given language.
    ///
    /// Resolution order:
    /// 1. Per-language config override (`[python] serde_rename_all = "..."`)
    /// 2. Language default (idiomatic per-language JSON wire convention):
    ///    - camelCase: node, wasm, java, csharp, php, kotlin, swift, dart
    ///    - snake_case: python, ruby, go, ffi, elixir, r, rust, gleam, zig, c
    pub fn serde_rename_all_for_language(&self, lang: Language) -> String {
        let override_val = match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Node => self.node.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Php => self.php.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Ffi => self.ffi.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Go => self.go.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Java => self.java.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::R => self.r.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Zig => self.zig.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.serde_rename_all.as_deref()),
            Language::Rust | Language::C => None,
        };

        if let Some(val) = override_val {
            return val.to_string();
        }

        match lang {
            Language::Node
            | Language::Wasm
            | Language::Java
            | Language::Csharp
            | Language::Php
            | Language::Kotlin
            | Language::Swift
            | Language::Dart => "camelCase".to_string(),
            Language::Python
            | Language::Ruby
            | Language::Go
            | Language::Ffi
            | Language::Elixir
            | Language::R
            | Language::Rust
            | Language::Gleam
            | Language::Zig
            | Language::C => "snake_case".to_string(),
        }
    }

    /// Rewrite a Rust path using `path_mappings`.
    ///
    /// Matches the longest prefix first so more-specific mappings take
    /// priority over broader ones.
    pub fn rewrite_path(&self, rust_path: &str) -> String {
        let mut mappings: Vec<_> = self.path_mappings.iter().collect();
        mappings.sort_by_key(|b| Reverse(b.0.len()));

        for (from, to) in &mappings {
            if rust_path.starts_with(from.as_str()) {
                return format!("{}{}", to, &rust_path[from.len()..]);
            }
        }
        rust_path.to_string()
    }

    /// Collect all associated type names declared across every configured trait bridge.
    ///
    /// Returns the union of [`crate::config::TraitBridgeConfig::associated_type_names`]
    /// for all bridges. Backends use this set to skip generic record/enum codegen for
    /// these types, deferring to visitor-specific generators instead.
    pub fn bridge_associated_types(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        for bridge in &self.trait_bridges {
            for name in bridge.associated_type_names() {
                set.insert(name.to_string());
            }
        }
        set
    }

    /// Attempt to read the resolved version string from the configured `version_from` file.
    ///
    /// Returns `None` if the file cannot be read or the version cannot be found.
    /// Checks `[workspace.package] version` first, then `[package] version`.
    pub fn resolved_version(&self) -> Option<String> {
        let content = std::fs::read_to_string(&self.version_from).ok()?;
        let value: toml::Value = toml::from_str(&content).ok()?;
        if let Some(v) = value
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
        {
            return Some(v.to_string());
        }
        value
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::config::extras::Language;
    use crate::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> super::super::ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn serde_rename_all_python_defaults_to_snake_case() {
        let r = minimal();
        assert_eq!(r.serde_rename_all_for_language(Language::Python), "snake_case");
    }

    #[test]
    fn serde_rename_all_node_defaults_to_camel_case() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.serde_rename_all_for_language(Language::Node), "camelCase");
    }

    #[test]
    fn serde_rename_all_java_defaults_to_camel_case() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["java"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(r.serde_rename_all_for_language(Language::Java), "camelCase");
    }

    #[test]
    fn serde_rename_all_per_language_override_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
serde_rename_all = "camelCase"
"#,
        );
        assert_eq!(r.serde_rename_all_for_language(Language::Python), "camelCase");
    }

    #[test]
    fn resolved_resolve_field_name_keyword_escapes_python() {
        use crate::keywords::python_safe_name;
        let r = minimal();
        // "class" is a Python keyword, should be escaped
        let result = r.resolve_field_name(Language::Python, "MyType", "class");
        assert_eq!(result, python_safe_name("class"));
    }

    #[test]
    fn resolved_resolve_field_name_explicit_rename_wins_over_keyword_escape() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
rename_fields = { "MyType.class" = "klass" }
"#,
        );
        let result = r.resolve_field_name(Language::Python, "MyType", "class");
        assert_eq!(result, Some("klass".to_string()));
    }

    #[test]
    fn resolved_resolve_field_name_non_keyword_returns_none() {
        let r = minimal();
        let result = r.resolve_field_name(Language::Python, "MyType", "my_field");
        assert_eq!(result, None);
    }

    #[test]
    fn rewrite_path_applies_longest_prefix_first() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
path_mappings = { "foo::bar" = "baz::qux", "foo" = "zzz" }
"#,
        );
        // Longer prefix "foo::bar" wins over "foo"
        assert_eq!(r.rewrite_path("foo::bar::Struct"), "baz::qux::Struct");
        assert_eq!(r.rewrite_path("foo::Other"), "zzz::Other");
        assert_eq!(r.rewrite_path("unrelated"), "unrelated");
    }
}
