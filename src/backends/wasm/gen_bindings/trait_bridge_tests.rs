//! Trait-bridge forwarding, visitor-field forwarding, and export-reachability coverage.
//!
//! Split out of `tests.rs` (see `file-modularization` in CLAUDE.md): these tests share a single
//! concern -- whether the wasm backend correctly threads a trait-bridge or visitor handle through
//! a generated `From` impl, and whether it correctly decides a function or bridge operation is
//! reachable/exported for a given `ApiSurface` and config.

use super::{WasmCallability, forward_trait_bridge_builder_fields, function_is_exported, wasm_callability};
use crate::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

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

#[test]
fn test_visitor_field_substitution_in_post_process() {
    let mut content = "impl From<WasmConversionOptions> for sample_markup_rs::options::ConversionOptions {\n    fn from(val: WasmConversionOptions) -> Self {\n        Self {\n            heading_style: val.heading_style.into(),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\nimpl From<WasmConversionOptionsUpdate> for sample_markup_rs::options::ConversionOptionsUpdate {\n    fn from(val: WasmConversionOptionsUpdate) -> Self {\n        Self {\n            heading_style: val.heading_style.map(Into::into),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\n".to_string();

    let field_name = "visitor";
    let patterns = &[
        ("            ", "\n            "),
        ("        ", "\n        "),
        ("  ", "\n  "),
    ];
    for (indent, newline_indent) in patterns {
        let old_pattern = format!("{indent}{field_name}: Default::default(),{newline_indent}..Default::default()");
        let new_pattern = format!(
            "{indent}{field_name}: val.{field_name}.map(|v| (*v.inner).clone()),{newline_indent}..Default::default()"
        );
        if content.contains(&old_pattern) {
            content = content.replace(&old_pattern, &new_pattern);
        }
    }

    assert!(
        content.contains("visitor: val.visitor.map(|v| (*v.inner).clone()),"),
        "Visitor field not forwarded in From impl"
    );
    assert!(
        !content.contains("visitor: Default::default(),\n            ..Default::default()"),
        "Unreplaced visitor: Default::default() with 12 spaces still present"
    );
}

#[test]
fn trait_bridge_builder_field_forwards_the_handle() {
    let bridge = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        param_name: Some("renderer".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        ..Default::default()
    };
    let content = "core_options.renderer(renderer.as_ref().map(|v| &v.inner));".to_string();
    let mut config = make_config();
    config.trait_bridges = vec![bridge];

    let generated = forward_trait_bridge_builder_fields(content, &config);

    assert_eq!(
        generated,
        "core_options.renderer(renderer.map(|v| (*v.inner).clone()));"
    );
    assert!(!generated.contains(".renderer(None)"));
}

#[test]
fn wasm_function_reachability_follows_target_features() {
    let functions = vec![
        FunctionDef {
            name: "download".into(),
            rust_path: "sample::download".into(),
            cfg: Some(r#"feature = "download""#.into()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "prefetch".into(),
            rust_path: "sample::prefetch".into(),
            cfg: Some(r#"not(feature = "download")"#.into()),
            ..FunctionDef::default()
        },
    ];
    let config = make_config();

    assert!(!function_is_exported("download", &functions, &config));
    assert!(function_is_exported("prefetch", &functions, &config));
}

fn reachability_functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef {
            name: "download_assets".into(),
            rust_path: "sample::download_assets".into(),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "gated_download".into(),
            rust_path: "sample::gated_download".into(),
            cfg: Some(r#"feature = "download""#.into()),
            ..FunctionDef::default()
        },
    ]
}

#[test]
fn wasm_callability_accepts_the_javascript_spelling_of_an_exported_function() {
    let functions = reachability_functions();
    let config = make_config();

    assert_eq!(
        wasm_callability("downloadAssets", &functions, &config),
        WasmCallability::Callable,
        "`overrides.wasm.function` names the symbol the way wasm-bindgen exports it"
    );
    assert_eq!(
        wasm_callability("download_assets", &functions, &config),
        WasmCallability::Callable,
        "the Rust spelling must keep working for calls that carry no override"
    );
}

#[test]
fn wasm_callability_accepts_a_bridge_registry_operation_under_either_spelling() {
    let functions = reachability_functions();
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "RerankerBackend".into(),
        clear_fn: Some("clear_reranker_backends".into()),
        unregister_fn: Some("unregister_reranker_backend".into()),
        ..Default::default()
    }];

    assert_eq!(
        wasm_callability("clearRerankerBackends", &functions, &config),
        WasmCallability::Callable
    );
    assert_eq!(
        wasm_callability("unregister_reranker_backend", &functions, &config),
        WasmCallability::Callable
    );
    assert!(
        !function_is_exported("clear_reranker_backends", &functions, &config),
        "the codegen predicate must keep answering `false` -- the plain function generator does \
         not emit bridge-managed functions, the trait-bridge generator does"
    );
}

#[test]
fn wasm_callability_tells_an_unknown_name_apart_from_an_unexported_one() {
    let functions = reachability_functions();
    let config = make_config();

    assert_eq!(
        wasm_callability("gatedDownload", &functions, &config),
        WasmCallability::NotExported,
        "a real function the target drops is a capability gap"
    );
    assert_eq!(
        wasm_callability("fetchAssets", &functions, &config),
        WasmCallability::UnknownSymbol,
        "a name nothing answers to is a config error and must not be reported as a capability gap"
    );
    assert_eq!(
        wasm_callability("", &functions, &config),
        WasmCallability::UnknownSymbol,
        "an unresolved name must never be answered with a confident `not exported`"
    );
}

#[test]
fn wasm_callability_honours_an_exclusion_reached_by_the_javascript_spelling() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
exclude_functions = ["download_assets"]
"#,
    )
    .expect("an exclusion list must deserialize");
    let config = cfg.resolve().expect("config must resolve").remove(0);

    assert_eq!(
        wasm_callability("downloadAssets", &reachability_functions(), &config),
        WasmCallability::NotExported,
        "resolving the JavaScript spelling must not route around `exclude_functions`"
    );
}

#[test]
fn wasm_callability_rejects_a_zero_arg_function_returning_an_excluded_type() {
    let functions = vec![FunctionDef {
        name: "hidden".into(),
        rust_path: "sample::hidden".into(),
        return_type: TypeRef::Named("Hidden".into()),
        ..Default::default()
    }];
    let mut config = make_config();
    config.wasm.as_mut().expect("WASM config").exclude_types = vec!["Hidden".into()];

    assert_eq!(
        wasm_callability("hidden", &functions, &config),
        WasmCallability::NotExported
    );
    assert!(!function_is_exported("hidden", &functions, &config));
}

#[test]
fn wasm_callability_rejects_a_function_accepting_an_excluded_type() {
    let functions = vec![FunctionDef {
        name: "consume_hidden".into(),
        rust_path: "sample::consume_hidden".into(),
        params: vec![ParamDef {
            name: "hidden".into(),
            ty: TypeRef::Named("Hidden".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut config = make_config();
    config.wasm.as_mut().expect("WASM config").exclude_types = vec!["Hidden".into()];

    assert_eq!(
        wasm_callability("consumeHidden", &functions, &config),
        WasmCallability::NotExported
    );
    assert!(!function_is_exported("consume_hidden", &functions, &config));
}
