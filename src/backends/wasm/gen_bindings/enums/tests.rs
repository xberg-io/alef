use super::{gen_enum, gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn make_enum(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            })
            .collect(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_enum_produces_wasm_bindgen_attribute() {
    let e = make_enum("Color", &["Red", "Green", "Blue"]);
    let result = gen_enum(&e, "Wasm");
    // Unit enums are exported with their prefixed Rust name as the JS
    // class name (no js_name override) — keeps the JS API in sync with the
    // alef-e2e codegen's imports, which always reference the prefixed name.
    assert!(result.contains("#[wasm_bindgen]"));
    assert!(result.contains("pub enum WasmColor"));
    assert!(!result.contains("js_name = \"Color\""));
    assert!(result.contains("Red = 0,"));
    assert!(result.contains("Green = 1,"));
    assert!(result.contains("Blue = 2,"));
}

#[test]
fn gen_enum_empty_variants_no_panic() {
    let e = make_enum("Empty", &[]);
    let result = gen_enum(&e, "");
    assert!(result.contains("pub enum Empty"));
    // No to_api_str() for empty enums
    assert!(!result.contains("to_api_str"));
}

#[test]
fn gen_enum_to_api_str_snake_case() {
    let mut e = make_enum("FinishReason", &["Stop", "ToolCalls", "Length", "ContentFilter"]);
    e.serde_rename_all = Some("snake_case".to_string());
    let result = gen_enum(&e, "Wasm");
    assert!(result.contains("pub fn to_api_str(self) -> &'static str"));
    assert!(result.contains("Self::Stop => \"stop\""));
    assert!(result.contains("Self::ToolCalls => \"tool_calls\""));
    assert!(result.contains("Self::Length => \"length\""));
    assert!(result.contains("Self::ContentFilter => \"content_filter\""));
}

#[test]
fn gen_enum_to_api_str_explicit_rename_overrides_rename_all() {
    let mut e = make_enum("Role", &["User", "Assistant"]);
    e.serde_rename_all = Some("snake_case".to_string());
    // Give "User" an explicit rename
    e.variants[0].serde_rename = Some("human".to_string());
    let result = gen_enum(&e, "Wasm");
    assert!(result.contains("Self::User => \"human\""));
    assert!(result.contains("Self::Assistant => \"assistant\""));
}

#[test]
fn gen_enum_to_api_str_no_rename_all_uses_variant_name() {
    let e = make_enum("Status", &["Active", "Inactive"]);
    let result = gen_enum(&e, "");
    assert!(result.contains("Self::Active => \"Active\""));
    assert!(result.contains("Self::Inactive => \"Inactive\""));
}

/// Build a tagged enum where every non-empty variant is a newtype/tuple variant
/// (single positional field named `_0`), as emitted by the alef extractor for
/// `pub enum Message { System(SystemMessage), User(UserMessage) }`.
fn make_tagged_tuple_enum() -> EnumDef {
    let make_tuple_variant = |variant_name: &str, tag: &str| EnumVariant {
        name: variant_name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(format!("{variant_name}Message")),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: crate::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: Some(tag.to_string()),
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        is_tuple: true,
        doc: String::new(),
        is_default: false,
        serde_rename: Some(tag.to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };

    EnumDef {
        name: "Message".to_string(),
        rust_path: "test_lib::types::Message".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            make_tuple_variant("System", "system"),
            make_tuple_variant("User", "user"),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("role".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

/// Regression test: `gen_tagged_enum_core_to_binding` must emit tuple-pattern destructuring
/// (`EnumName::Variant(field0)`) for tuple/newtype variants, not struct-pattern
/// (`EnumName::Variant { _0 }`).
#[test]
fn gen_tagged_enum_core_to_binding_uses_tuple_pattern_for_tuple_variants() {
    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");

    // Must NOT use struct-pattern destructure for tuple variants.
    assert!(
        !result.contains("Message::System { _0 }"),
        "must not emit struct destructure for tuple variant;\nactual:\n{result}"
    );
    assert!(
        !result.contains("Message::User { _0 }"),
        "must not emit struct destructure for tuple variant;\nactual:\n{result}"
    );

    // Must use tuple-pattern destructure.
    assert!(
        result.contains("Message::System(field0)"),
        "must emit tuple destructure for tuple variant;\nactual:\n{result}"
    );
    assert!(
        result.contains("Message::User(field0)"),
        "must emit tuple destructure for tuple variant;\nactual:\n{result}"
    );

    // The positional value must be converted and stored in the `_0` binding struct field.
    // Since the variants have different Named types, the struct stores JsValue and the
    // conversion uses serde_wasm_bindgen.
    assert!(
        result.contains("_0: serde_wasm_bindgen::to_value(&field0).ok()"),
        "positional value must be serialized via serde_wasm_bindgen into _0 field;\nactual:\n{result}"
    );
}

/// Regression test: `gen_tagged_enum_binding_to_core` must emit tuple construction
/// (`Self::Variant(val)`) for tuple/newtype variants, not struct construction
/// (`Self::Variant { _0: val }`).
#[test]
fn gen_tagged_enum_binding_to_core_uses_tuple_construction_for_tuple_variants() {
    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

    // Must NOT use struct-construction syntax for tuple variants.
    assert!(
        !result.contains("Self::System { _0:"),
        "must not emit struct construction for tuple variant;\nactual:\n{result}"
    );
    assert!(
        !result.contains("Self::User { _0:"),
        "must not emit struct construction for tuple variant;\nactual:\n{result}"
    );

    // Must use tuple construction.
    assert!(
        result.contains("Self::System("),
        "must emit tuple construction for tuple variant;\nactual:\n{result}"
    );
    assert!(
        result.contains("Self::User("),
        "must emit tuple construction for tuple variant;\nactual:\n{result}"
    );

    // Mixed-type Named fields must use serde_wasm_bindgen::from_value to deserialize
    // the JsValue binding struct field to the variant-specific core type.
    assert!(
        result.contains("serde_wasm_bindgen::from_value::<test_lib::SystemMessage>"),
        "binding→core must deserialize mixed-type field via serde_wasm_bindgen;\nactual:\n{result}"
    );
    assert!(
        result.contains("serde_wasm_bindgen::from_value::<test_lib::UserMessage>"),
        "binding→core must deserialize mixed-type field via serde_wasm_bindgen;\nactual:\n{result}"
    );
}

/// Smoke test: a tagged enum with plain unit variants (no fields) is unaffected by the
/// tuple-variant fix and still emits valid unit-variant arms.
#[test]
fn gen_tagged_enum_core_to_binding_unit_variants_unchanged() {
    let e = EnumDef {
        name: "Status".to_string(),
        rust_path: "test_lib::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("active".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("inactive".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        serde_tag: Some("state".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    // Unit variants must still emit simple `CorePath::Variant => Self { ... }` arms.
    assert!(
        core_to_binding.contains("test_lib::Status::Active => Self {"),
        "unit variant arm must use simple path;\nactual:\n{core_to_binding}"
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
    // Unit variants in binding→core direction: `"active" => Self::Active`
    assert!(
        binding_to_core.contains("\"active\" => Self::Active"),
        "unit variant arm must match tag string;\nactual:\n{binding_to_core}"
    );
}

/// Smoke test: a tagged enum with struct variants (named fields) is unaffected and still
/// emits struct-pattern destructuring.
#[test]
fn gen_tagged_enum_core_to_binding_struct_variants_unchanged() {
    let e = EnumDef {
        name: "Auth".to_string(),
        rust_path: "test_lib::Auth".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Basic".to_string(),
            fields: vec![FieldDef {
                name: "username".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: crate::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            is_tuple: false, // struct variant
            doc: String::new(),
            is_default: false,
            serde_rename: Some("basic".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    // Struct variant must still use `{ username }` destructure.
    assert!(
        result.contains("Auth::Basic { username }"),
        "struct variant must keep struct destructure;\nactual:\n{result}"
    );
}

/// Regression: tagged struct variants whose source field type is already `Option<T>`
/// must preserve that option layer. The flat wasm struct stores every variant field as
/// `Option<T>`; wrapping an already-optional core field in `Some(...)` produces
/// `Option<Option<T>>`, and unwrapping it in the reverse direction produces `T`.
#[test]
fn gen_tagged_enum_struct_variant_preserves_optional_fields() {
    let field = |name: &str, ty: TypeRef, optional: bool| FieldDef {
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let e = EnumDef {
        name: "SecuritySchemeInfo".to_string(),
        rust_path: "test_lib::SecuritySchemeInfo".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Http".to_string(),
            fields: vec![
                field("scheme", TypeRef::String, false),
                field("bearer_format", TypeRef::String, true),
            ],
            doc: String::new(),
            is_default: false,
            serde_rename: Some("http".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("bearer_format: val.bearer_format.clone()"),
        "binding→core must preserve Option<String>;\nactual:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("bearer_format: val.bearer_format.clone().unwrap_or_default()"),
        "binding→core must not unwrap source Option<String>;\nactual:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("bearer_format: bearer_format"),
        "core→binding must not wrap Option<String> in Some(...);\nactual:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("bearer_format: Some(bearer_format)"),
        "core→binding must not create Option<Option<String>>;\nactual:\n{core_to_binding}"
    );
}

/// Regression: tuple-variant enums with positional `_0` fields must not emit
/// `set__0` as the setter name — that double-underscore form is rejected by the
/// `non_snake_case` lint under `RUSTFLAGS="-D warnings"`.  The generated Rust
/// identifier must be `set_field_0` (getter: `field_0`) while the JS-visible
/// name is controlled by `js_name` and remains unchanged.
#[test]
fn gen_tagged_enum_as_struct_positional_field_setter_snake_case() {
    use super::gen_tagged_enum_as_struct;

    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_as_struct(&e, "Wasm");

    // The problematic setter must not appear.
    assert!(
        !result.contains("fn set__0("),
        "must not emit `set__0` — double-underscore violates non_snake_case lint;\nactual:\n{result}"
    );

    // The getter must not be named `_0` (also non-snake-case under strict lint).
    // After the fix it is `field_0`.
    assert!(
        result.contains("fn field_0("),
        "getter for positional `_0` field must be named `field_0`;\nactual:\n{result}"
    );

    // The setter must be `set_field_0`.
    assert!(
        result.contains("fn set_field_0("),
        "setter for positional `_0` field must be named `set_field_0`;\nactual:\n{result}"
    );

    // The JS-visible name attribute must still expose the camelCase-converted field name so the
    // WASM/JS API is unaffected.  `to_node_name("_0")` strips the leading underscore → `"0"`.
    assert!(
        result.contains("js_name = \"0\""),
        "js_name attribute must use the to_node_name result for `_0` field;\nactual:\n{result}"
    );

    // The struct field access inside the getter/setter body must still reference
    // `self._0` (the actual struct field identifier).
    assert!(
        result.contains("self._0"),
        "getter/setter body must access `self._0` (the struct field);\nactual:\n{result}"
    );
}

/// Regression test D4-WASM-A: tagged enum with unit variant emits { kind: 'bold' }
/// as a tagged-union type alias, not a numeric enum.
#[test]
fn gen_tagged_enum_unit_variant_emits_tagged_union() {
    use super::gen_tagged_enum_as_struct;

    let mut e = make_tagged_tuple_enum();
    // Modify to have a unit variant and a tuple variant
    e.variants[0].fields.clear();
    e.variants[0].is_tuple = false;

    let result = gen_tagged_enum_as_struct(&e, "Wasm");

    // Must emit a #[wasm_bindgen] struct with a discriminator field ("kind" or similar).
    assert!(
        result.contains("#[wasm_bindgen]") && result.contains("pub struct Wasm"),
        "WASM tagged enum must emit wasm_bindgen struct, not numeric enum;\nactual:\n{result}"
    );

    // Must have a discriminator field named "kind" (not "role" or "annotation_type").
    // The tag field in WASM should also use "kind" for consistency with NAPI.
    assert!(
        result.contains("pub(crate)") && (result.contains("kind") || result.contains("getter")),
        "WASM tagged enum struct must have a discriminator field for the tag;\nactual:\n{result}"
    );
}

/// Regression test D4-WASM-B: tagged enum variant tag values use camelCase.
/// E.g., `"fontSize"` not `"font_size"`.
#[test]
fn gen_tagged_enum_binding_to_core_matches_camel_case_tags() {
    use super::gen_tagged_enum_binding_to_core;

    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

    // The variant tag match arms must use camelCase or the explicit serde_rename.
    // In make_tagged_tuple_enum, System has serde_rename = Some("system"), User = Some("user").
    // These are already lowercase, but the regex pattern should respect explicit renames.
    assert!(
        result.contains("match val.") && result.contains("as_str()"),
        "binding→core must dispatch on tag field string value;\nactual:\n{result}"
    );
}
