use alef::{
    backends::pyo3::Pyo3Backend,
    core::{backend::Backend, config::NewAlefConfig, ir::*},
};

fn generated(optional: bool, fallible_core: bool) -> String {
    generated_with_generation(optional, fallible_core, false)
}

fn generated_with_generation(optional: bool, fallible_core: bool, with_generation: bool) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
core_import = "sample_core"
error_type = "CoreError"
error_constructor = "sample_core::CoreError::new({msg})"
[[crates.trait_bridges]]
trait_name = "ParserHost"
type_alias = "ParserHost"
param_name = "host"
"#,
    )
    .unwrap();
    let mut config = config.resolve().unwrap();
    let mut api = ApiSurface {
        types: vec![TypeDef {
            name: "ParserHost".into(),
            rust_path: "sample_core::ParserHost".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "descriptor".into(),
                return_type: TypeRef::String,
                error_type: Some("CoreError".into()),
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "register_parser_host".into(),
            rust_path: "sample_core::register_parser_host".into(),
            params: vec![ParamDef {
                name: "host".into(),
                ty: TypeRef::Named("ParserHost".into()),
                core_wrapper: CoreWrapper::Arc,
                optional,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            error_type: fallible_core.then(|| "CoreError".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    if with_generation {
        api.functions[0].params.push(ParamDef {
            name: "expected_generation".into(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            ..Default::default()
        });
    }
    Pyo3Backend
        .generate_bindings(&api, &config.remove(0))
        .unwrap()
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap()
        .content
}

#[test]
fn required_argument_after_required_arc_host_is_not_promoted_to_option() {
    let code = generated_with_generation(false, true, true);
    assert!(code.contains("host: Py<PyAny>, expected_generation: u64"), "{code}");
    assert!(
        code.contains("#[pyo3(signature = (host, expected_generation))]"),
        "{code}"
    );
    assert!(
        code.contains("sample_core::register_parser_host(host, expected_generation)"),
        "{code}"
    );
    assert!(!code.contains("expected_generation: Option<u64>"), "{code}");
}

#[test]
fn arc_trait_parameter_matches_fallible_wrapper_constructor_and_core_type() {
    for optional in [false, true] {
        let code = generated(optional, true);
        assert!(
            code.contains("std::sync::Arc::new(bridge) as std::sync::Arc<dyn sample_core::ParserHost>"),
            "{code}"
        );
        assert!(!code.contains("Mutex::new(bridge)"), "{code}");
        assert!(
            code.contains(if optional {
                "PyParserHostBridge::new(v)?"
            } else {
                "PyParserHostBridge::new(host)?"
            }),
            "{code}"
        );
        assert!(
            !code.contains("core_error_to_py_err"),
            "must not call a converter absent from generated code"
        );
        assert!(code.contains("PyRuntimeError::new_err(e.to_string())"));
        if optional {
            assert!(code.contains("}).transpose()?;"));
        }
    }
}

#[test]
fn infallible_core_still_has_fallible_host_construction() {
    let code = generated(false, false);
    assert!(
        code.contains("pub fn register_parser_host(py: Python<'_>, host: Py<PyAny>) -> PyResult<()>"),
        "{code}"
    );
    assert!(code.contains("Ok(val)"), "{code}");
}
