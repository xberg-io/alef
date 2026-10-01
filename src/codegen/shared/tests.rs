use super::*;

fn named_default_field(name: &str, type_name: &str, default_path: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        typed_default: Some(DefaultValue::PublicFunctionCall(default_path.to_string())),
        ..Default::default()
    }
}

/// A minimal owning `TypeDef` for `fields`, with `has_serde` set so
/// `rust_default_via_source_deserialize` recovery can be attempted against it.
fn owning_type(rust_path: &str, type_name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: type_name.to_string(),
        rust_path: rust_path.to_string(),
        fields,
        has_serde: true,
        ..Default::default()
    }
}

/// A `#[serde(default = "path")]` function must return exactly the field's declared type
/// (serde's own contract). When that type is `Named` — a mirrored/wrapped type in the
/// binding surface, e.g. wasm's `WasmSsrfPolicy` wrapping core `SsrfPolicy` — the literal
/// `path()` call yields the *core* type, so the constructor assignment needs `.into()` to
/// become the wrapper type the field actually holds. `PublicFunctionCall` is already known
/// callable from a binding crate, so no deserialize-recovery is needed — only the `.into()`
/// conversion. Covers the "converts" half of the wasm constructor-default fix.
#[test]
fn format_default_value_named_function_call_appends_into() {
    let field = named_default_field("ssrf", "SsrfPolicy", "sample_crate::SsrfPolicy::from_env");
    let typ = owning_type("sample_crate::CrawlConfig", "CrawlConfig", vec![field.clone()]);
    assert_eq!(
        format_default_value(&field, &typ, "WasmSsrfPolicy"),
        "sample_crate::SsrfPolicy::from_env().into()"
    );
}

/// The wasm backend does not give every `Named` field a wrapper type — `type_map` degrades
/// some of them to an opaque `JsValue`, which implements no `From<CoreType>`. Appending
/// `.into()` there is an `E0277`, not an `E0308`, so the recovered value has to cross the
/// boundary through serde instead, exactly as the generated `From<CoreType> for WasmType`
/// bodies already do for the same fields. Regression test for the two
/// `JsValue: From<ScoringModelType>` / `From<SecondaryModelType>` failures.
#[test]
fn format_default_value_named_function_call_uses_serde_when_mapped_to_js_value() {
    let field = named_default_field(
        "model",
        "ScoringModelType",
        "sample_crate::ScoringConfig::default_scoring_model",
    );
    let typ = owning_type("sample_crate::ScoringConfig", "ScoringConfig", vec![field.clone()]);
    let rendered = format_default_value(&field, &typ, "JsValue");
    assert!(
        !rendered.contains(".into()"),
        "a JsValue-mapped field must not use .into(): {rendered}"
    );
    assert!(
        rendered.starts_with("serde_wasm_bindgen::to_value(&")
            && rendered.ends_with(".unwrap_or(wasm_bindgen::JsValue::NULL)"),
        "expected a serde_wasm_bindgen bridge, got: {rendered}"
    );
}

/// The degraded-type branch must not depend on how the mapper *spells* `JsValue`. A mapper
/// that renders a fully-qualified or prelude path is describing the same opaque type, and a
/// spelling-sensitive check would silently fall back to `.into()` and reintroduce the E0277.
#[test]
fn maps_to_js_value_accepts_every_path_spelling() {
    for spelling in [
        "JsValue",
        "wasm_bindgen::JsValue",
        "::wasm_bindgen::JsValue",
        "wasm_bindgen::prelude::JsValue",
        " JsValue ",
    ] {
        assert!(maps_to_js_value(spelling), "{spelling} must be recognised as JsValue");
    }
    for other in ["WasmSsrfPolicy", "String", "Option<JsValue>", "Vec<JsValue>", ""] {
        assert!(!maps_to_js_value(other), "{other} must not be treated as JsValue");
    }
}

/// The same recovered default must switch to the serde bridge for every spelling of the
/// degraded type, not only the bare one the original defect happened to produce.
#[test]
fn format_default_value_uses_serde_for_qualified_js_value_spelling() {
    let field = named_default_field(
        "model",
        "ScoringModelType",
        "sample_crate::ScoringConfig::default_scoring_model",
    );
    let typ = owning_type("sample_crate::ScoringConfig", "ScoringConfig", vec![field.clone()]);
    let rendered = format_default_value(&field, &typ, "::wasm_bindgen::prelude::JsValue");
    assert!(
        !rendered.contains(".into()") && rendered.starts_with("serde_wasm_bindgen::to_value(&"),
        "expected a serde_wasm_bindgen bridge, got: {rendered}"
    );
}

