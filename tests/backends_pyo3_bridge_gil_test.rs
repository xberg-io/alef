//! A synchronous `#[pyfunction]` that carries a trait bridge must release the GIL around its
//! core call, as every other synchronous free function already does (#136). The two bridge
//! wrappers had their own emitters and called the core with the GIL held, so one long
//! conversion stalled every Python thread (xberg-io/html-to-markdown#974).

use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, NewAlefConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FieldDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use quote::ToTokens;

fn walker_trait() -> TypeDef {
    TypeDef {
        name: "Walker".into(),
        rust_path: "sample_core::Walker".into(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "on_text".into(),
            params: vec![ParamDef {
                name: "text".into(),
                ty: TypeRef::String,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            has_default_impl: false,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn walk_options() -> TypeDef {
    TypeDef {
        name: "WalkOptions".into(),
        rust_path: "sample_core::WalkOptions".into(),
        has_default: true,
        fields: vec![
            FieldDef {
                name: "label".into(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "walker".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("WalkerHandle".into()))),
                optional: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn text_param() -> ParamDef {
    ParamDef {
        name: "text".into(),
        ty: TypeRef::String,
        is_ref: true,
        ..Default::default()
    }
}

fn options_field_function(name: &str, is_async: bool, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        rust_path: format!("sample_core::{name}"),
        params: vec![
            text_param(),
            ParamDef {
                name: "options".into(),
                ty: TypeRef::Named("WalkOptions".into()),
                optional: true,
                ..Default::default()
            },
        ],
        return_type: TypeRef::String,
        is_async,
        error_type: fallible.then(|| "std::io::Error".into()),
        ..Default::default()
    }
}

fn function_param_function(name: &str, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        rust_path: format!("sample_core::{name}"),
        params: vec![
            text_param(),
            ParamDef {
                name: "walker".into(),
                ty: TypeRef::Named("WalkerHandle".into()),
                ..Default::default()
            },
        ],
        return_type: TypeRef::String,
        is_async,
        error_type: Some("std::io::Error".into()),
        ..Default::default()
    }
}

fn bridge(bind_via: BridgeBinding) -> TraitBridgeConfig {
    let options_field = bind_via == BridgeBinding::OptionsField;
    TraitBridgeConfig {
        trait_name: "Walker".into(),
        type_alias: Some("WalkerHandle".into()),
        param_name: Some("walker".into()),
        bind_via,
        options_type: options_field.then(|| "WalkOptions".into()),
        options_field: options_field.then(|| "walker".into()),
        ..TraitBridgeConfig::default()
    }
}

fn generated(api: &ApiSurface, bridge: TraitBridgeConfig) -> String {
    let source = r#"
[workspace]
languages=['python']
[[crates]]
name='sample-core'
sources=[]
[crates.python]
module_name='_sample'
"#;
    let mut config = toml::from_str::<NewAlefConfig>(source)
        .unwrap()
        .resolve()
        .unwrap()
        .remove(0);
    config.replace_trait_bridges(vec![bridge]);
    Pyo3Backend
        .generate_bindings(api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content
}

/// The signature and body of the generated free function `name`, as token strings.
fn function(source: &str, name: &str) -> (String, String) {
    let file = syn::parse_file(source).expect("generated Rust must parse");
    file.items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Fn(item) if item.sig.ident == name => Some((
                item.sig.to_token_stream().to_string(),
                item.block.to_token_stream().to_string(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("generated function missing: {name}\n{source}"))
}

/// The tokens inside the parentheses of the first `py . detach (...)` call in `body`.
fn detached_closure(body: &str) -> &str {
    let open = body.find("py . detach (").expect("no detach call") + "py . detach ".len();
    let mut depth = 0usize;
    for (offset, ch) in body[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &body[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced detach call: {body}")
}

fn api(functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".into(),
        types: vec![walker_trait(), walk_options()],
        functions,
        ..Default::default()
    }
}

#[test]
fn sync_options_field_bridge_function_detaches_only_the_core_call() {
    let source = generated(
        &api(vec![
            options_field_function("walk", false, true),
            options_field_function("walk_infallible", false, false),
        ]),
        bridge(BridgeBinding::OptionsField),
    );
    for name in ["walk", "walk_infallible"] {
        let (signature, body) = function(&source, name);
        assert!(
            signature.contains("py : Python < '_ >"),
            "{name}: a sync wrapper needs a GIL token to detach: {signature}"
        );
        assert!(
            body.contains("py . detach (| | sample_core :: walk"),
            "{name}: the core call must run with the GIL released: {body}"
        );
        let detach = body.find("py . detach").unwrap();
        assert!(
            body.find("WalkerBridge :: new").unwrap() < detach,
            "{name}: the host object must be wrapped while the GIL is held: {body}"
        );
        assert!(
            body.find("options_core").unwrap() < detach,
            "{name}: the options must be converted while the GIL is held: {body}"
        );
    }
}

#[test]
fn sync_options_field_bridge_function_converts_errors_after_reattaching() {
    let source = generated(
        &api(vec![options_field_function("walk", false, true)]),
        bridge(BridgeBinding::OptionsField),
    );
    let (_, body) = function(&source, "walk");
    let closure = detached_closure(&body);
    assert!(
        !closure.contains("map_err"),
        "the core error must become a PyErr outside the detached closure: {closure}"
    );
    let call = format!("py . detach ({closure})");
    let after = body.find(&call).unwrap() + call.len();
    assert!(
        body[after..].contains(". map_err"),
        "the detached call's Result must be converted after it returns: {body}"
    );
}

#[test]
fn async_options_field_bridge_function_stays_on_the_future() {
    let source = generated(
        &api(vec![options_field_function("walk_async", true, true)]),
        bridge(BridgeBinding::OptionsField),
    );
    let (_, body) = function(&source, "walk_async");
    assert!(body.contains("future_into_py"), "control: async shape expected: {body}");
    assert!(
        !body.contains("detach"),
        "an async core call already runs off the GIL on the runtime: {body}"
    );
}

#[test]
fn sync_function_param_bridge_function_detaches_only_the_core_call() {
    let source = generated(
        &api(vec![function_param_function("run", false)]),
        bridge(BridgeBinding::FunctionParam),
    );
    let (signature, body) = function(&source, "run");
    assert!(
        signature.contains("py : Python < '_ >"),
        "a sync wrapper needs a GIL token to detach: {signature}"
    );
    assert!(
        body.contains("py . detach (| | sample_core :: run"),
        "the core call must run with the GIL released: {body}"
    );
    let detach = body.find("py . detach").unwrap();
    assert!(
        body.find("WalkerBridge :: new").unwrap() < detach,
        "the host object must be wrapped while the GIL is held: {body}"
    );
    let closure = detached_closure(&body);
    assert!(
        !closure.contains("map_err") && detach < body.rfind("map_err").unwrap(),
        "the core error must become a PyErr outside the detached closure: {body}"
    );
}

#[test]
fn async_function_param_bridge_function_is_not_detached() {
    let source = generated(
        &api(vec![function_param_function("run_async", true)]),
        bridge(BridgeBinding::FunctionParam),
    );
    let (signature, body) = function(&source, "run_async");
    assert!(
        signature.contains("py : Python < 'py >"),
        "control: async shape expected: {signature}"
    );
    assert!(
        !body.contains("detach"),
        "async bridge wrappers must not detach: {body}"
    );
}

#[test]
fn sync_bridge_wrapper_with_many_parameters_allows_too_many_arguments() {
    // The GIL token is one more parameter; past seven, clippy -D warnings would reject the wrapper.
    let mut wide = options_field_function("walk_wide", false, true);
    for idx in 0..5 {
        wide.params.insert(
            1,
            ParamDef {
                name: format!("extra{idx}"),
                ty: TypeRef::String,
                is_ref: true,
                ..Default::default()
            },
        );
    }
    let narrow = options_field_function("walk", false, true);
    let source = generated(&api(vec![wide, narrow]), bridge(BridgeBinding::OptionsField));
    let file = syn::parse_file(&source).expect("generated Rust must parse");
    let allows_too_many = |name: &str| {
        file.items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(item) if item.sig.ident == name => Some(
                    item.attrs
                        .iter()
                        .any(|attr| attr.to_token_stream().to_string().contains("too_many_arguments")),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("generated function missing: {name}"))
    };
    assert!(allows_too_many("walk_wide"), "{source}");
    assert!(!allows_too_many("walk"), "control: a short wrapper needs no allow");
}
