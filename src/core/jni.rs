//! Shared symbol-naming utilities for JNI emission.
//!
//! Used by both `alef-backend-kotlin` (when `ffi_style = "jni"`) and
//! `alef-backend-jni` so Kotlin Bridge names and Rust `Java_*` symbols never
//! drift.
//!
//! All functions are pure string transformations — no I/O, no config access.

use crate::codegen::naming::to_class_name;
use crate::core::config::ResolvedCrateConfig;

/// Resolve the Kotlin package used for JNI symbols.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then derives a reverse-DNS package from the scaffold repository URL,
/// and finally falls back to `com.example.{clean_name}` derived from the crate
/// name (hyphens and underscores removed, lowercased) so generated JNI symbols
/// are always valid Java identifiers even when no package is configured.
///
/// # Examples
/// ```ignore
/// let package = alef::core::jni::jni_package(&config);
/// assert_eq!(package, "dev.sample_crate");
/// ```
pub fn jni_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .or_else(|| config.try_kotlin_package().ok())
        .unwrap_or_else(|| {
            let clean = config.name.replace(['-', '_'], "").to_lowercase();
            format!("com.example.{clean}")
        })
}

/// `<PascalCrateName>Bridge` — Kotlin `object` containing all `external fun`s.
///
/// # Examples
/// ```
/// assert_eq!(alef::core::jni::bridge_class_name("demo"), "DemoBridge");
/// assert_eq!(alef::core::jni::bridge_class_name("my-lib"), "MyLibBridge");
/// ```
pub fn bridge_class_name(crate_name: &str) -> String {
    format!("{}Bridge", to_class_name(crate_name))
}

/// `<PascalService>ServiceBridge` — the JVM `object`/class hosting a service's
/// `external fun` declarations. Shared by the jni backend (computing `Java_*` symbols via
/// [`jni_symbol`]) and the kotlin backend (emitting the `object`), so the two cannot drift.
/// Distinct from [`bridge_class_name`] (the crate-level regular-bindings bridge) to avoid a
/// name collision with it.
///
/// # Examples
/// ```
/// assert_eq!(alef::core::jni::service_bridge_class_name("App"), "AppServiceBridge");
/// assert_eq!(alef::core::jni::service_bridge_class_name("api_surface"), "ApiSurfaceServiceBridge");
/// ```
pub fn service_bridge_class_name(service_name: &str) -> String {
    format!("{}ServiceBridge", to_class_name(service_name))
}

/// `native<PascalOwner><PascalMethod>` for instance methods; `native<PascalMethod>`
/// for top-level functions (pass `""` for `owner`).
///
/// # Examples
/// ```
/// assert_eq!(alef::core::jni::bridge_method_name("DemoClient", "foo"), "nativeDemoClientFoo");
/// assert_eq!(alef::core::jni::bridge_method_name("", "createClient"), "nativeCreateClient");
/// ```
pub fn bridge_method_name(owner: &str, method: &str) -> String {
    let owner_pascal = to_class_name(owner);
    let method_pascal = to_class_name(method);
    if owner_pascal.is_empty() {
        format!("native{method_pascal}")
    } else {
        format!("native{owner_pascal}{method_pascal}")
    }
}

/// `(nativeStart<Owner><Adapter>, nativeNext<Owner><Adapter>, nativeFree<Owner><Adapter>)`
/// for streaming adapters owned by `owner`.
///
/// # Examples
/// ```
/// let (start, next, free) = alef::core::jni::streaming_method_names("DemoClient", "streamData");
/// assert_eq!(start, "nativeDemoClientStreamDataStart");
/// assert_eq!(next, "nativeDemoClientStreamDataNext");
/// assert_eq!(free, "nativeDemoClientStreamDataFree");
/// ```
pub fn streaming_method_names(owner: &str, method: &str) -> (String, String, String) {
    let owner_pascal = to_class_name(owner);
    let method_pascal = to_class_name(method);
    (
        format!("native{owner_pascal}{method_pascal}Start"),
        format!("native{owner_pascal}{method_pascal}Next"),
        format!("native{owner_pascal}{method_pascal}Free"),
    )
}

