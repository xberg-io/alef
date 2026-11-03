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

    #[test]
    fn unrelated_rust_default_retains_host_alias_errors_and_overrides() {
        Python::initialize();
        Python::attach(|py| {
            let globals = pyo3::types::PyDict::new(py);
            let hosts = std::ffi::CString::new(include_str!("hosts.py")).unwrap();
            py.run(&hosts, Some(&globals), None).unwrap();
            let legacy = wrapped(py, &globals, "Legacy");
            assert_eq!(legacy.describe("abc").unwrap(), 3);
            let error = legacy
                .describe("raise")
                .expect_err("the default must retain the fallible host path");
            assert!(error.contains("RuntimeError: host-count-sentinel"), "{error}");
            assert_eq!(wrapped(py, &globals, "Override").describe("abc").unwrap(), 97);
        });
    }

    #[test]
    fn checked_and_delegated_callbacks_preserve_conversion_failures() {
        Python::initialize();
        Python::attach(|py| {
            let globals = pyo3::types::PyDict::new(py);
            let hosts = std::ffi::CString::new(include_str!("hosts.py")).unwrap();
            py.run(&hosts, Some(&globals), None).unwrap();
            for (name, expected) in [("Negative", "OverflowError"), ("NonInteger", "TypeError")] {
                let backend = wrapped(py, &globals, name);
                let direct = backend.checked_count("abc").unwrap_err();
                assert!(direct.contains(expected), "{direct}");
                let delegated = backend.describe("abc").unwrap_err();
                assert!(delegated.contains(expected), "{delegated}");
            }
        });
    }
}
