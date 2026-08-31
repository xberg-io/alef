use super::*;
use crate::core::ir::{CoreWrapper, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn simple_method(name: &str, return_type: TypeRef, is_static: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        doc: String::new(),
        params: vec![],
        return_type,
        is_static,
        is_async: false,
        error_type: None,
        receiver: None,
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

#[test]
fn owned_receiver_is_not_freed_after_ffi_consumes_it() {
    let mut typ = opaque_type("Example");
    typ.is_opaque = false;
    let mut method = simple_method("into_owned", TypeRef::Named("Example".to_string()), false);
    method.receiver = Some(ReceiverKind::Owned);

    let generated = gen_method_wrapper(
        &typ,
        &method,
        "sample",
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(generated.contains("C.sample_example_into_owned(cRecv)"));
    assert!(!generated.contains("defer C.sample_example_free(cRecv)"));
    assert_go_syntax_is_valid(&generated);
}

#[test]
fn borrowed_value_receiver_marshalling_is_valid_go() {
    let mut typ = opaque_type("Example");
    typ.is_opaque = false;
    let method = simple_method("label", TypeRef::String, false);

    let generated = gen_method_wrapper(
        &typ,
        &method,
        "sample",
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(generated.contains("if cRecv == 0"));
    assert!(!generated.contains("cRecv == nil"));
    assert!(generated.contains("defer C.sample_example_free(cRecv)"));
    assert_go_syntax_is_valid(&generated);
}

fn assert_go_syntax_is_valid(generated: &str) {
    use std::io::Write as _;

    let Ok(mut child) = crate::test_support::spawn_from_stable_dir("gofmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    let source = format!("package sample\n\ntype Example struct {{}}\n\n{generated}");
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(source.as_bytes())
        .expect("write generated Go source");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(
        output.status.success(),
        "generated Go syntax is invalid: {}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn simple_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
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

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

#[test]
fn test_gen_method_wrapper_opaque_free_method_emits_ptr_cast() {
    let typ = opaque_type("Client");
    let method = simple_method("close", TypeRef::Unit, false);
    let opaque: std::collections::HashSet<&str> = ["Client"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(
        out.contains("func (h *Client) Close("),
        "expected receiver+method in: {out}"
    );
    assert!(out.contains("C.krz_client_close(h.ptr)"));
}

#[test]
fn test_gen_param_to_c_string_param_emits_cstring() {
    let param = simple_param("name", TypeRef::String);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);
    assert!(out.contains("C.CString("));
    assert!(out.contains("defer C.free("));
}

#[test]
fn test_gen_param_to_c_primitive_u64_emits_cgo_cast() {
    let param = simple_param("count", TypeRef::Primitive(PrimitiveType::U64));
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);
    assert!(out.contains("C.uint64_t("));
}

#[test]
fn test_gen_param_to_c_named_handle_uses_zero_sentinel() {
    let param = simple_param("config", TypeRef::Named("Config".to_string()));
    let out = gen_param_to_c(
        &param,
        "nil, ",
        true,
        "sample",
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(out.contains("if cConfig == 0"));
    assert!(!out.contains("if cConfig == nil"));
}

#[test]
fn test_gen_method_wrapper_non_opaque_static_emisample_package_func() {
    let mut typ = opaque_type("Config");
    typ.is_opaque = false;
    typ.fields = vec![simple_field("value", TypeRef::String)];
    let method = simple_method("default_value", TypeRef::String, true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains("func Config"));
}

#[test]
fn test_gen_method_wrapper_optional_string_getter_emits_nil_check_and_address() {
    let typ = opaque_type("GraphQLRouteConfig");
    let method = simple_method("get_description", TypeRef::Optional(Box::new(TypeRef::String)), false);
    let opaque: std::collections::HashSet<&str> = ["GraphQLRouteConfig"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "sample_router",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains(") *string {"), "expected *string return in:\n{out}");
    assert!(
        out.contains("if ptr == nil"),
        "missing nil check in optional-string getter body:\n{out}"
    );
    assert!(
        out.contains("return &s") || out.contains("return &result"),
        "missing take-address pattern in optional-string getter body:\n{out}"
    );
    assert!(
        !out.contains("\treturn C.GoString(ptr)\n"),
        "buggy bare `return C.GoString(ptr)` present:\n{out}"
    );
}

#[test]
fn test_gen_method_wrapper_bytes_result_emits_out_params() {
    let typ = opaque_type("Renderer");
    let method = MethodDef {
        name: "render_page".to_string(),
        doc: String::new(),
        params: vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
        return_type: TypeRef::Bytes,
        is_static: false,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        receiver: None,
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
    let opaque: std::collections::HashSet<&str> = ["Renderer"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
    assert!(out.contains("outLen"), "missing outLen in:\n{out}");
    assert!(out.contains("outCap"), "missing outCap in:\n{out}");
    assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}

#[test]
fn test_gen_method_wrapper_infallible_bytes_uses_owned_buffer_abi() {
    let typ = opaque_type("UploadFile");
    let method = simple_method("as_bytes", TypeRef::Bytes, false);
    let out = gen_method_wrapper(
        &typ,
        &method,
        "sample",
        &["UploadFile"].into(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(out.contains("&outPtr, &outLen, &outCap"));
    assert!(out.contains("C.GoBytes"));
    assert!(!out.contains("unmarshalBytes"));
}

/// Method-side companion to
/// `functions::tests::optional_bytes_return_reads_the_three_out_params_and_maps_null_to_nil`:
/// an opaque-type method returning `Optional<Bytes>` gets the same byte-identical C
/// signature as bare `Bytes`, so it must take the out-param path and map `*out_ptr == NULL`
/// to Go's nil slice rather than reading a direct `*mut u8`.
#[test]
fn optional_bytes_method_reads_the_three_out_params_and_maps_null_to_nil() {
    let typ = opaque_type("Renderer");
    let mut method = simple_method("thumbnail", TypeRef::Optional(Box::new(TypeRef::Bytes)), false);
    method.error_type = Some("SampleCrateError".to_string());

    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &["Renderer"].into(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    // Positive control: the fixture must actually render a method body, so the ABI
    // assertions below cannot pass over empty output. ~keep
    assert!(
        out.contains("func (h *Renderer) Thumbnail(") && out.contains("C.krz_renderer_thumbnail("),
        "fixture must emit a real method calling the C symbol, got:\n{out}"
    );

    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr declaration in:\n{out}");
    assert!(out.contains("var outLen, outCap"), "missing outLen/outCap in:\n{out}");
    assert!(
        out.contains("&outPtr, &outLen, &outCap"),
        "the C call must pass all three out-params in:\n{out}"
    );
    assert!(
        out.contains("if outPtr == nil"),
        "absence is carried by a NULL out_ptr and must map to Go's nil slice, got:\n{out}"
    );
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}

/// Control: widening the predicate must not put every optional-returning method on the
/// byte-buffer path. `Optional<String>` stays a nullable `*mut c_char`.
#[test]
fn optional_string_method_keeps_the_direct_pointer_shape() {
    let typ = opaque_type("Renderer");
    let mut method = simple_method("caption", TypeRef::Optional(Box::new(TypeRef::String)), false);
    method.error_type = Some("SampleCrateError".to_string());

    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &["Renderer"].into(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(
        out.contains("func (h *Renderer) Caption(") && out.contains("C.krz_renderer_caption("),
        "fixture must emit a real method calling the C symbol, got:\n{out}"
    );
    assert!(
        !out.contains("outPtr"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("outCap"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("([]byte, error)"),
        "Optional<String> must not be typed as []byte in:\n{out}"
    );
}

/// The defect: a `Vec<T>` method parameter was marshalled with a bare `json.Marshal`, and Go
/// writes `null` for a nil slice. Rust's serde writes `[]` for an empty `Vec`, and the FFI
/// shim's `serde_json::from_str::<Vec<T>>` rejects `null` outright — so an empty argument
/// crossed the ABI as either `[]` or `null` purely by how the caller had spelled "empty".
/// `param_named_type.jinja` already normalised the same way for a nil DTO pointer.
#[test]
fn a_vec_param_normalizes_a_nil_slice_to_the_empty_array_rust_emits() {
    let param = simple_param("tags", TypeRef::Vec(Box::new(TypeRef::String)));
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);

    // Positive first: the marshal really was emitted, so the presence check below means something.
    assert!(
        out.contains("jsonBytescTags, err := json.Marshal(tags)"),
        "the param must still be marshalled: {out}"
    );
    assert!(
        out.contains("if string(jsonBytescTags) == \"null\" {"),
        "a nil slice must be detected before it crosses the ABI: {out}"
    );
    assert!(
        out.contains("jsonBytescTags = []byte(\"[]\")"),
        "a nil slice must be normalized to the `[]` serde writes for an empty Vec: {out}"
    );
}

/// The map half of the same rule: serde writes `{}` for an empty `HashMap`, not `[]`.
#[test]
fn a_map_param_normalizes_a_nil_map_to_the_empty_object_rust_emits() {
    let param = simple_param(
        "labels",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    );
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);

    assert!(
        out.contains("jsonBytescLabels, err := json.Marshal(labels)"),
        "the param must still be marshalled: {out}"
    );
    assert!(
        out.contains("jsonBytescLabels = []byte(\"{}\")"),
        "a nil map must be normalized to the `{{}}` serde writes for an empty Map: {out}"
    );
}

#[test]
fn an_optional_vec_param_preserves_a_nil_value_as_json_null() {
    let mut param = simple_param("tags", TypeRef::Vec(Box::new(TypeRef::String)));
    param.optional = true;
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    assert_eq!(go_optional_type(&param.ty).as_ref(), "[]string");
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);

    assert!(
        out.contains("jsonBytescTags, err := json.Marshal(tags)"),
        "the optional slice must still be marshalled so nil becomes JSON null: {out}"
    );
    assert!(
        !out.contains("if string(jsonBytescTags) == \"null\"") && !out.contains("jsonBytescTags = []byte(\"[]\")"),
        "Option<Vec<T>> must preserve nil as None/null rather than fabricate Some(empty): {out}"
    );
}

#[test]
fn an_optional_map_param_preserves_a_nil_value_as_json_null() {
    let mut param = simple_param(
        "labels",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    );
    param.optional = true;
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    assert_eq!(go_optional_type(&param.ty).as_ref(), "map[string]string");
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);

    assert!(
        out.contains("jsonBytescLabels, err := json.Marshal(labels)"),
        "the optional map must still be marshalled so nil becomes JSON null: {out}"
    );
    assert!(
        !out.contains("if string(jsonBytescLabels) == \"null\"") && !out.contains("jsonBytescLabels = []byte(\"{}\")"),
        "Option<Map<K,V>> must preserve nil as None/null rather than fabricate Some(empty): {out}"
    );
}

/// The negative control that keeps the fix from over-reaching. A `TypeRef::Json` parameter is a
/// `serde_json::Value` on the Rust side, where `null` is a legitimate inhabitant — rewriting it
/// to `[]` would change the argument's meaning, not normalize its spelling.
#[test]
fn a_json_param_keeps_a_null_value_intact() {
    let param = simple_param("payload", TypeRef::Json);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);

    assert!(
        out.contains("jsonBytescPayload, err := json.Marshal(payload)"),
        "the param must still be marshalled: {out}"
    );
    assert!(
        !out.contains("== \"null\""),
        "a JSON-valued param must not have its null rewritten: {out}"
    );
}
