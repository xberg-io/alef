#[cfg(test)]
mod native_stub_runtime {
    use super::*;

    #[test]
    fn should_forward_instance_methods_and_keep_native_surface() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            let failure = module.getattr("fail").unwrap().call0().unwrap_err();
            assert_eq!(
                failure
                    .value(py)
                    .getattr("vendor")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "sample-vendor"
            );
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("native", &module).unwrap();
            let options = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sample/options.py");
            let package_root = options.parent().unwrap().parent().unwrap().to_str().unwrap();
            let public_script = std::ffi::CString::new(include_str!("public_probe.py")).unwrap();
            py.run(&public_script, Some(&globals), None).unwrap();
            globals
                .get_item("verify_public")
                .unwrap()
                .unwrap()
                .call1((&module, package_root))
                .unwrap();
            let conformance_script = std::ffi::CString::new(include_str!("conformance_probe.py")).unwrap();
            py.run(&conformance_script, Some(&globals), None).unwrap();
            globals
                .get_item("verify_conformance")
                .unwrap()
                .unwrap()
                .call1((module, failure.value(py), package_root))
                .unwrap();
        });
    }
}
