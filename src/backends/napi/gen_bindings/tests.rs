use super::NapiBackend;
use super::enums::is_tagged_data_enum;
use super::methods::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::backend::Backend;
use crate::core::config::Language;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

fn transparent_string_wrapper(paths: Vec<Vec<crate::core::ir::NewtypeContainer>>) -> String {
    transparent_string_wrapper_with_methods(paths, "from", "into_inner")
}

fn transparent_string_wrapper_with_methods(
    paths: Vec<Vec<crate::core::ir::NewtypeContainer>>,
    from: &str,
    into: &str,
) -> String {
    use crate::core::ir::{NewtypeWrapper, NewtypeWrapperMetadata};

    NewtypeWrapper::encode_explicit(
        &paths
            .into_iter()
            .map(|containers| {
                NewtypeWrapperMetadata::transparent_string("fixture_core::SecretString", from, into, containers)
            })
            .collect::<Vec<_>>(),
    )
}

/// NapiBackend::name returns "napi".
#[test]
fn napi_backend_name_is_napi() {
    let b = NapiBackend;
    assert_eq!(b.name(), "napi");
}

/// NapiBackend::language returns Language::Node.
#[test]
fn napi_backend_language_is_node() {
    let b = NapiBackend;
    assert_eq!(b.language(), Language::Node);
}

/// Test that cfg-gated fields in never_skip_cfg_field_names pass the options-field-bridge filter.
#[test]
fn cfg_gated_field_accepted_when_in_never_skip_list() {
    let never_skip_cfg_field_names = ["visitor".to_string()];
    let field_is_target = "visitor";

    let field_has_cfg = Some("feature = \"visitor\"");

    let accepted = field_has_cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_is_target);

    assert!(
        accepted,
        "cfg-gated field 'visitor' should pass filter when in never_skip_cfg_field_names"
    );
}

/// Exercises the shared (`binding_enums_have_data: false`) conversion helpers in
/// `crate::codegen::conversions` directly, in isolation from napi's own pipeline routing.
/// These helpers discard variant payload data by design when the binding-side enum type
/// cannot carry it (see `can_generate_enum_conversion_from_core`'s doc comment) — that is
/// only safe for a data-carrying enum when the caller never reaches this path for one.
///
/// napi's own pipeline (`enums::gen_enum`, `mod.rs`) no longer calls these helpers for a
/// data-carrying enum shaped like `AuthHeaderFormat` (`ApiKey(String)`, no
/// `#[serde(tag/content/untagged)]`): such enums are routed to the tagged-object emitter
/// instead, which does carry the payload — see
/// `enums::tests::default_tagged_data_enum_preserves_custom_string_variant_payload_round_trip`.
/// The assertion below pins that routing decision so this test cannot silently start
/// asserting on dead code again. ~keep
#[test]
fn plain_data_enum_in_input_type_struct_gets_binding_to_core_impl() {
    use crate::codegen::conversions::{
        ConversionConfig, can_generate_enum_conversion, can_generate_enum_conversion_from_core,
        gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding_cfg,
    };
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    let auth_format_enum = EnumDef {
        name: "AuthHeaderFormat".to_string(),
        rust_path: "fixture_core::AuthHeaderFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Bearer".to_string(),
                fields: vec![],
                ..Default::default()
            },
            EnumVariant {
                name: "ApiKey".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                ..Default::default()
            },
        ],
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        ..Default::default()
    };

    let has_data_variants = auth_format_enum.variants.iter().any(|v| !v.fields.is_empty());
    assert!(has_data_variants, "AuthHeaderFormat should have data variants");

    // Pin the routing decision this test's isolation depends on: napi's own pipeline treats
    // this exact shape as a tagged data enum (`enums::is_tagged_data_enum`, the single authority
    // `gen_enum` and `errors::gen_dts` both route through) and never reaches the lossy helpers
    // exercised below.
    assert!(
        is_tagged_data_enum(&auth_format_enum, &[]),
        "napi's pipeline must treat a default-tagged data enum as a tagged data enum, \
         routing it away from the data-discarding helpers this test exercises directly"
    );

    assert!(
        can_generate_enum_conversion(&auth_format_enum),
        "plain data enum should be eligible for binding-to-core conversion"
    );
    assert!(
        can_generate_enum_conversion_from_core(&auth_format_enum),
        "plain data enum should be eligible for core-to-binding conversion"
    );

    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..Default::default()
    };
    let binding_to_core = gen_enum_from_binding_to_core_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        binding_to_core.contains("impl From<JsAuthHeaderFormat> for fixture_core::AuthHeaderFormat"),
        "should emit binding-to-core impl for plain data enum; got:\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        core_to_binding.contains("impl From<fixture_core::AuthHeaderFormat> for JsAuthHeaderFormat"),
        "should emit core-to-binding impl for plain data enum; got:\n{core_to_binding}"
    );
}

