use super::{make_test_type, simple_type_mapper};
use crate::codegen::config_gen::{
    gen_csharp_record, gen_extendr_kwargs_constructor, gen_go_functional_options, gen_java_builder,
    gen_magnus_kwargs_constructor, gen_napi_defaults_constructor, gen_php_kwargs_constructor,
    gen_pyo3_kwargs_constructor, gen_rustler_kwargs_constructor,
};
use crate::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};

#[test]
fn test_gen_pyo3_kwargs_constructor() {
    let typ = make_test_type();
    let output = gen_pyo3_kwargs_constructor(&typ, &|tr: &TypeRef| match tr {
        TypeRef::Primitive(p) => format!("{:?}", p),
        TypeRef::String | TypeRef::Char => "str".to_string(),
        _ => "Any".to_string(),
    });

    assert!(output.contains("#[new]"));
    assert!(output.contains("#[pyo3(signature = ("));
    assert!(output.contains("timeout=30"));
    assert!(output.contains("enabled=True"));
    assert!(output.contains("name=\"default\""));
    assert!(output.contains("fn new("));
}

#[test]
fn test_gen_napi_defaults_constructor() {
    let typ = make_test_type();
    let output = gen_napi_defaults_constructor(&typ, &|tr: &TypeRef| match tr {
        TypeRef::Primitive(p) => format!("{:?}", p),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        _ => "Value".to_string(),
    });

    assert!(output.contains("pub fn new(mut env: napi::Env, obj: napi::Object)"));
    assert!(output.contains("timeout"));
    assert!(output.contains("enabled"));
    assert!(output.contains("name"));
}

#[test]
fn test_gen_go_functional_options() {
    let typ = make_test_type();
    let output = gen_go_functional_options(&typ, &|tr: &TypeRef| match tr {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::U64 => "uint64".to_string(),
            PrimitiveType::Bool => "bool".to_string(),
            _ => "interface{}".to_string(),
        },
        TypeRef::String | TypeRef::Char => "string".to_string(),
        _ => "interface{}".to_string(),
    });

    assert!(output.contains("type Config struct {"));
    assert!(output.contains("type ConfigOption func(*Config)"));
    assert!(output.contains("func WithConfigTimeout(val uint64) ConfigOption"));
    assert!(output.contains("func WithConfigEnabled(val bool) ConfigOption"));
    assert!(output.contains("func WithConfigName(val string) ConfigOption"));
    assert!(output.contains("func NewConfig(opts ...ConfigOption) *Config"));
}

#[test]
fn test_gen_java_builder() {
    let typ = make_test_type();
    let output = gen_java_builder(&typ, "dev.test", &|tr: &TypeRef| match tr {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::U64 => "long".to_string(),
            PrimitiveType::Bool => "boolean".to_string(),
            _ => "int".to_string(),
        },
        TypeRef::String | TypeRef::Char => "String".to_string(),
        _ => "Object".to_string(),
    });

    assert!(output.contains("package dev.test;"));
    assert!(output.contains("public class ConfigBuilder"));
    assert!(output.contains("withTimeout"));
    assert!(output.contains("withEnabled"));
    assert!(output.contains("withName"));
    assert!(output.contains("public Config build()"));
}

