use super::{
    EmitContext, OptionsFieldDefaults, gen_options_py, python_field_type, type_has_from_json, typed_default_to_python,
};
use crate::core::config::DtoConfig;
use crate::core::ir::{ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Build the three name-sets `python_field_type` consults: plain enums, data enums, and the
/// subset of data enums that also accept a bare string tag (those with a unit variant).
fn make_sets<'a>(
    enum_names: &[&'a str],
    data_enum_names: &[&'a str],
    str_coercible: &[&'a str],
) -> (AHashSet<&'a str>, AHashSet<&'a str>, AHashSet<&'a str>) {
    (
        enum_names.iter().copied().collect(),
        data_enum_names.iter().copied().collect(),
        str_coercible.iter().copied().collect(),
    )
}

/// `Map<String, Named("ExtractionPattern")>` in OptionsModule context resolves to the bare
/// data-enum class name (imported from the native module) — a payload-only enum, so no `| str`.
#[test]
fn test_map_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("ExtractionPattern".to_string())),
    );
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "dict[str, ExtractionPattern]");
}

/// `Map<String, Named("ExtractionPattern")>` in NativeStub context resolves to the
/// native PyO3 class — also bare name (no `_native.` prefix needed in a .pyi file that
/// IS the native module). The `| str` widening never applies to the stub.
#[test]
fn test_map_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &["ExtractionPattern"]);
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("ExtractionPattern".to_string())),
    );
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "dict[str, ExtractionPattern]");
}

/// `Vec<Named("Message")>` in OptionsModule context uses the bare data-enum class name.
#[test]
fn test_vec_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["Message"], &["Message"], &[]);
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "list[Message]");
}

/// `Vec<Named("Message")>` in NativeStub context uses the bare native-class name.
#[test]
fn test_vec_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["Message"], &["Message"], &[]);
    let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "list[Message]");
}

/// `Optional<Named("ExtractionPattern")>` in OptionsModule context appends `| None`.
#[test]
fn test_optional_named_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "ExtractionPattern | None");
}

/// `Optional<Named("ExtractionPattern")>` in NativeStub context appends `| None`.
#[test]
fn test_optional_named_data_enum_native_stub() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"], &[]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(result, "ExtractionPattern | None");
}

/// A data enum with a unit (tag-only) variant is widened to `<Class> | str` in OptionsModule
/// so the bare string tag (and string defaults like `= "native"`) type-check, while the
/// NativeStub keeps the class-only form.
#[test]
fn test_str_coercible_data_enum_options_module() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ImageOutputFormat"], &["ImageOutputFormat"], &["ImageOutputFormat"]);
    let ty = TypeRef::Named("ImageOutputFormat".to_string());
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "ImageOutputFormat | str");
    assert_eq!(native, "ImageOutputFormat");
}

/// The `| str` widening reaches inside containers: `Optional<ImageOutputFormat>` becomes
/// `ImageOutputFormat | str | None`.
#[test]
fn test_str_coercible_data_enum_optional() {
    let (enum_names, data_enum_names, str_coercible) =
        make_sets(&["ImageOutputFormat"], &["ImageOutputFormat"], &["ImageOutputFormat"]);
    let ty = TypeRef::Optional(Box::new(TypeRef::Named("ImageOutputFormat".to_string())));
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "ImageOutputFormat | str | None");
}

/// A payload-only data enum (no unit variant, e.g. EmbeddingModelType) stays class-only — the
/// flattened `str | int | LlmConfig` alias is gone, and a bare string is NOT a valid value.
#[test]
fn test_payload_only_data_enum_class_only() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["EmbeddingModelType"], &["EmbeddingModelType"], &[]);
    let ty = TypeRef::Named("EmbeddingModelType".to_string());
    let result = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    assert_eq!(result, "EmbeddingModelType");
}

/// Plain (non-data) enum field always uses `EnumName | str` regardless of context.
#[test]
fn test_plain_enum_field_both_contexts() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&["HeadingStyle"], &[], &[]);
    let ty = TypeRef::Named("HeadingStyle".to_string());
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "HeadingStyle | str");
    assert_eq!(native, "HeadingStyle | str");
}

/// Primitive types are unaffected by context.
#[test]
fn test_primitive_unaffected_by_context() {
    let (enum_names, data_enum_names, str_coercible) = make_sets(&[], &[], &[]);
    let ty = TypeRef::Primitive(PrimitiveType::Bool);
    let options = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::OptionsModule,
    );
    let native = python_field_type(
        &ty,
        false,
        &enum_names,
        &data_enum_names,
        &str_coercible,
        EmitContext::NativeStub,
    );
    assert_eq!(options, "bool");
    assert_eq!(native, "bool");
}

