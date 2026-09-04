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
    let mut fields: Vec<FieldDef> = (0..16)
        .map(|i| FieldDef {
            version: Default::default(),
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
    assert!(
        output.contains(
            "field_0: match kwargs.get(ruby.to_symbol(\"field_0\")) { Some(v) => Some(u32::try_convert(v).map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_type_error(), format!(\"invalid value for `field_0`: {}\", e)))?), None => None },"
        ),
        "optional field must default to None when the key is absent, and raise a TypeError \
         (never silently default) when the key is present but fails to convert; got:\n{output}"
    );
    assert!(
        output.contains("field_0:").then_some(()).is_some(),
        "field_0 should appear in output"
    );
}

/// A field with a real `#[serde(default = "Type::static_fn")]` default (resolved to
/// `PublicFunctionCall` once the fn is confirmed to be a public static method on a mirrored
/// type — see `resolve_public_default_functions`) must NOT be treated as a required Ruby
/// keyword argument, even though its type is `Named`. Covers the "applies" half of the
/// magnus fix: reverting the `!has_callable_default` guard on the required-field branch makes
/// this fail (the field falls back into the `ok_or_else("missing required field")` error path).
#[test]
fn test_gen_magnus_kwargs_constructor_named_function_call_default_is_not_required() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "ssrf".to_string(),
        ty: TypeRef::Named("SsrfPolicy".to_string()),
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "crawlberg::SsrfPolicy::from_env".to_string(),
        )),
        ..Default::default()
    });
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        !output.contains("missing required field: ssrf"),
        "a field with a real serde default must not become a required argument; got:\n{output}"
    );
    assert!(
        output.contains("None => crawlberg::SsrfPolicy::from_env()"),
        "expected the constructor to call the real default fn; got:\n{output}"
    );
}

/// The `#[serde(default = "...")]` function returns the field's core type; Magnus mirrors
/// `Named` types into its own `#[magnus::wrap]` struct, a distinct Rust type from the core one
/// under the same short name, so the call needs `.into()` to become the type the field holds.
/// Covers the "converts" half of the magnus fix: reverting the `.into()` append (while keeping
/// the required-field guard fix) makes this fail while
/// `test_gen_magnus_kwargs_constructor_named_function_call_default_is_not_required` still passes.
#[test]
fn test_gen_magnus_kwargs_constructor_named_function_call_default_converts_into_wrapper() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "ssrf".to_string(),
        ty: TypeRef::Named("SsrfPolicy".to_string()),
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "crawlberg::SsrfPolicy::from_env".to_string(),
        )),
        ..Default::default()
    });
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("ssrf: match kwargs.get(ruby.to_symbol(\"ssrf\")) { Some(v) => SsrfPolicy::try_convert(v)")
            && output.contains("None => crawlberg::SsrfPolicy::from_env().into() },"),
        "expected the default to be converted into the wrapper type via .into(); got:\n{output}"
    );
}

/// A field that is legitimately absent from `kwargs` (the Ruby caller never passed the key)
/// must still fall back to its default — this is the "absent" half of the fix, and must keep
/// working exactly as before the fix. `ngram_range`-shaped: a `Named` type whose
/// `#[serde(default)]` resolves to `DefaultValue::Empty`, which routes through the
/// `use_unwrap_or_default` branch. ~keep
#[test]
fn test_gen_magnus_kwargs_constructor_absent_field_still_defaults() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        typed_default: Some(DefaultValue::Empty),
        ..make_field("ngram_range", TypeRef::Named("NgramRange".to_string()))
    });
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains(
            "ngram_range: match kwargs.get(ruby.to_symbol(\"ngram_range\")) { Some(v) => NgramRange::try_convert(v)"
        ),
        "expected the present-value arm to convert via try_convert; got:\n{output}"
    );
    assert!(
        output.contains("None => Default::default() },"),
        "a key absent from kwargs must still default, not raise; got:\n{output}"
    );
}

/// A field that IS present in `kwargs` but fails to convert (wrong Ruby type, e.g. a String
/// where a Hash was expected) must raise a Ruby `TypeError` naming the field, never silently
/// fall back to the default. This is the regression covered by the reported defect: the old
/// `.and_then(|v| T::try_convert(v).ok()).unwrap_or_default()` shape could not distinguish
/// "absent" from "present but invalid" and defaulted in both cases. ~keep
#[test]
fn test_gen_magnus_kwargs_constructor_present_invalid_field_raises() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        typed_default: Some(DefaultValue::Empty),
        ..make_field("ngram_range", TypeRef::Named("NgramRange".to_string()))
    });
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        !output.contains("try_convert(v).ok()"),
        "a conversion failure must not be silently discarded via .ok(); got:\n{output}"
    );
    assert!(
        output.contains(
            "map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_type_error(), format!(\"invalid value for `ngram_range`: {}\", e)))?"
        ),
        "a present-but-invalid value must raise a TypeError naming the field; got:\n{output}"
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });
    let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);
    // Anchored on the whole initializer line: `output.contains("tag,")` also matches
    // `tag: tag,`, so it cannot tell a passthrough from `clippy::redundant_field_names`. The
    // initializer lines are emitted flush left; the formatter indents them later. ~keep
    assert!(
        output.contains("\ntag,\n"),
        "optional field should be passed through as field-init shorthand; got:\n{output}"
    );
    assert!(
        !output.contains("tag: tag"),
        "optional passthrough must not emit a redundant field name; got:\n{output}"
    );
    assert!(!output.contains("tag.unwrap"), "optional field should not call unwrap");
}

#[test]
fn test_gen_php_kwargs_constructor_unwrap_or_default_for_primitive() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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

/// A `String` field's own default must be spelled out; a `String` field with no default falls
/// back to the type's.
///
/// The first half previously asserted the opposite, because the branch decided what kind of
/// default it was looking at by sniffing the *rendered Rust* for a leading quote — and
/// `StringLiteral("default")` renders as `"default".to_string()`, which starts with one. An
/// ordinary `String` field was therefore misread as an enum-variant default and collapsed to
/// `.unwrap_or_default()`, so `Config.new(%{})` produced `""` where the source crate says
/// `"default"`. The old assertion named the mechanism ("quoted default") rather than the
/// contract, which is why it read as correct.
#[test]
fn test_gen_rustler_kwargs_constructor_string_field_keeps_its_literal_default() {
    let mut typ = make_test_type();
    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
    assert!(
        output.contains("name: opts.get(\"name\").and_then(|t| t.decode().ok()).unwrap_or(\"default\".to_string()),"),
        "a String field's own default must survive into the constructor; got:\n{output}"
    );
    typ.fields.push(FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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

/// The branch's real purpose, pinned so it survives the string-sniff removal. An `EnumVariant`
/// default on a `String`-typed field renders as the variant's *wire* string, which is not a value
/// the binding struct's field can be initialised from here, so it defers to the type's `Default`.
/// Without this test, `defers_to_field_type_default` reads as if only the `Named` arm mattered.
#[test]
fn test_gen_rustler_kwargs_constructor_enum_variant_on_string_field_defers_to_default() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "mode".to_string(),
        ty: TypeRef::String,
        typed_default: Some(DefaultValue::EnumVariant("Fast".to_string())),
        ..Default::default()
    });

    let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);

    assert!(
        output.contains("mode: opts.get(\"mode\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
        "an enum-variant default on a String field has no literal here; got:\n{output}"
    );
}
