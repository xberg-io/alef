//! `Backend::trait_bridge_registration_surface` must report the registration API the backend
//! actually generates.
//!
//! The reference-doc renderer (`docs::language_pages::trait_bridge_render`) asks the backend
//! rather than re-deriving naming itself, so the whole mechanism is only worth anything if the
//! answer tracks the emitter. Every test here therefore pairs two assertions:
//!
//! 1. the reported symbols equal an exact expected string, and
//! 2. the generated output declares those same symbols.
//!
//! Assertion 2 is the one that catches drift: rename the emitted entry point without updating
//! the surface and the declaration lookup stops matching. Asserting only that the surface is
//! non-empty would prove nothing, so no test does that.
//!
//! The fixture deliberately configures `register_fn` / `unregister_fn` / `clear_fn` values that
//! do *not* match what a trait-name-derived scheme would produce (`install_sample_plugin`, not
//! `register_sample_plugin`). Backends split into two camps — those that publish the configured
//! name and those that synthesise one from the trait — and a fixture where the two coincide
//! could not tell them apart.
//!
//! This file covers the C-ABI family and the backends whose registration hangs off a class;
//! `backends_trait_bridge_registration_scripting_test.rs` covers the scripting targets.

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::FfiBackend;
use alef::backends::go::GoBackend;
use alef::backends::java::JavaBackend;
use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::{Backend, TraitBridgeRegistrationSurface};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// The bridged trait. Chosen so its snake_case form (`sample_plugin`) is visibly different from
/// every configured function name below.
const TRAIT: &str = "SamplePlugin";

/// Configured registration names. None of the three is what a trait-derived scheme produces.
const REGISTER_FN: &str = "install_sample_plugin";
const UNREGISTER_FN: &str = "remove_sample_plugin";
const CLEAR_FN: &str = "clear_sample_plugins";

fn plugin_trait() -> TypeDef {
    TypeDef {
        name: TRAIT.to_owned(),
        rust_path: format!("sample_core::{TRAIT}"),
        is_trait: true,
        methods: vec![MethodDef {
            name: "handle".to_owned(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            error_type: Some("Error".to_owned()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn plugin_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![plugin_trait()],
        ..Default::default()
    }
}

fn plugin_bridge() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: TRAIT.to_owned(),
        registry_getter: Some("sample_core::plugins::registry::get_sample_plugin_registry".to_owned()),
        register_fn: Some(REGISTER_FN.to_owned()),
        unregister_fn: Some(UNREGISTER_FN.to_owned()),
        clear_fn: Some(CLEAR_FN.to_owned()),
        ..Default::default()
    }
}

fn config_with_bridge(toml: &str) -> ResolvedCrateConfig {
    let parsed: NewAlefConfig = toml::from_str(toml).expect("fixture config must parse");
    let mut config = parsed.resolve().expect("fixture config must resolve").remove(0);
    config.replace_trait_bridges(vec![plugin_bridge()]);
    config
}

/// The single reported surface, with a failure message that names the backend.
fn only_surface(backend: &dyn Backend, config: &ResolvedCrateConfig) -> TraitBridgeRegistrationSurface {
    let mut surfaces = backend.trait_bridge_registration_surface(&plugin_api(), config);
    assert_eq!(
        surfaces.len(),
        1,
        "{}: one configured trait bridge must report exactly one surface, got {surfaces:?}",
        backend.name()
    );
    surfaces.remove(0)
}

/// Every generated file's content, concatenated, so a declaration lookup does not depend on which
/// file the emitter chose to put it in.
fn generated_text(backend: &dyn Backend, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_bindings(&plugin_api(), config)
        .unwrap_or_else(|error| panic!("{}: generation failed: {error}", backend.name()))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_declares(backend: &dyn Backend, generated: &str, declaration: &str) {
    assert!(
        generated.contains(declaration),
        "{}: the reported registration surface names a symbol the generated output does not \
         declare -- expected to find `{declaration}`",
        backend.name()
    );
}

#[test]
fn ffi_surface_names_the_prefixed_c_symbols_the_extern_items_export() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let surface = only_surface(&FfiBackend, &config);

    // Register carries the configured name; unregister and clear are synthesised from the trait.
    assert_eq!(surface.trait_name, TRAIT);
    assert_eq!(surface.register_symbol.as_deref(), Some("sample_install_sample_plugin"));
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("sample_unregister_sample_plugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("sample_clear_sample_plugin"));

    let generated = generated_text(&FfiBackend, &config);
    for symbol in [
        "sample_install_sample_plugin",
        "sample_unregister_sample_plugin",
        "sample_clear_sample_plugin",
    ] {
        assert_declares(&FfiBackend, &generated, &format!("extern \"C\" fn {symbol}("));
    }
}

#[test]
fn csharp_surface_names_the_static_registry_class_methods() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "csharp"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.csharp]
namespace = "Sample.Core"
"#,
    );
    let surface = only_surface(&CsharpBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SamplePluginRegistry.RegisterSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SamplePluginRegistry.Unregister")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SamplePluginRegistry.Clear"));

    let generated = generated_text(&CsharpBackend, &config);
    assert_declares(&CsharpBackend, &generated, "public static class SamplePluginRegistry {");
    assert_declares(&CsharpBackend, &generated, "public static IntPtr RegisterSamplePlugin(");
    assert_declares(&CsharpBackend, &generated, "public static void Unregister(string name)");
    assert_declares(&CsharpBackend, &generated, "public static void Clear()");
}

