use super::*;
use crate::e2e::codegen::TestBackendEmission;

fn zig_type_for_stub(ty: &crate::core::ir::TypeRef, _excluded_types: &std::collections::HashSet<&str>) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "i32".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 | PrimitiveType::Usize => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 | PrimitiveType::Isize => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes => "[*c]const u8".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => {
            // In C FFI, optional values are passed as nullable pointers.
            // For Optional[String] returning from methods, use ?[*c]const u8.
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes => {
                    "?[*c]const u8".to_string()
                }
                _ => format!("?{}", zig_type_for_stub(inner, _excluded_types)),
            }
        }
        TypeRef::Vec(_inner) => {
            // All collections in trait bridge stubs are marshalled as JSON: [*c]const u8.
            // This includes Vec[String], Vec[Vec[f32]], Vec[Struct], etc.
            "[*c]const u8".to_string()
        }
        TypeRef::Map(_, _v) => "[*c]const u8".to_string(),
        // All Named types (structs, enums) map to opaque C FFI pointers.
        // The vtable thunks pass these as [*c]const u8 to user method stubs.
        TypeRef::Named(_) => "[*c]const u8".to_string(),
        TypeRef::Duration => "i64".to_string(),
    }
}

/// Emit FFI-appropriate default value for stub return type.
/// Stub types are C FFI types, so we use Zig/C appropriate literals.
fn zig_stub_default_value(stub_type: &str) -> String {
    match stub_type {
        "[*c]const u8" => "\"\"".to_string(),
        "?[*c]const u8" => "null".to_string(),
        "void" => "".to_string(),
        "i32" | "i16" | "i8" => "0".to_string(),
        "i64" => "0".to_string(),
        "u8" | "u16" | "u32" | "u64" => "0".to_string(),
        "f32" | "f64" => "0.0".to_string(),
        _ => "undefined".to_string(),
    }
}

/// Emit a stub return value for `ty`, using a non-degenerate default for
/// non-boolean integer primitives instead of the type's literal zero.
///
/// A stub that reports "zero of everything" is indistinguishable from a
/// backend that is broken, so callers that validate their inputs (e.g.
/// rejecting a non-empty declared count) reject it. Booleans, floats, and
/// every non-primitive type keep `zig_stub_default_value`'s type-based
/// default — `stub_type` alone can't distinguish an integer from a
/// same-width boolean, so this dispatches on the IR type instead.
fn zig_stub_nonzero_or_default(ty: &crate::core::ir::TypeRef, stub_type: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(
            PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::Isize
            | PrimitiveType::Usize,
        ) => "1".to_string(),
        _ => zig_stub_default_value(stub_type),
    }
}

/// Determine if a method needs JSON-encoded default values for out_result parameters.
/// This occurs for infallible (non-error) methods with complex return types that are
/// wrapped in out_result parameters at the FFI boundary.
fn method_needs_json_default(method: &crate::core::ir::MethodDef) -> bool {
    // Only infallible methods need JSON defaults
    if method.error_type.is_some() {
        return false;
    }

    // Skip Unit and primitive types
    use crate::core::ir::TypeRef;
    match &method.return_type {
        TypeRef::Unit => false,
        TypeRef::Primitive(_) => false,
        _ => true, // String, Vec, Named types, etc. need JSON encoding
    }
}

/// Generate appropriate JSON default for a method return type.
/// For complex types that are serialized to JSON, return a sensible empty/default JSON value.
fn zig_json_default_for_type(return_type: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::TypeRef;
    match return_type {
        TypeRef::Vec(_) => "\"[]\"".to_string(),    // Empty array
        TypeRef::Map(_, _) => "\"{}\"".to_string(), // Empty object
        TypeRef::String => "\"\"".to_string(),      // Empty string
        TypeRef::Named(_) => "\"{}\"".to_string(),  // Default JSON object for custom types
        _ => "\"{}\"".to_string(),                  // Fallback to empty object
    }
}

/// Deterministic name of the `out_error` pointer variable a `test_backend` arg's
/// stub setup declares in [`emit_test_backend_inner`]. Call-site codegen (the
/// register-shaped call in `test_file.rs`) needs the exact same name to read the
/// pointer back after the call, so both sides go through this one function
/// instead of duplicating the format string. ~keep
pub(super) fn test_backend_out_err_var_name(fixture_id: &str) -> String {
    let id_snake = crate::e2e::escape::sanitize_ident(&fixture_id.to_snake_case());
    format!("out_err_{id_snake}")
}

