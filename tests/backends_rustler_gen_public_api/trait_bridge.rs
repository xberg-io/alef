//! Trait-bridge public-API generation coverage for the Elixir/Rustler backend: NIF stub
//! parameter shapes, typed host behaviours, optional callbacks, and unregister/clear atom specs.
//!
//! Split out of `backends_rustler_gen_public_api_test.rs` (see `file-modularization` in
//! CLAUDE.md).

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

use super::{make_config, make_field};

#[test]
fn test_trait_bridge_unregister_and_clear_specs_match_atom_returns() {
    let backend = RustlerBackend;
    // The bridged trait has to be in the surface for the delegates to be emitted at all — a
    // bridge whose trait does not resolve emits no NIF to delegate to. It is declared with no
    // methods so no `Greeter.Host`-style behaviour block is emitted, keeping this test's negative
    // assertions about `@callback` return shapes about the delegate specs alone. ~keep
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "my_lib::OcrBackend".to_string(),
            is_trait: true,
            is_opaque: true,
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let files = backend.generate_public_api(&api, &config).unwrap();
    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");
    let content = &main.content;

    assert!(
        content.contains("@spec unregister_ocr_backend(String.t() | keyword(String.t())) :: :ok | :error")
            && content.contains("@spec clear_ocr_backends() :: :ok | :error"),
        "unregister/clear specs must match Rustler NIF atom returns; got:\n{content}"
    );
    assert!(
        !content.contains("{:ok, nil}") && !content.contains("{:error, atom, String.t()}"),
        "unregister/clear specs must not advertise tuple returns; got:\n{content}"
    );
}

fn make_plugin_bridge_trait() -> TypeDef {
    TypeDef {
        name: "Greeter".to_string(),
        rust_path: "my_lib::Greeter".to_string(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "process".to_string(),
            params: vec![ParamDef {
                name: "opts".to_string(),
                ty: TypeRef::Named("Opts".to_string()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("Doc".to_string()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            error_type: Some("Error".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn make_plugin_bridge_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            make_plugin_bridge_trait(),
            TypeDef {
                name: "Opts".to_string(),
                rust_path: "my_lib::Opts".to_string(),
                has_serde: true,
                fields: vec![make_field("label", TypeRef::String, false)],
                ..Default::default()
            },
            TypeDef {
                name: "Doc".to_string(),
                rust_path: "my_lib::Doc".to_string(),
                has_serde: true,
                is_return_type: true,
                fields: vec![make_field("text", TypeRef::String, false)],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn make_plugin_bridge_config() -> ResolvedCrateConfig {
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("my_lib::registry::get".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }];
    config
}

#[test]
fn test_plugin_bridge_emits_typed_host_behaviour() {
    let backend = RustlerBackend;
    let files = backend
        .generate_public_api(&make_plugin_bridge_api(), &make_plugin_bridge_config())
        .expect("generate_public_api should succeed");

    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib.ex"))
        .expect("main module file should be generated");
    let content = &main.content;

    assert!(
        content.contains("defmodule Greeter.Host do"),
        "plugin bridge must emit a typed host behaviour module; got:\n{content}"
    );
    assert!(
        content.contains("@callback process(map()) :: {:ok, map()} | {:error, atom, String.t()}"),
        "behaviour @callback must type the struct param and the result; got:\n{content}"
    );

    assert!(
        content.contains("def register_greeter(genserver_pid, plugin_name, implemented_methods \\\\ []) do")
            && content.contains("Greeter.Host"),
        "register delegate must reference the host behaviour and default the exports list; got:\n{content}"
    );
}

#[test]
fn test_visitor_bridge_does_not_emit_host_behaviour() {
    let backend = RustlerBackend;
    let api = make_plugin_bridge_api();
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: None,
        type_alias: Some("GreeterHandle".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api should succeed");
    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("my_lib.ex"))
        .expect("main module file should be generated");

    assert!(
        !main.content.contains("MyLib.Greeter.Host"),
        "bridges without register_* must not emit a host behaviour; got:\n{}",
        main.content
    );
}

#[test]
fn test_trait_behaviour_callback_params_are_maps_with_optional_callbacks() {
    let backend = RustlerBackend;

    let ocr_config = TypeDef {
        name: "OcrConfig".to_string(),
        rust_path: "my_lib::OcrConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![make_field("language", TypeRef::String, false)],
        ..Default::default()
    };
    let trait_def = TypeDef {
        name: "OcrBackend".to_string(),
        rust_path: "my_lib::OcrBackend".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![
            MethodDef {
                name: "process_image".to_string(),
                params: vec![ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("OcrConfig".to_string()),
                    is_ref: true,
                    ..Default::default()
                }],
                return_type: TypeRef::Named("OcrConfig".to_string()),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
                cfg: None,
                error_type: Some("Error".to_string()),
                ..Default::default()
            },
            MethodDef {
                name: "supports_table_detection".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
                cfg: None,
                has_default_impl: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![ocr_config, trait_def],
        ..Default::default()
    };
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }];

    let files = backend.generate_public_api(&api, &config).unwrap();
    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().replace('\\', "/").ends_with("my_lib.ex"))
        .expect("my_lib.ex should be generated");
    let content = &main.content;

    assert!(
        content.contains("@callback process_image(map())"),
        "callback struct param must be map(), not the input-direction JSON string; got:\n{content}"
    );
    assert!(
        !content.contains("@callback process_image(String.t() | nil)"),
        "stale JSON-string spec must be gone; got:\n{content}"
    );
    assert!(
        content.contains("@callback initialize() :: any()") && content.contains("@callback shutdown() :: any()"),
        "lifecycle hooks must be declared; got:\n{content}"
    );
    assert!(
        content.contains("@optional_callbacks [supports_table_detection: 0, initialize: 0, shutdown: 0]"),
        "defaulted + lifecycle methods must be optional callbacks; got:\n{content}"
    );
}

#[test]
fn test_register_nif_stub_has_implemented_methods_parameter() {
    let backend = RustlerBackend;
    // `native.ex` declares the NIFs `native::gen_trait_bridge` exports, and that pass runs only
    // when the bridged trait resolves — so the trait has to be in the surface for a stub to exist
    // at all. Method-less keeps the stub's parameter list the only thing under test. ~keep
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "my_lib::OcrBackend".to_string(),
            is_trait: true,
            is_opaque: true,
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let mut config = make_config("my_lib");
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }];

    let files = backend.generate_public_api(&api, &config).unwrap();
    let native = files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("my_lib/native.ex")
        })
        .expect("native.ex should be generated");

    assert!(
        native
            .content
            .contains("def register_ocr_backend(_pid, _name, _implemented_methods)"),
        "NIF stub register_ocr_backend must have 3 parameters (pid, name, implemented_methods); got:\n{}",
        native.content
    );
}
