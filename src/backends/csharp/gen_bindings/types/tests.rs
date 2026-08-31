use super::gen_record_type;
use super::records::csharp_type_zero_initializer;
use crate::backends::swift::gen_bindings::dto::swift_type_based_default;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
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

fn render_plain_record(typ: &TypeDef) -> String {
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
fn impl_default_scalar_fields() -> Vec<FieldDef> {
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
fn csharp_initializer(code: &str, cs_name: &str) -> String {
    let marker = format!(" {cs_name} {{ get; init; }} = ");
    code.lines()
        .find_map(|line| line.split_once(&marker))
        .map(|(_, rhs)| rhs.trim().trim_end_matches(';').to_string())
        .unwrap_or_else(|| panic!("no defaulted property `{cs_name}` in:\n{code}"))
}

/// A C# numeric literal carries a type suffix its Swift counterpart does not; the *value* either
/// side of it is what the two languages have to agree on.
fn without_numeric_suffix(literal: &str) -> &str {
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

/// Render `typ` with `types` visible to the emitter, so a nested-record initializer can resolve.
fn render_record_with_types(typ: &TypeDef, types: &[TypeDef]) -> String {
    gen_record_type(
        typ,
        types,
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

/// The declared type of `cs_name`, so a test can tell `T X = null` (nullable, the compiler sees
/// it) from `T X = default!` (a null wearing a non-nullable declaration).
fn csharp_property_type(code: &str, cs_name: &str) -> String {
    let marker = format!(" {cs_name} {{ get; init; }}");
    code.lines()
        .find_map(|line| line.split_once(&marker))
        .map(|(lhs, _)| {
            lhs.trim()
                .trim_start_matches("public ")
                .trim_start_matches("required ")
                .to_string()
        })
        .unwrap_or_else(|| panic!("no property `{cs_name}` in:\n{code}"))
}

/// Every `TypeRef` whose zero is a real value rather than a null. Swift's
/// [`swift_type_based_default`] is the reference *coverage* set: it is the only other backend
/// with the same shape of fallback table, and it has always spelled all of these.
fn type_refs_with_a_non_null_zero() -> Vec<(&'static str, TypeRef)> {
    vec![
        ("bool", TypeRef::Primitive(PrimitiveType::Bool)),
        ("u64", TypeRef::Primitive(PrimitiveType::U64)),
        ("f32", TypeRef::Primitive(PrimitiveType::F32)),
        ("f64", TypeRef::Primitive(PrimitiveType::F64)),
        ("String", TypeRef::String),
        ("Vec<String>", TypeRef::Vec(Box::new(TypeRef::String))),
        (
            "HashMap<String, String>",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        ),
    ]
}

/// The control that would have caught the `Map` hole. The literal-agreement control next door
/// compares *values* for `DefaultValue::*Literal`; nothing compared the type-zero *fallback*,
/// which is the arm that runs whenever a field's default is `Empty` or absent — and that is
/// where the null-in-a-non-nullable-property class lives.
///
/// Values cannot be compared across the two languages here (`[:]` against
/// `new Dictionary<string, string>()` is a spelling difference, not a disagreement), so the
/// comparison is *coverage*: a `TypeRef` Swift can spell a zero for is a `TypeRef` C# must also
/// spell a zero for. C# had no `Map` arm in one of its two tables, so that field fell through to
/// `default!` — a null in a property declared non-nullable.
#[test]
fn csharp_spells_a_zero_for_every_type_swift_does() {
    for (label, ty) in type_refs_with_a_non_null_zero() {
        assert!(
            swift_type_based_default(&ty).is_some(),
            "apparatus: the Swift reference must cover `{label}`, or this test compares nothing"
        );
        let csharp = csharp_type_zero_initializer(&ty);
        assert!(
            csharp.is_some(),
            "C# has no zero for `{label}`, so that field falls back to a null in a \
             non-nullable property"
        );
        assert_ne!(
            csharp.as_deref(),
            Some("default!"),
            "`default!` on `{label}` is a null wearing a non-nullable declaration"
        );
    }
}

/// The negative half. A `Named` field's zero is *not* a value the fallback table may invent —
/// constructing one is a decision that needs the whole type graph — so both backends must
/// decline, leaving the caller to resolve it.
#[test]
fn neither_backend_invents_a_zero_for_a_nested_record() {
    let nested = TypeRef::Named("ContentConfig".to_string());
    assert!(swift_type_based_default(&nested).is_none());
    assert!(csharp_type_zero_initializer(&nested).is_none());
}

/// A `HashMap` field with no default at all. The no-default fallback had no [`TypeRef::Map`] arm
/// and fell through to `default!`, which is `null` assigned into a non-nullable
/// `Dictionary<K, V>` property: `new T { }` compiles, and the first read throws. The defaulted
/// branch had spelled the empty dictionary correctly all along — the two tables had drifted.
#[test]
fn record_type_map_field_without_default_is_an_empty_dictionary_not_null() {
    let typ = record_type(vec![field(
        "metadata",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    )]);

    let code = render_plain_record(&typ);

    assert_eq!(
        csharp_initializer(&code, "Metadata"),
        "new Dictionary<string, string>()"
    );
    assert_eq!(csharp_property_type(&code, "Metadata"), "Dictionary<string, string>");
    assert!(
        !code.contains("default!"),
        "a non-nullable Dictionary property must not be initialized to null:\n{code}"
    );
}

/// The same map field, this time carrying `#[serde(default)]`, must land on the identical
/// initializer. The two branches resolving one `TypeRef` differently is the defect itself, so
/// pinning them together is what keeps the tables from drifting apart again.
#[test]
fn record_type_map_field_initializer_is_the_same_with_and_without_a_serde_default() {
    let map_ty = || TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));

    let bare = record_type(vec![field("metadata", map_ty())]);
    let mut defaulted_field = field("metadata", map_ty());
    defaulted_field.default = Some("/* serde(default) */".to_string());
    let defaulted = record_type(vec![defaulted_field]);

    assert_eq!(
        csharp_initializer(&render_plain_record(&bare), "Metadata"),
        csharp_initializer(&render_plain_record(&defaulted), "Metadata"),
        "the defaulted and no-default branches must resolve a Map to one expression"
    );
}

/// A nested record built with values a C# zero cannot produce: `true` where the zero is `false`,
/// and `9001` where the zero is `0`. A dropped nested default therefore shows up as a *value*
/// difference, not merely a spelling one.
fn nested_content_config() -> TypeDef {
    let mut verbose = field("verbose", TypeRef::Primitive(PrimitiveType::Bool));
    verbose.typed_default = Some(DefaultValue::BoolLiteral(true));
    let mut budget = field("budget", TypeRef::Primitive(PrimitiveType::U64));
    budget.typed_default = Some(DefaultValue::IntLiteral(9001));
    let mut nested = named_record_type("ContentConfig", vec![verbose, budget]);
    nested.has_default = true;
    nested
}

/// A non-optional field whose type is another emitted record, carrying `#[serde(default)]`.
/// It used to render `= default!` — a null in a property declared non-nullable `ContentConfig`,
/// so `new CrawlConfig { MaxDepth = 3 }` handed the caller a half-built value whose first
/// `.Content` read threw. The nested record's own body already spells every Rust default, so
/// `new ContentConfig()` is exactly `ContentConfig::default()`.
#[test]
fn record_type_nested_record_field_is_constructed_not_null() {
    let nested = nested_content_config();
    let mut content = field("content", TypeRef::Named("ContentConfig".to_string()));
    content.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![content]);

    let code = render_record_with_types(&typ, std::slice::from_ref(&nested));

    assert_eq!(csharp_initializer(&code, "Content"), "new ContentConfig()");
    assert_eq!(csharp_property_type(&code, "Content"), "ContentConfig");
    assert!(
        !code.contains("default!"),
        "a nested record property must not be initialized to null:\n{code}"
    );

    let nested_code = render_record_with_types(&nested, std::slice::from_ref(&nested));
    assert_eq!(csharp_initializer(&nested_code, "Verbose"), "true");
    assert_eq!(csharp_initializer(&nested_code, "Budget"), "9001");
}

/// `new T()` is `CS9035` when `T` declares a `required` member, and a compile error in the
/// consumer's build is strictly worse than a nullable property. The fallback must be `null` on a
/// `T?` — a null the C# nullable analysis can actually see — never `default!`.
#[test]
fn record_type_nested_record_with_a_required_member_falls_back_to_a_nullable_property() {
    let nested = named_record_type("ContentConfig", vec![field("name", TypeRef::String)]);
    let mut content = field("content", TypeRef::Named("ContentConfig".to_string()));
    content.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![content]);

    let nested_code = render_record_with_types(&nested, std::slice::from_ref(&nested));
    assert!(
        nested_code.contains("public required string Name { get; init; }"),
        "the fixture must actually declare a required member:\n{nested_code}"
    );

    let code = render_record_with_types(&typ, std::slice::from_ref(&nested));

    assert_eq!(csharp_initializer(&code, "Content"), "null");
    assert_eq!(csharp_property_type(&code, "Content"), "ContentConfig?");
    assert!(
        !code.contains("default!"),
        "the unconstructible fallback must be a visible null, not a hidden one:\n{code}"
    );
}

/// A record graph that reaches itself. Rust cannot express a non-`Box` cycle, but `Box<T>` can,
/// and `new A()` chained through one is a `StackOverflowException` in the consumer's process
/// rather than a generator error. Both ends of the cycle must degrade to a nullable property.
#[test]
fn record_type_cyclic_nested_records_do_not_emit_infinite_construction() {
    let mut back = field("owner", TypeRef::Named("RenderOptions".to_string()));
    back.default = Some("/* serde(default) */".to_string());
    let nested = named_record_type("ContentConfig", vec![back]);

    let mut content = field("content", TypeRef::Named("ContentConfig".to_string()));
    content.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![content]);

    let types = vec![typ.clone(), nested];
    let code = render_record_with_types(&typ, &types);

    assert_eq!(csharp_initializer(&code, "Content"), "null");
    assert_eq!(csharp_property_type(&code, "Content"), "ContentConfig?");
}