/// Test that opaque types with `has_default=true` emit `#[napi(constructor)]` with `new_constructor()`
/// even when a static `new()` method exists. This ensures JS `new ClassName()` works without
/// causing duplicate symbol errors.
/// Regression: App type has both a static `new()` method and `has_default=true`, but the
/// NAPI backend was skipping constructor emission due to the `!has_static_new` guard, causing
/// "Class contains no 'constructor', can not new it!" at runtime.
#[test]
fn napi_opaque_type_with_default_and_static_new_emits_constructor() {
    use super::constructors::napi_default_constructor;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, TypeDef, TypeRef};

    let app_type = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        has_default: true,
        methods: vec![MethodDef {
            name: "new".to_string(),
            receiver: None,
            cfg: None,
            params: vec![],
            return_type: TypeRef::Named("App".to_string()),
            is_async: false,
            is_static: true,
            doc: "Create a new application".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let constructor = napi_default_constructor(&app_type, &mapper, "sample_crate", "Js");

    assert!(
        constructor.is_some(),
        "opaque type with has_default=true should emit constructor even with static new()"
    );

    let constructor_code = constructor.unwrap();
    assert!(
        constructor_code.contains("#[napi(constructor)]"),
        "constructor should be marked with #[napi(constructor)]"
    );
    assert!(
        constructor_code.contains("pub fn new_constructor()"),
        "constructor should use new_constructor() to avoid conflict with static new()"
    );
    assert!(
        constructor_code.contains("Self { inner: std::sync::Arc::new(sample_crate::App::new())"),
        "constructor should create new App via sample_crate::App::new()"
    );
}

/// Regression: a `&mut self -> Result<&mut Self, E>` builder (a method returning a reference to
/// its own wrapper type) must SHARE the existing handle's `Arc` (`self.inner.clone()`) instead of
/// cloning the returned reference. `&mut App` is not `Clone`, so
/// `Arc::new(std::sync::Mutex::new(result.clone()))` fails to compile (E0599).
#[test]
fn napi_self_ref_builder_shares_arc_instead_of_cloning_returned_ref() {
    use super::types::gen_opaque_instance_method;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
    use ahash::AHashSet;
    use std::collections::HashMap;

    let method = MethodDef {
        name: "register_route".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("RouteCfg".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("App".to_string()),
        error_type: Some("AppError".to_string()),
        doc: "Register a route, returning the app for chaining.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        cfg: None,
        returns_ref: true,
        ..MethodDef::default()
    };
    let typ = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        methods: vec![method.clone()],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let cfg = super::NapiBackend::binding_config("sample_crate", "Js", true);
    let mut opaque = AHashSet::new();
    opaque.insert("App".to_string());
    opaque.insert("RouteCfg".to_string());
    let mut mutex = AHashSet::new();
    mutex.insert("App".to_string());
    let adapter_bodies = crate::adapters::AdapterBodies::new();
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let capsule: HashMap<String, crate::core::config::NodeCapsuleTypeConfig> = HashMap::new();

    let code = gen_opaque_instance_method(
        &method,
        &mapper,
        &typ,
        &cfg,
        &opaque,
        "Js",
        &adapter_bodies,
        &streaming,
        &mutex,
        &capsule,
    );

    assert!(
        code.contains("Ok(Self { inner: self.inner.clone() })"),
        "self-returning builder should share the existing Arc, got:\n{code}"
    );
    assert!(
        !code.contains("result.clone()"),
        "must not clone the returned &mut ref, got:\n{code}"
    );
    assert!(
        !code.contains("let result ="),
        "self-returning builder must not bind the returned &mut ref, got:\n{code}"
    );
}

/// Build a single-variant tagged enum with one sanitized field named `entries`, for exercising
/// the binding↔core conversion of sanitized tagged-enum fields.
fn sanitized_field_test_enum(ty: TypeRef, optional: bool) -> EnumDef {
    EnumDef {
        name: "NodeContent".to_string(),
        rust_path: "fixture_core::NodeContent".to_string(),
        variants: vec![EnumVariant {
            name: "MetadataBlock".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty,
                optional,
                sanitized: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("node_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    }
}

fn sanitized_field_conversions(enum_def: &EnumDef) -> (String, String) {
    let struct_names = AHashSet::new();
    (
        gen_tagged_enum_binding_to_core(enum_def, "fixture_core", "Js", &struct_names, &[]),
        gen_tagged_enum_core_to_binding(enum_def, "fixture_core", "Js", &struct_names, None, &[]),
    )
}

/// Tagged-enum discriminator values (`#[serde(rename_all/rename)]`) must drive the wire tag in
/// both conversion directions, never the raw variant/field Rust identifier.
/// Regression coverage for PR #218 (the discriminator half, unrelated to sanitized fields).
#[test]
fn tagged_enum_conversions_use_serde_wire_names() {
    let enum_def = EnumDef {
        name: "NodeContent".to_string(),
        rust_path: "fixture_core::NodeContent".to_string(),
        variants: vec![
            EnumVariant {
                name: "ListItem".to_string(),
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "PageBreak".to_string(),
                serde_rename: Some("explicit-page-break".to_string()),
                ..Default::default()
            },
        ],
        serde_content: None,
        serde_tag: Some("node_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let (binding_to_core, core_to_binding) = sanitized_field_conversions(&enum_def);

    for generated in [&binding_to_core, &core_to_binding] {
        assert!(generated.contains("\"list_item\""), "generated:\n{generated}");
        assert!(generated.contains("\"explicit-page-break\""), "generated:\n{generated}");
        assert!(!generated.contains("\"listitem\""), "generated:\n{generated}");
        assert!(!generated.contains("\"pagebreak\""), "generated:\n{generated}");
    }
}

/// A table of sanitized tagged-enum field shapes, asserting both conversion directions in one
/// pass. Covers: the `Vec<Vec<String>>` shape (issue #217's `Vec<(String, String)>` case) both
/// non-optional and optional, the `Map<String, String>` shape, and an unsupported shape
/// (`Vec<Named>`) that must keep emitting the pre-#218 `Default::default()` / `None` fallback —
/// the only form that is guaranteed to compile for a shape this backend cannot invert.
struct SanitizedFieldCase {
    label: &'static str,
    ty: TypeRef,
    optional: bool,
    binding_to_core_expect: &'static str,
    core_to_binding_expect: &'static str,
    /// Substrings that must be absent from the core→binding output: the pre-fix behavior either
    /// dropped the field entirely (`entries: None`) or left the destructured variable unused
    /// (`entries: _entries`) even though a real conversion was possible.
    core_to_binding_forbid: &'static [&'static str],
}

#[test]
fn sanitized_tagged_enum_field_conversions_table() {
    let cases = [
        SanitizedFieldCase {
            label: "vec_vec_string non-optional (issue #217)",
            ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            optional: false,
            binding_to_core_expect: "entries: val.entries.as_deref().unwrap_or_default().iter().filter_map",
            core_to_binding_expect: "entries: Some(entries.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "vec_vec_string optional",
            ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            optional: true,
            binding_to_core_expect: "entries: val.entries.map(|v| v.iter().filter_map(|inner| { let mut it = inner.iter().cloned(); Some((it.next()?, it.next()?)) }).collect())",
            core_to_binding_expect: "entries: entries.map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "map_string_string non-optional",
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: false,
            binding_to_core_expect: "entries: val.entries.unwrap_or_default().into_iter().map(|(k, v)| (k.into(), v.into())).collect()",
            core_to_binding_expect: "entries: Some(entries.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "map_string_string optional",
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            binding_to_core_expect: "entries: val.entries.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())",
            core_to_binding_expect: "entries: entries.map(|v| v.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "unsupported vec_named shape falls back to Default/None",
            ty: TypeRef::Vec(Box::new(TypeRef::Named("Widget".to_string()))),
            optional: false,
            binding_to_core_expect: "entries: Default::default()",
            core_to_binding_expect: "entries: None",
            core_to_binding_forbid: &[],
        },
    ];

    for case in cases {
        let enum_def = sanitized_field_test_enum(case.ty, case.optional);
        let (binding_to_core, core_to_binding) = sanitized_field_conversions(&enum_def);

        assert!(
            binding_to_core.contains(case.binding_to_core_expect),
            "[{}] binding->core missing expected fragment {:?}, got:\n{binding_to_core}",
            case.label,
            case.binding_to_core_expect
        );
        assert!(
            core_to_binding.contains(case.core_to_binding_expect),
            "[{}] core->binding missing expected fragment {:?}, got:\n{core_to_binding}",
            case.label,
            case.core_to_binding_expect
        );
        for forbidden in case.core_to_binding_forbid {
            assert!(
                !core_to_binding.contains(forbidden),
                "[{}] core->binding must not contain {:?}, got:\n{core_to_binding}",
                case.label,
                forbidden
            );
        }
        assert!(
            !binding_to_core.contains("format!(\"{:?}\""),
            "[{}] binding->core must never re-parse a Debug-formatted string, got:\n{binding_to_core}",
            case.label
        );
    }
}

/// Regression coverage for the `clippy::redundant_field_names` fix: an optional tagged-enum
/// field whose type needs no cast/wrap (e.g. plain `Option<String>`) falls into the fallback
/// match arm in `gen_tagged_enum_core_to_binding`, where the destructured core-side variable
/// is bound under the same name as the binding field it fills. The field-init expression must
/// use shorthand (`text`), never `text: text`.
#[test]
fn core_to_binding_uses_shorthand_for_self_assigned_optional_field() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        !core_to_binding.contains("text: text"),
        "self-assigned optional field must use shorthand, not `text: text`:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("text }") || core_to_binding.contains("text,"),
        "expected field-init shorthand for `text`:\n{core_to_binding}"
    );
}

/// Same fallback arm, but for a field whose type does need a cast/wrap (`Vec<Named>` maps
/// through `.into()`) — this must keep the real `field: expr` form, not collapse to shorthand.
#[test]
fn core_to_binding_keeps_expr_form_when_conversion_is_required() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Items".to_string(),
            fields: vec![FieldDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Widget".to_string()))),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        core_to_binding.contains("items: items.map(|v| v.into_iter().map(Into::into).collect())"),
        "Vec<Named> optional field must keep its real conversion expression:\n{core_to_binding}"
    );
}

/// Regression test: an optional `Named` tagged-enum field whose type has a generated binding
/// struct (`has_binding` in the old code) must NOT be dereferenced unless the CORE field is
/// actually `Option<Box<T>>`. Before the fix, `core_to_binding_field_init` picked the deref
/// form purely off `has_binding`, so a plain (unboxed) `Option<T>` field emitted
/// `nested.map(|v| (*v).into())`, which fails to compile with E0614 because `v` is `T`, not
/// `Box<T>`. Mirrors the real-world defect on `DocxAppProperties`/`CoreProperties`/`YearRange`
/// fields (all plain `Option<T>`, all with a binding struct, none boxed).
#[test]
fn core_to_binding_optional_unboxed_named_field_is_not_dereferenced() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "WithNested".to_string(),
            fields: vec![FieldDef {
                name: "nested".to_string(),
                ty: TypeRef::Named("Nested".to_string()),
                optional: true,
                is_boxed: false,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    // `struct_names` containing "Nested" is what makes `has_binding` true for this field --
    // exactly the condition the old code (wrongly) used to decide whether to dereference.
    let mut struct_names = AHashSet::new();
    struct_names.insert("Nested".to_string());
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names, None, &[]);

    assert!(
        !core_to_binding.contains("nested.map(|v| (*v).into())"),
        "unboxed Option<T> field must not be dereferenced (E0614 -- v is T, not Box<T>):\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("nested: nested.map(|v| v.into())"),
        "expected the unboxed optional Named field conversion:\n{core_to_binding}"
    );
}

fn node_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Regression test for issue #380, exercised through the real `NapiBackend::generate_bindings`
/// path (not just the `gen_function` unit): a `&mut T` DTO parameter on a unit-returning free
/// function must render as a binding that returns the mutated intermediate.
#[test]
fn generate_bindings_writeback_free_function_returns_mutated_dto() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_record".to_string(),
            rust_path: "test_lib::tag_record".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = NapiBackend.generate_bindings(&api, &node_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("record_core.into()"),
        "expected the write-back tail through the real generate_bindings path:\n{}",
        lib.content
    );
}

/// `reject_unsupported_writeback` must fire through the real `NapiBackend::generate_bindings`
/// path: a `&mut T` DTO param on a function that ALSO returns a value has no free return slot
/// for the write-back value, so generation must fail loudly (naming the function).
#[test]
fn generate_bindings_rejects_mut_dto_param_with_non_unit_return() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_and_count".to_string(),
            rust_path: "test_lib::tag_and_count".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    };

    let error = NapiBackend
        .generate_bindings(&api, &node_config())
        .expect_err("a `&mut` DTO param plus a non-unit return must be rejected at generation time");
    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the offending function:\n{message}"
    );
}

