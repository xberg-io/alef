//! `impl Default` / `#[serde(default)]` value-restoration tests, split out of
//! [`super`] (`tests.rs`), which the 1,000-line file-size cap no longer let hold
//! this concern inline.

use super::*;

#[test]
fn folded_string_collection_default_preserves_named_function_value() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        r#"
        #[derive(serde::Serialize, serde::Deserialize)]
        pub struct SsrfPolicy {
            #[serde(default = "default_scheme_allowlist", skip_serializing_if = "is_default_scheme_allowlist")]
            pub scheme_allowlist: Vec<String>,
        }
        fn default_scheme_allowlist() -> Vec<String> {
            vec!["http".to_owned(), "https".to_owned()]
        }
        fn is_default_scheme_allowlist(value: &Vec<String>) -> bool {
            *value == default_scheme_allowlist()
        }
        "#,
    )
    .unwrap();
    let surface = crate::extract::extractor::extract(&[&source], "sample_crate", "1.0.0", None).unwrap();
    let typ = surface.types.iter().find(|typ| typ.name == "SsrfPolicy").unwrap();
    assert_eq!(
        typ.fields[0].typed_default,
        Some(DefaultValue::ListLiteral(vec![
            DefaultValue::StringLiteral("http".into()),
            DefaultValue::StringLiteral("https".into()),
        ]))
    );
    let out = render_impl_default_record(typ);
    assert!(out.contains("schemeAllowlist = List.of(\"http\", \"https\")"), "{out}");
    assert!(
        out.contains("if (schemeAllowlist == null) { schemeAllowlist = List.of(\"http\", \"https\"); }"),
        "{out}"
    );
    assert!(!out.contains("List.of()"), "{out}");
}

#[test]
fn collection_default_literals_preserve_empty_escape_strings_and_decline_unknown_values() {
    use crate::backends::java::gen_bindings::helpers::serde_default_collection_literal;
    let string_list = TypeRef::Vec(Box::new(TypeRef::String));
    let render = |value| serde_default_collection_literal(&string_list, true, true, Some(&value));
    assert_eq!(render(DefaultValue::Empty).as_deref(), Some("List.of()"));
    assert_eq!(render(DefaultValue::ListLiteral(vec![])).as_deref(), Some("List.of()"));
    assert_eq!(
        render(DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral(
            "a\"\\\n".into()
        )]))
        .as_deref(),
        Some(r#"List.of("a\"\\\n")"#)
    );
    assert_eq!(render(DefaultValue::Unresolved("unknown()".into())), None);
    assert_eq!(render(DefaultValue::FunctionCall("default_tags".into())), None);
    assert_eq!(
        serde_default_collection_literal(
            &TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
            true,
            true,
            Some(&DefaultValue::ListLiteral(vec![DefaultValue::IntLiteral(7)])),
        ),
        None
    );
}

/// A record whose defaults come from an `impl Default` body: `field.default` is `None` and
/// `typed_default` is the only carrier of the value.
fn impl_default_record(fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "HeuristicsConfig".to_string(),
        rust_path: "sample_crate::HeuristicsConfig".to_string(),
        original_rust_path: "sample_crate::HeuristicsConfig".to_string(),
        fields,
        has_default: true,
        has_serde: true,
        ..Default::default()
    }
}

fn impl_default_field(name: &str, ty: TypeRef, value: DefaultValue) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(value),
        ..Default::default()
    }
}

fn render_impl_default_record(typ: &TypeDef) -> String {
    gen_record_type(
        "dev.sample_crate",
        typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    )
}

