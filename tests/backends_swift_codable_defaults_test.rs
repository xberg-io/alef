//! Tests for the custom `init(from decoder:)` emitted by `emit_first_class_struct`
//! when the source type has `has_default = true` (i.e. `#[derive(Default)]` or
//! `impl Default` on the Rust side).
//!
//! The custom decoder uses `decodeIfPresent + ?? <fallback>` so JSON inputs that
//! omit fields marked `#[serde(default)]` or `#[serde(skip_serializing_if = "...")]`
//! on the Rust side decode successfully (the auto-synthesised Codable init would
//! otherwise throw `keyNotFound`).

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

// ── helpers (duplicated from gen_bindings_test.rs to keep tests independent) ──

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
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
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn api_with_type(ty: TypeDef) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![ty],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[test]
fn struct_with_serde_default_bool_field_emits_custom_decoder_with_false_fallback() {
    let mut field = make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false);
    field.typed_default = Some(DefaultValue::BoolLiteral(false));
    let mut ty = make_type("Config", vec![field]);
    ty.has_serde = true;
    ty.has_default = true;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("public init(from decoder: any Decoder) throws"),
        "expected custom Codable decoder when has_default is true:\n{content}"
    );
    assert!(
        content.contains("try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? false"),
        "expected `?? false` fallback for BoolLiteral(false) typed default:\n{content}"
    );
    // CodingKeys must be emitted even though `enabled` already matches its wire key —
    // the custom decoder references `CodingKeys.enabled`.
    assert!(
        content.contains("private enum CodingKeys: String, CodingKey"),
        "expected CodingKeys enum to be emitted alongside custom decoder:\n{content}"
    );
}

#[test]
fn struct_with_serde_default_bool_field_emits_custom_decoder_with_true_fallback() {
    let mut field = make_field("structure", TypeRef::Primitive(PrimitiveType::Bool), false);
    field.typed_default = Some(DefaultValue::BoolLiteral(true));
    let mut ty = make_type("Config", vec![field]);
    ty.has_serde = true;
    ty.has_default = true;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("public init(from decoder: any Decoder) throws"),
        "expected custom Codable decoder when has_default is true:\n{content}"
    );
    assert!(
        content.contains("try container.decodeIfPresent(Bool.self, forKey: .structure) ?? true"),
        "expected `?? true` fallback for BoolLiteral(true) typed default:\n{content}"
    );
}

#[test]
fn struct_with_vec_field_and_default_impl_defaults_to_empty_array() {
    let mut field = make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false);
    field.typed_default = Some(DefaultValue::Empty);
    let mut ty = make_type("Bag", vec![field]);
    ty.has_serde = true;
    ty.has_default = true;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("public init(from decoder: any Decoder) throws"),
        "expected custom Codable decoder when has_default is true:\n{content}"
    );
    assert!(
        content.contains("try container.decodeIfPresent([String].self, forKey: .items) ?? []"),
        "expected `?? []` fallback for Vec<String> with Empty typed default:\n{content}"
    );
}

#[test]
fn struct_without_default_impl_does_not_emit_custom_decoder() {
    // `has_default = false` → the auto-synthesised Codable init is sufficient
    // (consumers must supply every field) — no custom decoder should be emitted.
    let mut ty = make_type(
        "Strict",
        vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32), false)],
    );
    ty.has_serde = true;
    ty.has_default = false;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("public struct Strict: Codable, Sendable, Hashable"),
        "must still emit first-class Codable struct:\n{content}"
    );
    assert!(
        !content.contains("public init(from decoder: any Decoder)"),
        "must NOT emit custom decoder for structs without Default impl:\n{content}"
    );
}

#[test]
fn struct_with_optional_field_decodes_to_nil_fallback() {
    let mut field = make_field("chunk_max_size", TypeRef::Primitive(PrimitiveType::Usize), true);
    field.typed_default = Some(DefaultValue::None);
    let mut ty = make_type("Config", vec![field]);
    ty.has_serde = true;
    ty.has_default = true;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("public init(from decoder: any Decoder) throws"),
        "expected custom Codable decoder for has_default struct with optional field:\n{content}"
    );
    // Optional fields use `decodeIfPresent` with the inner type and a `?? nil` fallback.
    assert!(
        content.contains("try container.decodeIfPresent(UInt.self, forKey: .chunkMaxSize) ?? nil"),
        "expected `?? nil` fallback for optional field:\n{content}"
    );
}

#[test]
fn struct_with_string_literal_default_uses_quoted_swift_literal() {
    let mut field = make_field("language", TypeRef::String, false);
    field.typed_default = Some(DefaultValue::StringLiteral("python".to_string()));
    let mut ty = make_type("Config", vec![field]);
    ty.has_serde = true;
    ty.has_default = true;

    let files = SwiftBackend
        .generate_bindings(&api_with_type(ty), &make_config())
        .expect("generate must succeed");
    let content = &files[0].content;

    assert!(
        content.contains("try container.decodeIfPresent(String.self, forKey: .language) ?? \"python\""),
        "expected quoted Swift string literal fallback:\n{content}"
    );
}
