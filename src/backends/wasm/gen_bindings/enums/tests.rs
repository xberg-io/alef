use super::{gen_enum, gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::backends::wasm::type_map::WasmMapper;

/// The override-free mapper every fixture here shares; `gen_enum` reads its `prefix`.
pub(super) fn mapper(prefix: &str) -> WasmMapper {
    WasmMapper::new(std::collections::HashMap::new(), prefix.to_string())
}
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

/// The payload structs [`make_tagged_tuple_enum`]'s variants wrap (`System(SystemMessage)`,
/// `User(UserMessage)`), so the resolution-aware `serde_flattens_newtype_payload` can actually
/// resolve them and the fixture keeps exercising the flattened shape it was built to test. ~keep
pub(super) fn make_tagged_tuple_enum_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SystemMessage".to_string(),
            ..Default::default()
        },
        TypeDef {
            name: "UserMessage".to_string(),
            ..Default::default()
        },
    ]
}

pub(super) fn make_enum(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                serde_untagged: false,
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
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
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

#[test]
fn gen_enum_produces_wasm_bindgen_attribute() {
    let e = make_enum("Color", &["Red", "Green", "Blue"]);
    let result = gen_enum(&e, &mapper("Wasm"), "", &std::collections::HashSet::new(), &[], &[]);
    assert!(result.contains("#[wasm_bindgen]"));
    assert!(result.contains("pub enum WasmColor"));
    assert!(!result.contains("js_name = \"Color\""));
    assert!(result.contains("Red = 0,"));
    assert!(result.contains("Green = 1,"));
    assert!(result.contains("Blue = 2,"));
}

/// Regression: a default-representation (externally tagged, no
/// `#[serde(tag/content/untagged)]`) data enum shaped like alef's own `FormatMetadata` fixture
/// (see `tests/backends_kotlin_android_gen_bindings_test.rs`), with a `Custom(String)` payload
/// variant alongside a unit variant, must not be emitted as a plain `#[wasm_bindgen]` C-style
/// enum — that representation only holds unit variants, so `Custom`'s payload was silently
/// dropped (`Custom = 1` with no field) before this fix. It must route through the same
/// discriminator-struct emitter used for an explicit `#[serde(tag = "...")]` enum, and the
/// binding<->core conversions must carry the payload both ways. ~keep
#[test]
fn default_tagged_data_enum_preserves_custom_string_variant_payload_round_trip() {
    let e = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "demo::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                serde_untagged: false,
                name: "Pdf".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let output = gen_enum(&e, &mapper("Wasm"), "", &std::collections::HashSet::new(), &[], &[]);
    assert!(
        output.contains("pub struct WasmFormatMetadata"),
        "a payload-carrying default-tagged enum must become a discriminator struct, \
         not a plain #[wasm_bindgen] C-style enum; got:\n{output}"
    );
    assert!(
        output.contains("pub(crate) _0: Option<String>"),
        "the Custom(String) payload field must survive on the binding struct; got:\n{output}"
    );
    assert!(
        !output.contains(" = 0,") && !output.contains(" = 1,"),
        "a data-carrying enum must not be emitted as a discriminant-valued C-style enum; got:\n{output}"
    );

    // config/input direction: JS object -> core enum.
    let binding_to_core = gen_tagged_enum_binding_to_core(&e, "demo", "Wasm");
    assert!(
        binding_to_core.contains(r#""Custom" => Self::Custom(val._0.clone().unwrap_or_default())"#),
        "binding-to-core conversion must forward the Custom payload, not discard it; got:\n{binding_to_core}"
    );

    // result/output direction: core enum -> JS object.
    let core_to_binding = gen_tagged_enum_core_to_binding(&e, "demo", "Wasm");
    assert!(
        core_to_binding.contains("demo::FormatMetadata::Custom(field0) => Self {")
            && core_to_binding.contains(r#"r#type: "Custom".to_string(),"#)
            && core_to_binding.contains("_0: Some(field0),"),
        "core-to-binding conversion must forward the Custom payload, not discard it; got:\n{core_to_binding}"
    );
}

#[test]
fn gen_enum_empty_variants_no_panic() {
    let e = make_enum("Empty", &[]);
    let result = gen_enum(&e, &mapper(""), "", &std::collections::HashSet::new(), &[], &[]);
    assert!(result.contains("pub enum Empty"));
    assert!(!result.contains("to_api_str"));
}

#[test]
fn gen_enum_to_api_str_snake_case() {
    let mut e = make_enum("FinishReason", &["Stop", "ToolCalls", "Length", "ContentFilter"]);
    e.serde_rename_all = Some("snake_case".to_string());
    let result = gen_enum(&e, &mapper("Wasm"), "", &std::collections::HashSet::new(), &[], &[]);
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
    e.variants[0].serde_rename = Some("human".to_string());
    let result = gen_enum(&e, &mapper("Wasm"), "", &std::collections::HashSet::new(), &[], &[]);
    assert!(result.contains("Self::User => \"human\""));
    assert!(result.contains("Self::Assistant => \"assistant\""));
}

#[test]
fn gen_enum_to_api_str_no_rename_all_uses_variant_name() {
    let e = make_enum("Status", &["Active", "Inactive"]);
    let result = gen_enum(&e, &mapper(""), "", &std::collections::HashSet::new(), &[], &[]);
    assert!(result.contains("Self::Active => \"Active\""));
    assert!(result.contains("Self::Inactive => \"Inactive\""));
}

/// alef #536/#538's shape, corrected: `#[wasm_bindgen]` cannot express a per-variant `#[cfg(...)]`
/// guard at all -- see `gen_enum`'s doc comment and
/// `codegen::conversions::enum_variant_declaration_without_cfg_attribute`'s doc comment
/// (rustwasm/wasm-bindgen#2058) for why the earlier fix here, which mirrored the arm's
/// `#[cfg(...)]` onto the wrapper's own declaration, was itself invalid: the macro parses
/// variants before cfg-stripping runs and unconditionally generates code referencing every one it
/// saw, so a declared-but-conditionally-compiled variant produces `E0599: no variant ... found`
/// pointing AT the declaration line. The correct fix resolves a host-owned cfg-gated variant
/// definitively at generation time: fully present with no `#[cfg(...)]` attribute anywhere, or
/// fully absent. This is the positive control -- the gating feature IS in `configured_features`,
/// so `Extended` must be declared, and declared unconditionally (no `#[cfg(...)]` token at all,
/// anywhere in the output, or wasm-bindgen's expansion breaks exactly as described above). ~keep
#[test]
fn gen_enum_declares_host_cfg_variant_unconditionally_when_feature_configured() {
    let enum_def = EnumDef {
        name: "RenderMode".to_string(),
        rust_path: "core_crate::RenderMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fast".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extended".to_string(),
                cfg: Some(r#"feature = "extended-mode""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let configured: std::collections::HashSet<&str> = ["extended-mode"].into_iter().collect();

    let output = gen_enum(&enum_def, &mapper("Wasm"), "core_crate", &configured, &[], &[]);

    assert!(
        !output.contains("#[cfg("),
        "a wasm_bindgen enum must never carry a #[cfg(...)] attribute on any variant, got:\n{output}"
    );
    assert!(
        output.contains("Extended = 1,"),
        "the configured variant must be declared unconditionally, got:\n{output}"
    );
    assert!(
        output.contains("Self::Extended => "),
        "to_api_str must handle the configured variant, got:\n{output}"
    );
    assert!(
        output.contains("Some(Self::Extended)"),
        "from_api_str must handle the configured variant, got:\n{output}"
    );
}

/// Negative control for the test above, and the actual #536/#538 regression test: the gating
/// feature is NOT in `configured_features` (mirrors a consumer dependency built without it), so
/// `Extended` must be omitted from the wrapper entirely -- not declared behind a `#[cfg(...)]`
/// guard (invalid, see above), just genuinely absent, along with every reference to it in
/// `to_api_str`/`from_api_str`. Load-bearing gating: a fixture with every feature enabled, or no
/// `cfg` at all, cannot reproduce this -- it would pass whether or not generation-time resolution
/// actually ran. ~keep
#[test]
fn gen_enum_omits_host_cfg_variant_entirely_when_feature_not_configured() {
    let enum_def = EnumDef {
        name: "RenderMode".to_string(),
        rust_path: "core_crate::RenderMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fast".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extended".to_string(),
                cfg: Some(r#"feature = "extended-mode""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let configured: std::collections::HashSet<&str> = ["other-feature"].into_iter().collect();

    let output = gen_enum(&enum_def, &mapper("Wasm"), "core_crate", &configured, &[], &[]);

    assert!(
        !output.contains("#[cfg("),
        "a wasm_bindgen enum must never carry a #[cfg(...)] attribute on any variant, got:\n{output}"
    );
    assert!(
        !output.contains("Extended"),
        "an unconfigured host-owned variant must not be declared or referenced at all, got:\n{output}"
    );
    assert!(
        output.contains("Fast = 0,"),
        "the still-configured variant must remain, got:\n{output}"
    );
}

/// A foreign-crate cfg-gated variant's declaration stays unconditional regardless of
/// `configured_features` -- see `enum_variant_declaration_without_cfg_attribute`'s doc comment:
/// `gen_enum` calls it directly and it hardcodes `None` for a foreign variant no matter what its
/// own caller passed in, so `gen_enum`'s declaration path is architecturally independent of
/// wasm's `ConversionConfig.configured_features` (that field now IS threaded, so the conversion
/// side's catch-all suppression for a proven-unreachable foreign variant is live -- see alef
/// #538 -- but the declaration side deliberately stays unconditional; see
/// `enum_variant_declaration_without_cfg_attribute`'s own doc comment for why: only a HOST-owned
/// variant's declaration is ever resolved definitively). ~keep
#[test]
fn gen_enum_keeps_foreign_cfg_variant_unconditionally_regardless_of_configured_features() {
    let enum_def = EnumDef {
        name: "RoutingStrategy".to_string(),
        rust_path: "dep_crate::RoutingStrategy".to_string(),
        variants: vec![
            EnumVariant {
                name: "Primary".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Extra".to_string(),
                cfg: Some(r#"feature = "extra-tier""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let configured: std::collections::HashSet<&str> = ["other-feature"].into_iter().collect();

    let output = gen_enum(&enum_def, &mapper("Wasm"), "core_crate", &configured, &[], &[]);

    assert!(
        !output.contains("#[cfg("),
        "a wasm_bindgen enum must never carry a #[cfg(...)] attribute on any variant, got:\n{output}"
    );
    assert!(
        output.contains("Extra = 1,"),
        "a foreign-crate variant must stay unconditionally declared, got:\n{output}"
    );
}

/// Build a tagged enum where every non-empty variant is a newtype/tuple variant
/// (single positional field named `_0`), as emitted by the alef extractor for
/// `pub enum Message { System(SystemMessage), User(UserMessage) }`.
pub(super) fn make_tagged_tuple_enum() -> EnumDef {
    let make_tuple_variant = |variant_name: &str, tag: &str| EnumVariant {
        serde_untagged: false,
        name: variant_name.to_string(),
        sensitive: false,
        fields: vec![FieldDef {
            version: Default::default(),
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            sensitive: false,
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
        serde_content: None,
        serde_tag: Some("role".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// Regression test: `gen_tagged_enum_core_to_binding` must emit tuple-pattern destructuring
/// (`EnumName::Variant(field0)`) for tuple/newtype variants, not struct-pattern
/// (`EnumName::Variant { _0 }`).
#[test]
fn gen_tagged_enum_core_to_binding_uses_tuple_pattern_for_tuple_variants() {
    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");

    assert!(
        !result.contains("Message::System { _0 }"),
        "must not emit struct destructure for tuple variant;\nactual:\n{result}"
    );
    assert!(
        !result.contains("Message::User { _0 }"),
        "must not emit struct destructure for tuple variant;\nactual:\n{result}"
    );

    assert!(
        result.contains("Message::System(field0)"),
        "must emit tuple destructure for tuple variant;\nactual:\n{result}"
    );
    assert!(
        result.contains("Message::User(field0)"),
        "must emit tuple destructure for tuple variant;\nactual:\n{result}"
    );

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

    assert!(
        !result.contains("Self::System { _0:"),
        "must not emit struct construction for tuple variant;\nactual:\n{result}"
    );
    assert!(
        !result.contains("Self::User { _0:"),
        "must not emit struct construction for tuple variant;\nactual:\n{result}"
    );

    assert!(
        result.contains("Self::System("),
        "must emit tuple construction for tuple variant;\nactual:\n{result}"
    );
    assert!(
        result.contains("Self::User("),
        "must emit tuple construction for tuple variant;\nactual:\n{result}"
    );

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
                serde_untagged: false,
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("active".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("inactive".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
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
        has_default: false,
        serde_content: None,
        serde_tag: Some("state".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("test_lib::Status::Active => Self {"),
        "unit variant arm must use simple path;\nactual:\n{core_to_binding}"
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
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
            serde_untagged: false,
            name: "Basic".to_string(),
            sensitive: false,
            fields: vec![FieldDef {
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            is_tuple: false,
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
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    assert!(
        result.contains("Auth::Basic { username }"),
        "struct variant must keep struct destructure;\nactual:\n{result}"
    );
}
