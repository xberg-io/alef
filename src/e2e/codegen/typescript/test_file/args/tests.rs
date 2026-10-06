use super::*;

fn message_enum_def() -> EnumDef {
    EnumDef {
        name: "Message".into(),
        serde_tag: Some("role".into()),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![crate::core::ir::EnumVariant {
            name: "User".into(),
            is_tuple: true,
            fields: vec![crate::core::ir::FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("UserMessage".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn user_message_type_def() -> TypeDef {
    TypeDef {
        name: "UserMessage".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "content".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn fixture() -> crate::e2e::fixture::Fixture {
    crate::e2e::fixture::Fixture {
        id: "chat".to_string(),
        description: "Chat".to_string(),
        ..Default::default()
    }
}

/// Regression for the E3 message-shape defect: an array-typed `json_object` arg (the real
/// site of the 108 x TS2353 failures, e.g. `messages: Message[]`) used to skip the typed
/// builder entirely and dump each element through `json_to_js_camel` — a pure key-casing
/// pass with no notion of a tagged-data enum's variant nesting. Each element must instead go
/// through `ts_builder_expression`, the same builder a single typed object uses.
///
/// Convention updated for the newtype-payload-flattening change
/// (`backends::napi::tagged_enum_flattened_newtype`): once `UserMessage` resolves in
/// `type_defs`, napi's wire shape flattens the payload's own fields beside the tag
/// (`{ role: 'user', content: '...' }`). `Message` has exactly one data-carrying variant, so
/// once resolved it also meets `enums::is_fully_flattened_internal_enum` ("EVERY data-carrying
/// variant flattens" -- true trivially for a single variant), which routes the napi binding
/// through the JSON-passthrough wrapper rather than a nominal struct, same as the single-object
/// case in `builders::tests::node_tagged_enum_variant_flattens_payload_onto_container_when_resolvable`.
/// The array element therefore renders as a `satisfies`-cast passthrough literal rather than an
/// `as`-cast typed object literal; the field values are unchanged. See
/// `node_array_of_tagged_enum_elements_nests_payload_when_type_unresolved` below for the
/// negative control covering the case that still nests.
#[test]
fn node_array_of_tagged_enum_elements_flattens_each_payload_when_type_resolves() {
    let enums = [message_enum_def()];
    let type_defs = [user_message_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({ "messages": [{ "role": "user", "content": "Hello" }] });
    let args = [ArgMapping {
        name: "messages".into(),
        field: "input.messages".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("Message".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (_setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        call_args, "[({ content: \"Hello\", role: \"user\" } satisfies Message)]",
        "array element must flatten the payload beside the tag once the payload type resolves"
    );
}

/// Negative control for the test above: when `UserMessage` does NOT resolve in `type_defs`
/// (the array-of-tagged-enum equivalent of napi's unresolvable-payload fallback in
/// `backends::napi::tagged_enum_flattened_newtype`), each element must keep nesting the
/// payload under its synthesized per-variant field — proving the flattening in the test
/// above is conditional on type resolution, not unconditional.
#[test]
fn node_array_of_tagged_enum_elements_nests_payload_when_type_unresolved() {
    let enums = [message_enum_def()];
    let type_defs: [TypeDef; 0] = [];
    let fixture = fixture();
    let input = serde_json::json!({ "messages": [{ "role": "user", "content": "Hello" }] });
    let args = [ArgMapping {
        name: "messages".into(),
        field: "input.messages".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("Message".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (_setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        call_args, "[{ role: \"user\", user: { content: \"Hello\" } } as Message]",
        "array element must nest the payload under the synthesized variant field when the \
             payload type does not resolve"
    );
}

fn extract_input_type_def() -> TypeDef {
    TypeDef {
        name: "ExtractInput".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: TypeRef::Named("ExtractInputKind".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// Regression for the #468 byte-payload typing defect (the wasm half of the 8 failing
/// `docs-site` snippets, e.g. `extractBatch`'s `inputs: WasmExtractInput[]` argument): an
/// array-typed `json_object` arg whose element type is a known IR struct used to route
/// through the typed builder for node only — wasm fell back to `json_to_js_camel`, a pure
/// key-casing dump with no notion of a `bytes` field or of wasm-bindgen's class-instance
/// requirement. That dump produced a plain object literal carrying the fixture's raw
/// `bytes` value (`{ bytes: [72, 105], kind: "bytes" }`), which is neither a
/// `WasmExtractInput` instance nor assignable to its `Uint8Array` field. Each element must
/// instead go through `ts_builder_expression`, exactly as node's array elements already do.
#[test]
fn wasm_array_of_known_element_type_lowers_bytes_field_via_builder() {
    let enums = [EnumDef {
        name: "ExtractInputKind".into(),
        ..Default::default()
    }];
    let type_defs = [extract_input_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({ "inputs": [{ "bytes": [72, 105], "kind": "bytes" }] });
    let args = [ArgMapping {
        name: "inputs".into(),
        field: "input.inputs".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("ExtractInput".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (_setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert!(
        call_args.contains("_u0.bytes = Uint8Array.from([72, 105]);"),
        "array element must build a real Uint8Array via the typed builder, not dump the raw fixture literal: {call_args}"
    );
    assert!(
        call_args.contains("WasmExtractInput.default()"),
        "the wasm element must be constructed as a real binding-class instance, not a plain object literal: {call_args}"
    );
    assert!(
        !call_args.contains("bytes: [72, 105]"),
        "must not fall back to the untyped json_to_js_camel dump: {call_args}"
    );
}

/// Regression: a preserved (`preserve_input_urls: true`) `mock_url_list` argument whose
/// fixture declares zero URLs used to render the bare `const urls = [];` -- no element is
/// ever pushed onto it in this scope, so under `strict`/`noImplicitAny` `tsc` cannot resolve
/// its element type and reports TS7034 at the declaration and TS7005 at the call site that
/// reads it. The element type is always `string` for this arg type, so the fix annotates
/// only the empty case as `string[]`.
#[test]
fn node_empty_preserved_url_list_declares_a_typed_empty_array() {
    let mut fixture = fixture();
    fixture.preserve_input_urls = true;
    let input = serde_json::json!({ "urls": [] });
    let args = [ArgMapping {
        name: "urls".into(),
        field: "input.urls".into(),
        arg_type: "mock_url_list".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &[],
        &[],
        "",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        setup_lines,
        vec!["const urls: string[] = [];".to_string()],
        "an empty URL list must declare an explicit string[] type, not a bare [] TypeScript cannot infer"
    );
    assert_eq!(call_args, "urls");
}

/// Control for `node_empty_preserved_url_list_declares_a_typed_empty_array`: a non-empty
/// preserved URL list already infers `string[]` from its own elements, so the fix must not
/// touch this rendering at all.
#[test]
fn node_non_empty_preserved_url_list_is_unchanged() {
    let mut fixture = fixture();
    fixture.preserve_input_urls = true;
    let input = serde_json::json!({ "urls": ["https://a.example/x", "https://b.example/y"] });
    let args = [ArgMapping {
        name: "urls".into(),
        field: "input.urls".into(),
        arg_type: "mock_url_list".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &[],
        &[],
        "",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        setup_lines,
        vec!["const urls = [\"https://a.example/x\", \"https://b.example/y\"];".to_string()],
        "a non-empty preserved URL list must keep its untyped literal -- TypeScript already infers string[]"
    );
    assert_eq!(call_args, "urls");
}

/// The wasm half of `node_empty_preserved_url_list_declares_a_typed_empty_array`. Wasm and
/// node share `build_args_and_setup` for this argument kind -- neither branches on `lang` --
/// so this pins that the shared code path is fixed for both targets, not only node's.
#[test]
fn wasm_empty_preserved_url_list_declares_a_typed_empty_array() {
    let mut fixture = fixture();
    fixture.preserve_input_urls = true;
    let input = serde_json::json!({ "urls": [] });
    let args = [ArgMapping {
        name: "urls".into(),
        field: "input.urls".into(),
        arg_type: "mock_url_list".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        None,
        &[],
        &[],
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        setup_lines,
        vec!["const urls: string[] = [];".to_string()],
        "the wasm target must render the same typed empty-array declaration as node"
    );
    assert_eq!(call_args, "urls");
}
