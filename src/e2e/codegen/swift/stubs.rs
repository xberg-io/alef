//! Swift e2e test-backend stub emission.

use crate::e2e::codegen::TestBackendEmission;

/// Emit a Swift test backend stub class for a trait bridge.
///
/// Generates a class conforming to `Swift{TraitName}Bridge`. Required methods
/// are overridden with Swift-idiomatic defaults. Async methods use `async throws`
/// and return the default value directly. The `name` computed property is emitted
/// when a Plugin super-trait is configured.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    enums: &[crate::core::ir::EnumDef],
) -> TestBackendEmission {
    use crate::backends::swift::type_map::SwiftMapper;
    use crate::codegen::defaults::language_defaults;
    use crate::codegen::type_mapper::TypeMapper as _;
    use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Use the canonical naming helper so this stays in sync with the production
    // codegen in `src/backends/swift/gen_bindings/trait_bridge.rs`.
    let protocol_name = crate::backends::swift::naming::bridge_protocol_name(&trait_bridge.trait_name);

    // Prefer the fixture's input "name" field (e.g. "test-extractor") over the
    // fixture id, which is an internal snake_case identifier, not a backend name.
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    // Extract the backend input block if present (e.g., fixture.input.backend).
    // Used to populate method defaults like dimensions(), backend name, etc.
    let backend_input = fixture.input.get("backend").and_then(|v| v.as_object());

    let defaults = language_defaults("swift");
    let mapper = SwiftMapper;

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name}: {protocol_name} {{");

    // Plugin super-trait conformance: emit all SwiftPluginBridge required methods
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "    var name: String {{ \"{plugin_name}\" }}");
        let _ = writeln!(setup, "    func version() -> String {{ \"1.0.0\" }}");
        let _ = writeln!(setup, "    func initialize() throws {{}}");
        let _ = writeln!(setup, "    func shutdown() throws {{}}");
    }

    // Required methods: trait bridge protocols marshal excluded types as JSON strings.
    // Use concrete Swift types, converting Named types to String (JSON marshalling).
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        let method_name = method.name.to_lower_camel_case();

        // Build parameter list. Named types (excluded/internal) are marshalled as String in trait bridges.
        let params: Vec<String> = method
            .params
            .iter()
            .map(|param| {
                let param_type = match &param.ty {
                    crate::core::ir::TypeRef::Named(_) => "String".to_string(),
                    _ => mapper.map_type(&param.ty).to_string(),
                };
                format!("{}: {}", param.name.to_lower_camel_case(), param_type)
            })
            .collect();
        let params_str = params.join(", ");

        // Return type: Named types are marshalled as String (JSON).
        let return_type = match &method.return_type {
            crate::core::ir::TypeRef::Named(_) => "String".to_string(),
            _ => mapper.map_type(&method.return_type).to_string(),
        };

        // Default value: try to extract from fixture.input.backend first,
        // then fall back to language defaults. For primitives that would emit 0,
        // emit 1 instead (downstream rejects 0 for counts like dimensions()).
        let default_val = match &method.return_type {
            crate::core::ir::TypeRef::Named(name) => {
                // Check if this Named type is an enum in the IR
                if let Some(enum_def) = enums.iter().find(|e| e.name == *name) {
                    // Emit JSON-encoded first variant of the enum
                    if let Some(first_variant) = enum_def.variants.first() {
                        format!("\"\\\"{}\\\"\"", first_variant.name)
                    } else {
                        // Enum with no variants (shouldn't happen), fall back to null
                        "\"null\"".to_string()
                    }
                } else {
                    // Named type is a struct or other non-enum: emit null
                    "\"null\"".to_string()
                }
            }
            _ => {
                // Try to find the fixture value keyed by snake_case method name.
                let fixture_val = backend_input
                    .and_then(|b| b.get(&method.name.to_snake_case()))
                    .or_else(|| {
                        // Also try the lower_camel_case variant in case fixture uses that.
                        backend_input.and_then(|b| b.get(&method_name))
                    });

                if let Some(val) = fixture_val {
                    // Emit the fixture value directly (primitives, numbers, etc.)
                    match val {
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::String(s) => format!("\"{}\"", s),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => {
                            // Complex types: fall back to default, but inject 1 for 0 values
                            let def = defaults.emit_default(&method.return_type);
                            if def == "0" { "1".to_string() } else { def }
                        }
                    }
                } else {
                    // No fixture value: use language default, but emit 1 instead of 0
                    // for numeric types (downstream requires counts > 0).
                    let def = defaults.emit_default(&method.return_type);
                    if def == "0" { "1".to_string() } else { def }
                }
            }
        };

        // NOTE: Swift trait bridge methods are always sync (no async), even if the Rust trait
        // declares async. The adapter/bridge layer handles async-to-sync conversion.
        if method.error_type.is_some() {
            let _ = writeln!(
                setup,
                "    func {method_name}({params_str}) throws -> {return_type} {{ {default_val} }}"
            );
        } else {
            let _ = writeln!(
                setup,
                "    func {method_name}({params_str}) -> {return_type} {{ {default_val} }}"
            );
        }
    }

    let _ = writeln!(setup, "}}");

    // Emit teardown: unregister call to prevent test backends from leaking into subsequent tests.
    // Use the actual plugin_name that the stub declares via its `name` property, not a
    // generic adapter name. This ensures the unregister matches what was registered.
    let unregister_fn = format!("unregister{}", trait_bridge.trait_name.to_upper_camel_case());
    // Emit without module qualification: caller will add it when needed.
    let teardown = format!("try? {unregister_fn}(\"{plugin_name}\")");

    TestBackendEmission {
        setup_block: setup,
        arg_expr: format!("{class_name}()"),
        type_imports: Vec::new(),
        teardown_block: teardown,
    }
}

#[cfg(test)]
mod tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    /// Verify that no sample-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn swift_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("do_work", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "class name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("SwiftTestTraitBridge"),
            "class must conform to the Swift protocol derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("doWork"),
            "required method must be emitted in camelCase, got:\n{output}"
        );
    }

    fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
        crate::core::ir::ParamDef {
            name: name.to_string(),
            ty,
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
        }
    }

    fn make_method_with_params(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![
                make_param("image_bytes", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            return_type: TypeRef::Named("ProcessingResult".to_string()),
            is_async: true,
            is_static: false,
            error_type: Some("anyhow::Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// Verify params use concrete Swift types (not `Any`) and named return types marshal as JSON strings.
    #[test]
    fn swift_stub_uses_typed_params_and_marshaled_named_return() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method_with_params("processImage", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains(": Any"),
            "param type must not be `Any`, got:\n{output}"
        );
        assert!(
            output.contains("imageBytes: Data"),
            "bytes param must map to Data, got:\n{output}"
        );
        assert!(
            output.contains("mimeType: String"),
            "string param must map to String, got:\n{output}"
        );
        assert!(
            output.contains("-> String"),
            "named return type must marshal as String, got:\n{output}"
        );
        assert!(
            output.contains("\"null\""),
            "named return type default must be JSON-valid (\\\"null\\\"), got:\n{output}"
        );
    }

    /// Verify that `fixture.input["name"]` is used as the plugin name when present.
    #[test]
    fn swift_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("do_work", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "name": "my-backend-name" });

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.name, got:\n{output}"
        );
    }
}
