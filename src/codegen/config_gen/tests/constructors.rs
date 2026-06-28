use super::*;

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

#[test]
fn test_gen_magnus_kwargs_constructor_hash_path_for_many_fields() {
    // Build a type with 16 fields (> MAGNUS_MAX_ARITY = 15) to force hash path
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
    // Make one field optional to exercise that branch in the hash constructor
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
        has_private_fields: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("Option<magnus::RHash>"),
        "should accept RHash via scan_args"
    );
    assert!(output.contains("ruby.to_symbol("), "should use symbol lookup");
    // Optional field uses and_then without unwrap_or
    assert!(
        output.contains("field_0: kwargs.get(ruby.to_symbol(\"field_0\")).and_then(|v|"),
        "optional field should use and_then"
    );
    assert!(
        output.contains("field_0:").then_some(()).is_some(),
        "field_0 should appear in output"
    );
}

// -------------------------------------------------------------------------
// gen_php_kwargs_constructor
// -------------------------------------------------------------------------

#[test]
fn test_gen_php_kwargs_constructor_basic() {
    let typ = make_test_type();
    let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("pub fn __construct("),
        "should use PHP constructor name"
    );
    // All params are Option<T>
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

// -------------------------------------------------------------------------
// gen_rustler_kwargs_constructor
// -------------------------------------------------------------------------

#[test]
fn test_gen_rustler_kwargs_constructor_basic() {
    let typ = make_test_type();
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("pub fn new(opts: std::collections::HashMap<String, rustler::Term>)"),
        "should accept HashMap of Terms"
    );
    assert!(output.contains("Self {"), "should construct Self");
    // timeout has IntLiteral(30) — explicit unwrap_or
    assert!(
        output.contains("timeout: opts.get(\"timeout\").and_then(|t| t.decode().ok()).unwrap_or(30),"),
        "should apply int default for timeout"
    );
    // enabled has BoolLiteral(true) — explicit unwrap_or
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
    // A String field with a StringLiteral default contains "::", triggering the
    // is_enum_variant_default check — should fall back to unwrap_or_default().
    let mut typ = make_test_type();
    // 'name' field in make_test_type() has StringLiteral("default") — verify it
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("name: opts.get(\"name\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
        "String field with quoted default should use unwrap_or_default"
    );
    // Also verify a plain string field (no default) also falls through to unwrap_or_default
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
