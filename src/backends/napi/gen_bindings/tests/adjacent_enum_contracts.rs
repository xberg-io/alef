use super::super::NapiBackend;
use super::super::enums::{is_adjacent_content_collision_enum, is_json_passthrough_data_enum, is_tagged_data_enum};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};
use quote::ToTokens;

fn config() -> ResolvedCrateConfig {
    let source = "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n";
    toml::from_str::<NewAlefConfig>(source)
        .expect("fixture configuration")
        .resolve()
        .expect("resolved configuration")
        .remove(0)
}

fn unit(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.into(),
        ..Default::default()
    }
}

fn newtype(name: &str, ty: TypeRef) -> EnumVariant {
    EnumVariant {
        name: name.into(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".into(),
            ty,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn adjacent_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.into(),
        rust_path: format!("test_core::{name}"),
        has_serde: true,
        serde_tag: Some("type".into()),
        serde_content: Some("output".into()),
        serde_rename_all: Some("snake_case".into()),
        variants,
        ..Default::default()
    }
}

fn function(name: &str, param: &str, ty: &str) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        params: vec![ParamDef {
            name: param.into(),
            ty: TypeRef::Named(ty.into()),
            ..Default::default()
        }],
        return_type: TypeRef::Unit,
        ..Default::default()
    }
}

fn rust_bindings(api: &ApiSurface) -> syn::File {
    let files = NapiBackend
        .generate_bindings(api, &config())
        .expect("complete NAPI producer");
    let matches: Vec<_> = files.iter().filter(|file| file.path.ends_with("lib.rs")).collect();
    assert_eq!(matches.len(), 1, "one Rust binding module must be rendered");
    syn::parse_file(&matches[0].content).unwrap_or_else(|error| panic!("{error}\n{}", matches[0].content))
}

fn try_from_impls(file: &syn::File) -> Vec<&syn::ItemImpl> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(implementation) => Some(implementation),
            _ => None,
        })
        .filter(|implementation| {
            implementation
                .trait_
                .as_ref()
                .and_then(|(path, _)| path.segments.last())
                .is_some_and(|segment| segment.ident == "TryFrom")
        })
        .collect()
}

