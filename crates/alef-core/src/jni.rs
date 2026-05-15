//! Shared symbol-naming utilities for JNI emission.
//!
//! Used by both `alef-backend-kotlin` (when `ffi_style = "jni"`) and
//! `alef-backend-jni` so Kotlin Bridge names and Rust `Java_*` symbols never
//! drift.
//!
//! All functions are pure string transformations — no I/O, no config access.

use heck::ToUpperCamelCase;

/// `<PascalCrateName>Bridge` — Kotlin `object` containing all `external fun`s.
///
/// # Examples
/// ```
/// assert_eq!(alef_core::jni::bridge_class_name("demo"), "DemoBridge");
/// assert_eq!(alef_core::jni::bridge_class_name("my-lib"), "MyLibBridge");
/// ```
pub fn bridge_class_name(crate_name: &str) -> String {
    format!("{}Bridge", crate_name.to_upper_camel_case())
}

/// `native<PascalOwner><PascalMethod>` for instance methods; `native<PascalMethod>`
/// for top-level functions (pass `""` for `owner`).
///
/// # Examples
/// ```
/// assert_eq!(alef_core::jni::bridge_method_name("DemoClient", "foo"), "nativeDemoClientFoo");
/// assert_eq!(alef_core::jni::bridge_method_name("", "createClient"), "nativeCreateClient");
/// ```
pub fn bridge_method_name(owner: &str, method: &str) -> String {
    let owner_pascal = owner.to_upper_camel_case();
    let method_pascal = method.to_upper_camel_case();
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
/// let (start, next, free) = alef_core::jni::streaming_method_names("DemoClient", "streamData");
/// assert_eq!(start, "nativeDemoClientStreamDataStart");
/// assert_eq!(next, "nativeDemoClientStreamDataNext");
/// assert_eq!(free, "nativeDemoClientStreamDataFree");
/// ```
pub fn streaming_method_names(owner: &str, method: &str) -> (String, String, String) {
    let owner_pascal = owner.to_upper_camel_case();
    let method_pascal = method.to_upper_camel_case();
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
/// assert_eq!(alef_core::jni::destructor_method_name("DemoClient"), "nativeFreeDemoClient");
/// ```
pub fn destructor_method_name(owner: &str) -> String {
    let owner_pascal = owner.to_upper_camel_case();
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
/// let sym = alef_core::jni::jni_symbol("dev.kreuzberg.demo", "DemoBridge", "nativeFoo");
/// assert_eq!(sym, "Java_dev_kreuzberg_demo_DemoBridge_nativeFoo");
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
        let sym = jni_symbol("dev.kreuzberg.demo", "DemoBridge", "nativeFoo");
        assert_eq!(sym, "Java_dev_kreuzberg_demo_DemoBridge_nativeFoo");
    }

    #[test]
    fn jni_symbol_underscore_in_class_encoded() {
        // JNI spec: underscore in identifier → _1
        let sym = jni_symbol("dev.demo", "Demo_Bridge", "nativeBar");
        assert_eq!(sym, "Java_dev_demo_Demo_1Bridge_nativeBar");
    }

    #[test]
    fn jni_symbol_empty_method_gives_prefix() {
        let prefix = jni_symbol("dev.kreuzberg.demo", "DemoBridge", "");
        assert_eq!(prefix, "Java_dev_kreuzberg_demo_DemoBridge");
    }
}
