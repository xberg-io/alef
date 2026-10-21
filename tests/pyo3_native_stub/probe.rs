#[cfg(test)]
mod native_stub_runtime {
    use super::*;

    #[test]
    fn should_preserve_native_methods_and_public_dataclass_surface() {
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
            globals.set_item("options_path", options.to_str().unwrap()).unwrap();
            let script = std::ffi::CString::new(
                "import importlib.util\nimport sys\nvalue = native.ContentConfig.default()\nassert value.label == 'native-default'\nassert value.validate() is None\nspec = importlib.util.spec_from_file_location('public_options', options_path)\npublic = importlib.util.module_from_spec(spec)\nsys.modules['public_options'] = public\nspec.loader.exec_module(public)\nassert public.ContentConfig(label='public').label == 'public'\nassert not hasattr(public.ContentConfig, 'default')\nassert not hasattr(public.ContentConfig, 'validate')\n",
            ).unwrap();
            py.run(&script, Some(&globals), None).unwrap();
            let conformance_script = std::ffi::CString::new(include_str!("conformance_probe.py")).unwrap();
            py.run(&conformance_script, Some(&globals), None).unwrap();
            globals
                .get_item("verify_conformance")
                .unwrap()
                .unwrap()
                .call1((
                    module,
                    failure.value(py),
                    options.parent().unwrap().parent().unwrap().to_str().unwrap(),
                ))
                .unwrap();
        });
    }
}