#[test]
fn go_surface_names_the_trait_derived_package_functions() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.go]
module = "github.com/sample/sample-core"
"#,
    );
    let surface = only_surface(&GoBackend, &config);

    // Go names register/unregister after the trait and only the clear wrapper after its config.
    assert_eq!(surface.register_symbol.as_deref(), Some("RegisterSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("UnregisterSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("ClearSamplePlugins"));

    let generated = generated_text(&GoBackend, &config);
    assert_declares(&GoBackend, &generated, "func RegisterSamplePlugin(");
    assert_declares(
        &GoBackend,
        &generated,
        "func UnregisterSamplePlugin(name string) error {",
    );
    assert_declares(&GoBackend, &generated, "func ClearSamplePlugins(");
}

#[test]
fn java_surface_names_the_bridge_class_static_methods() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "java"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "io.sample.core"
"#,
    );
    let surface = only_surface(&JavaBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SamplePluginBridge.registerSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SamplePluginBridge.unregisterSamplePlugin")
    );
    // Clear is the one Java method whose name comes from its configured value.
    assert_eq!(
        surface.clear_symbol.as_deref(),
        Some("SamplePluginBridge.clearSamplePlugins")
    );

    let generated = generated_text(&JavaBackend, &config);
    assert_declares(&JavaBackend, &generated, "public static void registerSamplePlugin(");
    assert_declares(&JavaBackend, &generated, "public static void unregisterSamplePlugin(");
    assert_declares(&JavaBackend, &generated, "public static void clearSamplePlugins()");
}

#[test]
fn java_emits_no_bridge_when_the_target_is_excluded() {
    let mut config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "java"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.java]
package = "io.sample.core"
"#,
    );
    config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec!["java".to_owned()]);

    let surfaces = JavaBackend.trait_bridge_registration_surface(&plugin_api(), &config);
    let generated = generated_text(&JavaBackend, &config);

    assert!(surfaces.is_empty(), "an excluded Java bridge must report no surface");
    assert!(
        !generated.contains("SamplePluginBridge"),
        "an excluded Java bridge must emit no bridge class references"
    );
}

#[test]
fn kotlin_android_surface_names_the_fixed_bridge_object_methods() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "kotlin_android", "jni"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.kotlin_android]
package = "io.sample.core"

[crates.jni]
"#,
    );
    let surface = only_surface(&KotlinAndroidBackend, &config);

    // None of the three names is derived from the configured values.
    assert_eq!(surface.register_symbol.as_deref(), Some("SamplePluginBridge.register"));
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SamplePluginBridge.unregister")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SamplePluginBridge.clearAll"));

    let generated = generated_text(&KotlinAndroidBackend, &config);
    assert_declares(&KotlinAndroidBackend, &generated, "object SamplePluginBridge {");
    assert_declares(&KotlinAndroidBackend, &generated, "    fun register(");
    assert_declares(&KotlinAndroidBackend, &generated, "    fun unregister(name: String)");
    assert_declares(&KotlinAndroidBackend, &generated, "    fun clearAll()");
}

#[test]
fn kotlin_android_reports_nothing_when_the_bridge_excludes_that_language() {
    let mut config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi", "kotlin_android", "jni"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.kotlin_android]
package = "io.sample.core"

[crates.jni]
"#,
    );
    config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec!["kotlin_android".to_owned()]);

    let surfaces = KotlinAndroidBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "an excluded bridge emits no bridge object, so it must document none; got {surfaces:?}"
    );
}

#[test]
fn ffi_reports_nothing_when_the_trait_is_absent_from_the_api_surface() {
    let config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api_without_trait = ApiSurface {
        crate_name: "sample-core".to_owned(),
        version: "0.1.0".to_owned(),
        ..Default::default()
    };

    let surfaces = FfiBackend.trait_bridge_registration_surface(&api_without_trait, &config);

    assert!(
        surfaces.is_empty(),
        "the emitter skips a bridge whose trait does not resolve, so no C symbol exists to \
         document; got {surfaces:?}"
    );
}

#[test]
fn ffi_reports_nothing_when_the_bridge_configures_no_register_fn() {
    let mut config = config_with_bridge(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    config.update_trait_bridge(0, |bridge| bridge.register_fn = None);

    let surfaces = FfiBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "the whole extern block is gated on register_fn, so unregister and clear go with it; \
         got {surfaces:?}"
    );
}