/// The emitter cannot resolve a nested record it was never shown. Falling back to `null` on a
/// nullable property keeps the failure visible instead of minting a `new ContentConfig()` that
/// does not compile.
#[test]
fn record_type_unknown_nested_record_falls_back_to_a_nullable_property() {
    let mut content = field("content", TypeRef::Named("ContentConfig".to_string()));
    content.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![content]);

    let code = render_plain_record(&typ);

    assert_eq!(csharp_initializer(&code, "Content"), "null");
    assert_eq!(csharp_property_type(&code, "Content"), "ContentConfig?");
}

/// The apparatus check for the assertions above. `!code.contains("default!")` is only evidence
/// that a null was avoided if `default!` is a string this emitter can still produce — otherwise
/// every one of those assertions passes while examining nothing. A `Named` field degraded to
/// `JsonElement` is the surviving legitimate use: `default(JsonElement)` is a struct value, not
/// a null.
#[test]
fn record_type_still_emits_default_bang_for_the_one_case_where_it_is_a_value() {
    let typ = record_type(vec![field("payload", TypeRef::Named("OpaqueBlob".to_string()))]);
    let complex = HashSet::from(["OpaqueBlob".to_string()]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &complex,
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
        code.contains("default!"),
        "`default!` must remain reachable, or the negative assertions above prove nothing:\n{code}"
    );
    assert!(
        csharp_property_type(&code, "Payload").starts_with("JsonElement"),
        "the surviving `default!` must be on a struct-typed property:\n{code}"
    );
}

