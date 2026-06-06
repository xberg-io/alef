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

        // Default value: use String for marshalled types, otherwise use defaults.emit_default.
        let default_val = match &method.return_type {
            crate::core::ir::TypeRef::Named(_) => "\"\"".to_string(),
            _ => defaults.emit_default(&method.return_type),
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
    // The adapter class emitted by alef-backend-swift uses a fixed name derived from the trait.
    // Pattern: `try? <Module>.unregister<Trait>("<adapter-name>")`
    let unregister_fn = format!("unregister{}", trait_bridge.trait_name.to_upper_camel_case());
    let adapter_name = format!("swift-bridge-{}", trait_bridge.trait_name.to_snake_case());
    // Emit without module qualification: caller will add it when needed.
    let teardown = format!("try? {unregister_fn}(\"{adapter_name}\")");

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

        let emission = emit_test_backend(&bridge, &methods, &fixture);

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
        }
    }

    /// Verify params use concrete Swift types (not `Any`) and named return types marshal as JSON strings.
    #[test]
    fn swift_stub_uses_typed_params_and_marshaled_named_return() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method_with_params("processImage", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture);
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
    }

    /// Verify that `fixture.input["name"]` is used as the plugin name when present.
    #[test]
    fn swift_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("do_work", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "name": "my-backend-name" });

        let emission = emit_test_backend(&bridge, &methods, &fixture);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.name, got:\n{output}"
        );
    }
}
