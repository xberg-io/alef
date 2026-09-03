//! Unit tests for `typed_values.rs`, split into a sibling file (matching the
//! `test_file.rs`/`lint_clean_python_tests.rs` split) to keep `typed_values.rs` itself under
//! the file-size cap after adding `Map` coverage.

use super::*;

#[test]
fn emit_bytes_arg_file_path_uses_path_read_bytes() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::Value::String("pdf/memo.pdf".to_string());
    emit_bytes_arg(&mut bindings, &mut exprs, &value, "content");
    assert!(bindings[0].contains("Path("), "got: {:?}", bindings[0]);
    assert!(bindings[0].contains("read_bytes"), "got: {:?}", bindings[0]);
}

#[test]
fn emit_bytes_arg_base64_uses_b64decode() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::Value::String("/9j/4AAQ".to_string());
    emit_bytes_arg(&mut bindings, &mut exprs, &value, "data");
    assert!(bindings[0].contains("b64decode"), "got: {:?}", bindings[0]);
}

#[test]
fn emit_json_object_arg_enum_field_emits_constructor_call() {
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    let enum_def = EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "demo::OutputFormat".to_string(),
        variants: vec![EnumVariant {
            name: "Markdown".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let type_def = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "output_format".to_string(),
            ty: TypeRef::Named("OutputFormat".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums = vec![enum_def];
    let type_defs = vec![type_def];

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"output_format": "markdown"});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);
    assert!(done);
    // Constructor-call form works for both (str, Enum) subclasses and #[pyclass] tagged-union
    // structs. Attribute access (OutputFormat.MARKDOWN) fails for the latter because they have
    // no class-level variant constants.
    assert!(
        bindings[0].contains("OutputFormat(\"markdown\")"),
        "expected constructor-call emission, got: {:?}",
        bindings[0]
    );
    assert!(
        !bindings[0].contains("OutputFormat.MARKDOWN"),
        "must not emit attribute access, got: {:?}",
        bindings[0]
    );
}

#[test]
fn emit_json_object_arg_dict_mode_emits_literal() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"key": "val"});
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let spec = ConstructorSpec {
        options_type: None,
        options_via: "dict",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);
    assert!(done);
    assert!(bindings[0].contains("\"key\""), "got: {:?}", bindings[0]);
}

#[test]
fn emit_json_object_arg_reads_documented_nested_file() {
    let mut bindings = Vec::new();
    let mut expressions = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut expressions,
    };
    let value = serde_json::json!({"bytes": "document.pdf"});
    let spec = ConstructorSpec {
        options_type: Some("DocumentInput"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let docs_files = [FixtureDocsFileInput {
        field: "/bytes".into(),
        path: "document.pdf".into(),
    }];
    let context = KwargRenderContext {
        type_defs: &[],
        enums: &[],
        enum_fields: &HashMap::new(),
        docs_files: &docs_files,
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "input", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    input = DocumentInput(bytes=Path("document.pdf").read_bytes())"#]
    );
}

/// Regression for the nested-config construction defect: a config field whose own type is
/// itself a generated pyclass (e.g. `nested: NestedConfig` inside
/// `ExtractionConfig`) must be constructed with that class, not emitted as a raw dict --
/// pyo3 rejects a dict where a native class instance is required.
#[test]
fn emit_json_object_arg_kwargs_mode_constructs_nested_struct_field() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"nested": {"model": "standard"}});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    opts = ExtractionConfig(nested=NestedConfig(model="standard"))"#],
        "nested struct field must be constructed with its own class, got: {bindings:?}"
    );
}

