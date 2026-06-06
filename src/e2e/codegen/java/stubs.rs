use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::e2e::codegen::TestBackendEmission;

pub(super) fn java_type_fqn(ty: &crate::core::ir::TypeRef) -> String {
    use crate::backends::java::type_map::java_type;
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(_) => "Object".to_string(),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "Object".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "java.util.List<Object>".to_string(),
        TypeRef::Vec(_) => {
            // Use JavaBoxedMapper to get boxed inner types, then qualify List
            format!("java.util.{}", java_type(ty).into_owned())
        }
        TypeRef::Map(_, _) => {
            // Use JavaBoxedMapper to get boxed inner types, then qualify Map
            format!("java.util.{}", java_type(ty).into_owned())
        }
        _ => {
            let t = java_type(ty).into_owned();
            match t.as_str() {
                "List" | "ArrayList" => format!("java.util.{}", t),
                "Map" | "HashMap" => format!("java.util.{}", t),
                _ => t,
            }
        }
    }
}

/// Map a TypeRef to its Java stub type with fully-qualified names.
///
/// Named types are qualified with `binding_pkg` (e.g. `dev.example`) which is the
/// actual Java package of the binding, matching what the Panama FFM interface declares.
/// Pass `""` to fall back to unqualified simple names (used by the generic dispatch path).
pub(super) fn java_stub_type_fqn(ty: &crate::core::ir::TypeRef, binding_pkg: &str) -> String {
    use crate::core::ir::TypeRef;
    let pkg_prefix = if binding_pkg.is_empty() {
        String::new()
    } else {
        format!("{binding_pkg}.")
    };
    match ty {
        TypeRef::Named(name) => {
            // Qualify all named types with the binding package so the generated stub
            // compiles against the actual interface in the binding jar/module.
            format!("{pkg_prefix}{name}")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => format!("{pkg_prefix}{name}"),
            other => java_stub_type_fqn(other, binding_pkg),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) => format!("java.util.List<{pkg_prefix}{name}>"),
            other => format!("java.util.List<{}>", java_stub_type_fqn(other, binding_pkg)),
        },
        TypeRef::Map(k, v) => {
            let key_type = java_stub_type_fqn(k, binding_pkg);
            let val_type = java_stub_type_fqn(v, binding_pkg);
            format!("java.util.Map<{}, {}>", key_type, val_type)
        }
        _ => java_type_fqn(ty),
    }
}

/// Map a TypeRef to its Java stub type with excluded-types context.
///
/// When a Named type is in `excluded_types`, it is substituted with `String`
/// (matching the trait-bridge interface which serializes excluded types to JSON strings).
/// Otherwise behaves like `java_stub_type_fqn`.
/// Box a Java type for use in generic parameters (List<T>, Map<K,V>).
/// Primitive types like `float` become `Float`, but already-boxed and complex types pass through.
pub(super) fn box_java_type_for_generic(ty: &str) -> String {
    match ty {
        "boolean" => "Boolean".to_string(),
        "byte" => "Byte".to_string(),
        "short" => "Short".to_string(),
        "int" => "Integer".to_string(),
        "long" => "Long".to_string(),
        "float" => "Float".to_string(),
        "double" => "Double".to_string(),
        "char" => "Character".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn java_stub_type_with_context(
    ty: &crate::core::ir::TypeRef,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            "String".to_string()
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
                "String".to_string()
            }
            other => java_stub_type_with_context(other, binding_pkg, excluded_types),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
                "java.util.List<String>".to_string()
            }
            other => {
                let inner_type = java_stub_type_with_context(other, binding_pkg, excluded_types);
                // Box primitives for use in generic parameters
                let boxed_inner = box_java_type_for_generic(&inner_type);
                format!("java.util.List<{boxed_inner}>")
            }
        },
        TypeRef::Map(k, v) => {
            let key_type = java_stub_type_with_context(k, binding_pkg, excluded_types);
            let val_type = java_stub_type_with_context(v, binding_pkg, excluded_types);
            // Box primitives for use in generic parameters
            let boxed_key = box_java_type_for_generic(&key_type);
            let boxed_val = box_java_type_for_generic(&val_type);
            format!("java.util.Map<{}, {}>", boxed_key, boxed_val)
        }
        _ => java_stub_type_fqn(ty, binding_pkg),
    }
}

/// Boxed version of java_stub_type_with_context for use as a CompletableFuture generic parameter.
#[allow(dead_code)]
pub(super) fn java_boxed_stub_type_with_context(
    ty: &crate::core::ir::TypeRef,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Unit => "Void".to_string(),
        _ => {
            let t = java_stub_type_with_context(ty, binding_pkg, excluded_types);
            // Box primitives for use as generic type parameters.
            match t.as_str() {
                "boolean" => "Boolean".to_string(),
                "byte" => "Byte".to_string(),
                "short" => "Short".to_string(),
                "int" => "Integer".to_string(),
                "long" => "Long".to_string(),
                "float" => "Float".to_string(),
                "double" => "Double".to_string(),
                "byte[]" => "byte[]".to_string(), // byte[] stays as-is (already boxed in Java)
                _ => t,
            }
        }
    }
}

