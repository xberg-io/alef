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

pub mod assertion_recipes;
pub mod brew;
pub mod c;
pub mod client;
pub mod csharp;
pub mod dart;
mod dart_visitors;
pub mod elixir;
pub mod gleam;
pub mod go;
pub mod homebrew;
pub mod java;
mod java_mvnw;
pub mod kotlin;
pub mod kotlin_android;
pub mod php;
pub mod php_ext;
pub mod python;
pub mod r;
pub mod recipe;
pub mod ruby;
pub mod rust;
pub mod streaming_assertions;
pub mod swift;
mod swift_visitors;
pub mod typescript;
pub mod wasm;
pub mod zig;
mod zig_visitors;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{MethodDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
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
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Also respect skip_languages on the resolved call (e.g. batch_scrape skips elixir).
    if call_config.skip_languages.iter().any(|l| l == language) {
        return false;
    }
    // HTTP/mock fixtures are exercised by issuing a request to the alef mock server
    // (`MOCK_SERVER_URL/fixtures/<id>`), not by invoking a binding function, so they are
    // includable even when no call `function` is resolved for the language. Function-call
    // consumers (fixtures without `mock_response`/`http`) still require a resolved function
    // or a per-language override, leaving their behaviour unchanged.
    let is_http_fixture = fixture.mock_response.is_some() || fixture.http.is_some();
    if !is_http_fixture && call_config.function.is_empty() && !call_config.overrides.contains_key(language) {
        return false;
    }
    true
}

/// Percent-encode a string for use as a URI query component per RFC 3986.
///
/// Only the unreserved set (`ALPHA / DIGIT / "-" / "." / "_" / "~"`) is left
/// literal; every other byte (spaces, `?`, `&`, `=`, non-ASCII, …) is `%XX`-escaped.
/// Used by per-language e2e generators that embed query parameters into a request URL
/// literal — without this, values like `hi there` produce an invalid URI and the
/// generated test throws at parse time instead of exercising the fixture.
pub(crate) fn percent_encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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

/// Placeholder that e2e fixtures can embed inside structured JSON arguments.
///
/// This is useful for APIs where a URL lives inside a request DTO rather than in a
/// top-level `mock_url` argument. Language generators replace the token at test
/// runtime with the per-fixture mock server base URL.
pub(crate) const MOCK_URL_PLACEHOLDER: &str = "$mock_url";

/// Return true when a fixture value recursively contains [`MOCK_URL_PLACEHOLDER`].
pub(crate) fn value_contains_mock_url_placeholder(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => value.contains(MOCK_URL_PLACEHOLDER),
        serde_json::Value::Array(values) => values.iter().any(value_contains_mock_url_placeholder),
        serde_json::Value::Object(values) => values.values().any(value_contains_mock_url_placeholder),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => false,
    }
}

/// Environment variable used by the mock server for fixtures with a host-root listener.
pub(crate) fn mock_url_env_key(fixture_id: &str) -> String {
    format!("MOCK_SERVER_{}", fixture_id.to_uppercase())
}

/// Trait for per-language e2e test code generation.
pub trait E2eCodegen: Send + Sync {
    /// Generate all e2e test project files for this language.
    ///
    /// `type_defs` is the IR type registry extracted from the source crate.
    /// It is used by backends that need to introspect struct field types at
    /// codegen time (e.g. the TypeScript/WASM generator uses it to
    /// auto-derive `nested_types` mappings for wasm-bindgen class wrapping).
    ///
    /// `enums` is the IR enum registry extracted from the source crate.
    /// For WASM, it is used to identify tagged-data enums so they are emitted
    /// as plain JS object literals instead of wrapper factories.
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[TypeDef],
        enums: &[crate::core::ir::EnumDef],
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
        Box::new(kotlin_android::KotlinAndroidE2eCodegen),
        Box::new(csharp::CSharpCodegen),
        Box::new(php::PhpCodegen),
        Box::new(php_ext::PhpExtCodegen),
        Box::new(ruby::RubyCodegen),
        Box::new(elixir::ElixirCodegen),
        Box::new(gleam::GleamE2eCodegen),
        Box::new(r::RCodegen),
        Box::new(wasm::WasmCodegen),
        Box::new(c::CCodegen),
        Box::new(zig::ZigE2eCodegen),
        Box::new(dart::DartE2eCodegen),
        Box::new(swift::SwiftE2eCodegen),
        Box::new(brew::BrewCodegen),
        Box::new(homebrew::HomebrewCodegen),
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

/// Select the best-matching call for a fixture based on input field availability.
///
/// When the initially resolved call config has required args whose fields are
/// missing from fixture input, search the named calls for one whose args better
/// match the available input fields. This allows generic call selection even when
/// select_when conditions are too specific (e.g., category-restricted).
///
/// Returns the passed-in `initial_call` if no better match is found.
pub(crate) fn select_best_matching_call<'a>(
    initial_call: &'a crate::e2e::config::CallConfig,
    e2e_config: &'a E2eConfig,
    fixture: &Fixture,
) -> &'a crate::e2e::config::CallConfig {
    // Check if initial call's required args can be satisfied from fixture input
    let initial_satisfied = initial_call.args.iter().all(|arg| {
        if arg.optional {
            return true;
        }
        // For mock_url_list args, use resolve_urls_field which handles aliasing
        // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
        let field_value = if arg.arg_type == "mock_url_list" {
            resolve_urls_field(&fixture.input, &arg.field)
        } else {
            resolve_field(&fixture.input, &arg.field)
        };
        field_value.as_null().is_none()
    });

    if initial_satisfied {
        return initial_call;
    }

    // Initial call has unsatisfied required args. Search named calls for a better match.
    for alt_call in e2e_config.calls.values() {
        let all_satisfied = alt_call.args.iter().all(|arg| {
            if arg.optional {
                return true;
            }
            // For mock_url_list args, use resolve_urls_field which handles aliasing
            // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
            let field_value = if arg.arg_type == "mock_url_list" {
                resolve_urls_field(&fixture.input, &arg.field)
            } else {
                resolve_field(&fixture.input, &arg.field)
            };
            field_value.as_null().is_none()
        });

        if all_satisfied {
            return alt_call;
        }
    }

    // No better call found; use initial
    initial_call
}

