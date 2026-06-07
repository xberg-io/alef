use super::assertions::render_json_assertion;
use super::*;
use crate::e2e::codegen::zig_visitors;

pub(super) struct ZigVisitorCallSymbols {
    visitor_prefix: String,
    visitor_create: String,
    visitor_free: String,
    options_from_json: String,
    options_free: String,
    options_set_visitor_handle: String,
    function_name: String,
    result_free: String,
    result_to_json: String,
    free_string: String,
    last_error_code: String,
}

pub(super) fn resolve_zig_visitor_call_symbols(
    call_config: &crate::core::config::e2e::CallConfig,
    recipe: &crate::e2e::codegen::recipe::ResolvedE2eCallRecipe<'_>,
    ffi_prefix: &str,
) -> ZigVisitorCallSymbols {
    let c_override = call_config.overrides.get("c");
    let function_name = c_override
        .and_then(|override_config| override_config.function.as_ref())
        .cloned()
        .or_else(|| {
            recipe
                .override_config
                .and_then(|override_config| override_config.function.as_ref())
                .cloned()
        })
        .unwrap_or_else(|| call_config.function.clone());
    let options_type_name = c_override
        .and_then(|override_config| override_config.options_type.as_deref())
        .or(recipe.options_type)
        .unwrap_or_default()
        .to_string();
    let options_type_snake = options_type_name.to_snake_case();
    let result_type_name = c_override
        .and_then(|override_config| override_config.result_type.as_ref())
        .cloned()
        .or_else(|| {
            recipe
                .override_config
                .and_then(|override_config| override_config.result_type.as_ref())
                .cloned()
        })
        .unwrap_or_else(|| call_config.function.to_pascal_case());
    let result_type_snake = result_type_name.to_snake_case();

    ZigVisitorCallSymbols {
        visitor_prefix: ffi_prefix.to_string(),
        visitor_create: format!("{ffi_prefix}_visitor_create"),
        visitor_free: format!("{ffi_prefix}_visitor_free"),
        options_from_json: format!("{ffi_prefix}_{options_type_snake}_from_json"),
        options_free: format!("{ffi_prefix}_{options_type_snake}_free"),
        options_set_visitor_handle: format!("{ffi_prefix}_options_set_visitor"),
        function_name,
        result_free: format!("{ffi_prefix}_{result_type_snake}_free"),
        result_to_json: format!("{ffi_prefix}_{result_type_snake}_to_json"),
        free_string: format!("{ffi_prefix}_free_string"),
        last_error_code: format!("{ffi_prefix}_last_error_code"),
    }
}

