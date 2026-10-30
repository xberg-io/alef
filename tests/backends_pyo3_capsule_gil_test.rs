use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{CapsuleTypeConfig, NewAlefConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use quote::ToTokens;

#[cfg(unix)]
#[path = "support/mise_task.rs"]
mod mise_task;
#[cfg(unix)]
#[path = "backends_pyo3_capsule_gil_test/runtime.rs"]
mod runtime;

#[cfg(unix)]
#[test]
fn generated_capsule_calls_release_the_gil_at_runtime() {
    for enabled in [false, true] {
        runtime::run(&generated(enabled), enabled);
    }
}

fn handle_function(name: &str, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        rust_path: format!("sample_core::{name}"),
        params: vec![ParamDef {
            name: "name".into(),
            ty: TypeRef::String,
            is_ref: true,
            ..Default::default()
        }],
        return_type: TypeRef::Named("Handle".into()),
        error_type: fallible.then(|| "std::io::Error".into()),
        ..Default::default()
    }
}

fn generated(release_gil: bool) -> String {
    generated_api(&capsule_api(), release_gil)
}

fn generated_api(api: &ApiSurface, release_gil: bool) -> String {
    let source = r#"
[workspace]
languages=['python']
[[crates]]
name='sample-core'
sources=[]
[crates.python]
module_name='_sample'
send_sync_types=['Registry']
capsule_types.Handle='capsule_probe.Handle'
"#;
    let mut config = toml::from_str::<NewAlefConfig>(source)
        .unwrap()
        .resolve()
        .unwrap()
        .remove(0);
    let python = config.python.as_mut().unwrap();
    python.release_gil = release_gil;
    python.capsule_types.insert(
        "Parser".into(),
        CapsuleTypeConfig::ConstructFrom {
            python_type: "capsule_probe.Parser".into(),
            construct_from: "Handle".into(),
        },
    );
    Pyo3Backend
        .generate_bindings(api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content
}

fn capsule_api() -> ApiSurface {
    let function = handle_function("get_handle", true);
    let mut parser = handle_function("get_parser", true);
    parser.return_type = TypeRef::Named("Parser".into());
    let mut empty = handle_function("empty_handle", false);
    empty.params.clear();
    ApiSurface {
        crate_name: "sample-core".into(),
        types: vec![
            TypeDef {
                name: "Handle".into(),
                rust_path: "sample_core::Handle".into(),
                is_opaque: true,
                is_return_type: true,
                ..Default::default()
            },
            registry_type(&function),
            TypeDef {
                name: "Parser".into(),
                rust_path: "sample_core::Parser".into(),
                is_opaque: true,
                is_return_type: true,
                ..Default::default()
            },
        ],
        functions: vec![function, handle_function("make_handle", false), parser, empty],
        ..Default::default()
    }
}

fn registry_type(function: &FunctionDef) -> TypeDef {
    let method = MethodDef {
        name: "get_handle".into(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        error_type: function.error_type.clone(),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    TypeDef {
        name: "Registry".into(),
        rust_path: "sample_core::Registry".into(),
        is_opaque: true,
        methods: vec![method],
        ..Default::default()
    }
}

fn function_body(source: &str, name: &str, method: bool) -> String {
    let file = syn::parse_file(source).expect("generated Rust must parse");
    for item in file.items {
        match item {
            syn::Item::Fn(item) if !method && item.sig.ident == name => {
                return item.block.to_token_stream().to_string();
            }
            syn::Item::Impl(item) if method => {
                for member in item.items {
                    if let syn::ImplItem::Fn(member) = member
                        && member.sig.ident == name
                    {
                        return member.block.to_token_stream().to_string();
                    }
                }
            }
            _ => {}
        }
    }
    panic!("generated function missing: {name}, method={method}");
}

#[test]
fn capsule_free_calls_respect_release_gil_before_python_conversion() {
    for enabled in [false, true] {
        let source = generated(enabled);
        for name in ["get_handle", "make_handle", "empty_handle"] {
            let body = function_body(&source, name, false);
            assert_eq!(
                body.contains("py . detach"),
                enabled,
                "{name}, release_gil={enabled}: {body}"
            );
            if enabled {
                assert!(body.find("py . detach").unwrap() < body.find("into_raw").unwrap());
                assert!(body.find("into_raw").unwrap() < body.find("PyCapsule_New").unwrap());
            }
        }
    }
}

#[test]
fn capsule_methods_respect_release_gil_before_python_conversion() {
    for enabled in [false, true] {
        let body = function_body(&generated(enabled), "get_handle", true);
        assert_eq!(body.contains("py . detach"), enabled, "release_gil={enabled}: {body}");
        if enabled {
            assert!(body.find("py . detach").unwrap() < body.find("map_err").unwrap());
            assert!(body.find("map_err").unwrap() < body.find("PyCapsule_New").unwrap());
        }
    }
}

#[test]
fn capsule_factories_delegate_to_the_detached_dependency_wrapper() {
    let source = generated(true);
    let body = function_body(&source, "get_parser", false);
    assert!(body.contains("get_handle (py , name)"), "{body}");
    assert!(
        !body.contains("py . detach"),
        "Python-side factory must remain attached: {body}"
    );
    assert!(function_body(&source, "get_handle", false).contains("py . detach"));
}

#[test]
fn raw_capsule_inputs_remain_on_the_attached_thread() {
    let mut api = capsule_api();
    let params = vec![ParamDef {
        name: "handle".into(),
        ty: TypeRef::Named("Handle".into()),
        ..Default::default()
    }];
    api.functions[0].params = params.clone();
    api.types[1].methods[0].params = params;
    let source = generated_api(&api, true);
    for method in [false, true] {
        let body = function_body(&source, "get_handle", method);
        assert!(body.contains("PyCapsule_GetPointer"), "{body}");
        assert!(
            !body.contains("py . detach"),
            "raw pointer must not cross a detached closure: {body}"
        );
    }
}