/// `nativeFree<Owner>` — destructor method name for an opaque client type.
///
/// # Examples
/// ```
/// assert_eq!(alef::core::jni::destructor_method_name("DemoClient"), "nativeFreeDemoClient");
/// ```
pub fn destructor_method_name(owner: &str) -> String {
    let owner_pascal = to_class_name(owner);
    format!("nativeFree{owner_pascal}")
}

/// JNI symbol per spec §5.11.3: `Java_<package_underscored>_<class>_<method>`.
///
/// `_` in any identifier component becomes `_1`. Package separator `.` becomes
/// `_`. Passing an empty `method` produces `Java_<package>_<class>` (useful for
/// deriving a common prefix).
///
/// # Examples
/// ```
/// let sym = alef::core::jni::jni_symbol("dev.sample_core.demo", "DemoBridge", "nativeFoo");
/// assert_eq!(sym, "Java_dev_sample_1core_demo_DemoBridge_nativeFoo");
/// ```
pub fn jni_symbol(package: &str, class: &str, method: &str) -> String {
    let encode = |s: &str| s.replace('_', "_1").replace('.', "_");
    let pkg_encoded = encode(package);
    let class_encoded = encode(class);
    if method.is_empty() {
        format!("Java_{pkg_encoded}_{class_encoded}")
    } else {
        let method_encoded = encode(method);
        format!("Java_{pkg_encoded}_{class_encoded}_{method_encoded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_class_name_basic() {
        assert_eq!(bridge_class_name("demo"), "DemoBridge");
        assert_eq!(bridge_class_name("my-lib"), "MyLibBridge");
        assert_eq!(bridge_class_name("my_lib"), "MyLibBridge");
    }

    #[test]
    fn bridge_method_name_with_owner() {
        assert_eq!(bridge_method_name("DemoClient", "foo"), "nativeDemoClientFoo");
        assert_eq!(bridge_method_name("demo_client", "bar_baz"), "nativeDemoClientBarBaz");
    }

    #[test]
    fn bridge_method_name_no_owner() {
        assert_eq!(bridge_method_name("", "createClient"), "nativeCreateClient");
        assert_eq!(bridge_method_name("", "create_client"), "nativeCreateClient");
    }

    #[test]
    fn streaming_method_names_basic() {
        let (s, n, f) = streaming_method_names("DemoClient", "streamData");
        assert_eq!(s, "nativeDemoClientStreamDataStart");
        assert_eq!(n, "nativeDemoClientStreamDataNext");
        assert_eq!(f, "nativeDemoClientStreamDataFree");
    }

    #[test]
    fn destructor_method_name_basic() {
        assert_eq!(destructor_method_name("DemoClient"), "nativeFreeDemoClient");
        assert_eq!(destructor_method_name("demo_client"), "nativeFreeDemoClient");
    }

    #[test]
    fn jni_symbol_basic() {
        let sym = jni_symbol("dev.sample_crate.demo", "DemoBridge", "nativeFoo");
        assert_eq!(sym, "Java_dev_sample_1crate_demo_DemoBridge_nativeFoo");
    }

    #[test]
    fn jni_symbol_underscore_in_class_encoded() {
        let sym = jni_symbol("dev.demo", "Demo_Bridge", "nativeBar");
        assert_eq!(sym, "Java_dev_demo_Demo_1Bridge_nativeBar");
    }

    #[test]
    fn jni_symbol_empty_method_gives_prefix() {
        let prefix = jni_symbol("dev.sample_crate.demo", "DemoBridge", "");
        assert_eq!(prefix, "Java_dev_sample_1crate_demo_DemoBridge");
    }

    #[test]
    fn jni_package_prefers_kotlin_android() {
        let config = ResolvedCrateConfig {
            name: "test-lib".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        assert_eq!(jni_package(&config), "com.example.testlib");
    }
}
