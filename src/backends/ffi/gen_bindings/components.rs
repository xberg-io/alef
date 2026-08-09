//! Shared C ABI for the verified downloadable-component manager.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn gen_component_manager(prefix: &str, config: &ResolvedCrateConfig) -> String {
    let runtime = crate::backends::native_components::generate(config, "/components.lock.json");
    format!(
        r#"{runtime}

unsafe fn alef_component_id<'a>(component: *const c_char) -> Result<&'a str, String> {{
    if component.is_null() {{
        return Err("component must not be NULL".to_string());
    }}
    // SAFETY: the caller promises a valid, NUL-terminated C string for the duration of this call.
    unsafe {{ CStr::from_ptr(component) }}
        .to_str()
        .map_err(|error| format!("component is not valid UTF-8: {{error}}"))
}}

fn alef_component_owned_string(value: String) -> Result<*mut c_char, String> {{
    CString::new(value)
        .map(CString::into_raw)
        .map_err(|_| "component result contains an interior NUL byte".to_string())
}}

/// Download, verify, dynamically load, and pin a configured component.
/// Returns 0 on success and -1 on failure. Failure details are available through
/// `{prefix}_last_error_code` and `{prefix}_last_error_context`.
/// # Safety
/// `component` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_component_load(component: *const c_char) -> i32 {{
    clear_last_error();
    let result = (|| {{
        // SAFETY: upheld by this function's caller contract.
        let component = unsafe {{ alef_component_id(component) }}?;
        alef_component_load(component)
    }})();
    match result {{
        Ok(()) => 0,
        Err(error) => {{
            set_last_error(99, &error);
            -1
        }}
    }}
}}

/// Download and verify one component, or all configured components when `component` is NULL.
/// Returns an owned JSON array of cache paths, freed with `{prefix}_free_string`.
/// # Safety
/// A non-null `component` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_component_prefetch(component: *const c_char) -> *mut c_char {{
    clear_last_error();
    let result = (|| {{
        let requested = if component.is_null() {{
            None
        }} else {{
            // SAFETY: upheld by this function's caller contract.
            Some(vec![unsafe {{ alef_component_id(component) }}?.to_string()])
        }};
        let paths = alef_component_prefetch(requested)?;
        let json = serde_json::to_string(&paths).map_err(|error| error.to_string())?;
        alef_component_owned_string(json)
    }})();
    match result {{
        Ok(value) => value,
        Err(error) => {{
            set_last_error(99, &error);
            std::ptr::null_mut()
        }}
    }}
}}

/// Return `missing`, `cached:<path>`, or `loaded:<path>` for a configured component.
/// The returned string is owned and must be freed with `{prefix}_free_string`.
/// # Safety
/// `component` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_component_status(component: *const c_char) -> *mut c_char {{
    clear_last_error();
    let result = (|| {{
        // SAFETY: upheld by this function's caller contract.
        let component = unsafe {{ alef_component_id(component) }}?;
        alef_component_owned_string(alef_component_status(component)?)
    }})();
    match result {{
        Ok(value) => value,
        Err(error) => {{
            set_last_error(99, &error);
            std::ptr::null_mut()
        }}
    }}
}}

/// Return the content-addressed cache path for a configured component.
/// The returned string is owned and must be freed with `{prefix}_free_string`.
/// # Safety
/// `component` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_component_cache_path(component: *const c_char) -> *mut c_char {{
    clear_last_error();
    let result = (|| {{
        // SAFETY: upheld by this function's caller contract.
        let component = unsafe {{ alef_component_id(component) }}?;
        alef_component_owned_string(alef_component_cache_path(component)?)
    }})();
    match result {{
        Ok(value) => value,
        Err(error) => {{
            set_last_error(99, &error);
            std::ptr::null_mut()
        }}
    }}
}}"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    #[test]
    fn emits_shared_c_component_manager_abi() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            ..ResolvedCrateConfig::default()
        };

        let generated = gen_component_manager("demo", &config);
        assert!(generated.contains("fn demo_component_load"));
        assert!(generated.contains("fn demo_component_prefetch"));
        assert!(generated.contains("fn demo_component_status"));
        assert!(generated.contains("fn demo_component_cache_path"));
        assert!(generated.contains("vec![\"fast\"]"));
        assert!(generated.contains("DEMO_CORE_COMPONENT_CACHE"));
        assert!(generated.contains("components.lock.json"));
        assert!(generated.contains("downloadable native components are unsupported"));
    }

    #[test]
    fn cbindgen_emits_documented_component_manager_declarations() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            component_contracts: vec![ComponentContractConfig {
                name: "engine".into(),
                trait_path: "demo_core::Engine".into(),
                interface_version: 1,
            }],
            components: vec![ComponentProfileConfig {
                name: "fast".into(),
                contract: "engine".into(),
                implementation: "demo_core::FastEngine".into(),
                features: vec!["fast".into()],
                default_features: false,
                targets: vec!["x86_64-unknown-linux-gnu".into()],
            }],
            ..ResolvedCrateConfig::default()
        };
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("lib.rs");
        std::fs::write(&source, gen_component_manager("demo", &config)).unwrap();

        let bindings = cbindgen::Builder::new()
            .with_language(cbindgen::Language::C)
            .with_src(source)
            .generate()
            .expect("cbindgen accepts generated component manager");
        let mut header = Vec::new();
        bindings.write(&mut header);
        let header = String::from_utf8(header).unwrap();

        assert!(header.contains("int32_t demo_component_load(const char *component);"));
        assert!(header.contains("char *demo_component_prefetch(const char *component);"));
        assert!(header.contains("char *demo_component_status(const char *component);"));
        assert!(header.contains("char *demo_component_cache_path(const char *component);"));
        assert!(header.contains("freed with `demo_free_string`"));
        assert!(header.contains("when `component` is NULL"));
    }
}
