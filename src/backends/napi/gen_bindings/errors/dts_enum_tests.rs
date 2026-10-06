//! NAPI d.ts enum declaration coverage, split from tests.rs.

use super::*;
use crate::core::ir::{EnumVariant, FieldDef, ParamDef, TypeDef, TypeRef};

/// The reported regression: an internally-tagged enum whose variants are struct variants
/// (e.g. `AuthConfig::Basic { username, password }`) must declare a real discriminated union
/// — one member per variant, each variant's own fields required — not a single flattened
/// object with every field made optional. Each variant serializes to its own flat object on
/// the wire (`{"type":"basic","username":"...","password":"..."}`), so the union is a
/// one-to-one match for what a caller actually receives.
#[test]
fn internally_tagged_struct_variants_declare_discriminated_union() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "AuthConfig".to_string(),
            serde_tag: Some("type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Basic".to_string(),
                    fields: vec![
                        FieldDef {
                            name: "username".to_string(),
                            ty: TypeRef::String,
                            ..Default::default()
                        },
                        FieldDef {
                            name: "password".to_string(),
                            ty: TypeRef::String,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                EnumVariant {
                    name: "Bearer".to_string(),
                    fields: vec![FieldDef {
                        name: "token".to_string(),
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

    assert_eq!(
        dts.lines()
            .skip_while(|l| *l != "export type AuthConfig =")
            .take(3)
            .collect::<Vec<_>>(),
        vec![
            "export type AuthConfig =",
            "  | { type: 'basic'; username: string; password: string }",
            "  | { type: 'bearer'; token: string }",
        ],
        "expected one discriminated-union member per variant with required fields, got:\n{dts}"
    );
    assert!(
        !dts.contains("username?:") && !dts.contains("password?:") && !dts.contains("token?:"),
        "a field belonging to only one variant must not be optional:\n{dts}"
    );
    assert!(
        !dts.contains("export type AuthConfig = {"),
        "must not emit a single flattened object type:\n{dts}"
    );
}

/// `#[serde(tag = "kind")] enum E { A, B }` serializes as `{"kind":"A"}` — internal tagging is
/// always an object, even when every variant is a unit variant. `is_data_enum` must not
/// require a data-bearing variant, or an all-unit internally-tagged enum wrongly falls back
/// to a plain string enum declaration.
#[test]
fn internally_tagged_all_unit_variants_declare_object_not_string_enum() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "InternalAllUnit".to_string(),
            serde_tag: Some("kind".to_string()),
            variants: vec![
                EnumVariant {
                    name: "A".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "B".to_string(),
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

    assert!(
        dts.contains("export type InternalAllUnit = { kind: 'A' | 'B' };"),
        "expected an object type matching the napi glue struct, got:\n{dts}"
    );
    assert!(
        !dts.contains("export declare enum InternalAllUnit"),
        "must not emit a plain string enum for an internally-tagged enum:\n{dts}"
    );
}

/// Regression test: a plain `#[napi(string_enum = "snake_case")]` declaration in `.d.ts` must
/// use the wire value napi-rs's own `convert_case`-based macro actually emits at runtime, not
/// the value `wire_variant_value`'s serde-oriented case transform computes. The two disagree for
/// a variant name with a letter-to-digit boundary (mirrors crawlberg's real
/// `JsContentFilterKind::Bm25`, whose only variant is `Bm25`): serde's helper gives `"bm25"`,
/// napi's actual runtime value is `"bm_25"`. Before this fix the `.d.ts` declared `Bm25 = "bm25"`,
/// so TypeScript accepted a string literal the Rust `FromNapiValue` conversion rejected.
#[test]
fn plain_string_enum_dts_uses_napis_own_case_algorithm_not_serdes() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ContentFilterKind".to_string(),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Bm25".to_string(),
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
        dts.contains("Bm25 = \"bm_25\","),
        "expected napi's own convert_case-derived wire value \"bm_25\", got:\n{dts}"
    );
    assert!(
        !dts.contains("Bm25 = \"bm25\","),
        "must not declare serde's wire value \"bm25\" — napi-rs rejects it at runtime:\n{dts}"
    );
}

/// `#[serde(untagged)]` enums serialize each variant as its own bare shape (no wrapper, no
/// discriminant) — a newtype variant as its inner value, a struct variant as its own object.
/// The napi glue already treats the whole enum as opaque `serde_json::Value`, so this is a
/// `.d.ts`-only fix: the union of real per-variant shapes.
#[test]
fn untagged_enum_declares_bare_union_of_variant_shapes() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Untagged".to_string(),
            serde_untagged: true,
            variants: vec![
                EnumVariant {
                    name: "Single".to_string(),
                    is_tuple: true,
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                EnumVariant {
                    name: "Pair".to_string(),
                    fields: vec![
                        FieldDef {
                            name: "x".to_string(),
                            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
                            ..Default::default()
                        },
                        FieldDef {
                            name: "y".to_string(),
                            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
                            ..Default::default()
                        },
                    ],
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

    assert!(
        dts.contains("export type Untagged =\n  | string\n  | { x: number; y: number }"),
        "expected a bare union of each variant's own shape, got:\n{dts}"
    );
    assert!(
        !dts.contains("export declare enum Untagged"),
        "must not emit a plain string enum for an untagged data enum:\n{dts}"
    );
}

/// The other shape with no idiomatic declaration to reference: an OPAQUE struct. It gets a
/// `Decl::Class` (a handle wrapper with no plain-object shape), never a `Decl::Interface`, so a
/// JSON-passthrough payload reaching one still needs its own synthesized structural type.
#[test]
fn untagged_enum_variant_wrapping_an_opaque_struct_still_synthesizes_a_wire_type() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Untagged".to_string(),
            serde_untagged: true,
            variants: vec![EnumVariant {
                name: "Handle".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("OpaqueThing".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "OpaqueThing".to_string(),
            is_opaque: true,
            fields: vec![FieldDef {
                name: "label".to_string(),
                ty: TypeRef::String,
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
        dts.contains("export type Untagged =\n  | __AlefWireOpaqueThing"),
        "an opaque struct has no idiomatic non-class declaration to reference, so the union \
         must still resolve through a synthesized wire type, got:\n{dts}"
    );
    assert!(
        dts.contains("export type __AlefWireOpaqueThing = { label"),
        "the synthesized wire type must describe the struct's own JSON shape, got:\n{dts}"
    );
}

/// A variant-level `#[serde(untagged)]` on an otherwise default-tagged enum (e.g.
/// `OutputFormat { Plain, Markdown, ..., #[serde(untagged)] Custom(String) }`) must declare a
/// flat string-literal union over the unit variants -- respecting `serde_rename_all` -- plus a
/// `(string & {})` tail that widens the type to accept the `Custom` payload while keeping
/// editor autocomplete for the known literals. This must NOT be the container-level-untagged
/// shape (`untagged_enum_declares_bare_union_of_variant_shapes` above): unit variants here are
/// NOT `null`, they keep their name as a literal. ~keep
#[test]
fn variant_untagged_enum_declares_literal_union_with_string_widening_tail() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            serde_rename_all: Some("lowercase".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Plain".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Markdown".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    is_tuple: true,
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    serde_untagged: true,
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

    assert!(
        dts.contains("export type OutputFormat = \"plain\" | \"markdown\" | (string & {});"),
        "expected a literal union with a string-widening tail, got:\n{dts}"
    );
    assert!(
        !dts.contains("export declare enum OutputFormat"),
        "must not emit a nominal #[napi(string_enum)] declaration for a data-carrying enum:\n{dts}"
    );
    assert!(
        !dts.contains("type_tag") && !dts.contains("{ type:"),
        "must not emit the tagged-object shape:\n{dts}"
    );
}

#[test]
fn gen_dts_includes_service_entrypoint_bridge_functions() {
    use crate::core::ir::{EntrypointDef, EntrypointKind, MethodDef, ReceiverKind, ServiceDef};
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![ServiceDef {
            name: "App".to_string(),
            rust_path: "test::App".to_string(),
            constructor: MethodDef {
                name: "new".to_string(),
                params: vec![],
                return_type: TypeRef::Named("App".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                receiver: Some(ReceiverKind::Owned),
                cfg: None,
                doc: String::new(),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![EntrypointDef {
                method: "into_router".to_string(),
                kind: EntrypointKind::Finalize,
                is_async: true,
                params: vec![],
                return_type: TypeRef::Unit,
                error_type: None,
                doc: String::new(),
            }],
            doc: String::new(),
            cfg: None,
        }],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };
    let dts = gen_dts(
        &api,
        "",
        &ahash::AHashSet::new(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );
    assert!(
        dts.contains("export declare function appIntoRouter"),
        "dts should declare appIntoRouter bridge function for App.into_router"
    );
    assert!(
        dts.contains("registrations: Array<[string, any[], (...args: any[]) => any]>"),
        "service entrypoint should have registrations parameter"
    );
    assert!(
        dts.contains("Promise<void>"),
        "async into_router entrypoint should return Promise<void>"
    );
}

/// Regression: `gen_opaque_struct_methods` (`types.rs`) never generates a `#[napi]` wrapper
/// for an opaque instance method that takes another opaque type by value — opaque types only
/// implement `FromNapiValue` by reference — unless an adapter overrides it. `gen_dts` used to
/// iterate every method with no such check, so `index.d.ts` promised a method the compiled
/// extension does not export.
#[test]
fn opaque_by_value_param_without_adapter_is_not_declared_in_dts() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let api = ApiSurface {
        types: vec![
            TypeDef {
                name: "Worker".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "process".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    cfg: None,
                    params: vec![ParamDef {
                        name: "handle".to_string(),
                        ty: TypeRef::Named("Handle".to_string()),
                        is_ref: false,
                        ..Default::default()
                    }],
                    return_type: TypeRef::Unit,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "Handle".to_string(),
                is_opaque: true,
                ..Default::default()
            },
        ],
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
        !dts.contains("process("),
        "no #[napi] wrapper exists for an opaque-by-value param with no adapter override: {dts}"
    );
}

/// Regression, static side: `gen_static_method` never registers a sanitized static method
/// with no adapter override either (see `opaque_static_method_is_dropped`).
#[test]
fn sanitized_static_method_without_adapter_is_not_declared_in_dts() {
    use crate::core::ir::MethodDef;

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Config".to_string(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "fromRaw".to_string(),
                receiver: None,
                cfg: None,
                is_static: true,
                sanitized: true,
                return_type: TypeRef::Named("Config".to_string()),
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
        !dts.contains("static fromRaw"),
        "gen_static_method also drops a sanitized static method with no adapter override: {dts}"
    );
}

/// Control for the two regressions above: a delegatable instance method and a non-sanitized
/// static method must still be declared, proving the new filter doesn't over-drop.
#[test]
fn delegatable_methods_are_still_declared_in_dts() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Worker".to_string(),
            is_opaque: true,
            methods: vec![
                MethodDef {
                    name: "run".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    cfg: None,
                    return_type: TypeRef::Unit,
                    ..Default::default()
                },
                MethodDef {
                    name: "create".to_string(),
                    receiver: None,
                    cfg: None,
                    is_static: true,
                    return_type: TypeRef::Named("Worker".to_string()),
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

    assert!(
        dts.contains("run("),
        "delegatable instance method must still be declared: {dts}"
    );
    assert!(
        dts.contains("static create("),
        "non-sanitized static method must still be declared: {dts}"
    );
}