#[test]
fn record_type_field_without_any_default_still_emits_the_type_zero() {
    let typ = record_type(vec![
        field("retries", TypeRef::Primitive(PrimitiveType::U32)),
        field("ratio", TypeRef::Primitive(PrimitiveType::F32)),
        field("enabled", TypeRef::Primitive(PrimitiveType::Bool)),
    ]);

    let code = render_plain_record(&typ);

    assert_eq!(csharp_initializer(&code, "Retries"), "0");
    assert_eq!(csharp_initializer(&code, "Ratio"), "0.0f");
    assert_eq!(csharp_initializer(&code, "Enabled"), "false");
}

/// Negative control for the `Unresolved` fix below: `Empty` really does mean "the type's own
/// zero", so a field carrying it must still emit the type-zero initializer. Without this, a fix
/// that suppressed every default (rather than only `Unresolved`) would pass every positive test
/// while silently dropping a legitimate one.
#[test]
fn record_type_empty_typed_default_still_emits_the_type_zero() {
    let mut retries = field("retries", TypeRef::Primitive(PrimitiveType::U32));
    retries.typed_default = Some(DefaultValue::Empty);
    let typ = record_type(vec![retries]);

    let code = render_plain_record(&typ);

    assert_eq!(csharp_initializer(&code, "Retries"), "0");
    assert!(
        !code.contains("public required"),
        "an `Empty` default is exact and must not force `required`:\n{code}"
    );
}

