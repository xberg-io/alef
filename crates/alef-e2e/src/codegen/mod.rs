//! E2e test code generation trait and language dispatch.
//!
//! ## DRY layer ([`client`])
//!
//! Per-language e2e codegen historically duplicated the structural shape of every
//! test (function header, request build, response assert) and only differed in
//! syntax. The [`client`] submodule pulls that shape into trait + driver pairs
//! ([`client::TestClientRenderer`] + [`client::http_call::render_http_test`])
//! so each language can be migrated to TestClient-driven tests by:
//!
//! 1. Implementing `TestClientRenderer` once per language (small, mechanical).
//! 2. Replacing the language's monolithic `render_http_test_function` with a
//!    call to `client::http_call::render_http_test(out, &MyRenderer, fixture)`.
//! 3. Optionally splitting the per-language file into a directory
//!    `<lang>/{mod.rs,client.rs,ws.rs,helpers.rs}` when the file gets unwieldy.
//!
//! Until a language migrates, it continues using the legacy monolithic renderer —
//! both can coexist behind the per-language [`E2eCodegen::generate`] entry.

pub mod brew;
pub mod c;
pub mod client;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod r;
pub mod ruby;
pub mod rust;
pub mod streaming_assertions;
pub mod swift;
pub mod typescript;
pub mod wasm;
pub mod zig;

use crate::config::E2eConfig;
use crate::fixture::{Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::TypeDef;
use anyhow::Result;

/// Check if a fixture should be included for the given language.
///
/// Returns false if:
/// - The fixture's resolved category is in `e2e_config.exclude_categories`
///   (fixture is excluded from every language's cross-language e2e codegen)
/// - The fixture has a skip condition that applies to this language
/// - The fixture's call has no resolvable function for this language (no base
///   `function` set and no override for the language). Calls that share a base
///   function but only carry per-language type/arg overrides are still emitted
///   for languages without an explicit override.
pub(crate) fn should_include_fixture(fixture: &Fixture, language: &str, e2e_config: &E2eConfig) -> bool {
    if !e2e_config.exclude_categories.is_empty() && e2e_config.exclude_categories.contains(&fixture.resolved_category())
    {
        return false;
    }
    if let Some(skip) = &fixture.skip {
        if skip.should_skip(language) {
            return false;
        }
    }
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    // Also respect skip_languages on the resolved call (e.g. batch_scrape skips elixir).
    if call_config.skip_languages.iter().any(|l| l == language) {
        return false;
    }
    if call_config.function.is_empty() && !call_config.overrides.contains_key(language) {
        return false;
    }
    true
}

/// Recursively rewrite a JSON value's object keys to the target wire case.
///
/// `wire_case` accepts the same vocabulary as serde's `rename_all` attribute:
/// `"snake_case"` (default), `"camelCase"`, `"PascalCase"`, `"SCREAMING_SNAKE_CASE"`,
/// `"kebab-case"`, `"SCREAMING-KEBAB-CASE"`. Unknown values fall back to `snake_case`.
///
/// Used by per-language e2e codegen to translate canonical (snake_case) fixture keys
/// to the wire case that each binding's `from_json` / typed deserializer expects, as
/// driven by `ResolvedCrateConfig::serde_rename_all_for_language`.
pub(crate) fn transform_json_keys_for_language(value: &serde_json::Value, wire_case: &str) -> serde_json::Value {
    use heck::{ToKebabCase, ToLowerCamelCase, ToPascalCase, ToShoutyKebabCase, ToShoutySnakeCase, ToSnakeCase};
    let rewrite_key: fn(&str) -> String = match wire_case {
        "camelCase" => |k| k.to_lower_camel_case(),
        "PascalCase" => |k| k.to_pascal_case(),
        "SCREAMING_SNAKE_CASE" => |k| k.to_shouty_snake_case(),
        "kebab-case" => |k| k.to_kebab_case(),
        "SCREAMING-KEBAB-CASE" => |k| k.to_shouty_kebab_case(),
        _ => |k| k.to_snake_case(),
    };
    fn walk(value: &serde_json::Value, rewrite_key: fn(&str) -> String) -> serde_json::Value {
        match value {
            serde_json::Value::Object(obj) => {
                let new_obj: serde_json::Map<String, serde_json::Value> = obj
                    .iter()
                    .map(|(k, v)| (rewrite_key(k), walk(v, rewrite_key)))
                    .collect();
                serde_json::Value::Object(new_obj)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| walk(v, rewrite_key)).collect())
            }
            other => other.clone(),
        }
    }
    walk(value, rewrite_key)
}

/// Trait for per-language e2e test code generation.
pub trait E2eCodegen: Send + Sync {
    /// Generate all e2e test project files for this language.
    ///
    /// `type_defs` is the IR type registry extracted from the source crate.
    /// It is used by backends that need to introspect struct field types at
    /// codegen time (e.g. the TypeScript/WASM generator uses it to
    /// auto-derive `nested_types` mappings for wasm-bindgen class wrapping).
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[TypeDef],
    ) -> Result<Vec<GeneratedFile>>;

    /// Language name for display and directory naming.
    fn language_name(&self) -> &'static str;
}

/// Get all available e2e code generators.
pub fn all_generators() -> Vec<Box<dyn E2eCodegen>> {
    vec![
        Box::new(rust::RustE2eCodegen),
        Box::new(python::PythonE2eCodegen),
        Box::new(typescript::TypeScriptCodegen),
        Box::new(go::GoCodegen),
        Box::new(java::JavaCodegen),
        Box::new(kotlin::KotlinE2eCodegen),
        Box::new(csharp::CSharpCodegen),
        Box::new(php::PhpCodegen),
        Box::new(ruby::RubyCodegen),
        Box::new(elixir::ElixirCodegen),
        Box::new(r::RCodegen),
        Box::new(wasm::WasmCodegen),
        Box::new(c::CCodegen),
        Box::new(zig::ZigE2eCodegen),
        Box::new(dart::DartE2eCodegen),
        Box::new(swift::SwiftE2eCodegen),
        Box::new(brew::BrewCodegen),
    ]
}

/// Get e2e code generators for specific language names.
pub fn generators_for(languages: &[String]) -> Vec<Box<dyn E2eCodegen>> {
    all_generators()
        .into_iter()
        .filter(|g| languages.iter().any(|l| l == g.language_name()))
        .collect()
}

/// Resolve a JSON field from a fixture input by path.
///
/// Field paths in call config are "input.path", "input.config", etc.
/// Since we already receive `fixture.input`, strip the leading "input." prefix.
/// When `field_path` is exactly `"input"`, the whole input object is returned.
pub(crate) fn resolve_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // "input" with no subpath means "the entire input object".
    if field_path == "input" {
        return input;
    }
    let path = field_path.strip_prefix("input.").unwrap_or(field_path);
    let mut current = input;
    for part in path.split('.') {
        current = current.get(part).unwrap_or(&serde_json::Value::Null);
    }
    current
}
