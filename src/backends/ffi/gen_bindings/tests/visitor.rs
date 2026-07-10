use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

#[test]
fn test_visitor_callbacks_disabled_by_default() {
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(!lib.content.contains("VisitorCallbacks"));
    assert!(!lib.content.contains("visit_text"));
    assert!(!lib.content.contains("_visitor_create"));
    assert!(!lib.content.contains("_visitor_free"));
    assert!(!lib.content.contains("_convert_with_visitor"));
}

#[test]
fn test_visitor_callbacks_without_matching_bridge_do_not_emit_fallback_conversion_api() {
    let api = visitor_api();
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"
visitor_callbacks = true
"#,
    );
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(!lib.content.contains("VisitorCallbacks"));
    assert!(!lib.content.contains("doc_convert"));
    assert!(!lib.content.contains("DocOptions"));
    assert!(!lib.content.contains("DocResult"));
}

#[test]
fn test_visitor_callbacks_enabled() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("struct HtmVisitorCallbacks"));
    assert!(lib.content.contains("pub struct HtmContext"));

    assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
    assert!(lib.content.contains("HTM_VISIT_SKIP"));
    assert!(lib.content.contains("HTM_VISIT_PRESERVE_HTML"));
    assert!(lib.content.contains("HTM_VISIT_CUSTOM"));
    assert!(lib.content.contains("HTM_VISIT_ERROR"));

    assert!(lib.content.contains("node_type: i32"));
    assert!(lib.content.contains("tag_name: *const std::ffi::c_char"));
    assert!(lib.content.contains("depth: usize"));
    assert!(lib.content.contains("index_in_parent: usize"));
    assert!(lib.content.contains("parent_tag: *const std::ffi::c_char"));
    assert!(lib.content.contains("is_inline: i32"));
}

/// Regression: when the visitor context's first field is a Named enum (e.g.
/// `node_type: NodeType` rather than `node_type: i32`), the FFI emitter must
/// still emit `node_type: i32` in the C struct. Without this, Java/Kotlin/Swift
/// CTX_LAYOUTTs that hardcode `node_type` at offset 0 read garbage from the
/// shifted `tag_name` pointer and crash with negative-index errors.
#[test]
fn test_visitor_callbacks_emit_enum_node_type_as_i32() {
    let mut api = visitor_api();
    api.enums.push(EnumDef {
        name: "NodeType".to_string(),
        rust_path: "my_lib::visitor::NodeType".to_string(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Element".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    let ctx = api
        .types
        .iter_mut()
        .find(|t| t.name == "NodeContext")
        .expect("NodeContext fixture missing");
    ctx.fields[0].ty = TypeRef::Named("NodeType".to_string());

    let config = visitor_config_htm();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("node_type: i32"),
        "expected `node_type: i32` in emitted FFI struct"
    );
    assert!(
        lib.content.contains("node_type: ctx.node_type as i32"),
        "expected `node_type: ctx.node_type as i32` initialization"
    );
}

#[test]
fn test_visitor_callbacks_visitor_handle_struct() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("pub struct HtmVisitor"));
    assert!(lib.content.contains("callbacks: HtmVisitorCallbacks"));
    assert!(lib.content.contains("_tag_scratch"));
}

#[test]
fn test_visitor_callbacks_callback_fields() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("visit_text"));
    assert!(lib.content.contains("visit_element_start"));
    assert!(lib.content.contains("visit_link"));
    assert!(lib.content.contains("visit_heading"));
    assert!(lib.content.contains("visit_blockquote"));
    assert!(lib.content.contains("visit_list_item"));
    assert!(lib.content.contains("visit_table_row"));
}

#[test]
fn test_visitor_callbacks_ffi_functions() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("htm_visitor_create"));
    assert!(lib.content.contains("htm_visitor_free"));
    assert!(lib.content.contains("htm_render_document_with_visitor"));

    assert!(lib.content.contains("extern \"C\" fn htm_visitor_create"));
    assert!(lib.content.contains("extern \"C\" fn htm_visitor_free"));
    assert!(lib.content.contains("extern \"C\" fn htm_render_document_with_visitor"));
}

#[test]
fn test_visitor_callbacks_callback_signatures() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("extern \"C\" fn("));
    assert!(lib.content.contains("*const HtmContext"));
    assert!(lib.content.contains("user_data: *mut std::ffi::c_void"));
    assert!(lib.content.contains("out_custom: *mut *mut std::ffi::c_char"));
    assert!(lib.content.contains("out_len: *mut usize"));

    assert!(lib.content.contains(") -> i32"));
}

#[test]
fn test_visitor_callbacks_generate_param_setup_blocks() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("let text_cs = match std::ffi::CString::new(text)"));
    assert!(
        lib.content
            .contains("let (title_ptr, _title_cs) = opt_str_to_c(title);")
    );
    assert!(lib.content.contains("let ordered_i = i32::from(ordered);"));
    assert!(
        lib.content
            .contains("let cells_cstrings: Vec<std::ffi::CString> = cells")
    );
    assert!(lib.content.contains("let cell_count = cells_ptrs.len();"));
    assert!(
        lib.content
            .contains("cb(c_ctx, user_data, cells_ptrs.as_ptr(), cell_count, is_header_i")
    );
}