/// Resolve a list-type argument field, trying both the declared field name and
/// common aliases (batch_urls, urls; urls_list, url_list).
///
/// Used by codegen for `mock_url_list` arguments when the fixture may use
/// alternative field names (e.g. some fixtures use `urls` while call config
/// declares `batch_urls`).
pub(crate) fn resolve_urls_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // Try the declared field first
    let result = resolve_field(input, field_path);
    if !result.is_null() {
        return result;
    }

    // Try common aliases if the primary field is not found
    let aliases = [
        ("batch_urls", "urls"),
        ("urls", "batch_urls"),
        ("batch_urls", "url_list"),
        ("batch_urls", "urls_list"),
        ("urls", "url_list"),
        ("urls", "urls_list"),
    ];

    for (orig, alias) in &aliases {
        if field_path.ends_with(orig) {
            let aliased_path = field_path.replace(orig, alias);
            let result = resolve_field(input, &aliased_path);
            if !result.is_null() {
                return result;
            }
        }
    }

    // Nothing found; return null
    &serde_json::Value::Null
}

/// Emission result for a test backend stub.
#[derive(Debug, Clone, Default)]
pub struct TestBackendEmission {
    /// Code emitted at the top of the test function: stub class/struct definition.
    pub setup_block: String,
    /// Expression passed as the register_X arg: stub instance or Bridge-wrapped instance.
    pub arg_expr: String,
    /// Short symbol names that must be imported at the file or function scope
    /// for the generated stub to compile.  Rust backend populates this with
    /// the trait name and any named return/parameter types so that callers can
    /// emit the appropriate `use module::Symbol;` statements.  Other language
    /// backends leave this empty — they manage imports internally.
    pub type_imports: Vec<String>,
    /// Optional teardown statements emitted after the fixture call and its
    /// assertions, used to undo registry mutations performed by trait-bridge
    /// fixtures (e.g. `unregister_ocr_backend("test-backend")`).
    ///
    /// Test runners that share a process across tests (python pytest, ruby
    /// rspec, dart `test`, etc.) leak registered test backends into later
    /// tests; without a teardown the next OCR-using fixture fails because the
    /// global registry contains only `test-backend` and the core's
    /// `ensure_ocr_backends_initialized` self-heal only triggers when the
    /// registry is empty. Emitting `unregister_<trait>(<name>)` here drains
    /// the test backend so the next access re-seeds the defaults.
    ///
    /// Languages that run each test in its own process (Rust cargo
    /// integration tests, Go) leave this empty.
    pub teardown_block: String,
}

impl TestBackendEmission {
    /// Placeholder for unimplemented backends.
    pub fn unimplemented(language: &str) -> Self {
        Self {
            setup_block: String::new(),
            arg_expr: format!("/* test_backend unimplemented for {} */", language),
            type_imports: Vec::new(),
            teardown_block: String::new(),
        }
    }
}

/// Dispatch test backend emission to per-language implementations.
///
/// When a fixture argument has `arg_type = "test_backend"`, this dispatcher
/// resolves the trait bridge config and calls the language-specific emitter.
/// Backends that haven't implemented test backend emission yet return
/// `TestBackendEmission::unimplemented(...)`.
pub fn emit_test_backend(
    language: &str,
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&MethodDef],
    fixture: &Fixture,
) -> TestBackendEmission {
    match language {
        "rust" => rust::emit_test_backend(trait_bridge, methods, fixture),
        "python" => python::emit_test_backend(trait_bridge, methods, fixture),
        "typescript" | "wasm" => typescript::emit_test_backend(trait_bridge, methods, fixture),
        "node" => typescript::emit_test_backend(trait_bridge, methods, fixture), // node uses typescript codegen
        "go" => go::emit_test_backend(trait_bridge, methods, fixture),
        "java" => java::emit_test_backend(trait_bridge, methods, fixture, ""),
        "kotlin" => kotlin::emit_test_backend(trait_bridge, methods, fixture),
        "kotlin_android" => kotlin_android::emit_test_backend(trait_bridge, methods, fixture),
        "csharp" => csharp::emit_test_backend(trait_bridge, methods, fixture),
        "php" => php::emit_test_backend(trait_bridge, methods, fixture),
        "ruby" => ruby::emit_test_backend(trait_bridge, methods, fixture),
        "elixir" => elixir::emit_test_backend(trait_bridge, methods, fixture, ""),
        "gleam" => gleam::emit_test_backend(trait_bridge, methods, fixture),
        "r" => r::emit_test_backend(trait_bridge, methods, fixture),
        "c" => c::emit_test_backend(trait_bridge, methods, fixture),
        "zig" => zig::emit_test_backend(trait_bridge, methods, fixture),
        "dart" => dart::emit_test_backend(trait_bridge, methods, fixture, &[]),
        "swift" => swift::emit_test_backend(trait_bridge, methods, fixture, &[]),
        "brew" => brew::emit_test_backend(trait_bridge, methods, fixture),
        "php_ext" => php_ext::emit_test_backend(trait_bridge, methods, fixture),
        "homebrew" => homebrew::emit_test_backend(trait_bridge, methods, fixture),
        _ => TestBackendEmission::unimplemented(language),
    }
}
