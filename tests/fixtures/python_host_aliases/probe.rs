#[cfg(test)]
mod host_alias_runtime {
    use super::*;
    use sample::Counter;

    fn wrapped(_py: Python<'_>, globals: &Bound<'_, pyo3::types::PyDict>, name: &str) -> PyCounterBridge {
        let object = globals.get_item(name).unwrap().unwrap().call0().unwrap().unbind();
        PyCounterBridge::new(object).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    #[test]
    fn legacy_host_errors_and_explicit_overrides_reach_checked_callbacks() {
        Python::initialize();
        Python::attach(|py| {
            let globals = pyo3::types::PyDict::new(py);
            let hosts = std::ffi::CString::new(include_str!("hosts.py")).unwrap();
            py.run(&hosts, Some(&globals), None).unwrap();
            let legacy = wrapped(py, &globals, "Legacy");
            assert_eq!(legacy.checked_count("abc").unwrap(), 3);
            let error = legacy.checked_count("raise").unwrap_err();
            assert!(error.contains("RuntimeError: host-count-sentinel"), "{error}");
            assert_eq!(wrapped(py, &globals, "Override").checked_count("abc").unwrap(), 97);
            for (name, sentinel) in [
                ("DescriptorFailure", "descriptor-sentinel"),
                ("AttributeDescriptorFailure", "attribute-descriptor-sentinel"),
            ] {
                let error = wrapped(py, &globals, name).checked_count("abc").unwrap_err();
                assert!(error.contains(sentinel), "{error}");
            }
            let missing = globals.get_item("Missing").unwrap().unwrap().call0().unwrap().unbind();
            assert!(PyCounterBridge::new(missing).is_err());
        });
    }
}