#[test]
fn test_visitor_callbacks_custom_prefix() {
    let api = visitor_api();
    let config = visitor_config_ml();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("MlVisitorCallbacks"));
    assert!(lib.content.contains("MlContext"));
    assert!(lib.content.contains("ml_visitor_create"));
    assert!(lib.content.contains("ml_visitor_free"));
    assert!(lib.content.contains("ml_render_document_with_visitor"));
    assert!(lib.content.contains("ML_VISIT_CONTINUE"));
}

#[test]
fn test_visitor_callbacks_visitor_ref_wrapper() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("struct VisitorRef"));
    assert!(lib.content.contains("impl std::fmt::Debug for VisitorRef"));
    assert!(lib.content.contains("impl my_lib::visitor::HtmlVisitor for VisitorRef"));
}

#[test]
fn test_visitor_callbacks_safety_comments() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("// SAFETY:"));
    assert!(lib.content.contains("unsafe"));
    assert!(lib.content.contains("unsafe extern \"C\" fn"));
}

#[test]
fn test_visitor_callbacks_decode_visit_result() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("decode_visit_result"));
    assert!(lib.content.contains("VisitorResult::Skip"));
    assert!(lib.content.contains("VisitorResult::PreserveHtml"));
    assert!(lib.content.contains("VisitorResult::Custom"));
    assert!(lib.content.contains("VisitorResult::Error"));
}

#[test]
fn test_legacy_visitor_callbacks_use_configured_context_and_result_metadata() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "prs"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "SyntaxVisitor"
type_alias = "SyntaxVisitorHandle"
param_name = "visitor"
options_type = "ParseOptions"
context_type = "ParseContext"
result_type = "WalkOutcome"
"#,
    );
    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "SyntaxVisitor".to_string(),
        rust_path: "my_lib::syntax::SyntaxVisitor".to_string(),
        methods: vec![MethodDef {
            name: "visit_token".to_string(),
            params: vec![
                ParamDef {
                    name: "context".to_string(),
                    ty: TypeRef::Named("ParseContext".to_string()),
                    is_ref: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "token".to_string(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("WalkOutcome".to_string()),
            receiver: Some(ReceiverKind::RefMut),
            doc: "Visit parser tokens.".to_string(),
            is_async: false,
            is_static: false,
            error_type: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_trait: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseContext".to_string(),
        rust_path: "my_lib::syntax::ParseContext".to_string(),
        fields: vec![
            FieldDef {
                name: "rule_name".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
            FieldDef {
                name: "byte_offset".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Usize),
                ..FieldDef::default()
            },
            FieldDef {
                name: "source_path".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..FieldDef::default()
            },
            FieldDef {
                name: "is_recovery".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseTree".to_string(),
        rust_path: "my_lib::ParseTree".to_string(),
        is_clone: true,
        is_return_type: true,
        ..TypeDef::default()
    });
    api.enums.push(EnumDef {
        name: "WalkOutcome".to_string(),
        rust_path: "my_lib::syntax::WalkOutcome".to_string(),
        variants: vec![
            EnumVariant {
                name: "Proceed".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "StopHere".to_string(),
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "ReplaceWith".to_string(),
                fields: vec![visitor_result_string_field("replacement")],
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        has_default: false,
        ..EnumDef::default()
    });
    api.functions.push(FunctionDef {
        name: "parse".to_string(),
        rust_path: "my_lib::parse".to_string(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("SyntaxVisitorHandle".to_string()),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ParseTree".to_string()),
        error_type: Some("ParseError".to_string()),
        sanitized: true,
        ..FunctionDef::default()
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("pub struct PrsContext"));
    assert!(lib.content.contains("pub rule_name: *const std::ffi::c_char"));
    assert!(lib.content.contains("pub byte_offset: usize"));
    assert!(lib.content.contains("pub source_path: *const std::ffi::c_char"));
    assert!(lib.content.contains("pub is_recovery: i32"));
    assert!(lib.content.contains("PRS_VISIT_STOP_HERE"));
    assert!(lib.content.contains("my_lib::syntax::WalkOutcome::StopHere"));
    assert!(lib.content.contains("VisitorResult::ReplaceWith(msg)"));
    assert!(lib.content.contains("context: &my_lib::syntax::ParseContext"));
    assert!(!lib.content.contains("my_lib::visitor::VisitResult"));
    assert!(!lib.content.contains("my_lib::visitor::NodeContext"));
}

#[test]
fn test_visitor_callbacks_call_with_ctx() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("call_with_ctx"));
    assert!(lib.content.contains("HtmContext"));
    assert!(lib.content.contains("tag_cstring"));
    assert!(lib.content.contains("parent_tag_cstring"));
}

#[test]
fn test_visitor_callbacks_opt_str_to_c() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("opt_str_to_c"));
}

#[test]
fn test_visitor_callbacks_repr_c() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // FFI-crossing types must use #[repr(C)]
    assert!(lib.content.contains("#[repr(C)]"));
}

#[test]
fn test_visitor_callbacks_send_impl() {
    let api = visitor_api();
    let config = visitor_config_htm();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("unsafe impl Send for HtmVisitorCallbacks"));
}