/// Batch-call counterpart of the nested-config regression above: a "batch" argument passes
/// an array of typed items via `element_type` (see `emit_python_typed_instance`), and each
/// item's own nested struct fields must resolve the same way a single top-level config does.
#[test]
fn emit_json_object_arg_batch_mode_constructs_nested_struct_field_in_each_item() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let item_type = TypeDef {
        name: "SampleFileItem".to_string(),
        rust_path: "demo::SampleFileItem".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![item_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!([{"nested": {"model": "standard"}}]);
    let element_type = Some("SampleFileItem".to_string());
    let spec = ConstructorSpec {
        options_type: None,
        options_via: "kwargs",
        element_type: &element_type,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "items", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    items = [SampleFileItem(nested=NestedConfig(model="standard"))]"#],
        "each batch item's nested struct field must be constructed with its own class, got: {bindings:?}"
    );
}

/// Map counterpart of the nested-config regression above: a field typed `Map<String,
/// NestedConfig>` must construct every value with its own class, not fall through to a raw
/// dict-of-dicts. Before `resolve_field_map_value_struct_type`/`render_nested_map_field_value`
/// existed, `render_kwarg_field_value` never inspected `TypeRef::Map`, so this exact shape
/// fell all the way through to `json_to_python_literal` and emitted a plain dict.
#[test]
fn emit_json_object_arg_kwargs_mode_constructs_nested_struct_map_values() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "profiles".to_string(),
            ty: TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("NestedConfig".to_string())),
            ),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"profiles": {"first": {"model": "standard"}}});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    opts = ExtractionConfig(profiles={"first": NestedConfig(model="standard")})"#],
        "map values must be constructed with their own class, got: {bindings:?}"
    );
}

/// A map value type that is not itself a known struct (e.g. `Map<String, String>`) must fall
/// through -- [`render_value_for_type_ref`] returns `None` so the caller reaches the plain-dict
/// fallback unchanged.
#[test]
fn render_value_for_type_ref_returns_none_for_non_struct_map_value() {
    use crate::core::ir::TypeRef;

    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let type_ref = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    let value = serde_json::json!({"a": "b"});
    let mut used_types = UsedTypeNames::default();

    let result = render_value_for_type_ref(&type_ref, &value, "", context, &mut used_types);
    assert!(result.is_none(), "got: {result:?}");
}

/// `Optional<Map<String, Struct>>` must unwrap the same way `Optional<Vec<Struct>>` does --
/// direct coverage of the `Optional` arm wrapping the `Map` arm in
/// [`render_value_for_type_ref`].
#[test]
fn render_value_for_type_ref_unwraps_optional_map_of_structs() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let type_ref = TypeRef::Optional(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("NestedConfig".to_string())),
    )));
    let value = serde_json::json!({"first": {"model": "standard"}});
    let mut used_types = UsedTypeNames::default();

    let result = render_value_for_type_ref(&type_ref, &value, "", context, &mut used_types);
    assert_eq!(result.as_deref(), Some(r#"{"first": NestedConfig(model="standard")}"#));
}

/// Uniform-recursion regression, the "mixed null/object" control: a `Map<String,
/// Optional<Struct>>` field where individual entries may independently be null or an object --
/// neither shape the former shape-by-shape dispatch enumerated (it only handled a direct
/// `Map<K, Struct>` value, not one whose values are themselves `Optional`). Each entry must
/// resolve on its own: a null entry renders `None`, an object entry still constructs its own
/// class -- one null entry must not revert the whole map to a raw-dict fallback.
#[test]
fn emit_json_object_arg_kwargs_mode_handles_mixed_null_and_object_map_values() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "profiles".to_string(),
            ty: TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Optional(Box::new(TypeRef::Named("NestedConfig".to_string())))),
            ),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"profiles": {"first": {"model": "standard"}, "second": null}});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    opts = ExtractionConfig(profiles={"first": NestedConfig(model="standard"), "second": None})"#],
        "a null map entry must render as None without reverting the whole map to a raw dict, got: {bindings:?}"
    );
}

