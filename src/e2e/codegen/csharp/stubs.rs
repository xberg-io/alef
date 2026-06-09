//! C# e2e test-backend stub emission.

use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::escape::sanitize_ident;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

/// Map an IR `TypeRef` to a C# type string for stub method signatures.
///
/// Used only by `emit_test_backend` — not the full production type-map used by
/// the C# backend generator.  Keeps stub generation self-contained and avoids
/// a dependency on the private `backends::csharp::type_map` module.
pub(super) fn csharp_type_for_stub(ty: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "byte".to_string(),
            PrimitiveType::U16 => "ushort".to_string(),
            PrimitiveType::U32 => "uint".to_string(),
            PrimitiveType::U64 => "ulong".to_string(),
            PrimitiveType::I8 => "sbyte".to_string(),
            PrimitiveType::I16 => "short".to_string(),
            PrimitiveType::I32 => "int".to_string(),
            PrimitiveType::I64 => "long".to_string(),
            PrimitiveType::F32 => "float".to_string(),
            PrimitiveType::F64 => "double".to_string(),
            PrimitiveType::Usize => "ulong".to_string(), // usize maps to ulong in C# (not long!)
            PrimitiveType::Isize => "long".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
        TypeRef::Bytes => "byte[]".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => format!("{}?", csharp_type_for_stub(inner)),
        TypeRef::Vec(inner) => format!("List<{}>", csharp_type_for_stub(inner)),
        TypeRef::Map(k, v) => format!("Dictionary<{}, {}>", csharp_type_for_stub(k), csharp_type_for_stub(v)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Json => "object".to_string(),
        TypeRef::Duration => "ulong?".to_string(),
    }
}

fn csharp_type_for_stub_visible(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => {
            if excluded_types.contains(name.as_str()) {
                "string".to_string()
            } else {
                name.clone()
            }
        }
        TypeRef::Optional(inner) => {
            let inner_str = csharp_type_for_stub_visible(inner, excluded_types);
            format!("{}?", inner_str)
        }
        TypeRef::Vec(inner) => {
            let inner_str = csharp_type_for_stub_visible(inner, excluded_types);
            format!("List<{}>", inner_str)
        }
        TypeRef::Map(k, v) => {
            let key_str = csharp_type_for_stub_visible(k, excluded_types);
            let val_str = csharp_type_for_stub_visible(v, excluded_types);
            format!("Dictionary<{}, {}>", key_str, val_str)
        }
        _ => csharp_type_for_stub(ty),
    }
}

/// Emit the correct default value for a C# test stub return type.
/// When the original type is non-visible (e.g., HiddenRecord), it's mapped to `string`,
/// so we need to return the appropriate default for the visible type, not the original.
fn emit_csharp_stub_default(
    original_type: &crate::core::ir::TypeRef,
    visible_type: &str,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    excluded_types: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;

    // Check if this type or its inner types are non-visible
    fn contains_non_visible(ty: &TypeRef, excluded_types: &std::collections::HashSet<&str>) -> bool {
        match ty {
            TypeRef::Named(name) => excluded_types.contains(name.as_str()),
            TypeRef::Optional(inner) => contains_non_visible(inner, excluded_types),
            TypeRef::Vec(inner) => contains_non_visible(inner, excluded_types),
            TypeRef::Map(k, v) => contains_non_visible(k, excluded_types) || contains_non_visible(v, excluded_types),
            _ => false,
        }
    }

    if contains_non_visible(original_type, excluded_types) {
        // Type contains non-visible parts, map to string default
        if visible_type.contains("?") {
            "null".to_string()
        } else {
            "\"\"".to_string()
        }
    } else if matches!(original_type, TypeRef::Named(_)) {
        format!("default({visible_type})")
    } else {
        // Visible type, use the default logic
        defaults.emit_default(original_type)
    }
}

/// Emit a single C# stub method body into `out`.
///
/// Used by both the main method loop and the super-trait method section of
/// `emit_test_backend` so both paths share the same formatting logic.
/// `method_cs` is the already-PascalCased method name (caller's responsibility).
fn emit_csharp_stub_method(
    out: &mut String,
    method_cs: &str,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    excluded_types: &std::collections::HashSet<&str>,
) {
    use crate::core::ir::TypeRef;

    // C# trait bridge interfaces expose synchronous methods even though Rust traits are async.
    // The bridge implementation blocks on the async Rust call. So stubs must always be sync
    // (never emit `async Task<T>`). Always use the actual return type.
    let ret_ty = csharp_type_for_stub_visible(&method.return_type, excluded_types);
    // Use the visible type to determine the default value, not the original type
    // (e.g., HiddenRecord → string → "")
    // Special case: methods with validation requirements (e.g., Dimensions must be > 0)
    // use a sensible default instead of the language-wide default.
    let default_val = if method.params.is_empty()
        && matches!(
            method.return_type,
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize | crate::core::ir::PrimitiveType::U64)
        ) {
        // For zero-parameter methods returning usize/u64 (properties), check for known
        // properties that have validation requirements.
        match method.name.to_lowercase().as_str() {
            "dimensions" | "embedding_dimensions" | "model_dimensions" => "1".to_string(),
            _ => emit_csharp_stub_default(&method.return_type, &ret_ty, defaults, excluded_types),
        }
    } else {
        emit_csharp_stub_default(&method.return_type, &ret_ty, defaults, excluded_types)
    };

    // Build parameter list using visible types (internal types like HiddenRecord
    // are mapped to string to avoid stub referencing non-public types).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            format!(
                "{} {}",
                csharp_type_for_stub_visible(&p.ty, excluded_types),
                p.name.to_lower_camel_case()
            )
        })
        .collect();
    let param_list = params.join(", ");

    // 8-space indent for method declarations (class body level); the caller's
    // class declaration is at 4-space, and the emitter adds 4 more — giving 8+4=12
    // for methods and 4+4=8 for the class line in the final file.
    // ALWAYS emit sync stubs, regardless of is_async in the Rust trait.
    if matches!(method.return_type, TypeRef::Unit) {
        let _ = writeln!(out, "        public void {method_cs}({param_list}) {{ }}");
    } else if method.params.is_empty() {
        // Zero-parameter methods with non-void return become properties in C#
        let _ = writeln!(out, "        public {ret_ty} {method_cs} {{ get; }} = {default_val};");
    } else {
        let _ = writeln!(out, "        public {ret_ty} {method_cs}({param_list})");
        let _ = writeln!(out, "            => {default_val};");
    }
}

