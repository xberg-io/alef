use super::*;
use crate::core::ir::{EnumVariant, FieldDef, ParamDef, TypeDef, TypeRef};

#[test]
fn format_jsdoc_escapes_embedded_block_comment_closers() {
    let lines = format_jsdoc("Supports literal `/** example */` syntax.", "  ");

    assert_eq!(lines, vec!["  /** Supports literal `/** example * /` syntax. */"]);
}

pub(super) fn make_param(name: &str, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

/// TypeScript TS1016 is avoided without changing the runtime's positional ABI: an optional
/// parameter before a required parameter is expressed as a required `T | undefined | null` slot.
#[test]
fn dts_params_preserves_runtime_order_when_required_follows_optional() {
    let params = vec![
        make_param("ctx", false),
        make_param("lang", true),
        make_param("code", false),
    ];
    let result = dts_params(&params, &ahash::AHashSet::new());
    assert_eq!(result, "ctx: string, lang: string | undefined | null, code: string");
}

/// When params are already in valid order (all required before all optional),
/// the output must be unchanged — no unnecessary reordering.
#[test]
fn dts_params_preserves_already_valid_order() {
    let params = vec![
        make_param("ctx", false),
        make_param("code", false),
        make_param("lang", true),
    ];
    let result = dts_params(&params, &ahash::AHashSet::new());
    assert_eq!(result, "ctx: string, code: string, lang?: string | undefined | null");
}

/// All-required params: order must be preserved exactly.
#[test]
fn dts_params_all_required_preserves_order() {
    let params = vec![make_param("a", false), make_param("b", false), make_param("c", false)];
    let result = dts_params(&params, &ahash::AHashSet::new());
    assert_eq!(result, "a: string, b: string, c: string");
}

#[test]
fn dts_params_treats_defaulted_params_as_optional() {
    let mut params = vec![make_param("path", false), make_param("config", false)];
    params[1].default = Some("Default::default()".to_string());
    let result = dts_params(&params, &ahash::AHashSet::new());
    assert_eq!(
        result, "path: string, config?: string | undefined | null",
        "defaulted params must be optional in generated declarations"
    );
}

/// Regression test for a `.d.ts` that does not typecheck standalone: a type's own
/// declaration and every reference to that type elsewhere in the file must use the exact
/// same public name. `gen_dts` is called with a real NAPI-RS wrapper prefix ("Js") — the
/// prefix `#[napi(js_name = "...")]` strips off the compiled Rust struct name — to prove the
/// prefix can never leak into either side. Both `dts_type`'s `TypeRef::Named` arm and every
/// declaration site in `gen_dts` route the public name through the single
/// `naming::node_type_name` function, so they cannot independently disagree.
#[test]
fn dts_declaration_and_reference_names_agree_for_every_named_type() {
    let api = ApiSurface {
        types: vec![
            TypeDef {
                name: "Message".to_string(),
                ..Default::default()
            },
            TypeDef {
                name: "ChatCompletionRequest".to_string(),
                fields: vec![
                    FieldDef {
                        name: "messages".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string()))),
                        optional: true,
                        ..Default::default()
                    },
                    FieldDef {
                        name: "stop".to_string(),
                        ty: TypeRef::Named("StopSequence".to_string()),
                        optional: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            TypeDef {
                name: "StopSequence".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "Js",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    let cases = [
        ("export interface Message {", "declaration of Message"),
        (
            "readonly messages?: Array<Message>",
            "reference to Message from ChatCompletionRequest.messages",
        ),
        ("export interface StopSequence {", "declaration of StopSequence"),
        (
            "readonly stop?: StopSequence",
            "reference to StopSequence from ChatCompletionRequest.stop",
        ),
    ];
    for (expected_line, description) in cases {
        assert!(
            dts.contains(expected_line),
            "{description}: expected to find {expected_line:?} in:\n{dts}"
        );
    }
    for leaked in ["JsMessage", "JsStopSequence", "JsChatCompletionRequest"] {
        assert!(
            !dts.contains(leaked),
            "no NAPI-RS wrapper prefix may leak into the .d.ts, found {leaked:?}:\n{dts}"
        );
    }
}

#[test]
fn trait_bridge_dts_return_type_wraps_async_methods_in_promise() {
    assert_eq!(
        trait_bridge_dts_return_type(&TypeRef::Named("ExtractionResult".to_string()), true),
        "Promise<ExtractionResult>"
    );
    assert_eq!(trait_bridge_dts_return_type(&TypeRef::Unit, true), "Promise<void>");
    assert_eq!(
        trait_bridge_dts_return_type(&TypeRef::Named("ExtractionResult".to_string()), false),
        "ExtractionResult"
    );
}

#[test]
fn plugin_trait_bridge_requires_name_in_typescript_interface() {
    let typ = TypeDef {
        name: "DocumentExtractor".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        fields: Vec::new(),
        methods: Vec::new(),
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,

        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let bridges = vec![crate::core::config::TraitBridgeConfig {
        trait_name: "DocumentExtractor".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    }];
    assert!(trait_bridge_requires_plugin_name(&typ, &bridges));
}

#[test]
fn adjacent_enum_dts_declares_runtime_namespace() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Action".to_string(),
            serde_tag: Some("type".to_string()),
            serde_content: Some("output".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Skip".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );
    assert!(dts.contains("| { type: 'custom'; output: string }"));
    assert!(dts.contains("export declare const Action: {"));
    assert!(dts.contains("readonly Skip: Action;"));
    assert!(dts.contains("Custom(output: string): Action;"));
}

/// Internally-tagged enums whose variants are newtype wrappers around struct types must
/// declare a discriminated union keyed by the variant-derived field name (e.g. `system`,
/// `user`) — not the tuple field's synthetic `_0` name, and not the napi glue's internal
/// flattened `#[napi(object)]` representation. Regression test for the `0:` key bug and for
/// the flattening regression introduced alongside its original fix (see
/// `internally_tagged_struct_variants_declare_discriminated_union` for the more common
/// struct-variant case).
#[test]
fn internally_tagged_newtype_variants_declare_discriminated_union() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "InternalNewtype".to_string(),
            serde_tag: Some("role".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::Named("SystemMessage".to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                EnumVariant {
                    name: "User".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::Named("UserMessage".to_string()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    assert_eq!(
        dts.lines()
            .skip_while(|l| *l != "export type InternalNewtype =")
            .take(3)
            .collect::<Vec<_>>(),
        vec![
            "export type InternalNewtype =",
            "  | { role: 'system'; system: SystemMessage }",
            "  | { role: 'user'; user: UserMessage }",
        ],
        "expected a discriminated union keyed by the variant-derived field name, got:\n{dts}"
    );
    assert!(
        !dts.contains("0:"),
        "must not emit the tuple field's synthetic `_0` name as a `0:` key:\n{dts}"
    );
    assert!(
        !dts.contains("system?:") && !dts.contains("user?:"),
        "a field belonging to only one variant must not be optional:\n{dts}"
    );
}

fn excel_flatten_api() -> ApiSurface {
    ApiSurface {
        enums: vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Excel".to_string(),
                // `is_tuple` mirrors what the extractor emits for `Excel(ExcelMetadata)`: a
                // `Fields::Unnamed` variant sets the flag AND names the field `_0`. Setting only
                // the name models a NAMED-fields variant whose field is literally called `_0`,
                // which serde does not flatten. ~keep
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("ExcelMetadata".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "ExcelMetadata".to_string(),
            fields: vec![
                FieldDef {
                    name: "sheet_count".to_string(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                    ..Default::default()
                },
                FieldDef {
                    name: "sheet_names".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The `FormatMetadata` defect this task fixes: when `ApiSurface::types` resolves the wrapped
/// struct, an internally-tagged newtype variant's `.d.ts` union member must flatten that
/// struct's OWN fields onto the tag object -- `{"format_type":"excel","sheet_count":2,...}` is
/// the real serde wire, never `{"format_type":"excel","excel":{"sheet_count":2,...}}`.
#[test]
fn internally_tagged_newtype_variant_flattens_wrapped_struct_when_type_resolves() {
    let api = excel_flatten_api();

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    assert_eq!(
        dts.lines()
            .skip_while(|l| *l != "export type FormatMetadata =")
            .take(2)
            .collect::<Vec<_>>(),
        vec![
            "export type FormatMetadata =",
            // References the SAME idiomatic `ExcelMetadata` interface every other field in the
            // file uses -- not a second, JSON-shaped `__AlefWireExcelMetadata` duplicate. A
            // binding presents exactly one public shape per Rust type; `ExcelMetadata` already
            // gets `export interface ExcelMetadata { readonly sheetCount?: number; ... }`
            // declared below via the ordinary `Decl::Interface` path (`ExcelMetadata` is a plain,
            // non-opaque, non-transparent type in `api.types`), so this reuses it instead of
            // resynthesizing a snake_case sibling nobody but this passthrough path could see. ~keep
            "  | ({ formatType: \"excel\" } & ExcelMetadata)",
        ],
        "expected the wrapped struct's own fields flattened onto the tag object, got:\n{dts}"
    );
    assert!(
        !dts.contains("excel: ExcelMetadata") && !dts.contains("excel?: ExcelMetadata"),
        "must not declare the old nested `excel: ExcelMetadata` member once the type resolves:\n{dts}"
    );
    assert!(
        !dts.contains("__AlefWire"),
        "must not synthesize a duplicate JSON-shaped wire type once the payload resolves to an \
         idiomatic declared interface:\n{dts}"
    );
    assert!(
        dts.contains("export interface ExcelMetadata {") && dts.contains("readonly sheetCount: number"),
        "the idiomatic interface referenced by the union must actually be declared, in host \
         camelCase, elsewhere in the file:\n{dts}"
    );
}

/// The one struct shape that still needs a synthesized wire type: a
/// `#[serde(transparent)]` single-field newtype. Its wire form is the BARE inner value with no
/// wrapper object at all (`sheet_count` alone, not `{ inner: sheet_count }`), which the
/// idiomatic napi interface -- a real one-field struct -- cannot express, so referencing it
/// would describe the wrong shape.
#[test]
fn transparent_newtype_payload_still_collapses_to_its_inner_type() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Excel".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("SheetCount".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "SheetCount".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                ..Default::default()
            }],
            serde_container_conversion: crate::core::ir::SerdeContainerConversion {
                transparent: true,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    assert!(
        dts.contains("({ formatType: \"excel\" } & __AlefWireSheetCount)"),
        "a transparent newtype has no idiomatic interface to reference (its wire form is the \
         bare inner value, not a wrapper object), so it must still synthesize its own alias:\n{dts}"
    );
    assert!(
        dts.contains("export type __AlefWireSheetCount = number;"),
        "the synthesized alias must collapse to the transparent field's inner type, not a \
         one-field wrapper object:\n{dts}"
    );
}

/// A struct with an explicit `#[serde(rename = "...")]` field must NOT reuse its idiomatic
/// `Decl::Interface`: that interface's `js_name` comes from `to_node_name` on the bare Rust
/// identifier alone (napi's ABI-level marshalling name), never from `serde_rename` -- but the
/// JSON this passthrough path actually emits IS keyed by the serde rename. Referencing the
/// interface here would advertise a property (`internalName`) the real JSON never carries,
/// while silently omitting the one it does (`external_name`). This must keep synthesizing its
/// own wire alias so the declared shape matches the actual JSON exactly.
#[test]
fn payload_field_with_explicit_serde_rename_declines_the_idiomatic_interface() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Excel".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("ExcelMetadata".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "ExcelMetadata".to_string(),
            fields: vec![FieldDef {
                name: "internal_name".to_string(),
                ty: TypeRef::String,
                serde_rename: Some("external_name".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    assert!(
        dts.contains("({ formatType: \"excel\" } & __AlefWireExcelMetadata)"),
        "must fall back to a synthesized wire alias once a field's serde rename could disagree \
         with the idiomatic interface's `to_node_name`-derived key, got:\n{dts}"
    );
    assert!(
        dts.contains("export type __AlefWireExcelMetadata = { external_name: string };"),
        "the synthesized wire type must key the field by its explicit serde rename VERBATIM \
         (`external_name` -- author intent, never recased), not the Rust identifier's camelCase \
         (`internalName`):\n{dts}"
    );
}

/// Negative control: adjacent tagging (`#[serde(tag, content)]`) nests a newtype variant's
/// payload under the `content` key by design and is handled by a different `gen_dts` branch
/// entirely -- it must never flatten, even when `ApiSurface::types` resolves the wrapped struct.
#[test]
fn adjacently_tagged_newtype_variant_keeps_nested_content_even_when_type_resolves() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            serde_content: Some("data".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Excel".to_string(),
                // `is_tuple` mirrors what the extractor emits for `Excel(ExcelMetadata)`: a
                // `Fields::Unnamed` variant sets the flag AND names the field `_0`. Setting only
                // the name models a NAMED-fields variant whose field is literally called `_0`,
                // which serde does not flatten. ~keep
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("ExcelMetadata".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "ExcelMetadata".to_string(),
            fields: vec![FieldDef {
                name: "sheet_count".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let dts = gen_dts(
        &api,
        "",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    // Scoped to the `FormatMetadata` union member itself, not a global substring search:
    // `ExcelMetadata`'s own standalone `.d.ts` interface legitimately declares `sheetCount`
    // regardless of the flattening decision, so a bare `dts.contains("sheetCount")` would fire
    // even when adjacent tagging correctly nests the payload. ~keep
    assert_eq!(
        dts.lines()
            .skip_while(|l| *l != "export type FormatMetadata =")
            .take(2)
            .collect::<Vec<_>>(),
        vec![
            "export type FormatMetadata =",
            "  | { format_type: 'excel'; data: ExcelMetadata }",
        ],
        "adjacent tagging must keep the payload nested under its content field, not flatten it \
         onto the tag object:\n{dts}"
    );
}
