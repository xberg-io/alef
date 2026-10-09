use super::*;

#[test]
fn async_python_functions_raise_declared_exception_variants() {
    let converters = vec!["my_error_to_py_err".to_owned()];
    for has_serde in [false, true] {
        for return_type in [TypeRef::String, TypeRef::Unit] {
            let mut func = simple_function_def();
            func.is_async = true;
            func.error_type = Some("my_crate::MyError".to_owned());
            func.return_type = return_type;
            let mut cfg = default_cfg();
            cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
            cfg.has_serde = has_serde;
            cfg.error_converters = Some(&converters);
            let output = gen_function(&func, &RustMapper, &cfg, &AdapterBodies::default(), &AHashSet::new());
            assert!(output.contains(".map_err(my_error_to_py_err)"), "{output}");
            assert!(!output.contains("PyRuntimeError"), "{output}");
        }
    }
}

#[test]
fn async_python_functions_keep_generic_errors_for_unregistered_types() {
    let mut func = simple_function_def();
    func.is_async = true;
    func.error_type = Some("ExternalError".to_owned());
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
    let output = gen_function(&func, &RustMapper, &cfg, &AdapterBodies::default(), &AHashSet::new());
    assert!(output.contains("PyRuntimeError"), "{output}");
    assert!(!output.contains("external_error_to_py_err"), "{output}");
}

#[test]
fn async_python_instance_and_static_methods_preserve_declared_errors() {
    let converters = vec!["my_error_to_py_err".to_owned()];
    for has_serde in [false, true] {
        for return_type in [TypeRef::String, TypeRef::Unit] {
            let method = MethodDef {
                name: "fetch".into(),
                is_async: true,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("my_crate::MyError".into()),
                return_type,
                ..Default::default()
            };
            let mut cfg = default_cfg();
            cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
            cfg.has_serde = has_serde;
            cfg.error_converters = Some(&converters);
            let typ = simple_type_def();
            let opaque = AHashSet::from_iter([typ.name.clone()]);
            let instance = gen_method(
                &method,
                &RustMapper,
                &cfg,
                &typ,
                true,
                &opaque,
                &AHashSet::new(),
                &AdapterBodies::default(),
            );
            let static_method = gen_static_method(
                &method,
                &RustMapper,
                &cfg,
                &typ,
                &AdapterBodies::default(),
                &opaque,
                &AHashSet::new(),
            );
            for output in [instance, static_method] {
                assert!(output.contains(".map_err(my_error_to_py_err)"), "{output}");
                assert!(!output.contains("PyRuntimeError"), "{output}");
            }
        }
    }
}

#[test]
fn async_python_serde_parameters_preserve_declared_errors_after_conversion() {
    let converters = vec!["my_error_to_py_err".to_owned()];
    let mut func = simple_function_def();
    func.is_async = true;
    func.error_type = Some("my_crate::MyError".into());
    func.params = vec![ParamDef {
        name: "config".into(),
        ty: TypeRef::Named("Config".into()),
        ..Default::default()
    }];
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
    cfg.has_serde = true;
    cfg.error_converters = Some(&converters);
    let output = gen_function(&func, &RustMapper, &cfg, &AdapterBodies::default(), &AHashSet::new());
    assert!(
        output.contains("config_core: my_crate::Config"),
        "conversion branch: {output}"
    );
    assert!(output.contains(".map_err(my_error_to_py_err)"), "{output}");
}