/// A default function returning a non-`Named` type (e.g. `String`, `u32`) needs no
/// conversion — the call's return type already matches the field's binding representation.
/// Guards against over-broadly appending `.into()` to every function-call default.
#[test]
fn format_default_value_non_named_function_call_has_no_into() {
    let field = FieldDef {
        name: "retry_limit".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "sample_crate::defaults::retry_limit".to_string(),
        )),
        ..Default::default()
    };
    let typ = owning_type("sample_crate::CrawlConfig", "CrawlConfig", vec![field.clone()]);
    assert_eq!(
        format_default_value(&field, &typ, ""),
        "sample_crate::defaults::retry_limit()"
    );
}

/// The defect this fix addresses: a private (plain `FunctionCall`, not yet resolved to a
/// public method) `#[serde(default = "path")]` function on a non-`Named` field must never
/// be emitted as a direct `path()` call — `path` is frequently not `pub` and/or
/// `#[cfg(feature = "serde")]`-gated, so the generated binding crate cannot call it
/// (`E0425`, the exact wasm backend defect: `default_archive_depth()`,
/// `default_sample_crate_crawl_config()`, `default_scoring_model()` are all unresolved
/// in the generated `lib.rs`). It must instead be recovered by deserializing a minimal
/// JSON stub through the owning type's own `Deserialize` impl. No `.into()` is needed here
/// because the field's own type already matches what the recovery expression evaluates to.
#[test]
fn wasm_constructor_recovers_private_serde_default_via_deserialize() {
    let field = FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::FunctionCall("default_archive_depth".to_string())),
        ..Default::default()
    };
    let typ = owning_type(
        "sample_crate::ExtractionConfig",
        "ExtractionConfig",
        vec![field.clone()],
    );

    let rendered = format_default_value(&field, &typ, "");

    assert_eq!(
        rendered,
        "serde_json::from_str::<sample_crate::ExtractionConfig>(r#\"{}\"#).expect(\"alef-generated \
         default JSON for `ExtractionConfig` failed to deserialize\").max_archive_depth"
    );
    assert!(
        !rendered.contains("default_archive_depth()"),
        "the private source-crate function must never be emitted as a bare callable: {rendered}"
    );
}

/// Same private-`FunctionCall` recovery as
/// `wasm_constructor_recovers_private_serde_default_via_deserialize`, but on a `Named`
/// field (mirroring wasm's `crawl: sample_crate::CrawlConfig` field, whose
/// `default_sample_crate_crawl_config()` default is private): the recovered expression evaluates
/// to the *core* type, so wasm's distinct wrapper struct field needs `.into()` appended —
/// the exact conversion `format_default_value`'s doc comment requires for `Named` fields.
#[test]
fn wasm_constructor_appends_into_for_named_field_default() {
    let field = FieldDef {
        name: "crawl".to_string(),
        ty: TypeRef::Named("CrawlConfig".to_string()),
        typed_default: Some(DefaultValue::FunctionCall(
            "default_sample_crate_crawl_config".to_string(),
        )),
        ..Default::default()
    };
    let typ = owning_type(
        "sample_crate::ExtractionConfig",
        "ExtractionConfig",
        vec![field.clone()],
    );

    let rendered = format_default_value(&field, &typ, "");

    assert_eq!(
        rendered,
        "serde_json::from_str::<sample_crate::ExtractionConfig>(r#\"{}\"#).expect(\"alef-generated \
         default JSON for `ExtractionConfig` failed to deserialize\").crawl.into()"
    );
}

/// When the owning type cannot support deserialize-recovery (here: a required sibling of
/// `Named` type has no safe JSON placeholder), generation must fail loudly with a
/// `compile_error!` naming the field and the uncallable function — never fall back to a
/// bare `path()` call (uncompilable) or to `Default::default()` (compiles but silently
/// ships the wrong value). Production generation is protected from ever emitting this
/// string by `validate_rust_default_functions`, which the wasm backend must call before
/// generating bindings.
#[test]
fn wasm_constructor_default_recovery_failure_emits_compile_error() {
    let owner_field = FieldDef {
        name: "owner".to_string(),
        ty: TypeRef::Named("Author".to_string()),
        ..Default::default()
    };
    let retry_field = FieldDef {
        name: "retry_limit".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::FunctionCall("defaults::retry_limit".to_string())),
        ..Default::default()
    };
    let typ = owning_type(
        "sample_crate::RetryHolder",
        "RetryHolder",
        vec![owner_field, retry_field.clone()],
    );

    let rendered = format_default_value(&retry_field, &typ, "");

    assert!(
        rendered.starts_with("compile_error!"),
        "unrecoverable private default must fail generation, not a bare path call: {rendered}"
    );
    assert!(
        rendered.contains("defaults::retry_limit") && rendered.contains("retry_limit"),
        "the failure must name the uncallable function and the field: {rendered}"
    );
    assert!(
        !rendered.contains("defaults::retry_limit()"),
        "the uncallable source function must never be emitted as a callable: {rendered}"
    );
}

