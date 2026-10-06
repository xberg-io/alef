//! Regression coverage for the wasm-only camelCase recasing branch in
//! `field_conversion_from_core_cfg`. Every test builds a `ConversionConfig` by hand (no wasm
//! backend, no `ApiSurface`) so these are true unit tests of the string-building logic, not an
//! end-to-end codegen run -- the wasm backend wiring that populates
//! `wasm_camel_recased_enums` from a real `JsonWireTypes` instance is a separate, not-yet-made
//! change (see the task assessment).

use super::field_conversion_from_core_cfg;
use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::config::WasmCamelRecasedEnum;
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use std::collections::HashMap;

fn format_metadata_rec() -> WasmCamelRecasedEnum<'static> {
    WasmCamelRecasedEnum {
        out_wire_type: "__AlefWireOutWasmFormatMetadata",
        in_wire_type: "__AlefWireInWasmFormatMetadata",
        retag_fn_name: "__alef_wire_retag_Wasm",
        core_tag_key: "format_type",
        js_tag_key: "formatType",
    }
}

fn config<'a>(
    tagged_names: &'a AHashSet<String>,
    recased: Option<&'a HashMap<String, WasmCamelRecasedEnum<'a>>>,
) -> ConversionConfig<'a> {
    ConversionConfig {
        map_uses_jsvalue: true,
        tagged_data_enum_names: Some(tagged_names),
        wasm_camel_recased_enums: recased,
        ..ConversionConfig::default()
    }
}

/// CONTROL: with no `wasm_camel_recased_enums` entry at all, a bare tagged-enum field must
/// keep emitting the exact raw expression the pre-existing `tagged_data_enum_names` path
/// always has -- proving the new branch is a strict opt-in with zero effect until a backend
/// actually populates the map.
#[test]
fn bare_field_with_no_recased_map_keeps_the_raw_snake_case_passthrough() {
    let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
    let cfg = config(&tagged, None);
    let out = field_conversion_from_core_cfg(
        "format",
        &TypeRef::Named("FormatMetadata".to_string()),
        false,
        false,
        &AHashSet::new(),
        &cfg,
    );
    assert_eq!(
        out, "format: serde_wasm_bindgen::to_value(&val.format).unwrap_or(JsValue::NULL)",
        "unexpected output: {out}"
    );
}

/// A name present in `tagged_data_enum_names` but ABSENT from `wasm_camel_recased_enums`
/// (e.g. a plain tagged-discriminator enum, or a genuinely untagged one) must also keep the
/// raw passthrough -- this is the safety property behind shape 2 of the assessment: only
/// enums explicitly registered as fully-flattened get recased.
#[test]
fn name_absent_from_recased_map_keeps_the_raw_passthrough_even_with_map_present() {
    let tagged: AHashSet<String> = ["FormatMetadata".to_string(), "ChatRole".to_string()]
        .into_iter()
        .collect();
    let mut recased = HashMap::new();
    recased.insert("FormatMetadata".to_string(), format_metadata_rec());
    let cfg = config(&tagged, Some(&recased));
    let out = field_conversion_from_core_cfg(
        "role",
        &TypeRef::Named("ChatRole".to_string()),
        false,
        false,
        &AHashSet::new(),
        &cfg,
    );
    assert_eq!(
        out, "role: serde_wasm_bindgen::to_value(&val.role).unwrap_or(JsValue::NULL)",
        "unexpected output: {out}"
    );
}

/// The reported defect's actual shape: a bare `FormatMetadata` field, recased.
#[test]
fn bare_recased_field_routes_through_the_wire_type_pipeline() {
    let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
    let mut recased = HashMap::new();
    recased.insert("FormatMetadata".to_string(), format_metadata_rec());
    let cfg = config(&tagged, Some(&recased));
    let out = field_conversion_from_core_cfg(
        "format",
        &TypeRef::Named("FormatMetadata".to_string()),
        false,
        false,
        &AHashSet::new(),
        &cfg,
    );
    assert_eq!(
        out,
        "format: js_sys::JSON::parse(&serde_json::to_string(&__alef_wire_retag_Wasm(serde_json::to_value(&val.format).ok()\
             .and_then(|raw| serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw).ok())\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"format_type\", \"formatType\")).unwrap_or_default()).unwrap_or(JsValue::NULL)",
        "unexpected output: {out}"
    );
}

#[test]
fn optional_recased_field_maps_over_the_option() {
    let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
    let mut recased = HashMap::new();
    recased.insert("FormatMetadata".to_string(), format_metadata_rec());
    let cfg = config(&tagged, Some(&recased));
    let out = field_conversion_from_core_cfg(
        "format",
        &TypeRef::Named("FormatMetadata".to_string()),
        true,
        false,
        &AHashSet::new(),
        &cfg,
    );
    assert_eq!(
        out,
        "format: val.format.as_ref().map(|v| js_sys::JSON::parse(&serde_json::to_string(&__alef_wire_retag_Wasm(serde_json::to_value(&v).ok()\
             .and_then(|raw| serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw).ok())\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"format_type\", \"formatType\")).unwrap_or_default()).unwrap_or(JsValue::NULL))",
        "unexpected output: {out}"
    );
}

#[test]
fn vec_recased_field_maps_each_element_through_the_pipeline() {
    let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
    let mut recased = HashMap::new();
    recased.insert("FormatMetadata".to_string(), format_metadata_rec());
    let cfg = config(&tagged, Some(&recased));
    let out = field_conversion_from_core_cfg(
        "formats",
        &TypeRef::Vec(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
        false,
        false,
        &AHashSet::new(),
        &cfg,
    );
    assert!(
        out.starts_with("formats: js_sys::JSON::parse(&serde_json::to_string(&val.formats.iter().map(|item| "),
        "unexpected output: {out}"
    );
    assert!(
        out.contains("serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw)"),
        "must decode each element through the wire type, got: {out}"
    );
    assert!(
        out.ends_with(").collect::<Vec<serde_json::Value>>()).unwrap_or_default()).unwrap_or(JsValue::NULL)"),
        "unexpected output: {out}"
    );
}