/// Emit the body of a visitor-bearing test. Drives the FFI directly so we
/// can attach a generated visitor callbacks vtable to the configured options
/// handle before calling the configured FFI function. The high-level wrapper
/// cannot carry a visitor because the visitor is a Rust
/// trait object, not a JSON-encodable field.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_visitor_test_body(
    out: &mut String,
    fixture_id: &str,
    html: &str,
    options_value: Option<&serde_json::Value>,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    module_name: &str,
    symbols: &ZigVisitorCallSymbols,
    assertions: &[Assertion],
    expects_error: bool,
    field_resolver: &FieldResolver,
) {
    // Allocator for the JSON-parse of the result blob (and any helper allocs).
    let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
    let _ = writeln!(out, "    defer _ = gpa.deinit();");
    let _ = writeln!(out, "    const allocator = gpa.allocator();");
    let _ = writeln!(out);

    // 1. Per-fixture visitor struct + callbacks table.
    let c_prefix = symbols.visitor_prefix.to_uppercase();
    let visitor_type_stem = symbols.visitor_prefix.to_pascal_case();
    // The C FFI re-defines visitor context as a stem-prefixed struct (e.g.
    // `HtmContext`) — distinct from the opaque core `NodeContext`. The
    // callbacks in `HtmVisitorCallbacks` take `*const HtmContext`, so Zig
    // sees `c.HTMHtmContext` (NOT `c.HTMNodeContext`). Both context and
    // callbacks types follow the `{prefix}{stem}…` pattern.
    let c_types = zig_visitors::ZigVisitorCTypes {
        context_type: format!("{c_prefix}{visitor_type_stem}Context"),
        callbacks_type: format!("{c_prefix}{visitor_type_stem}VisitorCallbacks"),
    };
    let visitor_block = zig_visitors::build_zig_visitor(fixture_id, module_name, visitor_spec, &c_types);
    out.push_str(&visitor_block);

    // 2. Materialise the visitor handle and attach it to the configured options handle.
    let _ = writeln!(
        out,
        "    const _visitor = {module_name}.c.{visitor_create}(&_callbacks);",
        visitor_create = symbols.visitor_create
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{visitor_free}(_visitor);",
        visitor_free = symbols.visitor_free
    );

    // 3. Options handle: always allocate one (even when the fixture supplies
    //    no `options`) so we have somewhere to attach the visitor. The FFI
    //    accepts `"{}"` as an empty options JSON.
    let options_json = match options_value {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    };
    let escaped_options = escape_zig(&options_json);
    let _ = writeln!(
        out,
        "    const _options_z = try std.heap.c_allocator.dupeZ(u8, \"{escaped_options}\");"
    );
    let _ = writeln!(out, "    defer std.heap.c_allocator.free(_options_z);");
    let _ = writeln!(
        out,
        "    const _options = {module_name}.c.{options_from_json}(_options_z.ptr);",
        options_from_json = symbols.options_from_json
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{options_free}(_options);",
        options_free = symbols.options_free
    );
    let _ = writeln!(
        out,
        "    {module_name}.c.{options_set_visitor_handle}(_options, _visitor);",
        options_set_visitor_handle = symbols.options_set_visitor_handle
    );

    // 4. HTML buffer + convert call.
    let escaped_html = escape_zig(html);
    let _ = writeln!(
        out,
        "    const _html_z = try std.heap.c_allocator.dupeZ(u8, \"{escaped_html}\");"
    );
    let _ = writeln!(out, "    defer std.heap.c_allocator.free(_html_z);");
    let _ = writeln!(
        out,
        "    const _result = {module_name}.c.{function_name}(_html_z.ptr, _options);",
        function_name = symbols.function_name
    );

    if expects_error {
        // Error-path: _result null OR last error code non-zero.
        let _ = writeln!(
            out,
            "    try testing.expect(_result == null or {module_name}.c.{last_error_code}() != 0);",
            last_error_code = symbols.last_error_code
        );
        let _ = writeln!(
            out,
            "    if (_result) |r| {module_name}.c.{result_free}(r);",
            result_free = symbols.result_free
        );
        return;
    }

    let _ = writeln!(out, "    try testing.expect(_result != null);");
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{result_free}(_result.?);",
        result_free = symbols.result_free
    );
    let _ = writeln!(
        out,
        "    const _json_ptr = {module_name}.c.{result_to_json}(_result.?);",
        result_to_json = symbols.result_to_json
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{free_string}(_json_ptr);",
        free_string = symbols.free_string
    );
    let _ = writeln!(out, "    const _result_json = std.mem.sliceTo(_json_ptr, 0);");
    let _ = writeln!(
        out,
        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
    );
    let _ = writeln!(out, "    defer _parsed.deinit();");
    let _ = writeln!(out, "    const result = &_parsed.value;");

    for assertion in assertions {
        if assertion.assertion_type != "error" {
            render_json_assertion(out, assertion, "result", field_resolver, false);
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-struct assertion rendering (for result_is_json_struct = true)
// ---------------------------------------------------------------------------

/// Convert a dot-separated field path into a chain of `std.json.Value` lookups.
///
/// Each segment uses `.object.get("key").?` to traverse the JSON object tree.
/// The final segment stops before the leaf-type accessor so callers can append
/// the appropriate accessor (`.string`, `.integer`, `.array.items`, etc.).
///
/// Returns `(base_expr, last_key)` where `base_expr` already includes all
/// intermediate `.object.get("…").?` dereferences up to (but not including)
/// the leaf, and `last_key` is the last path segment.
/// Variant names of `FormatMetadata` (snake_case, from `#[serde(rename_all = "snake_case")]`).
/// These appear as typed accessors in fixture paths (e.g. `format.excel.sheet_count`)
/// but are NOT JSON keys — `FormatMetadata` is internally tagged so variant fields are
#[cfg(test)]
mod zig_visitor_tests {
    use super::{emit_visitor_test_body, resolve_zig_visitor_call_symbols};
    use crate::core::config::e2e::{CallConfig, CallOverride};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};
    use std::collections::{BTreeMap, HashMap, HashSet};

    #[test]
    fn visitor_body_uses_configured_ffi_call_symbols() {
        let c_override = CallOverride {
            function: Some("abc_render_document".to_string()),
            options_type: Some("RenderOptions".to_string()),
            result_type: Some("RenderResult".to_string()),
            ..Default::default()
        };
        let zig_override = CallOverride {
            function: Some("renderDocument".to_string()),
            options_type: Some("WrapperOptions".to_string()),
            result_type: Some("WrapperResult".to_string()),
            ..Default::default()
        };
        let call = CallConfig {
            function: "render".to_string(),
            overrides: [("c".to_string(), c_override), ("zig".to_string(), zig_override)].into(),
            ..Default::default()
        };
        let fixture = crate::e2e::fixture::Fixture {
            id: "configured_symbols".to_string(),
            category: None,
            description: "configured symbols".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "html": "<p>Hello</p>", "options": { "trim": true } }),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: String::new(),
            http: None,
        };
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("zig", &fixture, &call, &[]);
        let symbols = resolve_zig_visitor_call_symbols(&call, &recipe, "abc");
        let mut callbacks = BTreeMap::new();
        callbacks.insert("visit_text".to_string(), CallbackAction::Continue);
        let visitor_spec = VisitorSpec { callbacks };
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        let mut content = String::new();
        emit_visitor_test_body(
            &mut content,
            "configured_symbols",
            "<p>Hello</p>",
            fixture.input.get("options"),
            &visitor_spec,
            "sample",
            &symbols,
            &[],
            false,
            &resolver,
        );

        assert!(content.contains("sample.c.abc_render_options_from_json"));
        assert!(content.contains("sample.c.abc_options_set_visitor_handle"));
        assert!(content.contains("sample.c.abc_render_document(_html_z.ptr, _options)"));
        assert!(content.contains("sample.c.abc_render_result_to_json"));
        assert!(content.contains("sample.c.abc_render_result_free"));

        for hardcoded in [
            "htm_conversion_options_from_json",
            "htm_options_set_visitor_handle",
            "htm_convert",
            "htm_conversion_result_to_json",
            "htm_conversion_result_free",
            "WrapperOptions",
            "WrapperResult",
            "renderDocument",
        ] {
            assert!(
                !content.contains(hardcoded),
                "visitor Zig output leaked `{hardcoded}`:\n{content}"
            );
        }
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
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };

        let fixture = Fixture {
            id: "my_fixture".to_string(),
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
}
