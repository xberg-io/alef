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
//!    `__alef_{enum}_from_swift_string` -- a helper `enums.rs` emits only for fieldless enums
//!    (`E0425`) -- and forced the shim's return type to `Result` while the paired extern
//!    declaration, fed the real `unit_enum_names`, declared the infallible one (`E0308`).
//! 2. `trait_bridge::emit_trait_bridge_wrapper` received `unit_enum_names`, so a data-carrying
//!    enum in a trampoline's return position was wrapped with the tuple-struct call
//!    `{t}(value)` against an enum type (`E0423`).

use super::emit;
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
/// present in `enum_names`, and `enums::emit_enum_wrapper` emits no from-string helper for it.
fn tagged_enum() -> EnumDef {
    EnumDef {
        name: "Routing".to_string(),
        rust_path: "test_lib::Routing".to_string(),
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

fn lib_rs(files: &[crate::core::backend::GeneratedFile]) -> String {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("emit must produce the bridge crate's lib.rs")
        .content
        .clone()
}

#[test]
fn data_carrying_enum_method_param_never_references_the_unit_enum_from_string_helper() {
    let files = emit(&api_with_tagged_enum(), &swift_config_with_trait_bridge()).unwrap();
    let generated = lib_rs(&files);

    assert!(
        !generated.contains("__alef_routing_from_swift_string"),
        "no call site may name a from-string helper that is never emitted for a data-carrying \
         enum, got:\n{generated}"
    );
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