/// The bug this whole fix targets: `Unresolved` means alef could not read the real Rust default,
/// so C# must never guess the type's zero underneath a doc comment quoting a value it did not
/// actually use. `required` is the honest outcome, for every type shape — including `Vec`, which
/// `should_emit_required` normally reports as not required.
#[test]
fn record_type_unresolved_typed_default_scalar_field_is_required_not_a_type_zero() {
    let mut retries = field("retries", TypeRef::Primitive(PrimitiveType::U32));
    retries.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let mut tags = field("tags", TypeRef::Vec(Box::new(TypeRef::String)));
    tags.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let typ = record_type(vec![retries, tags]);

    let code = render_plain_record(&typ);

    assert!(
        code.contains("public required uint Retries { get; init; }"),
        "an unresolved scalar default must become required, not `= 0`:\n{code}"
    );
    assert!(
        code.contains("public required List<string> Tags { get; init; }"),
        "an unresolved default must become required even where `should_emit_required` is false for the type:\n{code}"
    );
    assert!(
        !code.contains(" Retries { get; init; } = "),
        "no initializer may follow a required property:\n{code}"
    );
    assert!(
        !code.contains(" Tags { get; init; } = "),
        "no initializer may follow a required property:\n{code}"
    );
}

/// A field can carry both a `#[serde(default)]` marker (`field.default.is_some()`) and an
/// unresolved `impl Default` (`typed_default: Unresolved`) at once — the marker records only that
/// *some* default exists, not what it is. The dedicated `Unresolved` arm must win regardless, or
/// the `field.default.is_some() || carries_renderable_default(..)` gate lets the fabricated-zero
/// bug back in through the `field.default` side.
#[test]
fn record_type_unresolved_typed_default_wins_over_a_serde_default_marker() {
    let mut retries = field("retries", TypeRef::Primitive(PrimitiveType::U32));
    retries.default = Some("/* serde(default) */".to_string());
    retries.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let typ = record_type(vec![retries]);

    let code = render_plain_record(&typ);

    assert!(
        code.contains("public required uint Retries { get; init; }"),
        "a `#[serde(default)]` marker must not smuggle an unresolved default into a type-zero:\n{code}"
    );
}

/// Mirrors `record_type_nested_record_with_a_required_member_falls_back_to_a_nullable_property`:
/// a nested record whose only field carries an unresolved default must itself be reported
/// not-default-constructible, or the outer field emits `new ContentConfig()` against a type that
/// (post-fix) declares a `required` member — `CS9035` in the consumer's build.
#[test]
fn record_type_nested_record_with_an_unresolved_default_member_falls_back_to_a_nullable_property() {
    let mut name = field("name", TypeRef::String);
    name.typed_default = Some(DefaultValue::Unresolved("Self::builder().build()".to_string()));
    let nested = named_record_type("ContentConfig", vec![name]);

    let mut content = field("content", TypeRef::Named("ContentConfig".to_string()));
    content.default = Some("/* serde(default) */".to_string());
    let typ = record_type(vec![content]);

    let nested_code = render_record_with_types(&nested, std::slice::from_ref(&nested));
    assert!(
        nested_code.contains("public required string Name { get; init; }"),
        "the fixture must actually declare a required member:\n{nested_code}"
    );

    let code = render_record_with_types(&typ, std::slice::from_ref(&nested));

    assert_eq!(csharp_initializer(&code, "Content"), "null");
    assert_eq!(csharp_property_type(&code, "Content"), "ContentConfig?");
}

/// Regression: an opaque instance method returning bytes used to free the native buffer inline,
/// right after `Marshal.Copy`, with no `try`/`finally` around it at all — an exception thrown by
/// `Marshal.Copy` (or by `rc != 0`, before the buffer even existed) skipped the free entirely,
/// leaking it. `NativeMethods.FreeBytes` is a safe no-op on a null pointer (see
/// `{{ ffi_prefix }}_free_bytes` in the FFI crate), so calling it unconditionally from `finally`
/// is correct on every exit path and cannot double-free — there is exactly one call site now. ~keep
#[test]
fn opaque_method_bytes_result_frees_inside_finally_not_inline() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let method = MethodDef {
        name: "render".to_string(),
        return_type: TypeRef::Bytes,
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };

    let code = super::opaque::gen_opaque_method(
        &method,
        &[],
        "Widget",
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains(
            "        finally\n        {\n            NativeMethods.FreeBytes(outPtr, outLen, outCap);\n        }\n"
        ),
        "FreeBytes must be the sole statement in a guaranteed `finally` block:\n{code}"
    );
    let try_pos = code.find("        try\n        {\n").expect("missing try block");
    let free_pos = code.find("NativeMethods.FreeBytes").expect("missing FreeBytes call");
    let return_pos = code.find("return result;").expect("missing return result;");
    assert!(
        try_pos < return_pos && return_pos < free_pos,
        "the free must come after the normal-path return, proving it only runs via `finally`, \
         not inline before the return:\n{code}"
    );
}