#[test]
fn test_gen_csharp_record() {
    let typ = make_test_type();
    let output = gen_csharp_record(&typ, "MyNamespace", &|tr: &TypeRef| match tr {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::U64 => "ulong".to_string(),
            PrimitiveType::Bool => "bool".to_string(),
            _ => "int".to_string(),
        },
        TypeRef::String | TypeRef::Char => "string".to_string(),
        _ => "object".to_string(),
    });

    assert!(output.contains("namespace MyNamespace;"));
    assert!(output.contains("public record Config"));
    assert!(output.contains("public ulong Timeout"));
    assert!(output.contains("public bool Enabled"));
    assert!(output.contains("public string Name"));
    assert!(output.contains("init;"));
}
fn test_gen_magnus_kwargs_constructor_hash_path_for_many_fields() {
    let mut fields: Vec<FieldDef> = (0..16)
        .map(|i| FieldDef {
            name: format!("field_{i}"),
            ty: TypeRef::Primitive(PrimitiveType::U32),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        })
        .collect();
    fields[0].optional = true;

    let typ = TypeDef {
        name: "BigConfig".to_string(),
        rust_path: "crate::BigConfig".to_string(),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("Option<magnus::RHash>"),
        "should accept RHash via scan_args"
    );
    assert!(output.contains("ruby.to_symbol("), "should use symbol lookup");
    assert!(
        output.contains("field_0: kwargs.get(ruby.to_symbol(\"field_0\")).and_then(|v|"),
        "optional field should use and_then"
    );
    assert!(
        output.contains("field_0:").then_some(()).is_some(),
        "field_0 should appear in output"
    );
}

#[test]
fn test_gen_php_kwargs_constructor_basic() {
    let typ = make_test_type();
    let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("pub fn __construct("),
        "should use PHP constructor name"
    );
    assert!(
        output.contains("timeout: Option<u64>"),
        "timeout param should be Option<u64>"
    );
    assert!(
        output.contains("enabled: Option<bool>"),
        "enabled param should be Option<bool>"
    );
    assert!(
        output.contains("name: Option<String>"),
        "name param should be Option<String>"
    );
    assert!(output.contains("-> Self {"), "should return Self");
    assert!(
        output.contains("timeout: timeout.unwrap_or(30),"),
        "should apply int default for timeout"
    );
    assert!(
        output.contains("enabled: enabled.unwrap_or(true),"),
        "should apply bool default for enabled"
    );
    assert!(
        output.contains("name: name.unwrap_or(\"default\".to_string()),"),
        "should apply string default for name"
    );
}

#[test]
fn test_gen_php_kwargs_constructor_optional_field_passthrough() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "tag".to_string(),
        ty: TypeRef::String,
        optional: true,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("tag,"),
        "optional field should be passed through directly"
    );
    assert!(!output.contains("tag.unwrap"), "optional field should not call unwrap");
}

#[test]
fn test_gen_php_kwargs_constructor_unwrap_or_default_for_primitive() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "retries".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("retries: retries.unwrap_or_default(),"),
        "primitive with no default should use unwrap_or_default"
    );
}

#[test]
fn test_gen_rustler_kwargs_constructor_basic() {
    let typ = make_test_type();
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("pub fn new(opts: std::collections::HashMap<String, rustler::Term>)"),
        "should accept HashMap of Terms"
    );
    assert!(output.contains("Self {"), "should construct Self");
    assert!(
        output.contains("timeout: opts.get(\"timeout\").and_then(|t| t.decode().ok()).unwrap_or(30),"),
        "should apply int default for timeout"
    );
    assert!(
        output.contains("enabled: opts.get(\"enabled\").and_then(|t| t.decode().ok()).unwrap_or(true),"),
        "should apply bool default for enabled"
    );
}

#[test]
fn test_gen_rustler_kwargs_constructor_optional_field() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "extra".to_string(),
        ty: TypeRef::String,
        optional: true,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("extra: opts.get(\"extra\").and_then(|t| t.decode().ok()),"),
        "optional field should decode without unwrap"
    );
}

#[test]
fn test_gen_rustler_kwargs_constructor_skips_binding_excluded_fields() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "internal_cache".to_string(),
        ty: TypeRef::String,
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
        binding_excluded: true,
        binding_exclusion_reason: Some("internal implementation detail".to_string()),
        original_type: None,
    });

    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        !output.contains("internal_cache"),
        "binding-excluded fields must not be exposed in Rustler constructors; got:\n{output}"
    );
}

#[test]
fn test_gen_rustler_kwargs_constructor_named_type_uses_unwrap_or_default() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "inner".to_string(),
        ty: TypeRef::Named("InnerConfig".to_string()),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("inner: opts.get(\"inner\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
        "Named type with no default should use unwrap_or_default"
    );
}

