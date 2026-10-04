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
    use_test_assertions: bool,
) {
    // Allocator for the JSON-parse of the result blob (and any helper allocs).
    let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
    let _ = writeln!(out, "    defer _ = gpa.deinit();");
    let _ = writeln!(out, "    const allocator = gpa.allocator();");
    let _ = writeln!(out);

    // 1. Per-fixture visitor struct + callbacks table.
    // Zig reaches these types through build-system translation of the same generated header, so
    // they carry cbindgen's `[export] prefix` — shouty-snake, not a bare uppercase. Derive it from the
    // helper the header producer uses rather than re-deriving it here. ~keep
    let c_prefix = crate::codegen::c_consumer::export_type_prefix(&symbols.visitor_prefix);
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
        if use_test_assertions {
            let _ = writeln!(
                out,
                "    try testing.expect(_result == 0 or {module_name}.c.{last_error_code}() != 0);",
                last_error_code = symbols.last_error_code
            );
        } else {
            let _ = writeln!(
                out,
                "    if (_result != 0 and {module_name}.c.{last_error_code}() == 0) return error.ExpectedCallFailure;",
                last_error_code = symbols.last_error_code
            );
        }
        let _ = writeln!(
            out,
            "    if (_result != 0) {module_name}.c.{result_free}(_result);",
            result_free = symbols.result_free
        );
        return;
    }

    if use_test_assertions {
        let _ = writeln!(out, "    try testing.expect(_result != 0);");
    } else {
        let _ = writeln!(out, "    if (_result == 0) return error.CallFailed;");
    }
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{result_free}(_result);",
        result_free = symbols.result_free
    );
    let _ = writeln!(
        out,
        "    const _json_ptr = {module_name}.c.{result_to_json}(_result);",
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

    // ~keep: Zig errors on an unused local constant, and `render_json_assertion` can emit an
    // assertion as a comment only (e.g. "skipped: ..." / "not implemented for zig"), so an empty
    // or comment-only assertion list must not bind `result` at all. Render the assertion bodies
    // first, then only declare `result` if something in them actually reads it.
    let mut assertions_body = String::new();
    for assertion in assertions {
        if assertion.assertion_type != "error" {
            render_json_assertion(&mut assertions_body, assertion, "result", field_resolver, false);
        }
    }
    // ~keep: Scanning the whole rendered body is not enough on its own — a comment-only
    // line can legitimately mention "result" in prose (e.g. FieldSkip's "not available on
    // the JSON-struct result"), which `contains_word` cannot tell apart from a real Zig
    // identifier reference. Drop comment-only lines before scanning so prose in a skip
    // comment can never be mistaken for live code reading `result`.
    let references_result = assertions_body
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .any(|line| contains_word(line, "result"));
    if references_result {
        let _ = writeln!(out, "    const result = &_parsed.value;");
    }
    out.push_str(&assertions_body);
}

