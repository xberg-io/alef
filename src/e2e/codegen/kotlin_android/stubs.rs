/// Emit a Kotlin Android test backend stub class for a trait bridge.
///
/// Generates a class implementing `I{TraitName}`. Required methods are overridden
/// with Kotlin-idiomatic defaults. Suspend (async) methods use `suspend fun`.
/// The `name()` function is emitted when a Plugin super-trait is configured.
/// Registration uses `{TraitName}Bridge.register(stub)` (the static object pattern).
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    enums: &[crate::core::ir::EnumDef],
) -> crate::e2e::codegen::TestBackendEmission {
    use crate::backends::kotlin::type_map::KotlinMapper;
    use crate::codegen::defaults::language_defaults;
    use crate::codegen::type_mapper::TypeMapper as _;
    use heck::{ToLowerCamelCase, ToUpperCamelCase};
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Kotlin Android uses I{TraitName} as the interface.
    let interface_name = format!("I{}", trait_bridge.trait_name);
    // Use the canonical naming helper so both production and e2e emit the same bridge object name.
    let bridge_object = crate::backends::kotlin_android::naming::bridge_object_name(&trait_bridge.trait_name);

    // Prefer the fixture's input "name" field (e.g. "test-extractor") over the
    // fixture id, which is an internal snake_case identifier, not a backend name.
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("kotlin_android");
    let mapper = KotlinMapper;

    // Collect all type imports needed by method parameters and return types.
    // Exclude Kotlin built-in types and the interface itself (which is always imported).
    let mut type_imports = std::collections::HashSet::new();
    type_imports.insert(interface_name.clone());

    const KOTLIN_BUILTINS: &[&str] = &[
        "String",
        "Int",
        "Long",
        "Short",
        "Byte",
        "Boolean",
        "Char",
        "Float",
        "Double",
        "Unit",
        "Any",
        "Nothing",
        "List",
        "Map",
        "Set",
        "ByteArray",
    ];

    for method in methods {
        // Collect parameter types.
        for param in &method.params {
            if let crate::core::ir::TypeRef::Named(name) = &param.ty
                && !KOTLIN_BUILTINS.contains(&name.as_str())
            {
                type_imports.insert(name.clone());
            }
        }
        // Collect return type.
        if let crate::core::ir::TypeRef::Named(name) = &method.return_type
            && !KOTLIN_BUILTINS.contains(&name.as_str())
        {
            type_imports.insert(name.clone());
        }
    }

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name} : {interface_name} {{");

    // Plugin super-trait `name()` function.
    let mut emitted_methods = std::collections::HashSet::new();
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "    override fun name(): String = \"{plugin_name}\"");
        emitted_methods.insert("name".to_string());
    }

    // Emit all methods to ensure test stubs are concrete and non-abstract.
    // Even methods marked with has_default_impl=true must be overridden in test stubs
    // to ensure the stub class is not abstract and can be instantiated. The Kotlin
    // interface may declare abstract methods without defaults that the Rust metadata
    // incorrectly marks as having defaults.
    for method in methods {
        // Skip if already emitted (e.g., super-trait name method).
        if emitted_methods.contains(&method.name) {
            continue;
        }
        let method_name = method.name.to_lower_camel_case();

        // Build parameter list with concrete Kotlin types.
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name.to_lower_camel_case(), mapper.map_type(&p.ty)))
            .collect();
        let params_str = params.join(", ");

        let return_type = mapper.map_type(&method.return_type);

        // For Unit return types, use block syntax {} instead of assignment.
        // For other types, use expression syntax = default_val.
        let is_unit = matches!(&method.return_type, crate::core::ir::TypeRef::Unit);

        if is_unit {
            if method.is_async {
                let _ = writeln!(
                    setup,
                    "    override suspend fun {method_name}({params_str}): {return_type} {{}}"
                );
            } else {
                let _ = writeln!(
                    setup,
                    "    override fun {method_name}({params_str}): {return_type} {{}}"
                );
            }
        } else {
            // Try to extract default from fixture.input.backend first.
            let default_val = super::enum_fixtures::extract_kotlin_android_fixture_default(&method.name, fixture)
                .unwrap_or_else(|| {
                    // Fall back to language defaults.
                    if let crate::core::ir::TypeRef::Named(name) = &method.return_type {
                        kotlin_android_enum_default(name, enums)
                            .unwrap_or_else(|| defaults.emit_default(&method.return_type))
                    } else {
                        // A stub that reports "zero of everything" is indistinguishable
                        // from a broken backend, so callers that validate their inputs
                        // reject it. Use a non-degenerate default for non-boolean
                        // primitive returns instead of the type's literal zero.
                        let def = defaults.emit_default(&method.return_type);
                        if def == "0" { "1".to_string() } else { def }
                    }
                });

            if method.is_async {
                let _ = writeln!(
                    setup,
                    "    override suspend fun {method_name}({params_str}): {return_type} = {default_val}"
                );
            } else {
                let _ = writeln!(
                    setup,
                    "    override fun {method_name}({params_str}): {return_type} = {default_val}"
                );
            }
        }
        emitted_methods.insert(method.name.clone());
    }

    let _ = writeln!(setup, "}}");

    // Registration: `{TraitName}Bridge.register(stub)` — static object pattern.
    let arg_expr = format!("{class_name}()");
    // Emit a registration comment in the setup block so the caller can see the bridge object.
    let _ = writeln!(setup, "// register via: {bridge_object}.register({class_name}())");

    let mut sorted_imports: Vec<String> = type_imports.into_iter().collect();
    sorted_imports.sort();

    crate::e2e::codegen::TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: sorted_imports,
        teardown_block: String::new(),
    }
}

