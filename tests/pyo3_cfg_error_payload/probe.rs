#[cfg(test)]
mod cfg_payload_runtime {
    use super::*;

    #[test]
    fn should_preserve_unconditional_payload_without_gated_attributes() {
        assert_eq!(sample_core::http_enabled(), cfg!(feature = "http"));
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            let error = module.getattr("fail").unwrap().call0().unwrap_err();
            let value = error.value(py);
            assert_eq!(
                value.getattr("reason").unwrap().extract::<String>().unwrap(),
                "condition"
            );
            assert!(!value.hasattr("status").unwrap());
            assert_eq!(
                value.getattr("args").unwrap().extract::<(String,)>().unwrap(),
                ("blocked".into(),)
            );
        });
    }
}