/// End-to-end check of the exact call wasm's `gen_new_method` makes
/// (`config_constructor_parts_with_options`, mirroring the `ssrf: SsrfPolicy` field with
/// `#[serde(default = "SsrfPolicy::from_env")]`): the constructor assignment must both
/// *apply* the default when the JS caller omits the field (an `unwrap_or_else` fallback,
/// which wasm already did before this fix) and *convert* the core default value into the
/// wrapper type via `.into()` (the regression this fix addresses — see
/// `format_default_value_named_function_call_appends_into` for the isolated unit check of
/// just that half).
#[test]
fn config_constructor_parts_named_default_field_applies_and_converts() {
    let field = named_default_field("ssrf", "SsrfPolicy", "sample_crate::SsrfPolicy::from_env");
    let fields = vec![field.clone()];
    let type_mapper = |ty: &TypeRef| match ty {
        TypeRef::Named(name) => format!("Wasm{name}"),
        _ => "String".to_string(),
    };
    let typ = owning_type("sample_crate::CrawlConfig", "CrawlConfig", vec![field]);

    let (_, _, assignments) = config_constructor_parts_with_options(&fields, &type_mapper, true, &typ);

    assert!(
        assignments.contains("ssrf.unwrap_or_else(|| sample_crate::SsrfPolicy::from_env().into())"),
        "expected an unwrap_or_else default that converts via .into(), got: {assignments}"
    );
}

/// End-to-end check that a genuinely private serde default (plain `FunctionCall`, the wasm
/// `E0425` shape) is recovered rather than emitted as a bare call, through the exact same
/// `config_constructor_parts_with_options` entry point wasm's `gen_new_method` calls.
#[test]
fn config_constructor_parts_private_default_field_recovers_via_deserialize() {
    let field = FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        typed_default: Some(DefaultValue::FunctionCall("default_archive_depth".to_string())),
        ..Default::default()
    };
    let fields = vec![field.clone()];
    let type_mapper = |ty: &TypeRef| match ty {
        TypeRef::Named(name) => format!("Wasm{name}"),
        _ => "u32".to_string(),
    };
    let typ = owning_type("sample_crate::ExtractionConfig", "ExtractionConfig", vec![field]);

    let (_, _, assignments) = config_constructor_parts_with_options(&fields, &type_mapper, true, &typ);

    assert!(
        assignments.contains(
            "max_archive_depth.unwrap_or_else(|| serde_json::from_str::<sample_crate::ExtractionConfig>(r#\"{}\"#)"
        ) && assignments.contains(".max_archive_depth)"),
        "expected an unwrap_or_else default that recovers via deserialize, got: {assignments}"
    );
    assert!(
        !assignments.contains("default_archive_depth()"),
        "the private source-crate function must never be emitted as a bare callable: {assignments}"
    );
}

/// End-to-end check of the exact CI failure shape: a *public*, zero-argument, non-`Named`
/// field default (`max_archive_depth: i64` with `#[serde(default =
/// "sample_crate::ExtractionConfig::default_archive_depth")]`) must not wrap the call in a closure
/// — `unwrap_or_else(|| path())` trips `clippy::redundant_closure` under `-D warnings` on
/// every backend that reaches `config_constructor_parts_inner` (wasm directly; pyo3 and
/// extendr via `gen_constructor_with_renames`). `unwrap_or_else(path)` passes the function
/// item directly and is the only form clippy accepts here.
#[test]
fn config_constructor_parts_bare_zero_arg_default_drops_redundant_closure() {
    let field = FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::I64),
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "sample_crate::ExtractionConfig::default_archive_depth".to_string(),
        )),
        ..Default::default()
    };
    let fields = vec![field.clone()];
    let type_mapper = |_: &TypeRef| "i64".to_string();
    let typ = owning_type("sample_crate::ExtractionConfig", "ExtractionConfig", vec![field]);

    let (_, _, assignments) = config_constructor_parts_with_options(&fields, &type_mapper, true, &typ);

    assert!(
        assignments.contains("max_archive_depth.unwrap_or_else(sample_crate::ExtractionConfig::default_archive_depth)"),
        "expected the redundant closure dropped in favor of a bare function reference, got: {assignments}"
    );
    assert!(
        !assignments.contains("unwrap_or_else(|| sample_crate::ExtractionConfig::default_archive_depth())"),
        "clippy::redundant_closure: a bare zero-arg call must never stay wrapped in a closure, got: {assignments}"
    );
}