/// Emit a C# test backend stub.
///
/// Generates a nested private class implementing the bridge interface
/// (`I{TraitName}`) with minimal stub methods, then returns a
/// `{TraitName}Bridge.Register(new TestStub_{fixture_id}())` expression
/// as the registration call site.
///
/// Rules:
/// - The stub class name is `TestStub_{sanitized_fixture_id}` where the id
///   has been converted to PascalCase (safe C# identifier).
/// - Super-trait properties (Name, Version) are emitted first with literal values;
///   then lifecycle methods (Initialize, Shutdown) are emitted with default bodies.
/// - Required methods are emitted with return-type defaults produced by `CSharpDefaults`.
/// - Async methods return `Task<T>` and are `async`; sync methods are plain.
/// - Type names come from `csharp_type_for_stub()` — no crate-domain names
///   are hardcoded here. Non-visible types
///   are NOT referenced in test stubs.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    emit_test_backend_with_class_name(
        trait_bridge,
        methods,
        fixture,
        "GeneratedBinding",
        &std::collections::HashSet::new(),
    )
}

pub(super) fn emit_test_backend_with_class_name(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    class_name: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;

    let defaults = language_defaults("csharp");

    // Derive a safe C# class identifier from the fixture id.
    let stub_class = format!("TestStub_{}", sanitize_ident(&fixture.id).to_upper_camel_case());

    // Interface name: I{TraitName} following C# convention.
    let trait_pascal = trait_bridge.trait_name.to_upper_camel_case();
    let iface_name = format!("I{trait_pascal}");

    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let mut setup = String::new();

    // Emit a private nested class declaration. This block will be placed at class scope
    // (not inside any method body) by the caller — the emitter adds 4 more spaces of
    // indentation, so each line here carries a 4-space prefix matching the visitor pattern.
    let _ = writeln!(setup, "    private class {stub_class} : {iface_name}");
    let _ = writeln!(setup, "    {{");

    // Track which super-trait methods we've already emitted to avoid duplication.
    let mut emitted_methods = std::collections::HashSet::new();

    // Super-trait properties and methods: when super_trait is configured, emit
    // the required Name and Version properties, then emit lifecycle methods
    // (initialize, shutdown) and domain-specific methods.
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        // Emit hardcoded Name and Version properties (required by Plugin super-trait)
        let _ = writeln!(setup, "        public string Name => \"{plugin_name}\";");
        let _ = writeln!(setup, "        public string Version => \"1.0.0\";");
        let _ = writeln!(setup);
        // Mark name and version as emitted so they won't be re-emitted as methods
        emitted_methods.insert("name".to_string());
        emitted_methods.insert("version".to_string());

        // Emit super-trait methods (initialize, shutdown) and domain methods
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_cs = method.name.to_upper_camel_case();
            emit_csharp_stub_method(&mut setup, &method_cs, method, &*defaults, excluded_types);
            emitted_methods.insert(method.name.clone());
        }
    }

    // All remaining methods (including those with default implementations).
    // Skip super-trait methods already emitted above.
    for method in methods.iter() {
        // Skip methods already emitted.
        if emitted_methods.contains(&method.name) {
            continue;
        }
        let method_cs = method.name.to_upper_camel_case();
        emit_csharp_stub_method(&mut setup, &method_cs, method, &*defaults, excluded_types);
    }

    let _ = writeln!(setup, "    }}");

    // Registration expression.
    // Always use the high-level `Bridge.Register(impl)` factory — it handles
    // FFI registration internally. The low-level `Bridge.RegisterXxx(impl)`
    // overloads (derived from reg_fn name) return IntPtr and are not the public API.
    let arg_expr = format!("{}Bridge.Register(new {}())", trait_pascal, stub_class);

    // Teardown: each trait-bridge registration leaks into the host registry and
    // pollutes subsequent tests in the same xUnit test run. Emit a cleanup unregister
    // keyed by the stub's Name property — same value we wrote into the stub above.
    let escaped_plugin_name = plugin_name.replace('\\', "\\\\").replace('"', "\\\"");
    let teardown_block = format!("{class_name}.Unregister{trait_pascal}(\"{escaped_plugin_name}\");");

    TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block,
    }
}