fn widget_request() -> TypeDef {
    TypeDef {
        name: "WidgetRequest".to_string(),
        rust_path: "my_lib::WidgetRequest".to_string(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The gate `gen_bindings::mod` uses to inject a raw-text `from_json` staticmethod: per-type
/// serde derives, crate-wide serde availability, and the type being in the core<->binding
/// convertible set — all three independently necessary (see
/// `crate::codegen::conversions::pyo3_from_json_eligible`).
#[test]
fn type_has_from_json_true_when_all_three_conditions_hold() {
    let typ = widget_request();
    let api = ApiSurface {
        types: vec![typ.clone()],
        ..Default::default()
    };

    assert!(type_has_from_json(&typ, &api, true));
}

#[test]
fn type_has_from_json_false_when_crate_has_no_serde() {
    let typ = widget_request();
    let api = ApiSurface {
        types: vec![typ.clone()],
        ..Default::default()
    };

    assert!(!type_has_from_json(&typ, &api, false));
}

/// The per-type half of the gate: a type without `Deserialize` derives cannot meaningfully get
/// a `from_json` staticmethod even when the crate has serde and the type is otherwise
/// convertible — `serde_json::from_str::<Self>` would have no `Deserialize` impl to target.
#[test]
fn type_has_from_json_false_when_type_lacks_serde() {
    let typ = TypeDef {
        has_serde: false,
        ..widget_request()
    };
    let api = ApiSurface {
        types: vec![typ.clone()],
        ..Default::default()
    };

    assert!(!type_has_from_json(&typ, &api, true));
}

/// Opaque types are excluded from `core_to_binding_convertible_types` up front, so an opaque
/// type never gets a `from_json` staticmethod even when the crate has serde.
#[test]
fn type_has_from_json_false_for_an_opaque_type() {
    let typ = TypeDef {
        is_opaque: true,
        ..widget_request()
    };
    let api = ApiSurface {
        types: vec![typ.clone()],
        ..Default::default()
    };

    assert!(!type_has_from_json(&typ, &api, true));
}

#[test]
fn sensitive_dataclass_fields_are_excluded_from_repr_without_changing_values() {
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Credentials".to_string(),
            has_default: true,
            fields: vec![
                FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    typed_default: Some(DefaultValue::StringLiteral("public".to_string())),
                    ..Default::default()
                },
                FieldDef {
                    name: "token".to_string(),
                    ty: TypeRef::String,
                    sensitive: true,
                    typed_default: Some(DefaultValue::StringLiteral("planted-secret".to_string())),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let generated = gen_options_py(&api, "_native", &DtoConfig::default(), &[], false);

    assert!(generated.contains("label: str = \"public\""), "{generated}");
    assert!(
        generated.contains("token: str = field(default=\"planted-secret\", repr=False)"),
        "{generated}"
    );

    let directory = tempfile::tempdir().expect("temporary Python module");
    std::fs::write(directory.path().join("options.py"), generated).expect("generated options module must be written");
    let python = which::which("python3")
        .or_else(|_| which::which("python"))
        .expect("Python is required for the generated representation acceptance probe");
    let probe = r#"
from dataclasses import asdict
from options import Credentials

secret = "planted-secret"
credentials = Credentials(label="public", token=secret)
assert secret not in repr(credentials)
assert secret not in str(credentials)
assert asdict(credentials)["token"] == secret
"#;
    let output = std::process::Command::new(python)
        .arg("-c")
        .arg(probe)
        .current_dir(directory.path())
        .output()
        .expect("Python representation probe must start");

    assert!(
        output.status.success(),
        "generated Python representation probe failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The bug this fix targets: alef could not read the real default out of `impl Default`
/// (`Unresolved`), and this renderer used to fall through to the same branch as `Empty` and
/// spell the *type's* zero underneath a doc comment quoting the real (unreadable) Rust default —
/// a value the source never actually specified. `None` is the only honest rendering, and the
/// dataclass field emitter (`gen_options_py`) already widens the type hint to `T | None`
/// whenever the default string is exactly `"None"`, so this reuses that mechanism rather than
/// guessing a type-specific zero.
#[test]
fn unresolved_default_renders_as_none_not_a_type_zero() {
    let enum_defaults: AHashMap<String, String> = AHashMap::default();
    let data_enum_names: AHashSet<&str> = AHashSet::default();
    let unresolved = DefaultValue::Unresolved("Self::builder().build()".to_string());

    for ty in [
        TypeRef::Primitive(PrimitiveType::U32),
        TypeRef::Primitive(PrimitiveType::Bool),
        TypeRef::Primitive(PrimitiveType::F64),
        TypeRef::String,
        TypeRef::Vec(Box::new(TypeRef::String)),
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    ] {
        let rendered = typed_default_to_python(&unresolved, &ty, &enum_defaults, &data_enum_names);
        assert_eq!(
            rendered, "None",
            "an unresolved default must render as `None` for {ty:?}, not a fabricated zero, got `{rendered}`"
        );
    }
}

/// `Empty` on a `Named` enum field legitimately consults `enum_defaults` to name the type's own
/// `#[default]` variant — but `Unresolved` must never do that lookup, since alef does not
/// actually know the default is that variant. Sharing the `Empty` arm let this guess leak through
/// even though the type happened to be an enum.
#[test]
fn unresolved_default_on_a_named_enum_does_not_guess_the_default_variant() {
    let mut enum_defaults: AHashMap<String, String> = AHashMap::default();
    enum_defaults.insert("Mode".to_string(), "Fast".to_string());
    let data_enum_names: AHashSet<&str> = AHashSet::default();
    let ty = TypeRef::Named("Mode".to_string());
    let unresolved = DefaultValue::Unresolved("Self::builder().build()".to_string());

    let rendered = typed_default_to_python(&unresolved, &ty, &enum_defaults, &data_enum_names);

    assert_eq!(
        rendered, "None",
        "an unresolved default must not guess the enum's default variant, got `{rendered}`"
    );
}

/// Negative control: `Empty` really does mean "the type's own zero", so it must still render the
/// type-zero table this same function used to share with `Unresolved`. Without this, a fix that
/// suppressed every default (rather than only `Unresolved`) would pass the positive tests above
/// while silently dropping a legitimate one.
#[test]
fn empty_default_still_renders_the_type_zero() {
    let enum_defaults: AHashMap<String, String> = AHashMap::default();
    let data_enum_names: AHashSet<&str> = AHashSet::default();

    let cases: [(TypeRef, &str); 4] = [
        (TypeRef::Primitive(PrimitiveType::U32), "0"),
        (TypeRef::Primitive(PrimitiveType::Bool), "False"),
        (TypeRef::String, "\"\""),
        (TypeRef::Vec(Box::new(TypeRef::String)), "field(default_factory=list)"),
    ];
    for (ty, expected) in cases {
        let rendered = typed_default_to_python(&DefaultValue::Empty, &ty, &enum_defaults, &data_enum_names);
        assert_eq!(rendered, expected, "`Empty` must still render the type zero for {ty:?}");
    }
}

fn detection_policy_enum() -> EnumDef {
    EnumDef {
        name: "DetectionPolicy".to_string(),
        has_default: true,
        variants: vec![
            EnumVariant {
                name: "PreferContent".to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "ContentOnly".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "TrustExtension".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// Security control (task #558): `OptionsFieldDefaults::literal` is what `options.py` actually
/// calls to render a field's default expression. An `Option<Enum>` field with no explicit
/// default resolves to `typed_default = Some(DefaultValue::Empty)` after extraction (see
/// `extract::extractor::postprocess::resolve_enum_field_defaults`), and the emitted Python
/// default must stay `None` — never the enum's own `#[default]` variant. Forwarding a
/// materialized variant here is indistinguishable from a caller's explicit choice once it
/// crosses into the native constructor, and can silently override a stricter policy the caller
/// set elsewhere (e.g. a global content-only mode).
#[test]
fn optional_enum_field_with_no_explicit_default_renders_none_not_the_variant() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..Default::default()
    };
    let field = FieldDef {
        name: "detection_policy".to_string(),
        ty: TypeRef::Named("DetectionPolicy".to_string()),
        optional: true,
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };

    let defaults = OptionsFieldDefaults::new(&api);
    let rendered = defaults.literal(&field);

    assert_eq!(
        rendered, "None",
        "an Option<Enum> field with no explicit default must render `None`, not a materialized \
         variant like `\"prefer_content\"`; got `{rendered}`"
    );
    assert!(
        defaults.admits_none(&field),
        "a field rendering `None` must also report `admits_none`, since callers use that to \
         decide whether omitting the keyword argument is meaningful"
    );
}

/// Negative control for the test above: a *required* (non-optional) `Enum` field with the same
/// `Empty` typed default legitimately does resolve to the enum's own default variant. A fix that
/// suppressed every `Empty`-on-`Named` default (rather than only the optional-field case) would
/// pass the positive test above while silently breaking this legitimate one.
#[test]
fn required_enum_field_with_empty_default_still_renders_the_default_variant() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..Default::default()
    };
    let field = FieldDef {
        name: "detection_policy".to_string(),
        ty: TypeRef::Named("DetectionPolicy".to_string()),
        optional: false,
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };

    let rendered = OptionsFieldDefaults::new(&api).literal(&field);

    assert_eq!(
        rendered, "\"prefer_content\"",
        "a required enum field must still render its own `#[default]` variant"
    );
}

/// Negative control: an `Option<Enum>` field that genuinely does have an explicit default naming
/// a concrete variant (`typed_default = Some(EnumVariant(..))`, not narrowed from `Empty`) must
/// still render that real value. This is the case
/// `optional_enum_field_default_stays_empty_never_a_materialized_variant`'s sibling
/// (`extract::extractor::tests::defaults::enum_field_defaults::
/// optional_enum_field_with_explicit_default_still_resolves_to_the_named_variant`) feeds.
#[test]
fn optional_enum_field_with_explicit_variant_default_still_renders_it() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..Default::default()
    };
    let field = FieldDef {
        name: "detection_policy".to_string(),
        ty: TypeRef::Named("DetectionPolicy".to_string()),
        optional: true,
        typed_default: Some(DefaultValue::EnumVariant("ContentOnly".to_string())),
        ..Default::default()
    };

    let rendered = OptionsFieldDefaults::new(&api).literal(&field);

    assert_eq!(
        rendered, "\"content_only\"",
        "an explicit variant default on an optional field must still be rendered"
    );
}
