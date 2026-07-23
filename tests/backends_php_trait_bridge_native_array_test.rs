//! PHP trait-bridge native-array callback args + type-conditional return decoding (#1304).
//!
//! Mirrors `backends_napi_trait_bridge_native_array_test.rs`. Neutral fixture: a plugin trait
//! `Embedder` with three async methods, all taking a `Vec<String>` param:
//!   - `embed`    returns `Vec<Vec<f32>>` — arg is native-encodable, and (unlike napi-rs, which
//!     lacks `FromNapiValue for f32`) ext-php-rs 0.15.15 implements `FromZval` for every
//!     `PrimitiveType` including `f32` (`impl FromZval<'_> for f32` in
//!     `ext-php-rs-0.15.15/src/types/mod.rs`, via `zval.double().map(|v| v as f32)`), so the
//!     return decodes natively too — there is no php-side f32 exclusion.
//!   - `tag`      returns `Vec<String>` — both arg and return are php-native end to end.
//!   - `describe` returns the known serde struct `Doc` — MUST keep the existing native-struct /
//!     JSON fallback path (regression guard: the new generic native-decode branch must not touch
//!     struct returns, which are handled by the pre-existing `native_return_binding` mechanism).
//!
//! Asserts:
//!   (a) every method's `texts: Vec<String>` argument is passed as a native PHP array via
//!       `IntoZval`, never `format!("{:?}", texts)`.
//!   (b) `tag`'s return is decoded natively via `FromZval`, no `val.string()` + `serde_json`.
//!   (c) `embed`'s `Vec<Vec<f32>>` return ALSO decodes natively via `FromZval` (ext-php-rs can
//!       decode `f32`, unlike napi-rs) — this is the documented php/napi divergence.
//!   (d) `describe`'s return (`Doc` struct) keeps its existing native-struct-or-JSON fallback,
//!       not the new generic native-decode branch.

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
    alef::backends::php::trait_bridge::gen_trait_bridge(
        &trait_def,
        &embedder_bridge_cfg(),
        "test_lib",
        "TestLibError",
        "TestLibError::from({msg})",
        &api,
    )
    .code
}

#[test]
fn vec_string_param_is_passed_as_native_php_array_for_every_method() {
    let code = gen_bridge_code();
    for name in ["embed", "tag", "describe"] {
        let m = method_source(&code, name);
        assert!(
            m.contains("ext_php_rs::convert::IntoZval::into_zval(texts.clone(), false)"),
            "`{name}`'s `texts: Vec<String>` arg must be passed as a native PHP array:\n{m}"
        );
        assert!(
            !m.contains("format!(\"{:?}\", texts)"),
            "`{name}`'s `texts` arg must NOT be Debug-string encoded:\n{m}"
        );
    }
}

#[test]
fn native_decodable_vec_string_return_type_decodes_via_from_zval() {
    let code = gen_bridge_code();
    let tag = method_source(&code, "tag");
    assert!(
        tag.contains("<Vec<String> as ext_php_rs::convert::FromZval>::from_zval(&val)"),
        "`tag`'s `Vec<String>` return is fully php-native and must decode via FromZval:\n{tag}"
    );
    assert!(
        !tag.contains("val.string()") && !tag.contains("serde_json::from_str"),
        "`tag`'s native return must NOT go through the JSON string fallback:\n{tag}"
    );
}

#[test]
fn native_decodable_nested_f32_return_type_also_decodes_via_from_zval() {
    // Divergence from napi: ext-php-rs 0.15.15 implements `FromZval` for `f32`
    // (`impl FromZval<'_> for f32` in `src/types/mod.rs`), unlike napi-rs which has no
    // `FromNapiValue for f32`. So `Vec<Vec<f32>>` is php-decodable and must NOT fall back to JSON.
    let code = gen_bridge_code();
    let embed = method_source(&code, "embed");
    assert!(
        embed.contains("<Vec<Vec<f32>> as ext_php_rs::convert::FromZval>::from_zval(&val)"),
        "`embed`'s `Vec<Vec<f32>>` return must decode natively via FromZval (ext-php-rs supports \
         f32 decoding):\n{embed}"
    );
    assert!(
        !embed.contains("val.string()") && !embed.contains("serde_json::from_str"),
        "`embed` must NOT fall back to the JSON string path:\n{embed}"
    );
}

#[test]
fn named_struct_return_type_keeps_native_struct_or_json_fallback_no_regression() {
    let code = gen_bridge_code();
    let describe = method_source(&code, "describe");
    // `Doc` is picked up by the pre-existing native-struct-return mechanism (gated on
    // `struct_return_types`), not the new generic native-decode branch added for #1304.
    assert!(
        describe.contains("<&Doc as ext_php_rs::convert::FromZval>::from_zval(&val)"),
        "`describe`'s `Doc` struct return must keep the native-struct extraction path:\n{describe}"
    );
    assert!(
        describe.contains("val.string()") && describe.contains("serde_json::from_str"),
        "`describe` must still fall back to JSON when the PHP value isn't the native object:\n{describe}"
    );
    assert!(
        !describe.contains("<test_lib::Doc as ext_php_rs::convert::FromZval>::from_zval(&val)\n")
            && !describe.contains(".ok_or_else("),
        "`describe` must NOT go through the new generic native-decode branch (that branch is only \
         for String/primitive/Vec returns, not Named struct returns):\n{describe}"
    );
}
