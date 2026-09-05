//! Regression coverage for `#[serde(untagged)]` data enums in the WASM backend.
//!
//! A payload-carrying untagged enum (e.g. `enum EmbeddingInput { Single(String),
//! Multiple(Vec<String>) }`) cannot be represented as a fieldless `#[wasm_bindgen]` C-style enum
//! without discarding every variant's data. These tests assert the exact emitted Rust for a
//! struct field of that type — before the fix, `gen_enum` silently degraded it to
//! `pub enum WasmEmbeddingInput { Single = 0, Multiple = 1 }` and the containing struct's setter
//! accepted that fieldless enum, so no JS caller could ever supply the payload.

use super::{WasmBackend, enums::is_untagged_data_enum};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

/// A free function taking `type_name` by value, so `input_type_names` (see
/// `crate::codegen::conversions`) treats it as an input type and emits the binding->core `From`
/// impl the test asserts on — without a caller, that impl is dead code the generator skips.
fn function_taking(type_name: &str) -> FunctionDef {
    FunctionDef {
        name: format!("use_{}", type_name.to_lowercase()),
        rust_path: format!("test_lib::use_{}", type_name.to_lowercase()),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::Named(type_name.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// `enum EmbeddingInput { Single(String), Multiple(Vec<String>) }` with `#[serde(untagged)]`.
/// Covers both a scalar-payload variant and a `Vec`-payload variant in one enum, which is the
/// shape that used to collapse to a bare discriminant.
fn embedding_input_enum() -> EnumDef {
    EnumDef {
        name: "EmbeddingInput".to_string(),
        rust_path: "test_lib::EmbeddingInput".to_string(),
        variants: vec![
            EnumVariant {
                name: "Single".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Multiple".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    }
}

#[test]
fn is_untagged_data_enum_true_for_payload_carrying_untagged_enum() {
    assert!(is_untagged_data_enum(&embedding_input_enum()));
}

#[test]
fn is_untagged_data_enum_false_for_fieldless_untagged_enum() {
    let mut e = embedding_input_enum();
    for variant in &mut e.variants {
        variant.fields.clear();
        variant.is_tuple = false;
    }
    assert!(
        !is_untagged_data_enum(&e),
        "an untagged enum with only unit variants has nothing to lose and must keep the old \
         fieldless C-style representation"
    );
}

#[test]
fn is_untagged_data_enum_false_for_internally_tagged_data_enum() {
    let mut e = embedding_input_enum();
    e.serde_untagged = false;
    e.serde_tag = Some("type".to_string());
    assert!(
        !is_untagged_data_enum(&e),
        "internally-tagged data enums take the discriminator-struct path, not the JsValue-field \
         path — the two predicates must stay mutually exclusive"
    );
}

/// A struct with a *required* field of the untagged data enum type, mirroring
/// `EmbeddingRequest { pub input: EmbeddingInput, .. }`.
fn embedding_request_type() -> TypeDef {
    TypeDef {
        name: "EmbeddingRequest".to_string(),
        rust_path: "test_lib::EmbeddingRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: false,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }
}

#[test]
fn required_untagged_data_enum_field_becomes_js_value_not_fieldless_wasm_enum() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.types = vec![embedding_request_type()];
    api.functions = vec![function_taking("EmbeddingRequest")];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        !lib_rs.contains("pub enum WasmEmbeddingInput"),
        "no fieldless discriminant enum must be emitted for a payload-carrying untagged enum — \
         it can never carry the variant's data;\nactual:\n{lib_rs}"
    );

    assert!(
        lib_rs.contains("input: JsValue,"),
        "the struct field must still be stored as JsValue so the payload round-trips;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: WasmEmbeddingInputValue)"),
        "the setter must accept the structural TS wrapper type, not bare JsValue;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("self.input = value.into();"),
        "the setter must convert the wrapper type into the JsValue field;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn input(&self) -> WasmEmbeddingInputValue"),
        "the getter must return the structural TS wrapper type so the .d.ts carries a real type \
         instead of `any`, not a wire string of the variant name;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("self.input.clone().unchecked_into()"),
        "the getter must convert the JsValue field into the wrapper type;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(r#"typescript_type = "WasmEmbeddingInput""#),
        "the untagged enum must get a typescript_type declaration describing its real shape;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmEmbeddingInput = string | string[];"),
        "the declared TS union must be the structural shape, not `any`;\nactual:\n{lib_rs}"
    );

    assert!(
        lib_rs.contains("input: serde_wasm_bindgen::to_value(&val.input).unwrap_or(JsValue::NULL)"),
        "core->binding conversion must serialize the real enum value via serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("input: serde_wasm_bindgen::from_value(val.input.clone()).unwrap_or_default()"),
        "binding->core conversion must deserialize the JsValue back into the real enum via \
         serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
}

/// Same shape as above but the field is `Option<EmbeddingInput>` — must degrade to
/// `Option<JsValue>` throughout, not `JsValue` alone (which would make `None` unrepresentable)
/// nor the old fieldless enum.
#[test]
fn optional_untagged_data_enum_field_becomes_option_js_value() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.functions = vec![function_taking("ModerationRequest")];
    api.types = vec![TypeDef {
        name: "ModerationRequest".to_string(),
        rust_path: "test_lib::ModerationRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: true,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        !lib_rs.contains("pub enum WasmEmbeddingInput"),
        "no fieldless discriminant enum must be emitted;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("input: Option<JsValue>,"),
        "an optional field of this type must still be stored as Option<JsValue>;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: Option<WasmEmbeddingInputValue>)"),
        "the setter must accept Option<the structural TS wrapper type>;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("self.input = value.map(Into::into);"),
        "the setter must convert each Some value into JsValue;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn input(&self) -> Option<WasmEmbeddingInputValue>"),
        "the getter must return Option<the structural TS wrapper type>;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("self.input.clone().map(|v| v.unchecked_into())"),
        "the getter must convert each Some JsValue into the wrapper type;\nactual:\n{lib_rs}"
    );
}

/// A genuinely fieldless enum (no `#[serde(untagged)]`, no data variants) used as a struct field
/// must be entirely unaffected by this fix: it keeps the `Wasm{Enum}` C-style representation,
/// the `to_api_str`/`from_api_str` wire-string getter/setter, and its own conversions.
#[test]
fn fieldless_enum_field_is_unaffected() {
    let mut api = empty_api();
    api.enums = vec![EnumDef {
        name: "Role".to_string(),
        rust_path: "test_lib::Role".to_string(),
        variants: vec![
            EnumVariant {
                name: "User".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Assistant".to_string(),
                ..Default::default()
            },
        ],
        has_serde: true,
        is_copy: true,
        ..Default::default()
    }];
    api.functions = vec![function_taking("Message")];
    api.types = vec![TypeDef {
        name: "Message".to_string(),
        rust_path: "test_lib::Message".to_string(),
        fields: vec![FieldDef {
            name: "role".to_string(),
            ty: TypeRef::Named("Role".to_string()),
            optional: false,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("pub enum WasmRole {"),
        "a genuinely fieldless enum must keep its wasm-bindgen C-style representation;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("role: WasmRole,"),
        "a field of a genuinely fieldless enum must keep the WasmRole wrapper type, not JsValue;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn role(&self) -> String"),
        "the getter for a genuinely fieldless enum field must be unchanged (wire-string via \
         to_api_str);\nactual:\n{lib_rs}"
    );
}

/// Every real-consumer shape from `ts_union.rs`'s module doc comment, generated together in one
/// crate — proves the combined-custom-section dedup holds end to end (not just in `ts_union`'s
/// own unit tests) and that a fieldless enum used both as a union member and as its own ordinary
/// field type coexists without a name collision. This exact generated source was also used to
/// manually verify the real `.d.ts` wasm-bindgen produces (see the PR description / commit
/// message for that evidence — a unit test cannot itself invoke wasm-bindgen).
#[test]
fn all_real_consumer_shapes_share_one_custom_section_without_collisions() {
    let mut api = empty_api();

    let embedding_input = embedding_input_enum();

    let moderation_input = EnumDef {
        name: "ModerationInput".to_string(),
        rust_path: "test_lib::ModerationInput".to_string(),
        variants: embedding_input.variants.clone(),
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    };

    let rerank_document = EnumDef {
        name: "RerankDocument".to_string(),
        rust_path: "test_lib::RerankDocument".to_string(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Object".to_string(),
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: false,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    };

    let user_content = EnumDef {
        name: "UserContent".to_string(),
        rust_path: "test_lib::UserContent".to_string(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Parts".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    };

    let content_part = TypeDef {
        name: "ContentPart".to_string(),
        rust_path: "test_lib::ContentPart".to_string(),
        fields: vec![
            FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
        ],
        has_serde: true,
        ..Default::default()
    };

    let tool_choice_mode = EnumDef {
        name: "ToolChoiceMode".to_string(),
        rust_path: "test_lib::ToolChoiceMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Auto".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Required".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "None".to_string(),
                ..Default::default()
            },
        ],
        has_serde: true,
        is_copy: true,
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let specific_tool_choice = TypeDef {
        name: "SpecificToolChoice".to_string(),
        rust_path: "test_lib::SpecificToolChoice".to_string(),
        fields: vec![FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    };

    let tool_choice = EnumDef {
        name: "ToolChoice".to_string(),
        rust_path: "test_lib::ToolChoice".to_string(),
        variants: vec![
            EnumVariant {
                name: "Mode".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("ToolChoiceMode".to_string()),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Specific".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("SpecificToolChoice".to_string()),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    };

    api.enums = vec![
        embedding_input,
        moderation_input,
        rerank_document,
        user_content,
        tool_choice_mode,
        tool_choice,
    ];
    api.types = vec![
        content_part,
        specific_tool_choice,
        TypeDef {
            name: "AllShapes".to_string(),
            rust_path: "test_lib::AllShapes".to_string(),
            fields: vec![
                FieldDef {
                    name: "embedding".to_string(),
                    ty: TypeRef::Named("EmbeddingInput".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "moderation".to_string(),
                    ty: TypeRef::Named("ModerationInput".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "document".to_string(),
                    ty: TypeRef::Named("RerankDocument".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "content".to_string(),
                    ty: TypeRef::Named("UserContent".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "choice".to_string(),
                    ty: TypeRef::Named("ToolChoice".to_string()),
                    ..Default::default()
                },
            ],
            has_serde: true,
            ..Default::default()
        },
    ];
    api.functions = vec![function_taking("AllShapes")];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap()
        .content;
    // All six untagged-union shapes share one combined typescript_custom_section (see
    // `ts_union::AllUntaggedEnumsTsPlan`) — each alias declared exactly once.
    assert!(lib_rs.contains("const ALEF_UNTAGGED_UNIONS_TS"), "actual:\n{lib_rs}");
    assert_eq!(
        lib_rs.matches("typescript_custom_section").count(),
        1,
        "every untagged union must share one custom section, not one each;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmEmbeddingInput = string | string[];"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmModerationInput = string | string[];"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmRerankDocument = string | { text: string; };"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export interface WasmContentPartWire {\n    text: string;\n    kind: string;\n}"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmUserContent = string | WasmContentPartWire[];"),
        "actual:\n{lib_rs}"
    );
    // `ToolChoiceMode` is ALSO independently emitted as a real `Wasm{Enum}` wasm-bindgen TS enum
    // below (it's a plain fieldless enum, so `gen_enum` always emits it) — the union member must
    // use the disambiguated `Wire` name, never the bare name the real enum already claims.
    assert!(
        lib_rs.contains(r#"export type WasmToolChoiceModeWire = "auto" | "required" | "none";"#),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export interface WasmSpecificToolChoiceWire {\n    name: string;\n}"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmToolChoice = WasmToolChoiceModeWire | WasmSpecificToolChoiceWire;"),
        "actual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub enum WasmToolChoiceMode {"),
        "the real fieldless enum must still be emitted unchanged alongside the union;\nactual:\n{lib_rs}"
    );
}

fn make_config_with_text_types(text_types: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
untagged_union_text_types = [{text_types}]
[crates.wasm]
"#
    ))
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// An untagged data enum that is *also* opted into `untagged_union_text_types` used to be
/// generated two different ways at once: the `type_overrides` entry pinned to `String` drove the
/// constructor, getter, and setter, while the JsValue-bridged set drove the struct field and both
/// conversions. The emitted struct declared `Option<JsValue>` and handed it to accessors typed
/// `Option<String>`, so the whole binding crate failed to compile with E0308. The text opt-in is
/// the more specific signal and must win on every surface.
#[test]
fn untagged_data_enum_in_text_types_is_string_on_every_surface() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.functions = vec![function_taking("ModerationRequest")];
    api.types = vec![TypeDef {
        name: "ModerationRequest".to_string(),
        rust_path: "test_lib::ModerationRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: true,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }];

    let config = make_config_with_text_types("\"EmbeddingInput\"");
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("input: Option<String>,"),
        "the struct field must follow the text opt-in, not the JsValue bridge;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("input: Option<JsValue>,"),
        "the JsValue-bridged representation must not be emitted for a text-typed union;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn input(&self) -> Option<String>"),
        "the getter must agree with the field type;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: Option<String>)"),
        "the setter must agree with the field type;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("serde_wasm_bindgen::to_value(&val.input)"),
        "conversions must use the display-text bridge, not serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
}

/// A struct reached as an untagged-enum payload is described twice in the SAME `.d.ts`: once as
/// `export class Wasm{Name}` (wasm-bindgen's rendering of the `#[wasm_bindgen] pub struct` every
/// `api.types` entry gets, whose members are `to_node_name` HOST accessors) and once as the
/// structural interface `ts_union` emits for the plain JSON object `serde_wasm_bindgen` actually
/// produces (whose members are serde WIRE keys).
///
/// TypeScript merges an `interface` into a same-named `class` silently -- it is a legal
/// declaration merge, not `TS2300` -- so sharing the bare name would not fail loudly. It would
/// graft the wire keys onto the class type, and `tsc` would then accept `detail.max_chars` on a
/// class instance where only `maxChars` exists at runtime: `undefined`, no error, on the exact
/// host/wire boundary the field-naming fix above exists to keep straight. The interface must
/// therefore carry the `Wire` suffix `map_named_enum` already gives a fieldless enum's alias.
#[test]
fn untagged_payload_struct_interface_cannot_merge_with_its_wasm_bindgen_class() {
    let mut api = empty_api();
    api.enums = vec![EnumDef {
        name: "Payload".to_string(),
        rust_path: "test_lib::Payload".to_string(),
        variants: vec![EnumVariant {
            name: "Part".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named("Detail".to_string()),
                ..Default::default()
            }],
            is_tuple: true,
            ..Default::default()
        }],
        has_serde: true,
        serde_untagged: true,
        ..Default::default()
    }];
    api.types = vec![
        TypeDef {
            name: "Detail".to_string(),
            rust_path: "test_lib::Detail".to_string(),
            fields: vec![FieldDef {
                name: "max_chars".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        },
        TypeDef {
            name: "Envelope".to_string(),
            rust_path: "test_lib::Envelope".to_string(),
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("Payload".to_string()),
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        },
    ];
    api.functions = vec![function_taking("Envelope")];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    // Without this the test proves nothing: if no `WasmDetail` class were emitted there would be
    // no declaration for a bare interface to merge with, and the suffix would be busywork.
    assert!(
        lib_rs.contains("pub struct WasmDetail {"),
        "the payload struct must still get its own wasm-bindgen class;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export interface WasmDetailWire {\n    max_chars: string;\n}"),
        "the structural interface must be declared under the Wire name, keyed by the serde wire \
         name;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("export interface WasmDetail {"),
        "an interface under the class's own name merges into it and publishes a phantom \
         `max_chars` member on every WasmDetail instance;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("export type WasmPayload = WasmDetailWire;"),
        "the union must reference the name actually declared, not the bare class name;\nactual:\n{lib_rs}"
    );
}

/// The companion to the merge test: a wire name that is not a legal TypeScript identifier must be
/// emitted as a quoted key. `content-type` interpolated bare parses as a subtraction, so the whole
/// `typescript_custom_section` string -- every union in the crate -- becomes a `.d.ts` syntax
/// error. Both this emitter and `backends::napi::gen_bindings::errors` route through
/// `codegen::naming::ts_property_key` so they cannot disagree about when quoting is needed.
#[test]
fn untagged_payload_struct_quotes_a_wire_name_that_is_not_an_identifier() {
    let mut api = empty_api();
    api.enums = vec![EnumDef {
        name: "Payload".to_string(),
        rust_path: "test_lib::Payload".to_string(),
        variants: vec![EnumVariant {
            name: "Part".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named("Header".to_string()),
                ..Default::default()
            }],
            is_tuple: true,
            ..Default::default()
        }],
        has_serde: true,
        serde_untagged: true,
        ..Default::default()
    }];
    api.types = vec![
        TypeDef {
            name: "Header".to_string(),
            rust_path: "test_lib::Header".to_string(),
            fields: vec![FieldDef {
                name: "content_type".to_string(),
                ty: TypeRef::String,
                serde_rename: Some("content-type".to_string()),
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        },
        TypeDef {
            name: "Envelope".to_string(),
            rust_path: "test_lib::Envelope".to_string(),
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("Payload".to_string()),
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        },
    ];
    api.functions = vec![function_taking("Envelope")];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("export interface WasmHeaderWire {\n    \"content-type\": string;\n}"),
        "a kebab-case wire name must be emitted as a quoted key;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("    content-type: string;"),
        "an unquoted kebab-case key is a TypeScript syntax error;\nactual:\n{lib_rs}"
    );
}