/// Uniform-recursion regression, the "genuinely nested containers" control: a `Map<String,
/// Vec<Struct>>` field -- a combination the former shape-by-shape dispatch never enumerated (it
/// handled a direct `Map<K, Struct>` value and a top-level `Vec<Struct>` field, but not a map
/// whose *values* are themselves arrays of structs). This falls out of the same recursion with
/// no per-combination code.
#[test]
fn emit_json_object_arg_kwargs_mode_constructs_nested_struct_map_of_vec_values() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "groups".to_string(),
            ty: TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Vec(Box::new(TypeRef::Named("NestedConfig".to_string())))),
            ),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"groups": {"team": [{"model": "standard"}, {"model": "pro"}]}});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);

    assert!(done);
    assert_eq!(
        bindings,
        [
            r#"    opts = ExtractionConfig(groups={"team": [NestedConfig(model="standard"), NestedConfig(model="pro")]})"#
        ],
        "a map value's own Vec<Struct> field must construct each element with its own class, got: {bindings:?}"
    );
}

/// Direct unit test on the `$mock_url` leaf renderer: a JSON-pointer path must become a chain
/// of Python subscripts on the runtime holder.
#[test]
fn runtime_dict_index_expression_builds_subscript_chain_from_pointer() {
    assert_eq!(
        runtime_dict_index_expression("opts_data", "/profiles/first/model"),
        r#"opts_data["profiles"]["first"]["model"]"#
    );
}

/// Array-index segments carrying the private `~2` tag must render as bare integer subscripts,
/// not quoted string keys -- `json.loads` turns a JSON array into a Python list.
#[test]
fn runtime_dict_index_expression_renders_array_index_as_integer_subscript() {
    assert_eq!(
        runtime_dict_index_expression("opts_data", "/items/~20/model"),
        r#"opts_data["items"][0]["model"]"#
    );
}

#[test]
fn runtime_dict_index_expression_preserves_empty_map_key_segments() {
    assert_eq!(
        runtime_dict_index_expression("opts_data", "/profiles//url"),
        r#"opts_data["profiles"][""]["url"]"#
    );
    assert_eq!(
        runtime_dict_index_expression("opts_data", "/profiles/"),
        r#"opts_data["profiles"][""]"#
    );
    assert_eq!(runtime_dict_index_expression("opts_data", ""), "opts_data");
    assert_eq!(
        runtime_dict_index_expression("opts_data", "/a~1b/~0/~020"),
        r#"opts_data["a/b"]["~"]["~20"]"#
    );
}

#[test]
fn canonical_docs_pointer_removes_only_runtime_array_tags() {
    assert_eq!(canonical_docs_pointer("/items/~20/nested"), "/items/0/nested");
    assert_eq!(canonical_docs_pointer("/profiles/~020/nested"), "/profiles/~020/nested");
}

/// Regression for the `$mock_url` short-circuit defect: before this fix,
/// `emit_json_object_arg_with_mock_url` never received `context` at all, so a nested struct
/// field inside a `$mock_url` fixture always fell back to `opts_type(**json.loads(...))` --
/// dict-typed nested fields, not the generated pyclass. The nested constructor must still
/// appear, with its leaf pulling the runtime-substituted value out of the parsed dict instead
/// of embedding the still placeholder-laden literal.
#[test]
fn emit_json_object_arg_with_mock_url_constructs_nested_struct_field_from_runtime_dict() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!({"nested": {"url": "$mock_url/path"}});
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &None,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &enums,
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };
    let done = emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context);

    assert!(done);
    let rendered = bindings.join("\n");
    assert!(
        rendered.contains(r#"NestedConfig(url=opts_data["nested"]["url"])"#),
        "the nested struct field must still be constructed with its own class under $mock_url \
         substitution, pulling the substituted value from the runtime dict, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("**json.loads"),
        "must not fall back to unpacking a raw dict, got:\n{rendered}"
    );
}

