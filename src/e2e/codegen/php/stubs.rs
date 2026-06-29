//! PHP e2e test-backend stub emission.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeRef;
use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::escape::{escape_php, sanitize_ident};
use heck::ToUpperCamelCase;
use std::fmt::Write as FmtWrite;

/// Extract the canonical backend name from fixture input JSON.
///
/// Mirrors the lookup strategy used by the Python and Rust e2e emitters.
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

pub(super) fn trait_bridge_options_type(config: &ResolvedCrateConfig) -> Option<&str> {
    crate::e2e::codegen::recipe::trait_bridge_options_type(config)
}

/// Emit a PHP test backend stub.
///
/// PHP is duck-typed: define an anonymous class inside the test method body.
/// Each method returns a sensible PHP default. The Plugin super-trait `name`
/// method returns the backend name extracted from `fixture.input`.
///
/// The returned `setup_block` contains the inline class declaration.
/// The `arg_expr` is `$stub`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    emit_test_backend_with_ns(trait_bridge, methods, fixture, "", "")
}

/// Namespace-aware variant called directly from the PHP e2e renderer.
/// `binding_namespace` is the PHP namespace where the binding interfaces live (e.g. `SampleCrate`).
/// `binding_class` is the unqualified class name used for static teardown calls
/// (e.g. `unregister<Trait>`). When empty, teardown is omitted.
pub fn emit_test_backend_with_ns(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    binding_namespace: &str,
    binding_class: &str,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;

    let defaults = language_defaults("php");
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);

    // Build setup_block lines without leading indentation: the Jinja template
    // prefixes each line with 8 spaces (two method-body indent levels in PHPUnit).
    let mut setup = String::new();
    // PHP anonymous class must implement the interface explicitly.
    // Qualify the interface with the binding namespace to avoid resolution against
    // the e2e test namespace (e.g. `SampleCrate\E2e\DocumentExtractor` not found).
    let interface_name = trait_bridge.trait_name.to_upper_camel_case();
    let qualified_interface = if binding_namespace.is_empty() {
        interface_name.clone()
    } else {
        format!("\\{binding_namespace}\\{interface_name}")
    };
    let _ = writeln!(setup, "$stub = new class implements {qualified_interface} {{");

    // Plugin super-trait: emit `name()` returning the backend name string.
    if trait_bridge.super_trait.is_some() {
        let escaped_name = escape_php(&backend_name);
        let _ = writeln!(
            setup,
            "    public function name(): string {{ return '{escaped_name}'; }}"
        );
    }

    // Emit stubs for all required methods.
    // PHP interfaces require ALL abstract methods to be implemented, even if they have
    // default implementations in the Rust trait.
    // When super_trait is set, name() is already hardcoded above, so exclude it from iteration.
    for method in methods
        .iter()
        .filter(|m| !(trait_bridge.super_trait.is_some() && m.name == "name"))
    {
        // Stubs must match the generated interface signature, which preserves
        // snake_case Rust names verbatim (the interface does not opt into the
        // ext-php-rs `#[php(name = ...)]` camelCase rename — see
        // packages/php/src/DocumentExtractor.php for the canonical contract).
        let php_name = method.name.clone();
        // Named types are not defined in the PHP binding scope.  The PHP bridge
        // deserialises the return value via json_decode, so return a JSON-safe
        // empty-object string instead of attempting a constructor call.
        //
        // For numeric types in test backends, use 1 instead of 0 to satisfy validation
        // constraints (e.g., EmbeddingBackend::dimensions() must return > 0).
        let default_val = match &method.return_type {
            TypeRef::Named(_) => "'{}'".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "false".to_string(),
            TypeRef::Primitive(_) => "1".to_string(), // all integer types: 1 instead of 0
            other => defaults.emit_default(other),
        };
        // Parameter list: positional only (PHP is duck-typed; we omit type hints for simplicity).
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("${}", sanitize_ident(&p.name)))
            .collect();
        let param_str = params.join(", ");
        // The PHP trait interface types scalar returns (e.g. `dimensions(): int`) and uses
        // `mixed` for everything else — see `rust_type_to_php_type` in the php trait-bridge
        // backend. The stub's return type MUST match the interface (PHP rejects a wider
        // `mixed` override of a typed `int` method as incompatible), so mirror that mapping.
        let php_return_type = match &method.return_type {
            TypeRef::String => "string",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "bool",
            TypeRef::Primitive(
                crate::core::ir::PrimitiveType::I32
                | crate::core::ir::PrimitiveType::I64
                | crate::core::ir::PrimitiveType::U32
                | crate::core::ir::PrimitiveType::U64
                | crate::core::ir::PrimitiveType::Usize,
            ) => "int",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64) => "float",
            _ => "mixed",
        };
        // Unit-returning methods (e.g. PostProcessor::process) map to `mixed`; emit a null
        // return so the stub is callable — the registry never reads the result.
        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(
                setup,
                "    public function {php_name}({param_str}): {php_return_type} {{ return null; }}"
            );
        } else {
            let _ = writeln!(
                setup,
                "    public function {php_name}({param_str}): {php_return_type} {{ return {default_val}; }}"
            );
        }
    }

    let _ = writeln!(setup, "}};");

    // PHP test runner (PHPUnit) runs each test in the same process, so registering a
    // test backend leaks into later tests. Emit `<BindingClass>::unregister<Trait>(\"backend_name\")`
    // after the call+assertions to drain the test backend from the global registry.
    // Use static method calls instead of standalone functions (which don't exist as PHP functions,
    // only as methods on the entry-point class).
    let (teardown_block, type_imports) = if binding_class.is_empty() {
        (String::new(), Vec::new())
    } else {
        trait_bridge
            .unregister_fn
            .as_deref()
            .map(|unregister_fn| {
                let escaped = escape_php(&backend_name);
                // Convert snake_case to camelCase: unregister_document_extractor -> unregisterDocumentExtractor
                let parts: Vec<&str> = unregister_fn.split('_').collect();
                let mut method_name = String::new();
                for (i, part) in parts.iter().enumerate() {
                    if i == 0 {
                        // "unregister" stays lowercase
                        method_name.push_str(part);
                    } else if let Some(first) = part.chars().next() {
                        // Capitalize each subsequent word
                        method_name.push_str(&first.to_uppercase().to_string());
                        method_name.push_str(&part[1..]);
                    }
                }
                let teardown = format!("        {binding_class}::{method_name}(\"{escaped}\");\n");
                (teardown, vec![])
            })
            .unwrap_or_else(|| (String::new(), Vec::new()))
    };

    TestBackendEmission {
        setup_block: setup,
        arg_expr: "$stub".to_string(),
        type_imports,
        teardown_block,
    }
}