/// Default-value expression for a `Named` return type that is a known enum, asking the IR
/// what the real Kotlin backend would generate instead of guessing.
///
/// `emit_enum` (`backends::kotlin::gen_bindings::object_wrapper::enums`) emits two different
/// shapes depending on whether any variant carries fields: an all-unit enum becomes a plain
/// `enum class` with `SCREAMING_SNAKE_CASE` constants, while a mix of unit and data variants
/// becomes a sealed class where a unit variant is a singleton `object VariantName : Parent()`
/// referenced by its own `PascalCase` name (no parentheses — it is not a constructor call) and
/// a data variant is a `data class` that needs real field values. A bare `TypeName()` call
/// previously used here always targets the (always-private, in a plain `enum class`, or
/// always-protected, in a sealed class) base constructor directly — never valid Kotlin for
/// either shape, and exactly the `cannot access 'constructor(): ...': it is protected/private`
/// failure this fixes.
///
/// Prefers the enum's own `#[default]` variant when it carries no fields, falling back to the
/// first fieldless variant otherwise, since that is the value the real Rust bridge itself falls
/// back to on a failed/uninitialised callback. When every variant carries fields, there is no
/// compilable value this function can synthesize without field-level default data it does not
/// have; it warns and leaves the caller to fall back to the (already best-effort, possibly
/// non-compiling) language default rather than silently guessing one. ~keep
fn kotlin_android_enum_default(name: &str, enums: &[crate::core::ir::EnumDef]) -> Option<String> {
    let enum_def = enums.iter().find(|e| e.name == name)?;
    let all_unit = enum_def.variants.iter().all(|v| v.fields.is_empty());
    let chosen = enum_def
        .variants
        .iter()
        .find(|v| v.is_default && v.fields.is_empty())
        .or_else(|| enum_def.variants.iter().find(|v| v.fields.is_empty()));
    let Some(variant) = chosen else {
        tracing::warn!(
            language = "kotlin_android",
            r#type = %name,
            "trait-bridge stub: enum has no fieldless variant to use as a default"
        );
        return None;
    };
    Some(if all_unit {
        format!("{name}.{}", crate::codegen::naming::to_constant_name(&variant.name))
    } else {
        format!("{name}.{}", variant.name)
    })
}

