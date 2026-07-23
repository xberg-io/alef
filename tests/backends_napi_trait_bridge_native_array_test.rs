//! NAPI trait-bridge native-array callback args + type-conditional return decoding (#1304).
//!
//! Neutral fixture: a plugin trait `Embedder` with three async methods, all taking a
//! `Vec<String>` param:
//!   - `embed`  returns `Vec<Vec<f32>>` — arg is native-encodable; `f32` has no
//!     `FromNapiValue` impl in napi-rs, so the return decodes via the f64-analog bridge
//!     (`Vec<Vec<f64>>` via `FromNapiValue`, then an element-wise `as f32` cast) instead of
//!     a JSON round-trip.
//!   - `tag`    returns `Vec<String>` — both arg and return are napi-native end to end, no
//!     bridging needed.
//!   - `describe` returns the known serde struct `Doc` — MUST keep the JSON fallback
//!     (regression guard: return-type branching must not touch struct returns).
//!
//! Asserts:
//!   (a) every method's `texts: Vec<String>` argument is passed as a native JS array via
//!       `ToNapiValue`, never `format!("{:?}", texts)`.
//!   (b) `tag`'s return is decoded natively via `FromNapiValue`, no `coerce_to_string()`.
//!   (c) `embed`'s return decodes via the f64 analog + element-wise `as f32` cast, matching
//!       its `Vec<Vec<_>>` nesting depth, and does NOT fall back to `coerce_to_string()`.
//!   (d) `describe`'s return (`Doc` struct) keeps the JSON fallback.

use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::*;

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn make_param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        is_ref,
        ..Default::default()
    }
}

fn async_method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![make_param("texts", TypeRef::Vec(Box::new(TypeRef::String)), false)],
        return_type,
        receiver: Some(ReceiverKind::Ref),
        error_type: Some("Error".to_string()),
        is_async: true,
        ..Default::default()
    }
}

/// `embed(texts) -> Vec<Vec<f32>>`, `tag(texts) -> Vec<String>`,
/// `describe(texts) -> Doc` (known serde struct).
fn embedder_trait() -> TypeDef {
    TypeDef {
        name: "Embedder".to_string(),
        rust_path: "test_lib::Embedder".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![
            async_method(
                "embed",
                TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))))),
            ),
            async_method("tag", TypeRef::Vec(Box::new(TypeRef::String))),
            async_method("describe", TypeRef::Named("Doc".to_string())),
        ],
        ..Default::default()
    }
}

fn embedder_api() -> ApiSurface {
    let mut doc = TypeDef {
        name: "Doc".to_string(),
        rust_path: "test_lib::Doc".to_string(),
        has_serde: true,
        fields: vec![make_field("text", TypeRef::String)],
        ..Default::default()
    };
    doc.is_return_type = true;

    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![embedder_trait(), doc],
        ..Default::default()
    }
}

fn embedder_bridge_cfg() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Embedder".to_string(),
        register_fn: Some("register_embedder".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }
}

/// Extracts the source for a single `async fn <name>(...)` method body from the full
/// bridge `impl` code, up to (but not including) the next `async fn` (or end of string).
fn method_source<'a>(code: &'a str, name: &str) -> &'a str {
    let marker = format!("async fn {name}(");
    let start = code
        .find(&marker)
        .unwrap_or_else(|| panic!("method `{name}` not found in:\n{code}"));
    let rest = &code[start..];
    match rest[marker.len()..].find("async fn ") {
        Some(next) => &rest[..marker.len() + next],
        None => rest,
    }
}

fn gen_bridge_code() -> String {
    let trait_def = embedder_trait();
    let api = embedder_api();
    alef::backends::napi::trait_bridge::gen_trait_bridge(
        &trait_def,
        &embedder_bridge_cfg(),
        "test_lib",
        "TestLibError",
        "TestLibError::from({msg})",
        &api,
    )
    .expect("gen_trait_bridge must succeed for Embedder")
    .code
}

#[test]
fn vec_string_param_is_passed_as_native_js_array_for_every_method() {
    let code = gen_bridge_code();
    for name in ["embed", "tag", "describe"] {
        let m = method_source(&code, name);
        assert!(
            m.contains("napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), texts.clone())"),
            "`{name}`'s `texts: Vec<String>` arg must be passed as a native JS array:\n{m}"
        );
        assert!(
            !m.contains("format!(\"{:?}\", texts)"),
            "`{name}`'s `texts` arg must NOT be Debug-string encoded:\n{m}"
        );
    }
}

#[test]
fn native_decodable_return_type_decodes_via_from_napi_value() {
    let code = gen_bridge_code();
    let tag = method_source(&code, "tag");
    assert!(
        tag.contains("napi::bindgen_prelude::FromNapiValue::from_napi_value(self.env().raw(), val.raw())"),
        "`tag`'s `Vec<String>` return is fully napi-native and must decode via FromNapiValue:\n{tag}"
    );
    assert!(
        !tag.contains("coerce_to_string()"),
        "`tag`'s native return must NOT go through the JSON string fallback:\n{tag}"
    );
}

#[test]
fn f32_leaved_return_type_decodes_via_f64_bridge_and_elementwise_cast() {
    let code = gen_bridge_code();
    let embed = method_source(&code, "embed");
    assert!(
        embed.contains("napi::bindgen_prelude::FromNapiValue::from_napi_value(self.env().raw(), val.raw())"),
        "`embed`'s `Vec<Vec<f32>>` return must decode natively via the f64 analog \
         (FromNapiValue has no impl for f32, but does for f64):\n{embed}"
    );
    assert!(
        embed.contains(": Vec<Vec<f64>>"),
        "`embed`'s decode must be typed as the f64 analog `Vec<Vec<f64>>`, matching the \
         `Vec<Vec<f32>>` return's nesting depth:\n{embed}"
    );
    assert!(
        embed.contains("__decoded.into_iter().map(|v| v.into_iter().map(|v| v as f32).collect()).collect()"),
        "`embed` must cast the decoded f64 values back to f32 element-wise at the correct \
         Vec<Vec<_>> nesting depth:\n{embed}"
    );
    assert!(
        !embed.contains("coerce_to_string()") && !embed.contains("serde_json::from_str"),
        "`embed`'s return must NOT go through the JSON string fallback now that the f64 \
         bridge makes it natively decodable:\n{embed}"
    );
}

#[test]
fn named_struct_return_type_keeps_json_fallback_no_regression() {
    let code = gen_bridge_code();
    let describe = method_source(&code, "describe");
    assert!(
        describe.contains("coerce_to_string()") && describe.contains("serde_json::from_str"),
        "`describe`'s `Doc` struct return must keep the JSON fallback (no FromNapiValue impl \
         for arbitrary core structs):\n{describe}"
    );
    assert!(
        !describe.contains("FromNapiValue::from_napi_value(self.env().raw(), val.raw())"),
        "`describe` must NOT attempt a native decode for a Named core struct return:\n{describe}"
    );
}
