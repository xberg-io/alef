use super::gen_record_type;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

pub(super) fn field(name: &str, ty: TypeRef) -> FieldDef {
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
        original_type: None,
        cfg: None,
        typed_default: None,
        core_wrapper: Default::default(),
        vec_inner_core_wrapper: Default::default(),
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

pub(super) fn record_type(fields: Vec<FieldDef>) -> TypeDef {
    named_record_type("RenderOptions", fields)
}

pub(super) fn named_record_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: format!("demo::{name}"),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn external_string_union_default_constructs_the_nested_unit_record() {
    let enum_def = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![
            EnumVariant {
                name: "Plain".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![field("_0", TypeRef::String)],
                serde_untagged: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };
    let mut output_format = field("output_format", TypeRef::Named("OutputFormat".into()));
    output_format.default = Some("/* serde(default) */".into());
    output_format.typed_default = Some(DefaultValue::EnumVariant("Plain".into()));
    let typ = named_record_type("ExtractionConfig", vec![output_format]);
    let tagged_union_enums = if crate::backends::csharp::gen_bindings::enums::emits_tagged_union(&enum_def) {
        HashSet::from(["OutputFormat".to_string()])
    } else {
        HashSet::new()
    };

    let emitted = gen_record_type(
        &typ,
        &[],
        "Xberg",
        "xberg",
        &HashSet::from(["OutputFormat".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "XbergException",
        &HashSet::new(),
        &tagged_union_enums,
        &HashSet::new(),
    );

    assert!(emitted.contains("= new OutputFormat.Plain();"), "got:\n{emitted}");
    assert!(!emitted.contains("= OutputFormat.Plain;"), "got:\n{emitted}");
}

#[test]
fn record_type_maps_configured_bridge_alias_to_trait_interface() {
    let typ = record_type(vec![
        field(
            "walker",
            TypeRef::Optional(Box::new(TypeRef::Named("WalkerHandle".to_string()))),
        ),
        field("visitor_count", TypeRef::Primitive(PrimitiveType::U32)),
    ]);
    let bridge = TraitBridgeConfig {
        trait_name: "XmlWalker".to_string(),
        type_alias: Some("WalkerHandle".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("walker".to_string()),
        ..TraitBridgeConfig::default()
    };
    let aliases = HashSet::from(["WalkerHandle".to_string()]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &aliases,
        &[bridge],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(code.contains("public IXmlWalker? Walker { get; init; } = null;"));
    assert!(code.contains("public uint VisitorCount"));
    assert!(!code.contains("IHtmlVisitor"));
    assert!(!code.contains("VisitorHandle"));
}

/// A record property and a `Self`-returning builder method with the same name both land in the
/// record body, and C# rejects the duplicate member name with `CS0102`. The property is
/// emitted first and wins.
#[test]
fn record_type_skips_method_whose_name_collides_with_a_property() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let mut typ = record_type(vec![field("providers", TypeRef::String)]);
    typ.methods = vec![MethodDef {
        name: "providers".to_string(),
        return_type: TypeRef::Named("RenderOptions".to_string()),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    }];

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("string Providers { get; init; }"),
        "the property must still be emitted:\n{code}"
    );
    let methods = code.matches("public RenderOptions Providers(").count();
    assert_eq!(
        methods, 0,
        "the same-named method must be skipped, found {methods}:\n{code}"
    );
}

/// Regression (Defect 1 / Defect 3): a required `Duration` field — no `#[serde(default)]` —
/// must be `required ulong`, not a nullable `ulong?` defaulted to `null`. Previously
/// `Duration` was unconditionally nullable regardless of whether the field was actually
/// wire-optional, so `new Foo { }` compiled clean and then serialized `null` against a
/// non-`Option` Rust field.
#[test]
fn record_type_required_duration_field_is_required_ulong() {
    let typ = record_type(vec![field("window", TypeRef::Duration)]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("[JsonConverter(typeof(DurationMillisJsonConverter))]"),
        "expected the non-nullable Duration converter:\n{code}"
    );
    assert!(
        code.contains("public required ulong Window { get; init; }"),
        "expected a required, non-nullable ulong property:\n{code}"
    );
    assert!(
        !code.contains("ulong?"),
        "a required Duration field must not be nullable:\n{code}"
    );
}

/// A `Duration` field that genuinely has `#[serde(default...)]` (modeled here via
/// `field.default`) stays nullable with a `null` default — the Rust side tolerates the key
/// being absent — but must carry the nullable-safe converter, not the non-nullable one.
#[test]
fn record_type_duration_field_with_real_default_is_nullable() {
    let mut window = field("window", TypeRef::Duration);
    window.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![window]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("[JsonConverter(typeof(NullableDurationMillisJsonConverter))]"),
        "expected the nullable Duration converter:\n{code}"
    );
    assert!(
        code.contains("public ulong? Window { get; init; } = null;"),
        "expected a nullable ulong property defaulted to null:\n{code}"
    );
}

/// A genuinely `Option<Duration>` field (not merely defaulted) also uses the nullable
/// converter and a `ulong?` type, exercising the `field.optional` branch specifically.
#[test]
fn record_type_optional_duration_field_is_nullable() {
    let mut window = field("window", TypeRef::Duration);
    window.optional = true;
    let typ = record_type(vec![window]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("[JsonConverter(typeof(NullableDurationMillisJsonConverter))]"),
        "expected the nullable Duration converter:\n{code}"
    );
    assert!(
        code.contains("public ulong? Window { get; init; } = null;"),
        "expected a nullable ulong property defaulted to null:\n{code}"
    );
}

/// Regression (Defect 2): a required field whose type is a Rust `enum` (e.g. sealed content
/// union) on a struct that derives `Default` must stay `required`, not nullable. `Empty` is
/// what the extractor puts on every field of a `Default`-deriving struct
/// (`extract::extractor::types` and `extract::extractor::defaults`) and it means "that type's
/// own `Default`" — a value C# cannot spell for a `Named` field. Resolving it to `null` and
/// widening the property to `UserContent?` let `new UserMessage { Name = "alice" }` compile
/// clean and then serialize `"content":null` against a required Rust field.
#[test]
fn record_type_required_enum_field_in_default_struct_stays_required() {
    let mut content = field("content", TypeRef::Named("UserContent".to_string()));
    content.typed_default = Some(DefaultValue::Empty);
    let mut typ = record_type(vec![content]);
    typ.has_default = true;
    let enum_names = HashSet::from(["UserContent".to_string()]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &enum_names,
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("public required UserContent Content { get; init; }"),
        "expected a required, non-nullable UserContent property:\n{code}"
    );
    assert!(
        !code.contains("UserContent?"),
        "a required field must not be nullable just because the struct derives Default:\n{code}"
    );
}

pub(super) fn render_plain_record(typ: &TypeDef) -> String {
    gen_record_type(
        typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// A record shaped like `xberg::HeuristicsConfig`: an `impl Default` body supplying a `true`
/// bool, an `f32` and a large `u64`, and no `#[serde(default)]` anywhere — so `field.default`
/// is `None` and `typed_default` is the only carrier of the value.
pub(super) fn impl_default_scalar_fields() -> Vec<FieldDef> {
    let mut enable = field("enable_pdf_text_heuristics", TypeRef::Primitive(PrimitiveType::Bool));
    enable.typed_default = Some(DefaultValue::BoolLiteral(true));
    let mut threshold = field("text_layer_threshold", TypeRef::Primitive(PrimitiveType::F32));
    threshold.typed_default = Some(DefaultValue::FloatLiteral(0.7));
    let mut size = field("file_size_threshold_bytes", TypeRef::Primitive(PrimitiveType::U64));
    size.typed_default = Some(DefaultValue::IntLiteral(10_485_760));
    vec![enable, threshold, size]
}

/// The property initializer a generated record declares for `cs_name`, or a panic naming the
/// property when it has none.
pub(super) fn csharp_initializer(code: &str, cs_name: &str) -> String {
    let marker = format!(" {cs_name} {{ get; init; }} = ");
    code.lines()
        .find_map(|line| line.split_once(&marker))
        .map(|(_, rhs)| rhs.trim().trim_end_matches(';').to_string())
        .unwrap_or_else(|| panic!("no defaulted property `{cs_name}` in:\n{code}"))
}

/// A C# numeric literal carries a type suffix its Swift counterpart does not; the *value* either
/// side of it is what the two languages have to agree on.
pub(super) fn without_numeric_suffix(literal: &str) -> &str {
    literal.trim_end_matches('f')
}

#[test]
fn record_type_emits_impl_default_scalar_literals_not_type_zeros() {
    let mut typ = record_type(impl_default_scalar_fields());
    typ.has_default = true;

    let code = render_plain_record(&typ);

    assert_eq!(csharp_initializer(&code, "EnablePdfTextHeuristics"), "true");
    assert_eq!(csharp_initializer(&code, "TextLayerThreshold"), "0.7f");
    assert_eq!(csharp_initializer(&code, "FileSizeThresholdBytes"), "10485760");
    assert!(
        !code.contains("public required"),
        "a field with an impl Default value must not become required:\n{code}"
    );
}

/// The control that would have caught the regression. Every backend reads the default off the
/// same `FieldDef::typed_default`, so two backends rendering one IR fixture must land on the same
/// value; a backend that silently stops consuming the field renders its type's zero instead and
/// only a cross-language comparison can tell the two apart. Swift is the reference because its
/// literals carry no type suffix — see `backends::swift::gen_bindings::dto`.
#[test]
fn record_type_scalar_defaults_agree_with_the_swift_renderer() {
    use crate::backends::swift::gen_bindings::dto::swift_typed_default_literal;

    let fields = impl_default_scalar_fields();
    let mut typ = record_type(fields.clone());
    typ.has_default = true;

    let code = render_plain_record(&typ);

    let cs_names = [
        "EnablePdfTextHeuristics",
        "TextLayerThreshold",
        "FileSizeThresholdBytes",
    ];
    for (field, cs_name) in fields.iter().zip(cs_names) {
        let typed_default = field.typed_default.as_ref().expect("fixture field carries a default");
        let swift = swift_typed_default_literal(typed_default).expect("swift renders every fixture default");
        let csharp = csharp_initializer(&code, cs_name);
        assert_eq!(
            without_numeric_suffix(&csharp),
            without_numeric_suffix(&swift),
            "C# and Swift disagree on the default for `{}`:\n{code}",
            field.name
        );
    }
}