/// Emit a Zig test backend stub with excluded type handling.
///
/// Wraps `emit_test_backend_inner` with an excluded types set passed through
/// to `zig_type_for_stub` for proper type substitution in trait bridge stubs.
pub(super) fn emit_test_backend_with_excluded(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
) -> TestBackendEmission {
    emit_test_backend_inner(trait_bridge, methods, fixture, excluded_types)
}

/// Emit a Zig test backend stub.
///
/// Generates a Zig struct type for the stub, then builds a vtable via the
/// `make_{trait_snake}_vtable` helper and registers it.
///
/// Rules:
/// - Struct name: `TestStub_{sanitized_snake_fixture_id}`.
/// - Required methods (without `has_default_impl`) are stubbed with Zig
///   defaults from `ZigDefaults`.
/// - Super-trait `name` method returns the literal `"test"` string.
/// - The `register_fn` from `trait_bridge.register_fn` drives the
///   registration expression; snake_case convention for Zig.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    let excluded_types = std::collections::HashSet::new();
    emit_test_backend_inner(trait_bridge, methods, fixture, &excluded_types)
}

/// Internal implementation of test backend emission with excluded type handling.
fn emit_test_backend_inner(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::core::ir::TypeRef;

    let _defaults = language_defaults("zig");
    let id_snake = crate::e2e::escape::sanitize_ident(&fixture.id.to_snake_case());
    let struct_name = format!("TestStub_{id_snake}");
    let var_name = format!("stub_{id_snake}");
    let vtable_var = format!("vtable_{id_snake}");
    let trait_snake = trait_bridge.trait_name.to_snake_case();

    let mut setup = String::new();

    // No leading indent: caller splits by lines and adds 4 spaces per line (test body indent).
    let _ = writeln!(setup, "const {struct_name} = struct {{");

    // Use standard defaults for super-trait methods that don't return test-specific values.
    let _defaults = language_defaults("zig");

    // Plugin super-trait: `name()` returns a sentinel C-string.
    // Driven from IR — no method names are hardcoded.
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_snake = method.name.to_snake_case();
            if method.name == "name" {
                let _ = writeln!(
                    setup,
                    "    pub fn {method_snake}() ?[*:0]const u8 {{ return \"test\"; }}"
                );
            } else if method.name == "version" {
                let _ = writeln!(
                    setup,
                    "    pub fn {method_snake}() ?[*:0]const u8 {{ return \"0.0.1\"; }}"
                );
            } else {
                // Initialize/shutdown and other super-trait methods: emit a void stub.
                // Use @This() instead of struct_name to avoid self-reference inside struct definition.
                let _ = writeln!(setup, "    pub fn {method_snake}(_: *@This()) !void {{}}");
            }
        }
    }

    // Emit ALL trait methods (both required and optional with defaults).
    // The trait-bridge vtable will call all of them, so stubs must implement them all.
    for method in methods.iter() {
        // Skip super-trait methods already emitted above.
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let ret_ty = zig_type_for_stub(&method.return_type, excluded_types);

        // For infallible methods with complex return types, use JSON-encoded defaults.
        // These methods are wrapped in out_result parameters at the FFI boundary.
        let default_val = if method_needs_json_default(method) {
            zig_json_default_for_type(&method.return_type)
        } else {
            zig_stub_nonzero_or_default(&method.return_type, &ret_ty)
        };
        let _ = _defaults; // unused but imported for future use

        // Build Zig parameter list (self first using @This(), then method params).
        // Zig does not allow using a type name inside its own definition, so use @This().
        let mut params = vec!["_: *@This()".to_string()];
        for p in &method.params {
            let p_ty = zig_type_for_stub(&p.ty, excluded_types);
            params.push(format!("_: {}", p_ty)); // Mark all method params as unused with _
        }
        let param_list = params.join(", ");

        // For trait bridge methods, emit error-union returns if the method is
        // fallible in the Rust trait. This lets the vtable thunk use `if` syntax
        // to handle the error union result.
        let ret_sig = if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                "!void".to_string()
            } else {
                format!("!{}", ret_ty)
            }
        } else {
            if matches!(method.return_type, TypeRef::Unit) {
                "void".to_string()
            } else {
                ret_ty.clone()
            }
        };

        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(setup, "    pub fn {method_snake}({param_list}) {ret_sig} {{}}");
        } else {
            let _ = writeln!(
                setup,
                "    pub fn {method_snake}({param_list}) {ret_sig} {{ return {default_val}; }}"
            );
        }
    }

    let _ = writeln!(setup, "}};");
    let _ = writeln!(setup, "var {var_name} = {struct_name}{{}};");
    // lib. is a placeholder; the caller replaces it with the real module name.
    let _ = writeln!(
        setup,
        "const {vtable_var} = lib.make_{trait_snake}_vtable({struct_name}, &{var_name});"
    );

    let out_err_var = test_backend_out_err_var_name(&fixture.id);
    let _ = writeln!(setup, "var {out_err_var}: ?[*c]u8 = null;");

    // arg_expr expands into the argument list for the registration call site:
    // `<binding>.register_fn("test", vtable, &stub, @ptrCast(&out_err))`
    // The caller places arg_expr into args_str, which is used as the full argument list
    // of the top-level `{module}.{register_fn}(args_str)` call.
    let arg_expr = format!("\"test\", {vtable_var}, &{var_name}, @ptrCast(&{out_err_var})");

    TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}

