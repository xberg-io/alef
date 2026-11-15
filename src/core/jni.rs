//! Shared symbol-naming utilities for JNI emission.
//!
//! Used by both `alef-backend-kotlin` (when `ffi_style = "jni"`) and
//! `alef-backend-jni` so Kotlin Bridge names and Rust `Java_*` symbols never
//! drift.
//!
//! All functions are pure string transformations — no I/O, no config access.

use std::collections::HashSet;

use crate::codegen::naming::to_class_name;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

/// A shared opaque argument crosses JNI as a native handle.
pub(crate) fn borrowed_opaque_param<'a>(param: &'a ParamDef, opaque_names: &HashSet<&str>) -> Option<&'a str> {
    match &param.ty {
        TypeRef::Named(name)
            if param.is_ref && !param.is_mut && !param.optional && opaque_names.contains(name.as_str()) =>
        {
            Some(name)
        }
        _ => None,
    }
}

/// Resolve the Kotlin package used for JNI symbols.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`, then
/// [`ResolvedCrateConfig::kotlin_package`] (which itself derives a reverse-DNS package from the
/// scaffold repository URL before falling back to the vendor-neutral `"unconfigured.alef"`
/// placeholder).
///
/// This is the single implementation both `alef-backend-jni` (the main shim surface, via
/// `emit_lib_rs`) and `alef-backend-kotlin`'s JNI-mode emitter (`ffi_style = "jni"`) call
/// directly, rather than each keeping its own copy of this precedence chain. Before this fix
/// each of the three call sites re-derived the same chain by hand, and this one's final fallback
/// (`com.example.{clean_name}`) had silently drifted from the other two (`"unconfigured.alef"`,
/// via `kotlin_package`) — so an unconfigured crate got a *different* package on the `service.rs`
/// symbols this function fed than on every other symbol the same JNI crate exports. Routing every
/// caller through one function makes that class of drift structurally impossible rather than
/// merely re-synced for today. ~keep
///
/// # Examples
/// ```
/// # use alef::core::config::ResolvedCrateConfig;
/// let config = ResolvedCrateConfig {
///     name: "sample-crate".to_owned(),
///     ..ResolvedCrateConfig::default()
/// };
/// assert_eq!(alef::core::jni::jni_package(&config), "unconfigured.alef");
/// ```
pub fn jni_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
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

/// Names of every type whose instance methods can be bridged as *value* methods.
///
/// A value type is a non-opaque, non-trait struct that the Kotlin side
/// materialises as a `data class`. It has no native handle, so the JNI shim
/// rebuilds the receiver by deserializing the caller-supplied JSON — which is
/// only sound when the core type derives serde.
pub fn value_bridge_serde_type_names(api: &ApiSurface) -> HashSet<&str> {
    let mut names: HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.is_trait && !t.binding_excluded && t.has_serde)
        .map(|t| t.name.as_str())
        .collect();
    names.extend(
        api.enums
            .iter()
            .filter(|e| !e.binding_excluded && e.has_serde)
            .map(|e| e.name.as_str()),
    );
    names
}

/// True when `method` is a `&mut self` method on a value type that returns nothing.
///
/// A Kotlin `data class` is immutable, so an in-place mutation has nowhere to
/// land. The bridge therefore returns the mutated receiver, matching the
/// `is_functional_ref_mut` convention the PyO3, NAPI and WASM backends already
/// apply to the same methods.
pub fn is_functional_ref_mut_value_method(method: &MethodDef) -> bool {
    matches!(method.receiver, Some(ReceiverKind::RefMut))
        && method.trait_source.is_none()
        && matches!(method.return_type, TypeRef::Unit)
}

/// The effective return type of a bridged value method: the owner type for a
/// functional `&mut self` method, otherwise the declared return type.
pub fn value_method_return_type(owner_type_name: &str, method: &MethodDef) -> TypeRef {
    if is_functional_ref_mut_value_method(method) {
        TypeRef::Named(owner_type_name.to_string())
    } else {
        method.return_type.clone()
    }
}

