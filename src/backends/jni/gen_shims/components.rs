const JNI_COMPONENT_UNSUPPORTED: &str = "downloadable native components are unsupported by the Android-only JNI backend; v1 supports desktop Linux, macOS, and Windows hosts";

fn emit_unsupported_component_shims(out: &mut String, package: &str, bridge: &str) {
    let load = jni_symbol(package, bridge, "nativeComponentLoad");
    let prefetch = jni_symbol(package, bridge, "nativeComponentPrefetch");
    let status = jni_symbol(package, bridge, "nativeComponentStatus");
    let cache_path = jni_symbol(package, bridge, "nativeComponentCachePath");

    out.push_str(&format!(
        r#"
/// Android JNI does not load downloadable native executable components.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn {load}(
    mut env: EnvUnowned,
    _class: JClass,
    _component: JString,
) {{
    // SAFETY: env is a valid EnvUnowned passed by the JVM for this native call frame.
    let mut guard = unsafe {{ jni::AttachGuard::from_unowned(env.as_raw()) }};
    throw_jni_error(guard.borrow_env_mut(), "{message}");
}}

/// Android JNI does not prefetch downloadable native executable components.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn {prefetch}(
    mut env: EnvUnowned,
    _class: JClass,
    _component: JString,
) -> jstring {{
    // SAFETY: env is a valid EnvUnowned passed by the JVM for this native call frame.
    let mut guard = unsafe {{ jni::AttachGuard::from_unowned(env.as_raw()) }};
    throw_jni_error(guard.borrow_env_mut(), "{message}");
    std::ptr::null_mut()
}}

/// Android JNI does not report status for downloadable native executable components.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn {status}(
    mut env: EnvUnowned,
    _class: JClass,
    _component: JString,
) -> jstring {{
    // SAFETY: env is a valid EnvUnowned passed by the JVM for this native call frame.
    let mut guard = unsafe {{ jni::AttachGuard::from_unowned(env.as_raw()) }};
    throw_jni_error(guard.borrow_env_mut(), "{message}");
    std::ptr::null_mut()
}}

/// Android JNI does not expose cache paths for downloadable native executable components.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn {cache_path}(
    mut env: EnvUnowned,
    _class: JClass,
    _component: JString,
) -> jstring {{
    // SAFETY: env is a valid EnvUnowned passed by the JVM for this native call frame.
    let mut guard = unsafe {{ jni::AttachGuard::from_unowned(env.as_raw()) }};
    throw_jni_error(guard.borrow_env_mut(), "{message}");
    std::ptr::null_mut()
}}
"#,
        message = JNI_COMPONENT_UNSUPPORTED,
    ));
}

#[cfg(test)]
mod component_tests {
    use super::*;

    #[test]
    fn jni_emits_explicit_android_unsupported_component_surface() {
        let mut generated = String::new();
        emit_unsupported_component_shims(&mut generated, "dev.demo", "DemoBridge");
        assert!(generated.contains("Java_dev_demo_DemoBridge_nativeComponentLoad"));
        assert!(generated.contains("Java_dev_demo_DemoBridge_nativeComponentPrefetch"));
        assert!(generated.contains("Java_dev_demo_DemoBridge_nativeComponentStatus"));
        assert!(generated.contains("Java_dev_demo_DemoBridge_nativeComponentCachePath"));
        assert!(generated.contains("Android-only JNI backend"));
    }
}
