//! NAPI trait-bridge native-object callback args + typed host interface.
//!
//! Neutral fixture: a plugin trait `Greeter` with a method that takes a known serde
//! struct `Opts` (plus an enum `Mood`, an opaque `Handle`, and an unknown `Mystery`),
//! and returns a struct `Doc`. Asserts:
//!   (a) the struct param is marshalled to JS as the binding's NATIVE object
//!       (`JsOpts::from(...)` via `ToNapiValue`), not a debug/JSON string;
//!   (b) enum / opaque / unknown `Named` params keep their prior (non-native) handling;
//!   (c) the emitted TypeScript host interface types the struct param and the return
//!       natively (`JsOpts` / `JsDoc`), not `string`;
//!   (d) the `register_*` entry point types its callback against that interface.

use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::*;

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn make_param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        is_ref,
        ..Default::default()
    }
}

/// Plain data-struct TypeDef (not a trait, not opaque) with serde.
fn serde_struct(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        has_serde: true,
        fields,
        ..Default::default()
    }
}

/// The neutral `Greeter` plugin trait: `process(&self, opts: &Opts, mood: &Mood, handle: &Handle,
/// mystery: &Mystery) -> Doc`.
fn greeter_trait() -> TypeDef {
    TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![MethodDef {
            name: "process".to_string(),
            params: vec![
                make_param("opts", TypeRef::Named("Opts".to_string()), true),
                make_param("mood", TypeRef::Named("Mood".to_string()), true),
                make_param("handle", TypeRef::Named("Handle".to_string()), true),
                make_param("mystery", TypeRef::Named("Mystery".to_string()), true),
            ],
            return_type: TypeRef::Named("Doc".to_string()),
            receiver: Some(ReceiverKind::Ref),
            error_type: Some("Error".to_string()),
            is_async: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn greeter_api() -> ApiSurface {
    let opts = serde_struct("Opts", vec![make_field("label", TypeRef::String)]);
    let mut doc = serde_struct("Doc", vec![make_field("text", TypeRef::String)]);
    doc.is_return_type = true;
    let mut handle = serde_struct("Handle", vec![make_field("id", TypeRef::String)]);
    handle.is_opaque = true;

    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![greeter_trait(), opts, doc, handle],
        enums: vec![EnumDef {
            name: "Mood".to_string(),
            rust_path: "test_lib::Mood".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn greeter_bridge_cfg() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }
}

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "test-lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn trait_bridge_marshals_struct_param_as_native_js_object() {
    let trait_def = greeter_trait();
    let api = greeter_api();
    let bridge = alef::backends::napi::trait_bridge::gen_trait_bridge(
        &trait_def,
        &greeter_bridge_cfg(),
        "test_lib",
        "TestLibError",
        "TestLibError::from({msg})",
        &api,
    )
    .expect("gen_trait_bridge must succeed for Greeter");
    let code = &bridge.code;

    assert!(
        code.contains("JsOpts::from"),
        "struct param must be constructed as the native JsOpts via From<core>:\n{code}"
    );
    assert!(
        !code.contains("format!(\"{:?}\", opts)"),
        "struct param must NOT be serialized to a debug string:\n{code}"
    );

    for non_struct in ["JsMood::from", "JsHandle::from", "JsMystery::from"] {
        assert!(
            !code.contains(non_struct),
            "{non_struct} must NOT be native-marshalled (enum/opaque/unknown):\n{code}"
        );
    }
    assert!(
        code.contains("format!(\"{:?}\", mood)")
            && code.contains("format!(\"{:?}\", handle)")
            && code.contains("format!(\"{:?}\", mystery)"),
        "enum/opaque/unknown params must keep the prior debug-string representation:\n{code}"
    );
}

#[test]
fn dts_plugin_bridge_emits_typed_interface_and_typed_register() {
    let backend = NapiBackend;
    let mut config = make_config();
    config.trait_bridges = vec![greeter_bridge_cfg()];

    let dts = backend.generate_type_stubs(&greeter_api(), &config).unwrap()[0]
        .content
        .clone();

    // struct carries `#[napi(object, js_name = "Opts")]`.
    assert!(
        dts.contains("export interface Greeter {"),
        "plugin bridge must emit a host-implementable interface:\n{dts}"
    );
    assert!(
        dts.contains("opts: Opts"),
        "interface method must type the struct param as the native Opts type:\n{dts}"
    );
    assert!(
        dts.contains("process(") && dts.contains("): Promise<Doc>"),
        "interface method must type its return as the native Doc type, not string:\n{dts}"
    );
    assert!(
        !dts.contains("): Promise<string>"),
        "interface method return must not fall back to Promise<string>:\n{dts}"
    );

    assert!(
        dts.contains("export declare function registerGreeter(impl: Greeter): void;"),
        "register fn must type its callback param against the Greeter interface:\n{dts}"
    );
}