/// Return true when `text` contains `word` as a standalone identifier (not as a substring of a
/// longer identifier such as `_result_json` or `result_free`).
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = 0;
    while let Some(idx) = text[start..].find(word) {
        let pos = start + idx;
        let before_ok = pos == 0 || !is_ident(bytes[pos - 1]);
        let after_pos = pos + word.len();
        let after_ok = after_pos == bytes.len() || !is_ident(bytes[after_pos]);
        if before_ok && after_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

#[cfg(test)]
mod zig_visitor_tests {
    use super::{ZigVisitorCallSymbols, emit_visitor_test_body, resolve_zig_visitor_call_symbols};
    use crate::core::config::e2e::{CallConfig, CallOverride};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::{Assertion, CallbackAction, VisitorSpec};
    use std::collections::{BTreeMap, HashMap, HashSet};

    /// Build the symbols/visitor-spec/field-resolver trio shared by the visitor-body
    /// tests below, mirroring the FFI call configuration used elsewhere in this module.
    fn default_test_fixtures() -> (ZigVisitorCallSymbols, VisitorSpec, FieldResolver) {
        let call = CallConfig {
            function: "convert".to_string(),
            ..Default::default()
        };
        let fixture = crate::e2e::fixture::Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "result_binding".to_string(),
            category: None,
            description: "result binding".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<p>Hello</p>" }),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("zig", &fixture, &call, &[]);
        let symbols = resolve_zig_visitor_call_symbols(&call, &recipe, "htm");
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
        (symbols, visitor_spec, resolver)
    }

    // Regression test for the generated Zig snippet compile failure: Zig errors on an
    // unused local constant, and `assertions` is empty for every non-error fixture in
    // doc-snippet mode (`render_snippet_body` clears them). The `const result = ...`
    // binding must not be emitted when nothing will read it.
    #[test]
    fn emit_visitor_test_body_omits_result_binding_when_no_assertions() {
        let (symbols, visitor_spec, resolver) = default_test_fixtures();
        let mut content = String::new();
        emit_visitor_test_body(
            &mut content,
            "result_binding",
            "<p>Hello</p>",
            None,
            &visitor_spec,
            "sample",
            &symbols,
            &[],
            false,
            &resolver,
            true,
        );

        assert!(
            !content.contains("const result ="),
            "expected no `result` binding for an empty assertion list, got:\n{content}"
        );
    }

    // A non-empty assertion list is not sufficient on its own: `render_json_assertion`
    // can render an assertion as a comment only (e.g. the "keywords" synthetic field,
    // which is a fixture alias with no JSON-struct-result counterpart and always emits
    // a "skipped: ..." comment). That still leaves `result` unused, so the binding must
    // still be omitted. `chunks_have_heading_context` no longer fits this test: it now
    // renders a real predicate over `result`, not a comment. ~keep
    #[test]
    fn emit_visitor_test_body_omits_result_binding_when_assertions_render_only_comments() {
        let (symbols, visitor_spec, resolver) = default_test_fixtures();
        let assertions = vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("keywords".to_string()),
            ..Default::default()
        }];
        let mut content = String::new();
        emit_visitor_test_body(
            &mut content,
            "result_binding",
            "<p>Hello</p>",
            None,
            &visitor_spec,
            "sample",
            &symbols,
            &assertions,
            false,
            &resolver,
            true,
        );

        assert!(
            content.contains("skipped:"),
            "expected the comment-only assertion to still render, got:\n{content}"
        );
        assert!(
            !content.contains("const result ="),
            "a comment-only assertion body must not bind an unused `result`, got:\n{content}"
        );
    }

    // Companion to the two tests above: when an assertion actually renders a reference
    // to `result` (the shape used by the real e2e suite, whose assertions are never
    // cleared), the binding must still be emitted — otherwise this fix would silently
    // break e2e zig output.
    #[test]
    fn emit_visitor_test_body_emits_result_binding_when_assertion_references_it() {
        let (symbols, visitor_spec, resolver) = default_test_fixtures();
        let assertions = vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: None,
            ..Default::default()
        }];
        let mut content = String::new();
        emit_visitor_test_body(
            &mut content,
            "result_binding",
            "<p>Hello</p>",
            None,
            &visitor_spec,
            "sample",
            &symbols,
            &assertions,
            false,
            &resolver,
            true,
        );

        assert!(
            content.contains("const result ="),
            "expected the `result` binding when an assertion references it, got:\n{content}"
        );
        assert!(
            content.contains("const _ne = result;"),
            "expected the rendered assertion to reference `result`, got:\n{content}"
        );
    }

    #[test]
    fn visitor_body_treats_scalar_result_handles_as_integers() {
        let (symbols, visitor_spec, resolver) = default_test_fixtures();

        for expects_error in [false, true] {
            let mut content = String::new();
            emit_visitor_test_body(
                &mut content,
                "scalar_result_handle",
                "<p>Hello</p>",
                None,
                &visitor_spec,
                "sample",
                &symbols,
                &[],
                expects_error,
                &resolver,
                true,
            );

            let expected_check = if expects_error {
                "_result == 0 or"
            } else {
                "_result != 0"
            };
            assert!(
                content.contains(expected_check),
                "expected integer sentinel check `{expected_check}` for the scalar result handle:\n{content}"
            );
            for invalid_pointer_form in ["_result == null", "_result != null", "_result.?", "if (_result) |"] {
                assert!(
                    !content.contains(invalid_pointer_form),
                    "scalar result handle used pointer form `{invalid_pointer_form}`:\n{content}"
                );
            }
            assert!(
                !expects_error || content.contains("if (_result != 0)"),
                "expected failed-call cleanup to guard the scalar handle:\n{content}"
            );
            let expected_result_to_json = format!("{}(_result)", symbols.result_to_json);
            assert!(
                expects_error || content.contains(&expected_result_to_json),
                "expected the scalar handle to pass directly into result serialization:\n{content}"
            );
        }
    }

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
            docs: None,
            requirements: Vec::new(),
            id: "configured_symbols".to_string(),
            category: None,
            description: "configured symbols".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<p>Hello</p>", "options": { "trim": true } }),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
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
            true,
        );

        assert!(content.contains("sample.c.abc_render_options_from_json"));
        assert!(content.contains("sample.c.abc_options_set_visitor"));
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
}