#[test]
fn test_gen_rustler_kwargs_constructor_string_field_uses_unwrap_or_default() {
    let mut typ = make_test_type();
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("name: opts.get(\"name\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
        "String field with quoted default should use unwrap_or_default"
    );
    typ.fields.push(FieldDef {
        name: "label".to_string(),
        ty: TypeRef::String,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output2 = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output2.contains("label: opts.get(\"label\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
        "String field with no default should use unwrap_or_default"
    );
}

#[test]
fn test_gen_extendr_kwargs_constructor_basic() {
    let typ = make_test_type();
    let empty_enums = ahash::AHashSet::new();
    let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);

    assert!(output.contains("#[extendr]"), "should have extendr attribute");
    assert!(
        output.contains("pub fn new_config("),
        "function name should be lowercase type name"
    );
    assert!(
        output.contains("timeout: Option<u64>"),
        "should accept timeout as Option<u64>: {output}"
    );
    assert!(
        output.contains("enabled: Option<bool>"),
        "should accept enabled as Option<bool>: {output}"
    );
    assert!(
        output.contains("name: Option<String>"),
        "should accept name as Option<String>: {output}"
    );
    assert!(output.contains("-> Config {"), "should return Config");
    assert!(
        output.contains("let mut __out = <Config>::default();"),
        "should base on Default impl: {output}"
    );
    assert!(
        output.contains("if let Some(v) = timeout { __out.timeout = v; }"),
        "should overlay caller-provided timeout"
    );
    assert!(
        output.contains("if let Some(v) = enabled { __out.enabled = v; }"),
        "should overlay caller-provided enabled"
    );
    assert!(
        output.contains("if let Some(v) = name { __out.name = v; }"),
        "should overlay caller-provided name"
    );
}

#[test]
fn test_gen_extendr_kwargs_constructor_uses_option_for_all_fields() {
    // extendr 0.9 only supports defaults via the `#[extendr(default = "...")]`
    let typ = make_test_type();
    let empty_enums = ahash::AHashSet::new();
    let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);
    assert!(
        !output.contains("= TRUE") && !output.contains("= FALSE") && !output.contains("= \"default\""),
        "constructor must not use Rust-syntax param defaults: {output}"
    );
}

#[test]
fn test_gen_go_functional_options_skips_tuple_fields() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "_0".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_go_functional_options(&typ, &simple_type_mapper);
    assert!(
        !output.contains("_0"),
        "tuple field _0 should be filtered out from Go output"
    );
}

#[test]
fn test_gen_magnus_hash_constructor_generic_type_prefix() {
    let fields: Vec<FieldDef> = (0..16)
        .map(|i| FieldDef {
            name: format!("field_{i}"),
            ty: if i == 0 {
                TypeRef::Vec(Box::new(TypeRef::String))
            } else {
                TypeRef::Primitive(PrimitiveType::U32)
            },
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        })
        .collect();
    let typ = TypeDef {
        name: "WideConfig".to_string(),
        rust_path: "crate::WideConfig".to_string(),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("<Vec<String>>::try_convert"),
        "generic types should use UFCS angle-bracket prefix: {output}"
    );
}

#[test]
fn test_magnus_hash_constructor_no_double_option_when_ty_is_optional() {
    let field = FieldDef {
        name: "max_depth".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
        optional: true,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let mut fields: Vec<FieldDef> = (0..15)
        .map(|i| FieldDef {
            name: format!("field_{i}"),
            ty: TypeRef::Primitive(PrimitiveType::U32),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        })
        .collect();
    fields.push(field);
    let typ = TypeDef {
        name: "UpdateConfig".to_string(),
        rust_path: "crate::UpdateConfig".to_string(),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        !output.contains("Option<Option<"),
        "hash constructor must not emit double Option: {output}"
    );
    assert!(
        output.contains("i64::try_convert"),
        "hash constructor should call inner-type::try_convert, not Option<T>::try_convert: {output}"
    );
}