#[test]
fn field_default_constructor_keeps_unrelated_sibling_required() {
    let llm = FieldDef {
        name: "llm".to_string(),
        ty: TypeRef::Named("LlmConfig".to_string()),
        ..Default::default()
    };
    let alt_text = FieldDef {
        name: "alt_text".to_string(),
        ty: TypeRef::Named("CaptionAltTextMode".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        rust_path: "sample_core::CaptioningConfig".to_string(),
        fields: vec![llm.clone(), alt_text.clone()],
        ..Default::default()
    };
    let mapper = |ty: &TypeRef| match ty {
        TypeRef::Named(name) => name.clone(),
        _ => unreachable!(),
    };

    let (params, defaults, assignments) = constructor_parts_with_field_defaults(&[llm, alt_text], &mapper, &typ);

    assert!(
        params.contains("llm: LlmConfig"),
        "required sibling was widened: {params}"
    );
    assert!(
        !params.contains("llm: Option<"),
        "required sibling was widened: {params}"
    );
    assert!(
        params.contains("alt_text: Option<CaptionAltTextMode>"),
        "serde-defaulted field must admit omission: {params}"
    );
    assert_eq!(defaults, "alt_text=None");
    assert!(
        assignments.contains("llm"),
        "required sibling must pass through: {assignments}"
    );
    assert!(
        !assignments.contains("llm.unwrap"),
        "required sibling must pass through: {assignments}"
    );
    assert!(
        assignments.contains("alt_text: alt_text.unwrap_or_default()"),
        "omitted bare serde default must use the field type's Default: {assignments}"
    );
}

#[test]
fn field_default_constructor_preserves_ordinary_optional_sibling() {
    let llm = FieldDef {
        name: "llm".to_string(),
        ty: TypeRef::Named("LlmConfig".to_string()),
        ..Default::default()
    };
    let output_format = FieldDef {
        name: "output_format".to_string(),
        ty: TypeRef::Named("OutputFormat".to_string()),
        optional: true,
        ..Default::default()
    };
    let alt_text = FieldDef {
        name: "alt_text".to_string(),
        ty: TypeRef::Named("CaptionAltTextMode".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
        ..Default::default()
    };
    let fields = vec![llm, output_format, alt_text];
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        rust_path: "sample_core::CaptioningConfig".to_string(),
        fields: fields.clone(),
        ..Default::default()
    };
    let mapper = |ty: &TypeRef| match ty {
        TypeRef::Named(name) => name.clone(),
        _ => unreachable!(),
    };

    let (params, defaults, assignments) = constructor_parts_with_field_defaults(&fields, &mapper, &typ);

    assert!(
        params.contains("output_format: Option<OutputFormat>"),
        "ordinary optional sibling must remain optional: {params}"
    );
    assert_eq!(defaults, "output_format=None, alt_text=None");
    assert!(
        assignments.contains("output_format"),
        "ordinary optional sibling must pass through: {assignments}"
    );
    assert!(
        !assignments.contains("output_format.unwrap"),
        "ordinary optional sibling must pass through: {assignments}"
    );
}

#[test]
fn field_default_constructor_does_not_double_wrap_already_optional_sibling() {
    let description = FieldDef {
        name: "description".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        ..Default::default()
    };
    let alt_text = FieldDef {
        name: "alt_text".to_string(),
        ty: TypeRef::Named("CaptionAltTextMode".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
        ..Default::default()
    };
    let fields = vec![description, alt_text];
    let typ = TypeDef {
        name: "CaptioningConfig".to_string(),
        rust_path: "sample_core::CaptioningConfig".to_string(),
        fields: fields.clone(),
        ..Default::default()
    };
    let mapper = |ty: &TypeRef| match ty {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => "Option<String>".to_string(),
        TypeRef::Named(name) => name.clone(),
        _ => unreachable!(),
    };

    let (params, defaults, assignments) = constructor_parts_with_field_defaults(&fields, &mapper, &typ);

    assert!(
        params.contains("description: Option<String>"),
        "already-optional sibling must retain its mapped type: {params}"
    );
    assert!(
        !params.contains("Option<Option<String>>"),
        "already-optional sibling must not be double-wrapped: {params}"
    );
    assert_eq!(defaults, "description=None, alt_text=None");
    assert!(
        !assignments.contains("description.unwrap"),
        "already-optional sibling must pass through: {assignments}"
    );
}

#[test]
fn field_default_constructor_does_not_widen_non_enum_bare_defaults() {
    let quality_thresholds = FieldDef {
        name: "quality_thresholds".to_string(),
        ty: TypeRef::Named("OcrQualityThresholds".to_string()),
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "OcrPipelineConfig".to_string(),
        fields: vec![quality_thresholds.clone()],
        ..Default::default()
    };
    let mapper = |ty: &TypeRef| match ty {
        TypeRef::Named(name) => name.clone(),
        _ => unreachable!(),
    };

    let (params, defaults, assignments) = constructor_parts_with_field_defaults(&[quality_thresholds], &mapper, &typ);

    assert_eq!(params, "quality_thresholds: OcrQualityThresholds");
    assert_eq!(defaults, "");
    assert_eq!(assignments, "quality_thresholds");
}
