//! End-to-end coverage for the two call sites in [`super::emit`] that used to hand a
//! `unit_enum_names` set to a consumer that needs EVERY enum.
//!
//! Both defects are invisible to a direct unit test of the emitter they mis-fed, because the
//! emitter behaves correctly for whatever set it is given -- the wiring is the bug. They are
//! exercised here through the real `emit` path instead, the pattern `cfg_variant_e2e_tests`
//! established for the sibling enum regressions.
//!
//! 1. `wrappers::emit_type_method_shims` received `enum_names` for its `unit_enum_names`
//!    parameter, so a data-carrying enum method parameter was reconstructed with
//!    `__alef_{enum}_from_swift_string` -- a discriminant-only compatibility helper -- and
//!    forced the shim's return type to `Result` while the paired extern
//!    declaration, fed the real `unit_enum_names`, declared the infallible one (`E0308`).
//! 2. `trait_bridge::emit_trait_bridge_wrapper` received `unit_enum_names`, so a data-carrying
//!    enum in a trampoline's return position was wrapped with the tuple-struct call
//!    `{t}(value)` against an enum type (`E0423`).

use super::emit;
use crate::backends::swift::gen_bindings::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef,
};

fn swift_config_with_trait_bridge() -> ResolvedCrateConfig {
    let toml_src = "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"test-lib\"\n\
         sources = [\"src/lib.rs\"]\n[[crates.trait_bridges]]\ntrait_name = \"Sink\"\n";
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// `Routing` carries a tuple variant, so it is a TAGGED enum: absent from `unit_enum_names`,
/// present in `enum_names`, but absent from `unit_enum_names`.
fn tagged_enum() -> EnumDef {
    EnumDef {
        name: "Routing".to_string(),
        rust_path: "test_lib::Routing".to_string(),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Direct".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Proxy".to_string(),
                fields: vec![FieldDef {
                    name: "0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn api_with_tagged_enum() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![tagged_enum()],
        types: vec![
            TypeDef {
                name: "Options".to_string(),
                rust_path: "test_lib::Options".to_string(),
                has_default: true,
                has_serde: true,
                fields: vec![FieldDef {
                    name: "routing".to_string(),
                    ty: TypeRef::Named("Routing".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "Client".to_string(),
                rust_path: "test_lib::Client".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "set_routing".to_string(),
                    params: vec![ParamDef {
                        name: "routing".to_string(),
                        ty: TypeRef::Named("Routing".to_string()),
                        ..Default::default()
                    }],
                    return_type: TypeRef::Unit,
                    receiver: Some(ReceiverKind::RefMut),
                    error_type: None,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "Sink".to_string(),
                rust_path: "test_lib::Sink".to_string(),
                is_trait: true,
                methods: vec![MethodDef {
                    name: "current_routing".to_string(),
                    return_type: TypeRef::Named("Routing".to_string()),
                    receiver: Some(ReceiverKind::Ref),
                    error_type: None,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn api_with_typealiased_tagged_enum_owner() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![tagged_enum()],
        types: vec![TypeDef {
            name: "LegacyOptions".to_string(),
            rust_path: "test_lib::LegacyOptions".to_string(),
            is_opaque: true,
            has_default: true,
            has_serde: true,
            fields: vec![
                FieldDef {
                    name: "routing".to_string(),
                    ty: TypeRef::Named("Routing".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "routes".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Routing".to_string()))),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn lib_rs(files: &[crate::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("emit must produce the bridge crate's lib.rs")
        .content
        .clone()
}

fn swift_source(files: &[crate::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| {
            f.path.extension().is_some_and(|extension| extension == "swift")
                && f.content.contains("public struct Options")
        })
        .expect("emit must produce the public Swift module")
        .content
        .clone()
}

#[test]
fn data_carrying_enum_method_param_uses_the_lossless_json_path() {
    let files = emit(&api_with_tagged_enum(), &swift_config_with_trait_bridge()).unwrap();
    let generated = lib_rs(&files);

    assert!(
        generated.contains("Routing>(&routing).expect(\"valid JSON for routing\")"),
        "the data-carrying enum parameter must be deserialized from JSON, got:\n{generated}"
    );
}

#[test]
fn data_carrying_enum_method_param_keeps_the_shim_and_extern_signatures_in_agreement() {
    let files = emit(&api_with_tagged_enum(), &swift_config_with_trait_bridge()).unwrap();
    let generated = lib_rs(&files);

    assert!(
        generated.contains("pub fn client_set_routing(client: &mut Client, routing: String)"),
        "the shim must take the enum as a bridged String, got:\n{generated}"
    );
    assert!(
        !generated.contains("client_set_routing(client: &mut Client, routing: String) -> Result"),
        "nothing about a data-carrying enum parameter is fallible, so neither the shim nor the \
         extern declaration may be forced into a Result return, got:\n{generated}"
    );
}

#[test]
fn data_carrying_enum_trait_return_is_converted_with_from_not_a_tuple_struct_call() {
    let files = emit(&api_with_tagged_enum(), &swift_config_with_trait_bridge()).unwrap();
    let generated = lib_rs(&files);

    assert!(
        generated.contains("Routing::from(this.0.current_routing())"),
        "a data-carrying enum in a trampoline's return position must go through the mirror \
         type's From impl, got:\n{generated}"
    );
    assert!(
        !generated.contains("Routing(this.0.current_routing())"),
        "must not call the enum type as a tuple-struct constructor, got:\n{generated}"
    );
}

#[test]
fn data_carrying_enum_option_uses_reversible_parent_json_bridge() {
    let config = swift_config_with_trait_bridge();
    let api = api_with_tagged_enum();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let generated_rust = lib_rs(&files);
    let generated_swift = swift_source(&files);

    assert!(
        generated_rust.contains("#[swift_bridge(init)]\n        fn new(routing: Routing) -> Options;"),
        "the legacy RustBridge initializer and C symbol must remain source compatible:\n{generated_rust}"
    );
    assert!(
        !generated_rust.contains("routing (Routing) is an enum; reverse From not generated — left at default"),
        "tagged enum options must never be silently left at their default:\n{generated_rust}"
    );
    assert!(
        generated_swift.contains("let data = try JSONEncoder().encode(self)"),
        "Options.intoRust() must serialize its complete tagged-enum payload:\n{generated_swift}"
    );
    assert!(
        generated_swift.contains("return try RustBridge.optionsFromJson(json)"),
        "Options.intoRust() must use the reversible parent JSON bridge:\n{generated_swift}"
    );
    assert!(
        !generated_swift.contains("return RustBridge.Options(try self.routing.intoRust())"),
        "the tagged enum's payload-erasing mirror must not feed the Options constructor:\n{generated_swift}"
    );
}

#[test]
fn typealiased_dto_keeps_its_public_initializer_when_it_contains_a_data_carrying_enum() {
    let config = swift_config_with_trait_bridge();
    let api = api_with_typealiased_tagged_enum_owner();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let generated_rust = lib_rs(&files);
    let generated_swift = files
        .iter()
        .find(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .expect("emit must produce the public Swift module")
        .content
        .as_str();

    assert!(
        generated_swift.contains("public typealias LegacyOptions = RustBridge.LegacyOptions"),
        "the compatibility case must exercise a public typealias:\n{generated_swift}"
    );
    assert!(
        generated_rust.contains(
            "#[swift_bridge(init)]\n        fn new(routing: Routing, routes: Vec<Routing>) -> LegacyOptions;"
        ),
        "a typealiased DTO must retain the bridge declaration that generates its public Swift convenience init:\n\
         {generated_rust}"
    );
    assert!(
        generated_rust.contains("__target.routing = __alef_routing_from_swift_string(&routing.to_string())"),
        "the compatibility initializer must deserialize the carrier's complete enum JSON:\n{generated_rust}"
    );
    assert!(
        generated_rust.contains(
            "__target.routes = routes.into_iter().map(|value| __alef_routing_from_swift_string(&value.to_string())"
        ),
        "the compatibility initializer must deserialize every carrier's complete enum JSON:\n{generated_rust}"
    );
    assert!(
        generated_rust.contains("pub struct Routing(pub test_lib::Routing);"),
        "the bridge carrier must retain the complete source enum value:\n{generated_rust}"
    );
    assert!(
        generated_rust.contains("serde_json::to_string(&self.0)"),
        "the bridge carrier must serialize the complete source enum value:\n{generated_rust}"
    );
    assert!(
        generated_rust.contains("serde_json::from_str::<test_lib::Routing>(value)"),
        "the compatibility helper must parse the complete source enum JSON:\n{generated_rust}"
    );
    assert!(
        !generated_rust.contains("Routing::Proxy(::std::default::Default::default())"),
        "the compatibility path must not invent a default payload:\n{generated_rust}"
    );
}