/// `Outcome` is an adjacent-collision passthrough (`serde_try_from_impl`), `Report` holds it
/// (`fallible_impl`), and `Event` is a tagged object made fallible by its `Outcome` field (the
/// `gen_tagged_enum_binding_to_core.jinja` fallible arm). Two of the three `Self` types are core
/// enums with an `Error` variant.
fn error_variant_api() -> ApiSurface {
    let outcome = adjacent_enum(
        "Outcome",
        vec![
            newtype("Text", TypeRef::String),
            newtype("Error", TypeRef::Primitive(PrimitiveType::U32)),
        ],
    );
    let event = EnumDef {
        name: "Event".into(),
        rust_path: "test_core::Event".into(),
        has_serde: true,
        serde_tag: Some("kind".into()),
        variants: vec![
            EnumVariant {
                name: "Done".into(),
                fields: vec![FieldDef {
                    name: "outcome".into(),
                    ty: TypeRef::Named("Outcome".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Error".into(),
                fields: vec![FieldDef {
                    name: "code".into(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    ApiSurface {
        crate_name: "test-lib".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Report".into(),
            rust_path: "test_core::Report".into(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "outcome".into(),
                ty: TypeRef::Named("Outcome".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        enums: vec![outcome, event],
        functions: vec![
            function("submit", "report", "Report"),
            function("emit", "event", "Event"),
            function("record", "outcome", "Outcome"),
        ],
        ..Default::default()
    }
}

/// Regression: a generated `impl TryFrom<JsX> for core::X` returned `Result<Self, Self::Error>`.
/// When the core enum has a variant named `Error`, `Self::Error` resolves to both that variant
/// and the impl's associated type (`ambiguous_associated_items`, a hard error in rustc). Every
/// fallible conversion must name the error type it declares.
#[test]
fn try_from_impls_name_the_error_type_when_the_core_enum_has_an_error_variant() {
    let file = rust_bindings(&error_variant_api());
    let impls = try_from_impls(&file);
    let mut targets: Vec<String> = impls
        .iter()
        .map(|implementation| implementation.self_ty.to_token_stream().to_string())
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        ["test_core :: Event", "test_core :: Outcome", "test_core :: Report"],
        "every fallible conversion route must be exercised"
    );
    for implementation in impls {
        let rendered = implementation.to_token_stream().to_string();
        // `Self :: Error { .. }` is the variant constructor; only the associated-type use is ambiguous.
        assert!(!rendered.contains("Self :: Error >"), "{rendered}");
        let function = implementation
            .items
            .iter()
            .find_map(|item| match item {
                syn::ImplItem::Fn(function) if function.sig.ident == "try_from" => Some(function),
                _ => None,
            })
            .unwrap_or_else(|| panic!("try_from must be generated: {rendered}"));
        assert_eq!(
            function.sig.output.to_token_stream().to_string(),
            "-> std :: result :: Result < Self , serde_json :: Error >",
            "{rendered}"
        );
    }
}

/// html-to-markdown's `VisitResult`: adjacently tagged, several data variants, every payload a
/// `String` under the shared `output` key.
fn visit_result() -> EnumDef {
    adjacent_enum(
        "VisitResult",
        vec![
            unit("Continue"),
            newtype("Custom", TypeRef::String),
            unit("Skip"),
            unit("PreserveHtml"),
            newtype("Error", TypeRef::String),
        ],
    )
}

fn visit_result_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".into(),
        version: "0.1.0".into(),
        enums: vec![visit_result()],
        functions: vec![function("visit", "result", "VisitResult")],
        ..Default::default()
    }
}

/// Regression: the adjacent content-collision route (#790) claimed every adjacently tagged enum
/// with two or more data variants, including ones whose payloads share one type and therefore one
/// content field. `VisitResult` became a `serde_json::Value` passthrough and lost its nominal
/// struct, its namespace constructors and its `VisitResult` value export from `index.d.ts` and
/// `index.mjs` (TS2693 for `VisitResult.Custom(...)`).
#[test]
fn adjacent_enum_with_one_shared_payload_type_keeps_its_value_export() {
    let enum_def = visit_result();
    assert!(!is_adjacent_content_collision_enum(&enum_def));
    assert!(is_tagged_data_enum(&enum_def, &[]));
    assert!(!is_json_passthrough_data_enum(&enum_def, &[]));

    let api = visit_result_api();
    let file = rust_bindings(&api);
    let binding = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "JsVisitResult" => Some(item),
            _ => None,
        })
        .expect("the nominal binding struct");
    let syn::Fields::Named(fields) = &binding.fields else {
        panic!(
            "must be a tagged object, not a JSON passthrough newtype: {}",
            binding.to_token_stream()
        )
    };
    let names: Vec<String> = fields
        .named
        .iter()
        .map(|field| field.ident.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(names, ["type_tag", "output"], "one shared content field");
    let constructors = file
        .items
        .iter()
        .filter(|item| match item {
            syn::Item::Fn(function) => function.attrs.iter().any(|attribute| {
                attribute
                    .to_token_stream()
                    .to_string()
                    .contains("namespace = \"VisitResult\"")
            }),
            _ => false,
        })
        .count();
    assert_eq!(constructors, 5, "one namespace constructor per variant");
    assert!(
        try_from_impls(&file).is_empty(),
        "the tagged object converts infallibly"
    );

    let stubs = NapiBackend.generate_type_stubs(&api, &config()).expect("type stubs");
    let stub = |suffix: &str| {
        let matches: Vec<_> = stubs
            .iter()
            .filter(|file| file.path.to_string_lossy().ends_with(suffix))
            .collect();
        assert_eq!(matches.len(), 1, "exactly one {suffix}");
        matches[0].content.clone()
    };
    let declarations = stub("index.d.ts");
    assert_eq!(
        declarations.matches("export declare const VisitResult: {").count(),
        1,
        "{declarations}"
    );
    let module = stub("index.mjs");
    assert_eq!(
        module
            .matches("export const VisitResult = binding.VisitResult;")
            .count(),
        1,
        "{module}"
    );
}

/// Payloads of different types under one content key still collide and stay on the passthrough.
#[test]
fn adjacent_enum_with_differing_payload_types_stays_a_json_passthrough() {
    let differing = adjacent_enum(
        "Choice",
        vec![
            newtype("Text", TypeRef::String),
            newtype("Count", TypeRef::Primitive(PrimitiveType::U32)),
        ],
    );
    assert!(is_adjacent_content_collision_enum(&differing));
    assert!(is_json_passthrough_data_enum(&differing, &[]));

    let mut struct_variant = visit_result();
    struct_variant.variants.push(EnumVariant {
        name: "Inline".into(),
        fields: vec![FieldDef {
            name: "raw".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(is_adjacent_content_collision_enum(&struct_variant));

    let named = adjacent_enum(
        "Policy",
        vec![
            newtype("Analyze", TypeRef::Named("Config".into())),
            newtype("Merge", TypeRef::Named("Config".into())),
        ],
    );
    assert!(
        is_adjacent_content_collision_enum(&named),
        "a Named payload is emitted once per variant"
    );
}