/// Return the default value for a type, substituting excluded types with `""`.
pub(super) fn java_stub_default_with_context(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) -> String {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            "\"\"".to_string()
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !excluded_types.is_empty() && excluded_types.contains(n.as_str())) => {
            "\"\"".to_string()
        }
        // For Named types that are NOT excluded, return null instead of trying to instantiate.
        // Complex types like ProcessingResult don't have no-arg constructors, and stub
        // methods are only used for testing trait bridge registration, not for exercising
        // the actual functionality. Returning null is safe here.
        TypeRef::Named(_) => "null".to_string(),
        _ => defaults.emit_default(ty),
    }
}

/// Emit a single Java stub method with excluded-types context.
///
/// Like `emit_java_stub_method` but with excluded_types substitution.
/// Excluded types are rendered as `String` in signatures and default to `""`.
pub(super) fn emit_java_stub_method_with_context(
    out: &mut String,
    method_java: &str,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
) {
    use std::fmt::Write as _;

    let ret_java = java_stub_type_with_context(&method.return_type, binding_pkg, excluded_types);
    let default_val = java_stub_default_with_context(&method.return_type, excluded_types, defaults);

    // Use java_stub_type_with_context for all parameter types to handle excluded types
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            format!(
                "{} {}",
                java_stub_type_with_context(&p.ty, binding_pkg, excluded_types),
                p.name.to_lower_camel_case()
            )
        })
        .collect();
    let params_str = params.join(", ");

    let _ = writeln!(out, "    @Override");
    // E2e test stubs must match the trait bridge interface signatures exactly.
    // The interface declares sync methods (not wrapped in CompletableFuture),
    // even if the Rust trait method is async. The trait bridge handles async
    // internally; test stubs just implement the interface signature.
    if ret_java == "void" {
        let _ = writeln!(out, "    public void {method_java}({params_str}) {{}}");
    } else {
        let _ = writeln!(out, "    public {ret_java} {method_java}({params_str}) {{");
        let _ = writeln!(out, "        return {default_val};");
        let _ = writeln!(out, "    }}");
    }
}

/// Emit a Java test backend stub class for a trait bridge.
///
/// Generates a class implementing `I{TraitName}` (the Panama FFM interface). Required
/// methods are overridden with `CompletableFuture.completedFuture(default)` for async
/// signatures or the direct default value for sync. The `name()` method is emitted when
/// a Plugin super-trait is configured.
///
/// `binding_pkg` is the Java package of the binding (e.g. `dev.example`). It is used
/// to fully-qualify named types in method signatures and the interface name. Pass `""`
/// when calling from the generic dispatch path (types will be unqualified).
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    binding_pkg: &str,
) -> TestBackendEmission {
    emit_test_backend_with_context(trait_bridge, methods, fixture, binding_pkg, &Default::default(), "")
}