/// The defect: a `f32` field carrying a `0.7` default from `impl Default` was emitted as an
/// unboxed `private float textLayerThreshold;` with no compact-constructor assignment. An
/// unboxed primitive has no `null`, so `@JsonInclude(NON_ABSENT)` cannot drop it from the wire
/// — every builder-constructed instance shipped `"text_layer_threshold": 0.0` to Rust, which
/// reads it as a deliberate caller choice and overrides its own `0.7`. That is a live value
/// change across the FFI, not a missing convenience.
#[test]
fn float_literal_default_is_boxed_and_restored_in_compact_ctor() {
    let typ = impl_default_record(vec![impl_default_field(
        "text_layer_threshold",
        TypeRef::Primitive(PrimitiveType::F32),
        DefaultValue::FloatLiteral(0.7),
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("textLayerThreshold == null"),
        "absence must select the Rust default:\n{out}"
    );
    assert!(
        out.contains("textLayerThreshold = 0.7f;"),
        "the f32 default must be restored with a Java float literal:\n{out}"
    );
    assert!(
        !out.contains("float textLayerThreshold"),
        "an unboxed float has no sentinel for 'not supplied', so the component must be boxed:\n{out}"
    );
}

/// An explicit `0.0` is a caller decision and must survive: boxing is what lets the compact
/// constructor tell it apart from "not supplied". Without boxing the only sentinel is the
/// type-zero, which silently overwrites exactly this value.
#[test]
fn explicit_zero_float_is_not_coerced_to_the_default() {
    let typ = impl_default_record(vec![impl_default_field(
        "text_layer_threshold",
        TypeRef::Primitive(PrimitiveType::F32),
        DefaultValue::FloatLiteral(0.7),
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("textLayerThreshold == 0"),
        "explicit zero must remain meaningful:\n{out}"
    );
}

/// `Float` and `Double` take different literal suffixes, and a Java `double` initialiser written
/// without a decimal point (`2`) is an `int` literal — legal in an assignment but not in the
/// `Double` boxing position the compact constructor uses.
#[test]
fn float_default_literals_carry_the_suffix_their_java_type_requires() {
    for (primitive, value, expected) in [
        (PrimitiveType::F32, 0.7_f64, "0.7f"),
        (PrimitiveType::F64, 0.7_f64, "0.7"),
        (PrimitiveType::F32, 2.0_f64, "2.0f"),
        (PrimitiveType::F64, 2.0_f64, "2.0"),
    ] {
        let primitive_label = format!("{primitive:?}");
        let typ = impl_default_record(vec![impl_default_field(
            "ratio",
            TypeRef::Primitive(primitive),
            DefaultValue::FloatLiteral(value),
        )]);

        let out = render_impl_default_record(&typ);

        assert!(
            out.contains(&format!("ratio = {expected};")),
            "{primitive_label} default {value} must render as `{expected}`:\n{out}"
        );
    }
}

/// A NaN or infinite default has no Java literal. Emitting one produces source that does not
/// parse, so the field must fall back to "no restore" rather than to broken code.
#[test]
fn non_finite_float_default_emits_no_compact_ctor_line() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let typ = impl_default_record(vec![impl_default_field(
            "ratio",
            TypeRef::Primitive(PrimitiveType::F64),
            DefaultValue::FloatLiteral(value),
        )]);

        let out = render_impl_default_record(&typ);

        assert!(
            !out.contains("ratio == null"),
            "with no literal to restore there is nothing for a compact constructor to do:\n{out}"
        );
        assert!(
            !out.contains("NaN") && !out.contains("Infinity") && !out.contains("inf"),
            "Rust's Display for a non-finite f64 is not a Java literal:\n{out}"
        );
    }
}

/// A `bool` default of `true` needs the same boxing as a numeric one, and for a sharper reason:
/// the unboxed sentinel would be `false`, which is itself a legitimate caller value. Before this,
/// only a field that additionally carried `#[serde(default)]` was restored, so a `true` coming
/// from a plain `impl Default` body reached the consumer as `false`.
#[test]
fn bool_true_default_from_impl_default_is_boxed_and_restored() {
    let typ = impl_default_record(vec![impl_default_field(
        "enable_pdf_text_heuristics",
        TypeRef::Primitive(PrimitiveType::Bool),
        DefaultValue::BoolLiteral(true),
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("enablePdfTextHeuristics == null"),
        "absence must select the Rust default:\n{out}"
    );
    assert!(
        out.contains("enablePdfTextHeuristics = true;"),
        "the bool default must be restored:\n{out}"
    );
    assert!(
        !out.contains("boolean enablePdfTextHeuristics"),
        "an unboxed boolean's only sentinel is `false`, which is a real caller value:\n{out}"
    );
}

/// The negative control. A default equal to the Java type-zero needs no restore — the component
/// already holds that value — and boxing for it would widen the record's public API for nothing.
/// Without this, "box everything that has a default" would pass every other test here.
#[test]
fn defaults_equal_to_the_java_type_zero_stay_unboxed() {
    for (name, ty, value) in [
        (
            "retries",
            TypeRef::Primitive(PrimitiveType::U32),
            DefaultValue::IntLiteral(0),
        ),
        (
            "ratio",
            TypeRef::Primitive(PrimitiveType::F32),
            DefaultValue::FloatLiteral(0.0),
        ),
        (
            "enabled",
            TypeRef::Primitive(PrimitiveType::Bool),
            DefaultValue::BoolLiteral(false),
        ),
    ] {
        let typ = impl_default_record(vec![impl_default_field(name, ty, value)]);

        let out = render_impl_default_record(&typ);

        assert!(
            !out.contains(&format!("{name} == null")),
            "a zero-valued default needs no sentinel:\n{out}"
        );
        assert!(
            !out.contains("@Nullable"),
            "a zero-valued default must not widen the component to nullable:\n{out}"
        );
    }
}

/// Not a defect: a `String` or enum component is already a reference type. It is `null` when the
/// caller does not set it, `@JsonInclude(NON_ABSENT)` drops a null key from the JSON handed to
/// Rust, and Rust's own `Default` then supplies the value. Restoring it in the compact
/// constructor would only duplicate a value Rust already owns, and boxing is a no-op. This test
/// pins that omission as deliberate so a later "make every default render" change has to argue
/// with it rather than silently churn every DTO.
#[test]
fn reference_typed_defaults_are_left_to_the_rust_side() {
    let typ = impl_default_record(vec![
        impl_default_field(
            "label",
            TypeRef::String,
            DefaultValue::StringLiteral("balanced".to_string()),
        ),
        impl_default_field(
            "mode",
            TypeRef::Named("Mode".to_string()),
            DefaultValue::EnumVariant("Fast".to_string()),
        ),
    ]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("label == null") && !out.contains("mode == null"),
        "reference components need no compact-constructor restore:\n{out}"
    );
    assert!(
        out.contains("@JsonInclude(JsonInclude.Include.NON_ABSENT)"),
        "the omission is only sound because a null key never reaches Rust:\n{out}"
    );
}

/// The bug this fix targets: alef could not read the real default out of `impl Default`
/// (`Unresolved`), and the builder used to fall through to the type-based fallback and emit the
/// unboxed primitive's own zero — a value the source crate never actually specified, underneath a
/// doc comment quoting the real (unreadable) Rust default. `null` on a boxed component is the
/// honest rendering: it is visibly "not supplied" rather than a guessed `0`.
#[test]
fn unresolved_default_scalar_field_is_boxed_with_no_fabricated_zero() {
    let typ = impl_default_record(vec![impl_default_field(
        "retries",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::Unresolved("Self::builder().build()".to_string()),
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("@Nullable"),
        "alef could not read the real default, so the component must be nullable, not a fabricated zero:\n{out}"
    );
    assert!(
        out.contains("Integer retries"),
        "an unresolved default must box the primitive so `null` can mean 'not supplied':\n{out}"
    );
    assert!(
        !out.contains("int retries"),
        "the field must not stay an unboxed primitive, whose only sentinel is the type-zero:\n{out}"
    );
    assert!(
        !out.contains("retries == null"),
        "there is no literal to restore, so the compact constructor must not gain a restore line:\n{out}"
    );
    assert!(
        !out.contains("retries = 0"),
        "the builder must never initialize the field to the fabricated type-zero:\n{out}"
    );
}

/// A field can carry both a `#[serde(default)]` marker (which alone would route the builder
/// through the enum-default lookup) and an unresolved `impl Default` at once — the marker records
/// only that some default exists, not what it is. The dedicated `Unresolved` handling must win
/// regardless, or the marker smuggles a guessed enum variant back in.
#[test]
fn unresolved_default_wins_over_a_serde_default_enum_lookup() {
    let mut mode_field = impl_default_field(
        "mode",
        TypeRef::Named("Mode".to_string()),
        DefaultValue::Unresolved("Self::builder().build()".to_string()),
    );
    mode_field.default = Some("/* serde(default) */".to_string());
    let typ = impl_default_record(vec![mode_field]);

    let mut enum_defaults = ahash::AHashMap::default();
    enum_defaults.insert(
        "Mode".to_string(),
        crate::extract::default_value_for_enum::DefaultEnumVariant {
            variant_name: "Fast".to_string(),
            is_zero_field: true,
        },
    );

    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Auto,
        &enum_defaults,
        &AHashSet::default(),
        &HashSet::default(),
    );

    assert!(
        !out.contains("Mode.Fast"),
        "a `#[serde(default)]` marker must not smuggle an unresolved default into a guessed enum variant:\n{out}"
    );
}

/// The same defect on a collection field: `should_emit_required`'s Java analogue
/// (`boxes_to_carry_literal_default`) reports `false` for a type it cannot restore, but the old
/// code used that as license to fall back to `List.of()` — a real, if empty, value the source
/// crate's unreadable default may not actually hold.
#[test]
fn unresolved_default_collection_field_is_not_the_empty_collection_literal() {
    let typ = impl_default_record(vec![impl_default_field(
        "tags",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::Unresolved("Self::builder().build()".to_string()),
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("List.of()"),
        "an unresolved default must never fall back to the empty-collection literal:\n{out}"
    );
    assert!(
        out.contains("@Nullable"),
        "the field must be nullable so 'not supplied' is a value the generated code can see:\n{out}"
    );
}

/// Negative control for the fix above: `Empty` really does mean "the type's own zero", so a
/// collection field carrying it must still emit the empty-collection literal. Without this, a fix
/// that suppressed every default (rather than only `Unresolved`) would pass every positive test
/// above while silently dropping a legitimate one.
#[test]
fn empty_default_collection_field_still_emits_the_empty_collection_literal() {
    let typ = impl_default_record(vec![impl_default_field(
        "tags",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::Empty,
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("List.of()"),
        "`Empty` is exactly the type's own zero and must still render the empty-collection literal:\n{out}"
    );
}

/// Regression: a non-optional `Vec` field carrying the bare `#[serde(default)]` MARKER (not an
/// `impl Default` literal — `field.default` is the `"/* serde(default) */"` sentinel and
/// `typed_default` is unset, exactly what extraction records for
/// `#[serde(default, skip_serializing_if = "Vec::is_empty")] children: Vec<DataNode>`) AND
/// `serde_skip_serializing_if` must build its builder default as the empty-collection literal
/// `List.of()`, the same outcome the plain non-defaulted `TypeRef::Vec` arm already emits below.
/// Before this fix, the marker branch fell through to a bare `"null"` regardless of field type,
/// so a builder-constructed instance whose wire JSON omitted the key (because
/// `skip_serializing_if` suppressed it) left the field genuinely `null` —
/// `NullPointerException` on `.isEmpty()` downstream, tslp's `DataNode.children()` regression.
/// ~keep
#[test]
fn serde_default_marker_collection_field_builds_the_empty_collection_literal() {
    let mut tags_field = impl_default_field("tags", TypeRef::Vec(Box::new(TypeRef::String)), DefaultValue::Empty);
    tags_field.typed_default = None;
    tags_field.default = Some("/* serde(default) */".to_string());
    tags_field.serde_skip_serializing_if = true;
    let typ = impl_default_record(vec![tags_field]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("List.of()"),
        "a serde(default)-marked collection field's builder default must be the empty-collection \
         literal, not null:\n{out}"
    );
    assert!(
        !out.contains("private List<String> tags = null"),
        "the builder field must not default to null:\n{out}"
    );
}

/// Negative control: a non-optional `Vec` field carrying only the bare `#[serde(default)]`
/// marker, with no `skip_serializing_if`, must stay `@Nullable` with no eager initializer —
/// the nullable-without-eager-default contract `is_serde_default_marker`'s doc describes for
/// every other `#[serde(default)]` field. `skip_serializing_if` is what makes the wire key
/// actually go missing on an empty collection (the failure mode the sibling test above pins);
/// without it there is no such gap, and defaulting to `List.of()` here would silently ship a
/// non-null value for a field the caller never set. ~keep
#[test]
fn serde_default_marker_collection_field_without_skip_serializing_if_stays_nullable() {
    let mut tags_field = impl_default_field("tags", TypeRef::Vec(Box::new(TypeRef::String)), DefaultValue::Empty);
    tags_field.typed_default = None;
    tags_field.default = Some("/* serde(default) */".to_string());
    let typ = impl_default_record(vec![tags_field]);

    let out = render_impl_default_record(&typ);

    assert!(
        out.contains("@Nullable private List<String> tags;"),
        "a bare serde(default) collection field without skip_serializing_if must stay nullable \
         with no eager collection initializer, but got:\n{out}"
    );
    assert!(
        !out.contains("List.of()"),
        "a bare serde(default) collection field without skip_serializing_if must not get an \
         eager List.of() initializer:\n{out}"
    );
}

/// The record-component half of the same defect. The builder already builds
/// `List.of()` for a `#[serde(default, skip_serializing_if = "Vec::is_empty")]` field (see the
/// test above), but the record component itself was independently marked `@Nullable` because
/// `records.rs` derives "must this be nullable" from `has_serde_default` alone, with no knowledge
/// of the builder's eager default. `skip_serializing_if` guarantees the wire key is absent for an
/// empty collection, so Jackson passes `null` for that constructor parameter on direct (no
/// builder) record deserialization — the exact same field that can never actually be `null` (Rust
/// `Vec<T>` is not `Option<Vec<T>>`) gets `@Nullable` on one emission path and a non-null eager
/// default on the other. The fix must keep the component `@Nullable`-free by normalizing `null` to
/// the same `List.of()` literal in the compact constructor, so the "cannot be null" claim implied
/// by dropping `@Nullable` is actually true.
#[test]
fn serde_default_marker_collection_field_record_component_is_not_nullable() {
    let mut tags_field = impl_default_field("tags", TypeRef::Vec(Box::new(TypeRef::String)), DefaultValue::Empty);
    tags_field.typed_default = None;
    tags_field.default = Some("/* serde(default) */".to_string());
    tags_field.serde_skip_serializing_if = true;
    let typ = impl_default_record(vec![tags_field]);

    let out = render_impl_default_record(&typ);

    let record_line = out
        .lines()
        .find(|line| line.contains("public record HeuristicsConfig"))
        .unwrap_or_else(|| panic!("no record declaration line in generated output:\n{out}"));
    assert!(
        !record_line.contains("@Nullable"),
        "a field whose builder eagerly defaults to a non-null value must not carry @Nullable on \
         the record component:\n{record_line}"
    );
    assert!(
        out.contains("tags == null") && out.contains("List.of()"),
        "the compact constructor must normalize a null argument to the same empty-collection \
         default the builder uses, since that normalization is what makes dropping @Nullable \
         correct:\n{out}"
    );
}

/// Negative control, scalar case: `Empty` on a primitive must stay unboxed at the Java type-zero,
/// the same outcome `defaults_equal_to_the_java_type_zero_stay_unboxed` pins for an explicit
/// zero-valued literal — `Empty` is the variant real fixtures actually carry for this case.
#[test]
fn empty_default_scalar_field_stays_unboxed_at_the_type_zero() {
    let typ = impl_default_record(vec![impl_default_field(
        "retries",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::Empty,
    )]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("@Nullable"),
        "`Empty` is the type's own zero and must not widen the component to nullable:\n{out}"
    );
    assert!(
        out.contains("int retries"),
        "the component must stay the unboxed primitive, whose implicit zero already matches `Empty`:\n{out}"
    );
}

/// A `#[alef(skip)]`ped field is not a record component, so a compact-constructor line naming it
/// would not compile. The old loop read `typ.fields` directly and only escaped this because the
/// single variant it handled rarely coincided with an excluded field.
#[test]
fn binding_excluded_field_with_a_default_emits_no_compact_ctor_line() {
    let mut excluded = impl_default_field(
        "internal_ratio",
        TypeRef::Primitive(PrimitiveType::F32),
        DefaultValue::FloatLiteral(0.7),
    );
    excluded.binding_excluded = true;
    let typ = impl_default_record(vec![
        excluded,
        impl_default_field(
            "text_layer_threshold",
            TypeRef::Primitive(PrimitiveType::F32),
            DefaultValue::FloatLiteral(0.7),
        ),
    ]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("internalRatio"),
        "an excluded field is not a record component:\n{out}"
    );
    assert!(
        out.contains("textLayerThreshold = 0.7f;"),
        "positive control: the visible field is still restored:\n{out}"
    );
}

/// Java's half of the cross-language default control. The C# emitter carries the same comparison
/// against the same oracle (`backends::csharp::gen_bindings::types::tests`), and Kotlin's lives in
/// `backends::kotlin::gen_bindings::object_wrapper::tests`, so all four backends agree
/// transitively on the value while each keeps its own spelling. See
/// `backends::default_agreement_tests` for why a single-backend test cannot catch this class of
/// defect: a backend that stops consuming `typed_default` emits its type's zero, which is
/// indistinguishable from a real initializer until something compares it to another backend.
#[test]
fn java_literal_defaults_agree_with_the_swift_renderer() {
    use crate::backends::java::gen_bindings::helpers::java_literal_default;
    use crate::backends::swift::gen_bindings::dto::swift_typed_default_literal;

    let fixture = [
        (
            "bool true",
            TypeRef::Primitive(PrimitiveType::Bool),
            DefaultValue::BoolLiteral(true),
        ),
        (
            "fractional f32",
            TypeRef::Primitive(PrimitiveType::F32),
            DefaultValue::FloatLiteral(0.7),
        ),
        (
            "whole-valued f64",
            TypeRef::Primitive(PrimitiveType::F64),
            DefaultValue::FloatLiteral(2.0),
        ),
        (
            "large integer",
            TypeRef::Primitive(PrimitiveType::U64),
            DefaultValue::IntLiteral(10_485_760),
        ),
    ];

    for (label, ty, value) in fixture {
        let swift = swift_typed_default_literal(&value).expect("the oracle renders every fixture value");
        let java = java_literal_default(&ty, Some(&value))
            .unwrap_or_else(|| panic!("java dropped the default for `{label}` entirely"));

        // A Java numeric literal carries a width suffix its Swift counterpart does not; the value
        // either side of it is what the two languages have to agree on. Normalising rather than
        // skipping the suffixed cases is deliberate — excluding them is how a backend drifts. ~keep
        let normalized = java.trim_end_matches(['f', 'L']);
        assert_eq!(
            normalized, swift,
            "Java and Swift disagree on the default for `{label}`: java `{java}`, swift `{swift}`"
        );
    }
}

/// The apparatus check for the test above. If `java_literal_default` started returning `None` for
/// everything, every agreement assertion would fail loudly — but if the *oracle* started returning
/// `None`, the `expect` would fire on the fixture rather than on a real disagreement. Pin that the
/// two disagree when they should, so a passing comparison means something.
#[test]
fn the_java_swift_comparison_can_still_fail() {
    use crate::backends::java::gen_bindings::helpers::java_literal_default;
    use crate::backends::swift::gen_bindings::dto::swift_typed_default_literal;

    let dropped = DefaultValue::FloatLiteral(0.0);
    assert!(
        java_literal_default(&TypeRef::Primitive(PrimitiveType::F32), Some(&dropped)).is_none(),
        "a zero-valued default is deliberately not restored"
    );
    assert_eq!(
        swift_typed_default_literal(&dropped).as_deref(),
        Some("0.0"),
        "the oracle still renders it, so the two genuinely differ here and the comparison is live"
    );
}

/// SECURITY. A `Vec` field carrying a named `#[serde(default = "default_scheme_allowlist")]`
/// alongside `skip_serializing_if` was given `List.of()` on all three emission paths — record
/// component nullability, compact-constructor restore, and builder initializer.
///
/// Alef never evaluates the referenced Rust function; extraction records only its name
/// (`DefaultValue::FunctionCall`). Substituting the empty collection is therefore a fabricated
/// value, and for the "return a populated list" case it OVERRIDES the real default: a scheme
/// allow-list of `["http", "https"]` arrives at Rust as `[]`. An allow-list that ships empty is
/// not a cosmetic default bug — it is a policy the caller never chose, and the same shape applies
/// to a deny-list, where the empty value fails open instead of closed.
///
/// The correct rendering is no eager value at all: `@Nullable` with a `null` builder default, so
/// `@JsonInclude(NON_ABSENT)` drops the key and Rust's own serde default supplies the value.
///
/// The fixture spells the post-fix IR: `extract::extractor::types` used to blanket-overwrite this
/// field's `typed_default` with `Empty` under a container `#[derive(Default)]`, and now applies
/// precedence so the `FunctionCall` survives. ~keep
#[test]
fn named_serde_default_vec_defers_to_rust_instead_of_shipping_an_empty_allowlist() {
    let mut allowlist = impl_default_field(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::FunctionCall("default_scheme_allowlist".to_string()),
    );
    allowlist.default = Some("serde(default = \"default_scheme_allowlist\")".to_string());
    allowlist.serde_skip_serializing_if = true;
    let typ = impl_default_record(vec![allowlist]);

    let out = render_impl_default_record(&typ);

    assert!(
        !out.contains("List.of()"),
        "alef does not know what default_scheme_allowlist() returns; List.of() is a fabricated \
         allow-list that overrides the real Rust one:\n{out}"
    );
    assert!(
        !out.contains("if (schemeAllowlist == null)"),
        "no compact-constructor restore may fabricate a value alef never read:\n{out}"
    );
    assert!(
        out.contains("@Nullable"),
        "the component must stay nullable so an unset field is dropped by @JsonInclude(NON_ABSENT) \
         and Rust's own serde default fires:\n{out}"
    );
}

/// The same refusal for a `Map`-typed named default, and the discrimination control that keeps
/// the fix from degenerating into "suppress every collection default".
///
/// A bare `#[serde(default)]` records no `FunctionCall` — it resolves to `Default::default()`, so
/// on a `Vec` the empty collection IS the exact Rust value and must still be emitted. The only
/// difference between the two fixtures below is `typed_default`; without the second one, a fix
/// that suppressed both would satisfy the test above while reintroducing the
/// `NullPointerException` on `.isEmpty()` that the eager `List.of()` exists to prevent. ~keep
#[test]
fn named_map_default_defers_while_a_bare_serde_default_still_builds_the_empty_collection() {
    let map_ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));

    let mut named = impl_default_field(
        "header_overrides",
        map_ty.clone(),
        DefaultValue::FunctionCall("default_header_overrides".to_string()),
    );
    named.default = Some("serde(default = \"default_header_overrides\")".to_string());
    named.serde_skip_serializing_if = true;
    let named_out = render_impl_default_record(&impl_default_record(vec![named]));

    assert!(
        !named_out.contains("Map.of()"),
        "a named Map default is as unknown to alef as a named Vec default:\n{named_out}"
    );

    let mut bare = impl_default_field("header_overrides", map_ty, DefaultValue::Empty);
    bare.default = Some("/* serde(default) */".to_string());
    bare.serde_skip_serializing_if = true;
    let bare_out = render_impl_default_record(&impl_default_record(vec![bare]));

    assert!(
        bare_out.contains("Map.of()"),
        "a bare serde(default) on a Map resolves to Default::default(), which IS the empty map; \
         suppressing it would be a different regression:\n{bare_out}"
    );
}

/// The predicate in isolation, so the four-argument contract is pinned independently of how
/// `gen_record_type` happens to weave it together today.
#[test]
fn serde_default_collection_literal_preserves_known_empty_defaults() {
    use crate::backends::java::gen_bindings::helpers::serde_default_collection_literal;

    let vec_ty = TypeRef::Vec(Box::new(TypeRef::String));
    let map_ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    let named = DefaultValue::FunctionCall("default_scheme_allowlist".to_string());
    let resolved = DefaultValue::PublicFunctionCall("sample_crate::Policy::allowlist".to_string());

    assert_eq!(
        serde_default_collection_literal(&vec_ty, true, true, Some(&DefaultValue::Empty)),
        Some("List.of()".to_string())
    );
    assert_eq!(
        serde_default_collection_literal(&vec_ty, true, true, None),
        Some("List.of()".to_string())
    );
    assert_eq!(
        serde_default_collection_literal(&vec_ty, true, true, Some(&named)),
        None
    );
    assert_eq!(
        serde_default_collection_literal(&vec_ty, true, true, Some(&resolved)),
        None
    );
    assert_eq!(
        serde_default_collection_literal(&map_ty, true, true, Some(&named)),
        None
    );
    assert_eq!(
        serde_default_collection_literal(&map_ty, true, true, Some(&DefaultValue::Empty)),
        Some("Map.of()".to_string())
    );
    // `skip_serializing_if` and the serde-default marker remain preconditions for either form.
    assert_eq!(
        serde_default_collection_literal(&vec_ty, true, false, Some(&DefaultValue::Empty)),
        None
    );
    assert_eq!(
        serde_default_collection_literal(&vec_ty, false, true, Some(&DefaultValue::Empty)),
        None
    );
}