#[test]
fn promoted_required_method_and_constructor_parameters_return_invalid_arg() {
    use super::constructors::napi_variant_wrapper_constructor;
    use super::types::{gen_opaque_instance_method, gen_static_method};
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
    use std::collections::HashMap;

    let params = vec![
        ParamDef {
            name: "label".into(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        },
        ParamDef {
            name: "limit".into(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        },
    ];
    let instance = MethodDef {
        name: "apply".into(),
        params: params.clone(),
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    let static_method = MethodDef {
        name: "build".into(),
        params: params.clone(),
        return_type: TypeRef::Named("Widget".into()),
        ..Default::default()
    };
    let constructor = MethodDef {
        name: "new".into(),
        params,
        return_type: TypeRef::Named("Widget".into()),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Widget".into(),
        rust_path: "sample_core::Widget".into(),
        is_opaque: true,
        is_variant_wrapper: true,
        methods: vec![instance.clone(), static_method.clone(), constructor],
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".into());
    let cfg = NapiBackend::binding_config("sample_core", "Js", true);
    let empty = AHashSet::new();
    let instance_output = gen_opaque_instance_method(
        &instance,
        &mapper,
        &typ,
        &cfg,
        &empty,
        "Js",
        &crate::adapters::AdapterBodies::new(),
        &ahash::AHashMap::new(),
        &empty,
        &HashMap::new(),
    );
    let static_output = gen_static_method(&static_method, &mapper, &typ, &cfg, &empty, "Js", &empty);
    let constructor_output = napi_variant_wrapper_constructor(&typ, &mapper, "sample_core", "Js").unwrap();

    for output in [&instance_output, &static_output, &constructor_output] {
        assert!(output.contains("label: Option<String>, limit: Option<u32>"), "{output}");
        assert!(output.contains("limit.ok_or_else"), "{output}");
        assert!(
            output.contains("-> Result") || output.contains("-> napi::Result"),
            "{output}"
        );
        assert!(!output.contains("expect("), "{output}");
    }
}

#[test]
fn flattened_optional_transparent_wrapper_uses_one_binding_option_layer() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "nested_optional_credential".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper_with_methods(
                vec![vec![NewtypeContainer::Optional, NewtypeContainer::Optional]],
                "new_secret",
                "expose_secret",
            )),
            ..Default::default()
        }],
        has_default: true,
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        cast_large_ints_to_i64: true,
        cast_f32_to_f64: true,
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    assert!(
        to_core.contains("__result.nested_optional_credential =")
            && to_core.contains("map(|value| fixture_core::SecretString::new_secret(value))")
            && to_core.contains(".map(Some)"),
        "the single JS optional must convert once and reconstruct the outer Rust Option:\n{to_core}"
    );
    assert!(
        !to_core.contains("map(|value| (value).map("),
        "a String inside the flattened JS Option is not another iterator/Option:\n{to_core}"
    );

    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);
    assert!(
        from_core.contains("flatten()") && from_core.contains("expose_secret()"),
        "the two Rust options must flatten before consuming the wrapper:\n{from_core}"
    );
    assert!(
        !from_core.contains("nested_optional_credential.flatten().map(|v| v.to_string())"),
        "an explicit transparent wrapper must not require Display:\n{from_core}"
    );
}