/// Same regression, async variant: `opaque_bytes_result_call.jinja`'s `is_async` branch had the
/// identical inline-free defect inside the `Task.Run` lambda. ~keep
#[test]
fn opaque_method_async_bytes_result_frees_inside_finally_not_inline() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let method = MethodDef {
        name: "render".to_string(),
        return_type: TypeRef::Bytes,
        receiver: Some(ReceiverKind::Ref),
        is_async: true,
        cfg: None,
        ..Default::default()
    };

    let code = super::opaque::gen_opaque_method(
        &method,
        &[],
        "Widget",
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains(concat!(
            "            finally\n            {\n                NativeMethods.FreeBytes(outPtr, outLen, outCap);\n",
            "            }\n"
        )),
        "async FreeBytes must be the sole statement in a guaranteed `finally` block:\n{code}"
    );
}

/// A raw `Handle` read on an opaque instance method races `Dispose`/consuming methods on
/// another thread, since nothing pins the native handle for the duration of the call. The
/// receiver must instead be borrowed via `BorrowHandle()`, holding a `HandleLease` (which
/// ref-counts through `SafeHandle.DangerousAddRef`) across the native call. ~keep
#[test]
fn opaque_borrowed_instance_method_reads_receiver_through_borrow_handle_not_raw_handle() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let method = MethodDef {
        name: "describe".to_string(),
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };

    let code = super::opaque::gen_opaque_method(
        &method,
        &[],
        "Widget",
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("        using var handleLease = BorrowHandle();\n"),
        "a borrowed instance method must pin the receiver via BorrowHandle() before the native call:\n{code}"
    );
    assert!(
        code.contains("            handleLease.Handle\n"),
        "the native call must pass the leased handle, not the raw Handle property:\n{code}"
    );
    assert!(
        !code.contains("            Handle\n") && !code.contains("            Handle,\n"),
        "the raw, unguarded Handle property must not appear as the native call's receiver arg:\n{code}"
    );
}

/// The consuming (owned-receiver) path must route through `TakeHandle()`/`HandleTransfer`,
/// which waits out any in-flight borrows and is mutually exclusive with them via `_handleLock`,
/// instead of calling `_safeHandle.Invalidate()` directly outside any lock. ~keep
#[test]
fn opaque_consuming_instance_method_commits_handle_transfer_not_raw_invalidate() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let method = MethodDef {
        name: "close".to_string(),
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
        ..Default::default()
    };

    let code = super::opaque::gen_opaque_method(
        &method,
        &[],
        "Widget",
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        code.contains("        using var handleTransfer = TakeHandle();\n"),
        "a consuming instance method must take the receiver via TakeHandle() before the native call:\n{code}"
    );
    assert!(
        code.contains("            handleTransfer.Handle\n"),
        "the native call must pass the transferred handle, not the raw Handle property:\n{code}"
    );
    assert!(
        code.contains("        handleTransfer.Commit();\n"),
        "a successful consuming call must commit the transfer through the guarded machinery:\n{code}"
    );
    assert!(
        !code.contains("_safeHandle.Invalidate()"),
        "consuming methods must not invalidate the SafeHandle directly, bypassing _handleLock:\n{code}"
    );
}

/// Negative control: a static method (or constructor) has no receiver, so it must not be given
/// a borrow/transfer guard — there is no `Handle` to protect. Without this assertion, a fix that
/// guards indiscriminately (even call sites with nothing to guard) would still pass the two
/// tests above. ~keep
#[test]
fn opaque_static_method_has_no_receiver_guard() {
    use crate::core::ir::MethodDef;

    let method = MethodDef {
        name: "parse".to_string(),
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        receiver: None,
        is_static: true,
        cfg: None,
        ..Default::default()
    };

    let code = super::opaque::gen_opaque_method(
        &method,
        &[],
        "Widget",
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        !code.contains("BorrowHandle()"),
        "a static method has no receiver and must not be given a borrow guard:\n{code}"
    );
    assert!(
        !code.contains("TakeHandle()"),
        "a static method has no receiver and must not be given a transfer guard:\n{code}"
    );
    assert!(
        !code.contains("handleLease") && !code.contains("handleTransfer"),
        "a static method body must not reference receiver-guard locals at all:\n{code}"
    );
}