/// Like `emit_test_backend` but with excluded_types context.
///
/// Excluded types are substituted with `String` in method signatures and default to `""`.
/// This matches how the trait-bridge interface serializes binding-excluded types to JSON strings.
///
/// `binding_class` is the unqualified class name used for static teardown calls
/// (e.g. `unregister_<trait>`). When empty, teardown is omitted.
pub(super) fn emit_test_backend_with_context(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    binding_pkg: &str,
    excluded_types: &std::collections::HashSet<&str>,
    binding_class: &str,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::escape_java;
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Java interface follows the I{TraitName} convention from the Panama FFM bridge.
    // Use fully-qualified name to avoid "cannot find symbol" errors in test compilation.
    let interface_name = if binding_pkg.is_empty() {
        format!("I{}", trait_bridge.trait_name)
    } else {
        format!("{binding_pkg}.I{}", trait_bridge.trait_name)
    };

    let plugin_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let backend_name = plugin_name.clone();

    let defaults = language_defaults("java");

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name} implements {interface_name} {{");

    // Super-trait methods — driven from IR, no names hardcoded.
    // The `name` method returns the fixture's plugin name; all others use defaults.
    // Method names must match the interface exactly (snake_case).
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_java = &method.name; // Keep snake_case to match interface
            if method.name == "name" {
                let _ = writeln!(setup, "    @Override");
                let _ = writeln!(
                    setup,
                    "    public String {method_java}() {{ return \"{plugin_name}\"; }}"
                );
            } else {
                emit_java_stub_method_with_context(
                    &mut setup,
                    method_java,
                    method,
                    &*defaults,
                    binding_pkg,
                    excluded_types,
                );
            }
        }
    }

    // All non-super-trait methods (including those with default impls).
    // Java interfaces require all abstract methods to be implemented, even if
    // Rust traits provide default implementations.
    // Method names must match the interface exactly (snake_case).
    for method in methods {
        // Skip super-trait methods already emitted above.
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        let method_java = &method.name; // Keep snake_case to match interface
        if method.name == "name" {
            let _ = writeln!(setup, "    @Override");
            let _ = writeln!(
                setup,
                "    public String {method_java}() {{ return \"{plugin_name}\"; }}"
            );
        } else {
            emit_java_stub_method_with_context(
                &mut setup,
                method_java,
                method,
                &*defaults,
                binding_pkg,
                excluded_types,
            );
        }
    }

    let _ = writeln!(setup, "}}");

    // Java test runner (JUnit) runs each test in the same process, so registering a
    // test backend leaks into later tests. Emit `<BindingClass>.unregister_<trait>("backend_name")`
    // after the call+assertions to drain the test backend from the global registry.
    let teardown_block = if binding_class.is_empty() {
        String::new()
    } else {
        trait_bridge
            .unregister_fn
            .as_deref()
            .map(|unregister_fn| {
                let escaped = escape_java(&backend_name);
                let camel_case_fn = unregister_fn.to_lower_camel_case();
                format!("        {binding_class}.{camel_case_fn}(\"{escaped}\");\n")
            })
            .unwrap_or_default()
    };

    TestBackendEmission {
        setup_block: setup,
        arg_expr: format!("new {class_name}()"),
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Extract a backend name string from the fixture input JSON.
///
/// Searches the top-level input object for the first string value at any depth
/// under keys commonly used for names (`name`, or the first string field found).
/// Falls back to the fixture id when no string is found.
pub(super) fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    // Walk the top-level object, then one level deeper, looking for "name".
    if let Some(obj) = input.as_object() {
        // Direct "name" key.
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        // One level deeper in any nested object.
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        // First string value at the top level.
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
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
    fn java_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process_item", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        // With empty binding_pkg (generic dispatch path): interface is unqualified.
        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

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
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded 'dev.sample_crate', got:\n{output}"
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
            output.contains("implements ITestTrait"),
            "class must implement interface with binding_pkg prefix, got:\n{output}"
        );
        assert!(
            output.contains("process_item"),
            "required method must be emitted in snake_case to match interface, got:\n{output}"
        );
    }

    /// Verify that when `binding_pkg` is provided (e.g. `dev.example`), the interface
    /// name and named types in method signatures are fully-qualified with that package.
    #[test]
    fn java_stub_uses_binding_pkg_for_interface_and_type_qualification() {
        let bridge = make_trait_bridge("DocumentExtractor");
        let method = MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![],
            return_type: TypeRef::Named("OperationOutput".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: Some("DocumentExtractor".to_string()),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let methods = [&method];
        let fixture = make_fixture("extract_bytes_test");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "dev.example");
        let output = &emission.setup_block;

        // Interface must be qualified with the binding package.
        assert!(
            output.contains("implements dev.example.IDocumentExtractor"),
            "class must implement dev.example.IDocumentExtractor, got:\n{output}"
        );
        // Named type must be qualified with the binding package.
        assert!(
            output.contains("dev.example.OperationOutput"),
            "return type must use dev.example.OperationOutput, got:\n{output}"
        );
        // Must NOT contain old hardcoded dev.sample_crate.
        assert!(
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded dev.sample_crate, got:\n{output}"
        );
    }

    /// Test that plugin name is correctly extracted from nested input object.
    #[test]
    fn java_stub_plugin_name_extracted_from_input_name_field() {
        let bridge = make_trait_bridge("DocumentExtractor");
        let mut name_method = make_method("name", true);
        name_method.trait_source = Some("Plugin".to_string());
        let methods = [&name_method];
        let fixture = Fixture {
            id: "register_document_extractor_trait_bridge".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({
                "extractor": {
                    "type": "test",
                    "name": "test-extractor"
                }
            }),
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = &emission.setup_block;

        // The name() method must return the value from input.extractor.name
        assert!(
            output.contains("public String name() { return \"test-extractor\"; }"),
            "name() method must return extracted name 'test-extractor', got:\n{output}"
        );
    }

    /// Test that stub method signatures use fully-qualified names for domain types
    /// when the actual binding package is unknown (empty string fallback).
    #[test]
    fn java_stub_method_uses_fqn_for_domain_types_no_pkg() {
        let bridge = make_trait_bridge("DocumentExtractor");
        // Method returning a domain type
        let method = MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![],
            return_type: TypeRef::Named("OperationOutput".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: Some("DocumentExtractor".to_string()),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let methods = [&method];
        let fixture = make_fixture("extract_bytes_test");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = &emission.setup_block;

        // With empty binding_pkg, named types are unqualified.
        // Method names must use snake_case to match the interface.
        assert!(
            output.contains("public OperationOutput extract_bytes"),
            "return type must use OperationOutput (unqualified, empty pkg) with snake_case method name, got:\n{output}"
        );
        // Must NOT contain hardcoded dev.sample_crate.
        assert!(
            !output.contains("dev.sample_crate"),
            "must not contain hardcoded dev.sample_crate, got:\n{output}"
        );
    }
}