/// True when a type can cross the value-method JNI boundary as JSON.
///
/// `Bytes`/`Vec<u8>` are rejected because Jackson encodes a Kotlin `ByteArray`
/// as base64 while `serde_json` expects a number array, and `Optional` is
/// rejected because the empty-string sentinel used by handle-based shims has no
/// equivalent in the JSON-object request encoding used here.
fn is_json_bridgeable(ty: &TypeRef, serde_type_names: &HashSet<&str>) -> bool {
    match ty {
        TypeRef::String | TypeRef::Primitive(_) | TypeRef::Path => true,
        TypeRef::Named(name) => serde_type_names.contains(name.as_str()),
        TypeRef::Vec(inner) => {
            let is_bytes = matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8));
            !is_bytes && is_json_bridgeable(inner, serde_type_names)
        }
        TypeRef::Map(key, value) => {
            is_json_bridgeable(key, serde_type_names) && is_json_bridgeable(value, serde_type_names)
        }
        _ => false,
    }
}

/// True when `method` on the value type `owner` can be bridged through JNI.
///
/// `serde_type_names` comes from [`value_bridge_serde_type_names`]; `owner` must
/// itself be present in that set because the shim deserializes the receiver.
pub fn value_method_is_bridgeable(owner: &TypeDef, method: &MethodDef, serde_type_names: &HashSet<&str>) -> bool {
    if !serde_type_names.contains(owner.name.as_str()) {
        return false;
    }
    if method.sanitized || method.binding_excluded || method.is_static || method.is_async {
        return false;
    }
    if method.receiver.is_none() || method.trait_source.is_some() {
        return false;
    }
    if !method
        .params
        .iter()
        .all(|param| !param.sanitized && !param.optional && is_json_bridgeable(&param.ty, serde_type_names))
    {
        return false;
    }
    if is_functional_ref_mut_value_method(method) && method.error_type.is_some() {
        // The shim would have to move the receiver out of a fallible call it is
        // still mutably borrowed by; no such method exists, so reject rather
        // than emit code that cannot be proven to borrow-check.
        return false;
    }
    let return_type = value_method_return_type(&owner.name, method);
    // `Path` is accepted as a parameter (the shim rebuilds a `PathBuf` from the
    // JSON string) but not as a return: the JNI return-type tables have no
    // mapping for it.
    matches!(return_type, TypeRef::Unit)
        || (!matches!(return_type, TypeRef::Path) && is_json_bridgeable(&return_type, serde_type_names))
}

/// Every bridgeable instance method on `owner`, in declaration order.
pub fn bridgeable_value_methods<'a>(owner: &'a TypeDef, serde_type_names: &HashSet<&str>) -> Vec<&'a MethodDef> {
    owner
        .methods
        .iter()
        .filter(|method| value_method_is_bridgeable(owner, method, serde_type_names))
        .collect()
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

    /// Regression coverage for the fallback-drift bug this function's doc comment describes:
    /// with nothing configured, `jni_package` must agree with `ResolvedCrateConfig::kotlin_package`
    /// (the vendor-neutral `"unconfigured.alef"` placeholder), not derive its own separate
    /// `com.example.{name}` default. Before this fix this assertion failed —
    /// `jni_package` returned `"com.example.testlib"` while `config.kotlin_package()` (and both
    /// the jni and kotlin backends' own package resolvers) returned `"unconfigured.alef"`, so the
    /// JNI service-API symbols alone carried a different package than every other symbol the same
    /// crate exports.
    #[test]
    fn jni_package_falls_back_to_kotlin_package_placeholder() {
        let config = ResolvedCrateConfig {
            name: "test-lib".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        assert_eq!(jni_package(&config), config.kotlin_package());
        assert_eq!(jni_package(&config), "unconfigured.alef");
    }

    #[test]
    fn jni_package_prefers_kotlin_android_package_over_everything() {
        let config = ResolvedCrateConfig {
            name: "test-lib".to_owned(),
            kotlin_android: Some(crate::core::config::KotlinAndroidConfig {
                package: Some("dev.sample.android".to_owned()),
                ..Default::default()
            }),
            kotlin: Some(crate::core::config::KotlinConfig {
                package: Some("dev.sample.kotlin".to_owned()),
                ..Default::default()
            }),
            ..ResolvedCrateConfig::default()
        };

        assert_eq!(jni_package(&config), "dev.sample.android");
    }

    #[test]
    fn jni_package_prefers_kotlin_package_over_derived_default() {
        let config = ResolvedCrateConfig {
            name: "test-lib".to_owned(),
            kotlin: Some(crate::core::config::KotlinConfig {
                package: Some("dev.sample.kotlin".to_owned()),
                ..Default::default()
            }),
            ..ResolvedCrateConfig::default()
        };

        assert_eq!(jni_package(&config), "dev.sample.kotlin");
    }
}