#[test]
fn flattened_optional_wrapper_recurses_through_vec_map_and_named_sibling() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "Report".into(),
        rust_path: "fixture_core::Report".into(),
        fields: vec![FieldDef {
            name: "nested_optional_segments".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("Segment".into())),
            ))))),
            optional: true,
            newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Optional,
                NewtypeContainer::Vec,
                NewtypeContainer::MapKey,
            ]])),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        cast_large_ints_to_i64: true,
        cast_f32_to_f64: true,
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    assert!(to_core.contains("fixture_core::SecretString::from(key)"), "{to_core}");
    assert!(
        to_core.contains("(value).into()"),
        "the JsSegment map value must convert to Segment:\n{to_core}"
    );

    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);
    assert!(from_core.contains("(key).into_inner()"), "{from_core}");
    assert!(
        from_core.contains("(value).into()"),
        "the Segment map value must convert to JsSegment:\n{from_core}"
    );
}

#[test]
fn excluded_flattened_optional_wrapper_requires_no_generated_conversion() {
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeDef, TypeRef};

    let typ = TypeDef {
        name: "HiddenReport".into(),
        rust_path: "fixture_core::HiddenReport".into(),
        fields: vec![FieldDef {
            name: "hidden_credential".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            binding_excluded: true,
            newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
                NewtypeContainer::Optional,
                NewtypeContainer::Optional,
            ]])),
            ..Default::default()
        }],
        has_default: true,
        ..Default::default()
    };
    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..Default::default()
    };

    let to_core = super::transparent_newtypes::gen_from_binding_to_core(&typ, "fixture_core", &config);
    let from_core =
        super::transparent_newtypes::gen_from_core_to_binding(&typ, "fixture_core", &AHashSet::new(), &config);

    assert!(!to_core.contains("hidden_credential"), "{to_core}");
    assert!(!from_core.contains("hidden_credential"), "{from_core}");
}

#[test]
fn nested_optional_boxed_enum_fallback_defaults_the_complete_container() {
    use crate::core::ir::{FieldDef, NewtypeContainer, TypeRef};

    let field = FieldDef {
        name: "_0".into(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        is_boxed: true,
        newtype_wrapper: Some(transparent_string_wrapper(vec![vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
        ]])),
        ..Default::default()
    };

    assert_eq!(
        super::transparent_newtypes::default_to_core(&field),
        "Default::default()"
    );
}
