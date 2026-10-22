//! R trait backend stubs for e2e tests.

use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::escape::sanitize_ident;
use std::fmt::Write as FmtWrite;

/// Emit an R test backend stub.
///
/// Generates an R named-list object that satisfies the Rust extendr bridge
/// validation for the given trait.  The list contains one entry per required
/// method (those without `has_default_impl`) as anonymous R functions, plus a
/// `name` string entry for the Plugin super-trait when
/// `trait_bridge.super_trait.is_some()`.
///
/// Rules:
/// - Variable name: `r_backend_{sanitized_fixture_id}`.
/// - `name` key is a plain string (`"test"`), not a function — the Rust bridge
///   reads it as `r_obj.dollar("name")` expecting a character vector.
/// - Each required method key is the Rust snake_case method name.
/// - Return defaults come from `RDefaults`.
/// - The registration call uses `{register_fn}(r_backend_{id})`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::escape_r;

    let defaults = language_defaults("r");
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let var_name = format!("r_backend_{}", sanitize_ident(&fixture.id));

    let mut setup = String::new();

    let _ = writeln!(setup, "  {var_name} <- list(");

    // Collect required methods (those without default implementations).
    let required: Vec<_> = methods.iter().filter(|m| !m.has_default_impl).collect();

    // Plugin super-trait: emit `name`, `initialize`, and `shutdown` entries.
    // The R extendr trait bridge unconditionally calls `initialize` and
    // `shutdown` on every registered plugin (mirroring the python/ruby
    // bridges), so the R `list` stub must define them or registration
    // fails with `Plugin '<name>' missing method 'initialize'`.
    let super_trait_entries: Vec<String> = if trait_bridge.super_trait.is_some() {
        let escaped_name = escape_r(&backend_name);
        vec![
            format!("    name = \"{escaped_name}\""),
            "    initialize = function() invisible(NULL)".to_string(),
            "    shutdown = function() invisible(NULL)".to_string(),
        ]
    } else {
        vec![]
    };

    let total_entries = super_trait_entries.len() + required.len();
    let mut emitted = 0usize;

    for entry in &super_trait_entries {
        emitted += 1;
        let trailing = if emitted < total_entries { "," } else { "" };
        let _ = writeln!(setup, "{entry}{trailing}");
    }

    for method in required.iter() {
        let method_name = &method.name;

        // Try to extract method return value from fixture input, fall back to default.
        let method_val = if let Some(backend_obj) = fixture.input.get("backend") {
            if let Some(val) = backend_obj.get(method_name) {
                match val {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => format!("\"{}\"", escape_r(s)),
                    serde_json::Value::Bool(b) => {
                        if *b {
                            "TRUE".to_string()
                        } else {
                            "FALSE".to_string()
                        }
                    }
                    serde_json::Value::Array(_) => "c()".to_string(), // empty vector fallback
                    serde_json::Value::Null | serde_json::Value::Object(_) => {
                        defaults.emit_default(&method.return_type)
                    }
                }
            } else {
                defaults.emit_default(&method.return_type)
            }
        } else {
            defaults.emit_default(&method.return_type)
        };

        // Build parameter list: skip `&self` (no receiver in R).
        let params: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
        let param_list = params.join(", ");

        emitted += 1;
        let trailing = if emitted < total_entries { "," } else { "" };
        let _ = writeln!(
            setup,
            "    {method_name} = function({param_list}) {method_val}{trailing}"
        );
    }

    let _ = writeln!(setup, "  )");

    // R test runner (testthat) runs each test in the same process, so registering a
    // test backend leaks into later tests. Emit `unregister_<trait>("backend_name")`
    // after the call+assertions to drain the test backend from the global registry.
    let teardown_block = trait_bridge
        .unregister_fn
        .as_deref()
        .map(|unregister_fn| {
            let escaped = escape_r(&backend_name);
            format!("  {unregister_fn}(\"{escaped}\")\n")
        })
        .unwrap_or_default();

    // The arg_expr is just the variable name — the outer call (the fixture's
    // configured function) supplies the registration wrapper.  The setup_block
    // containing the list definition must be emitted before the call site.
    TestBackendEmission {
        setup_block: setup,
        arg_expr: var_name,
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Extract a backend name string from the fixture input JSON.
///
/// Searches the top-level input object for the first string value at any depth
/// under keys commonly used for names (`name`, or the first string field found).
/// Falls back to the fixture id when no string is found.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    // Walk the top-level object, then one level deeper, looking for "name".
    if let Some(obj) = input.as_object() {
        // Direct "name" key.
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object()
                && let Some(s) = inner.get("name").and_then(|v| v.as_str())
            {
                return s.to_string();
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

#[cfg(test)]
mod tests {
    /// Verify `emit_test_backend` is generic: output must not contain any
    /// hardcoded domain trait or method names — only names derived from the
    /// synthetic `TestTrait` / `do_work` inputs.
    #[test]
    fn test_emit_test_backend_is_generic_no_domain_names() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
        use crate::e2e::fixture::Fixture;

        let method = MethodDef {
            name: "do_work".to_string(),
            params: vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "my_fixture".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };

        let methods = vec![&method];
        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        // The setup_block must contain the R method key name.
        assert!(
            emission.setup_block.contains("do_work"),
            "setup_block should contain the method 'do_work', got:\n{}",
            emission.setup_block
        );
        // The arg_expr is just the variable name — the outer fixture call handles registration.
        assert!(
            emission.arg_expr.contains("r_backend_"),
            "arg_expr should be the variable name (r_backend_*), got:\n{}",
            emission.arg_expr
        );
        // The super-trait name entry must be present and derived from the fixture id
        // (not a hardcoded backend name).
        assert!(
            emission.setup_block.contains("name = \"my_fixture\""),
            "setup_block should contain fixture-derived name = \"my_fixture\" for super-trait, got:\n{}",
            emission.setup_block
        );
        // The R extendr trait bridge unconditionally calls `initialize` and
        // `shutdown` on every registered plugin, so the stub must emit them
        // alongside `name` when `super_trait` is set.
        assert!(
            emission.setup_block.contains("initialize = function()"),
            "setup_block should contain initialize = function() for super-trait, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("shutdown = function()"),
            "setup_block should contain shutdown = function() for super-trait, got:\n{}",
            emission.setup_block
        );

        // Must not contain any hardcoded domain-specific names.
        for name in &[
            "ImageBackend",
            "RecordProvider",
            "process_image",
            "extract_bytes",
            "sample_lib",
        ] {
            assert!(
                !emission.setup_block.contains(name),
                "setup_block must not contain domain name '{name}', got:\n{}",
                emission.setup_block
            );
        }
    }
}