fn nested_profile_type_defs() -> Vec<crate::core::ir::TypeDef> {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    vec![
        TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "demo::ExtractionConfig".to_string(),
            fields: vec![FieldDef {
                name: "profiles".to_string(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("NestedConfig".to_string())),
                ),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "NestedConfig".to_string(),
            rust_path: "demo::NestedConfig".to_string(),
            fields: vec![FieldDef {
                name: "url".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn render_mock_url_profile(map_key: &str) -> String {
    let type_defs = nested_profile_type_defs();
    let mut profiles = serde_json::Map::new();
    profiles.insert(map_key.to_string(), serde_json::json!({"url": "$mock_url/path"}));
    let value = serde_json::json!({"profiles": profiles});
    let mut bindings = Vec::new();
    let mut expressions = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut expressions,
    };
    let element_type = None;
    let spec = ConstructorSpec {
        options_type: Some("ExtractionConfig"),
        options_via: "kwargs",
        element_type: &element_type,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &[],
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };

    assert!(emit_json_object_arg(&mut sink, &value, "opts", &spec, &mock, context));
    bindings.join("\n")
}

#[test]
fn emit_json_object_arg_with_mock_url_keeps_numeric_map_keys_as_strings() {
    let rendered = render_mock_url_profile("0");
    assert!(
        rendered.contains(r#"NestedConfig(url=opts_data["profiles"]["0"]["url"])"#),
        "numeric map keys must remain string subscripts, got:\n{rendered}"
    );
}

#[test]
fn emit_json_object_arg_with_mock_url_preserves_empty_map_keys() {
    let rendered = render_mock_url_profile("");
    assert!(
        rendered.contains(r#"NestedConfig(url=opts_data["profiles"][""]["url"])"#),
        "empty map keys must remain string subscripts, got:\n{rendered}"
    );
}

#[test]
fn emit_json_object_arg_with_mock_url_constructs_typed_array_elements() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let item_type = TypeDef {
        name: "BatchItem".to_string(),
        rust_path: "demo::BatchItem".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let nested_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![item_type, nested_type];
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!([{"nested": {"url": "$mock_url/path"}}]);
    let element_type = Some("BatchItem".to_string());
    let spec = ConstructorSpec {
        options_type: None,
        options_via: "kwargs",
        element_type: &element_type,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &type_defs,
        enums: &[],
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };

    assert!(emit_json_object_arg(&mut sink, &value, "items", &spec, &mock, context));
    let rendered = bindings.join("\n");
    assert!(
        rendered.contains(r#"BatchItem(nested=NestedConfig(url=items_data[0]["nested"]["url"]))"#),
        "mock-url arrays must construct typed elements and nested classes, got:\n{rendered}"
    );
}

#[test]
fn emit_json_object_arg_with_mock_url_preserves_explicit_dict_array_mode() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut exprs,
    };
    let value = serde_json::json!([{"url": "$mock_url/path"}]);
    let element_type = Some("BatchItem".to_string());
    let spec = ConstructorSpec {
        options_type: None,
        options_via: "dict",
        element_type: &element_type,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs: &[],
        enums: &[],
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };

    assert!(emit_json_object_arg(&mut sink, &value, "items", &spec, &mock, context));
    let rendered = bindings.join("\n");
    assert!(rendered.contains("items = json.loads(items_json)"), "got:\n{rendered}");
    assert!(
        !rendered.contains("BatchItem("),
        "dict mode must remain untyped, got:\n{rendered}"
    );
}

#[test]
fn resolve_field_enum_type_detects_enum_field() {
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    let enum_def = EnumDef {
        name: "TierStrategy".to_string(),
        rust_path: "module::TierStrategy".to_string(),
        variants: vec![EnumVariant {
            name: "Auto".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let type_def = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "module::ConversionOptions".to_string(),
        fields: vec![FieldDef {
            name: "tier_strategy".to_string(),
            ty: TypeRef::Named("TierStrategy".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums = vec![enum_def];
    let type_defs = vec![type_def];

    let result = resolve_field_enum_type("tier_strategy", Some("ConversionOptions"), &type_defs, &enums);
    assert_eq!(result, Some("TierStrategy".to_string()));
}

#[test]
fn resolve_field_enum_type_returns_none_for_non_enum_field() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let type_def = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "module::ConversionOptions".to_string(),
        fields: vec![FieldDef {
            name: "timeout".to_string(),
            ty: TypeRef::Named("u64".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums: Vec<crate::core::ir::EnumDef> = vec![];
    let type_defs = vec![type_def];

    let result = resolve_field_enum_type("timeout", Some("ConversionOptions"), &type_defs, &enums);
    assert_eq!(result, None);
}
