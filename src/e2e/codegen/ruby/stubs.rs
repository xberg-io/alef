//! Ruby e2e test-backend stub emission.

use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::escape::sanitize_ident;
use heck::ToSnakeCase;
use std::fmt::Write as FmtWrite;

/// Extract the canonical backend name from fixture input JSON.
///
/// Mirrors the lookup strategy used by the Python, PHP, and Rust e2e emitters.
/// Searches `input.name`, then any nested object's `name` field, then falls
/// back to `fixture_id`.
pub(super) fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Emit a Ruby test backend stub.
///
/// Ruby is duck-typed: define an anonymous class that responds to each required method
/// and return a sensible default value. The Plugin super-trait `name` method returns the
/// backend name extracted from `fixture.input`. All other methods return their
/// language-native defaults. Named return types return `'{}'` so the Magnus bridge can
/// deserialise the return value via JSON.
///
/// The returned `setup_block` defines a local variable `stub_<id>` holding the
/// anonymous class instance. The `arg_expr` is the variable name; callers emit
/// `<Module>.<register_fn>(arg_expr, "<fixture_id>")`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::core::ir::{PrimitiveType, TypeRef};

    let defaults = language_defaults("ruby");
    let safe_id = sanitize_ident(&fixture.id);
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let var_name = format!("stub_{safe_id}");

    let mut setup = String::new();
    let _ = writeln!(setup, "{var_name} = Class.new do");

    // Plugin super-trait: emit unconditional super-trait methods.
    // The Magnus bridge calls these on every registered plugin object regardless of
    // whether Rust has a default implementation, so stubs must define them.
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "  def name = '{backend_name}'");
        let _ = writeln!(setup, "  def initialize");
        let _ = writeln!(setup, "    nil");
        let _ = writeln!(setup, "  end");
        let _ = writeln!(setup, "  def shutdown");
        let _ = writeln!(setup, "    nil");
        let _ = writeln!(setup, "  end");
        let _ = writeln!(setup, "  def version = '1.0.0'");
    }

    // Emit stubs for all required methods (skip those with default implementations).
    for method in methods.iter().filter(|m| !m.has_default_impl) {
        let ruby_name = method.name.to_snake_case();
        // Build a parameter list: positional param names only (Ruby is duck-typed).
        let params: Vec<String> = method.params.iter().map(|p| sanitize_ident(&p.name)).collect();
        let param_str = params.join(", ");
        // Named types are not defined in the Ruby binding scope.  The Magnus bridge
        // tries String#to_s then falls back to .to_json, so return a JSON-safe empty
        // object string '{}'  that round-trips through serde_json.
        //
        // For numeric types in test backends, use a nonzero integer default.
        let default_val = match &method.return_type {
            TypeRef::Named(_) => "'{}'".to_string(),
            TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "1".to_string(),
            other => defaults.emit_default(other),
        };
        if param_str.is_empty() {
            let _ = writeln!(setup, "  def {ruby_name} = {default_val}");
        } else {
            let _ = writeln!(setup, "  def {ruby_name}({param_str}) = {default_val}");
        }
    }

    let _ = writeln!(setup, "end.new");

    TestBackendEmission {
        setup_block: setup,
        arg_expr: var_name,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}