#[cfg(test)]
mod test_backend_tests {
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
            cfg: None,
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
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
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
            preserve_input_urls: false,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    /// Verify that no sample_core-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn kotlin_android_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
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
        // The bridge object is "TestTraitBridge" not "SampleCrateBridge"
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "class name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("ITestTrait"),
            "class must implement interface derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("TestTraitBridge"),
            "setup block must reference the bridge object derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("processItem"),
            "required method must be emitted in camelCase, got:\n{output}"
        );
    }

    fn make_param(name: &str, ty: crate::core::ir::TypeRef) -> crate::core::ir::ParamDef {
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
                make_param("content", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            return_type: TypeRef::Named("ProcessingResult".to_string()),
            is_async: true,
            is_static: false,
            error_type: Some("anyhow::Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
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

    /// Verify params use concrete Kotlin types (not `Any`) and return type is concrete.
    #[test]
    fn kotlin_android_stub_uses_typed_params_not_any() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method_with_params("extractBytes", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains(": Any"),
            "param type must not be `Any`, got:\n{output}"
        );
        assert!(
            output.contains("content: ByteArray"),
            "bytes param must map to ByteArray in Kotlin, got:\n{output}"
        );
        assert!(
            output.contains("mimeType: String"),
            "string param must map to String in Kotlin, got:\n{output}"
        );
        assert!(
            output.contains("): ProcessingResult"),
            "return type must be concrete not Any, got:\n{output}"
        );
    }

    /// Verify that `fixture.input["name"]` is used as the plugin name when present.
    #[test]
    fn kotlin_android_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
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

    use crate::core::ir::{EnumDef, EnumVariant, FieldDef};

    fn make_enum_method(name: &str, return_type_name: &str) -> MethodDef {
        MethodDef {
            return_type: TypeRef::Named(return_type_name.to_string()),
            ..make_method(name, true)
        }
    }

    fn unit_variant(name: &str, is_default: bool) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: Vec::new(),
            is_default,
            ..Default::default()
        }
    }

    fn data_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: vec![FieldDef {
                name: "scale_max".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// The core regression: a sealed-class enum (one variant carries fields) must reference
    /// its default unit variant as a bare `Parent.Variant` singleton object, not call the
    /// (always-protected) base constructor `Parent()` -- "cannot access 'constructor(): ...':
    /// it is protected".
    #[test]
    fn sealed_class_enum_default_uses_object_reference_not_constructor_call() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_enum_method("sample_classification", "SampleClassification");
        let methods = [&method];
        let fixture = make_fixture("sealed_default_fixture");
        let enums = [EnumDef {
            name: "SampleClassification".to_string(),
            variants: vec![
                data_variant("Scored"),
                unit_variant("Baseline", true),
                unit_variant("Unset", false),
            ],
            ..Default::default()
        }];

        let emission = emit_test_backend(&bridge, &methods, &fixture, &enums);

        assert!(
            emission.setup_block.contains("= SampleClassification.Baseline"),
            "must reference the default unit variant as an object, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("SampleClassification()"),
            "must never call the sealed base class's constructor directly, got:\n{}",
            emission.setup_block
        );
    }

    /// An all-unit enum (a plain Kotlin `enum class`) must reference its default variant as
    /// a `SCREAMING_SNAKE_CASE` constant -- matching `emit_enum`'s own naming
    /// (`backends::kotlin::gen_bindings::object_wrapper::enums::to_screaming_snake`) -- and
    /// must never call the (always-private) enum class constructor directly either.
    #[test]
    fn all_unit_enum_default_uses_screaming_snake_constant() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_enum_method("sample_orientation", "SampleOrientation");
        let methods = [&method];
        let fixture = make_fixture("plain_enum_default_fixture");
        let enums = [EnumDef {
            name: "SampleOrientation".to_string(),
            variants: vec![
                unit_variant("AutoCorrected", false),
                unit_variant("PartiallyRotated", false),
                unit_variant("RequiresManualFix", true),
            ],
            ..Default::default()
        }];

        let emission = emit_test_backend(&bridge, &methods, &fixture, &enums);

        assert!(
            emission.setup_block.contains("= SampleOrientation.REQUIRES_MANUAL_FIX"),
            "must reference the default variant as a SCREAMING_SNAKE_CASE constant, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("SampleOrientation()"),
            "must never call the enum class's constructor directly, got:\n{}",
            emission.setup_block
        );
    }

    /// Negative control: when no variant is marked `#[default]`, the first *fieldless* variant
    /// wins -- proving the fallback does not simply take `variants[0]` (which here carries
    /// fields and would be uninstantiable without a real value for `scale_max`).
    #[test]
    fn no_default_variant_falls_back_to_first_fieldless_variant() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_enum_method("sample_classification", "SampleClassification");
        let methods = [&method];
        let fixture = make_fixture("no_default_variant_fixture");
        let enums = [EnumDef {
            name: "SampleClassification".to_string(),
            variants: vec![data_variant("Scored"), unit_variant("Baseline", false)],
            ..Default::default()
        }];

        let emission = emit_test_backend(&bridge, &methods, &fixture, &enums);

        assert!(
            emission.setup_block.contains("= SampleClassification.Baseline"),
            "must fall back to the first fieldless variant when none is marked default, got:\n{}",
            emission.setup_block
        );
    }

    /// Negative control: a `Named` return type with no matching entry in `enums` at all (e.g. a
    /// plain struct DTO) must be completely unaffected by the enum-lookup fallback.
    #[test]
    fn unknown_named_type_is_unaffected_by_enum_lookup() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_enum_method("get_config", "ProcessingResult");
        let methods = [&method];
        let fixture = make_fixture("unknown_named_fixture");

        // No panic and a method is still emitted, even with an empty enum registry.
        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

        assert!(
            emission.setup_block.contains("getConfig"),
            "method must still be emitted, got:\n{}",
            emission.setup_block
        );
    }

    fn make_method_with_return(name: &str, return_type: TypeRef) -> MethodDef {
        MethodDef {
            return_type,
            ..make_method(name, true)
        }
    }

    /// A stub method returning a non-boolean integer primitive must return a
    /// non-degenerate literal (`1`), not the type's zero — a caller that
    /// validates its inputs (e.g. rejecting a zero-valued count) would
    /// otherwise reject the stub itself.
    #[test]
    fn integer_return_is_nonzero() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_method_with_return("count", TypeRef::Primitive(PrimitiveType::Usize));
        let methods = [&method];
        let fixture = make_fixture("integer_return_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

        assert!(
            emission.setup_block.contains("count(): Long = 1"),
            "integer-returning stub method must return 1, got:\n{}",
            emission.setup_block
        );
    }

    /// A stub method returning a collection keeps today's empty-collection
    /// default — only the integer-primitive case is degenerate enough to
    /// reject a validating caller. Pins the collection behavior against a
    /// future change accidentally widening the non-degenerate-default fix.
    #[test]
    fn collection_return_stays_empty() {
        let bridge = make_trait_bridge("TestTrait");
        let method = make_method_with_return("items", TypeRef::Vec(Box::new(TypeRef::String)));
        let methods = [&method];
        let fixture = make_fixture("collection_return_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, &[]);

        assert!(
            emission.setup_block.contains("items(): List<String> = emptyList()"),
            "collection-returning stub method must stay empty, got:\n{}",
            emission.setup_block
        );
    }
}