#[cfg(test)]
mod tests_trait_bridge {
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
            preserve_input_urls: false,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };

        let methods = vec![&method];
        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        // The setup_block must contain the Zig struct with the method.
        assert!(
            emission.setup_block.contains("do_work"),
            "setup_block should contain method 'do_work', got:\n{}",
            emission.setup_block
        );
        // The vtable helper must use the trait snake name.
        assert!(
            emission.setup_block.contains("make_test_trait_vtable"),
            "setup_block should invoke make_test_trait_vtable, got:\n{}",
            emission.setup_block
        );
        // arg_expr expands into the argument list of the registration call.
        // It must contain the vtable variable and @ptrCast for the out_err pointer.
        assert!(
            emission.arg_expr.contains("vtable_my_fixture"),
            "arg_expr should reference vtable_my_fixture, got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.contains("@ptrCast"),
            "arg_expr should contain @ptrCast for out_err, got:\n{}",
            emission.arg_expr
        );

        // Must not contain any hardcoded domain-specific names.
        for name in &[
            "ImageBackend",
            "RecordProvider",
            "processImage",
            "process_image_fn",
            "sample_lib",
        ] {
            assert!(
                !emission.setup_block.contains(name),
                "setup_block must not contain domain name '{name}', got:\n{}",
                emission.setup_block
            );
        }
    }

    fn base_method(name: &str, return_type: crate::core::ir::TypeRef) -> crate::core::ir::MethodDef {
        use crate::core::ir::{MethodDef, ReceiverKind};
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
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
        }
    }

    fn base_fixture(id: &str) -> crate::e2e::fixture::Fixture {
        use crate::e2e::fixture::Fixture;
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

    /// A stub method returning a non-boolean integer primitive must return a
    /// non-degenerate literal (`1`), not the type's zero — a caller that
    /// validates its inputs (e.g. rejecting a zero-valued count) would
    /// otherwise reject the stub itself.
    #[test]
    fn integer_return_is_nonzero() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };
        let method = base_method("count", TypeRef::Primitive(PrimitiveType::Usize));
        let methods = vec![&method];
        let fixture = base_fixture("integer_return_fixture");

        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission
                .setup_block
                .contains("pub fn count(_: *@This()) u64 { return 1; }"),
            "integer-returning stub method must return 1, got:\n{}",
            emission.setup_block
        );
    }

    /// A stub method returning a collection keeps today's empty-JSON default —
    /// only the integer-primitive case is degenerate enough to reject a
    /// validating caller. Pins the collection behavior against a future
    /// change accidentally widening the non-degenerate-default fix.
    #[test]
    fn collection_return_stays_empty() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };
        let method = base_method("items", TypeRef::Vec(Box::new(TypeRef::String)));
        let methods = vec![&method];
        let fixture = base_fixture("collection_return_fixture");

        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission
                .setup_block
                .contains("pub fn items(_: *@This()) [*c]const u8 { return \"[]\"; }"),
            "collection-returning stub method must stay empty, got:\n{}",
            emission.setup_block
        );
    }
}
