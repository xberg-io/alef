//! `Backend::trait_bridge_registration_surface` for the scripting and dynamic-language targets.
//!
//! Same contract as `backends_trait_bridge_registration_surface_test.rs`: each test asserts the
//! exact reported symbols *and* that the generated output declares them, so renaming an emitted
//! entry point without updating the surface fails here.
//!
//! The configured names are deliberately not what a trait-derived scheme would produce
//! (`install_sample_plugin`, not `register_sample_plugin`), which is what makes Gleam's
//! trait-derived register name distinguishable from the verbatim-name backends.

use alef::backends::dart::DartBackend;
use alef::backends::extendr::ExtendrBackend;
use alef::backends::gleam::GleamBackend;
use alef::backends::jni::JniBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::backends::magnus::MagnusBackend;
use alef::backends::napi::NapiBackend;
use alef::backends::php::PhpBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::backends::rustler::RustlerBackend;
use alef::backends::swift::SwiftBackend;
use alef::backends::wasm::WasmBackend;
use alef::core::backend::{Backend, TraitBridgeRegistrationSurface};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

const TRAIT: &str = "SamplePlugin";
const REGISTER_FN: &str = "install_sample_plugin";
const UNREGISTER_FN: &str = "remove_sample_plugin";
const CLEAR_FN: &str = "clear_sample_plugins";

fn plugin_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![TypeDef {
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
        }],
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

fn generated_text(backend: &dyn Backend, config: &ResolvedCrateConfig) -> String {
    generated_text_for(backend, &plugin_api(), config)
}

fn generated_text_for(backend: &dyn Backend, api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_bindings(api, config)
        .unwrap_or_else(|error| panic!("{}: generation failed: {error}", backend.name()))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// PHP and Elixir emit their consumer-facing wrapper from `generate_public_api`, not from
/// `generate_bindings`, so the declaration lookup has to read that output instead.
fn generated_public_api_text(backend: &dyn Backend, config: &ResolvedCrateConfig) -> String {
    generated_public_api_text_for(backend, &plugin_api(), config)
}

fn generated_public_api_text_for(backend: &dyn Backend, api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    backend
        .generate_public_api(api, config)
        .unwrap_or_else(|error| panic!("{}: public API generation failed: {error}", backend.name()))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// An API surface with no trait at all — the shape a consumer gets when the bridged trait is
/// excluded from the binding surface or renamed out from under `alef.toml`. ~keep
fn api_without_the_trait() -> ApiSurface {
    ApiSurface {
        crate_name: "sample-core".to_owned(),
        version: "0.1.0".to_owned(),
        ..Default::default()
    }
}

fn assert_declares(backend: &dyn Backend, generated: &str, declaration: &str) {
    assert!(
        generated.contains(declaration),
        "{}: the reported registration surface names a symbol the generated output does not \
         declare -- expected to find `{declaration}`",
        backend.name()
    );
}

fn minimal_config(language: &str, extra: &str) -> ResolvedCrateConfig {
    config_with_bridge(&format!(
        "[workspace]\nlanguages = [\"{language}\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.{language}]\n{extra}"
    ))
}

#[test]
fn gleam_surface_names_the_trait_derived_register_shim_and_the_verbatim_rest() {
    let config = minimal_config("gleam", "");
    let surface = only_surface(&GleamBackend, &config);

    // Gleam derives only `register_*` from the trait; the configured `install_sample_plugin`
    // names the Erlang NIF the shim binds to, never the Gleam function.
    assert_eq!(surface.register_symbol.as_deref(), Some("register_sample_plugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&GleamBackend, &config);
    assert_declares(&GleamBackend, &generated, "pub fn register_sample_plugin(pid: Dynamic");
    assert_declares(&GleamBackend, &generated, "pub fn remove_sample_plugin(name: String)");
    assert_declares(&GleamBackend, &generated, "pub fn clear_sample_plugins()");
}

#[test]
fn dart_surface_names_the_lower_camel_methods_on_the_bridge_class() {
    let config = minimal_config("dart", "");
    let surface = only_surface(&DartBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCoreBridge.installSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCoreBridge.removeSamplePlugin")
    );
    assert_eq!(
        surface.clear_symbol.as_deref(),
        Some("SampleCoreBridge.clearSamplePlugins")
    );

    let generated = generated_text(&DartBackend, &config);
    assert_declares(&DartBackend, &generated, "class SampleCoreBridge");
    for method in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&DartBackend, &generated, method);
    }
}

#[test]
fn swift_surface_names_the_top_level_forwarder_functions() {
    let config = minimal_config("swift", "");
    let surface = only_surface(&SwiftBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&SwiftBackend, &config);
    assert_declares(&SwiftBackend, &generated, "public func installSamplePlugin(");
    assert_declares(&SwiftBackend, &generated, "public func removeSamplePlugin(");
    assert_declares(&SwiftBackend, &generated, "public func clearSamplePlugins(");
}

#[test]
fn swift_reports_nothing_for_an_options_field_bridge() {
    let mut config = minimal_config("swift", "");
    config.update_trait_bridge(0, |bridge| {
        bridge.bind_via = alef::core::config::BridgeBinding::OptionsField
    });
    config.update_trait_bridge(0, |bridge| bridge.options_type = Some("SampleOptions".to_owned()));
    config.update_trait_bridge(0, |bridge| bridge.options_field = Some("plugin".to_owned()));

    let surfaces = SwiftBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "an options_field bridge hands the host a handle instead of a registry, and the \
         forwarder emitter skips it; got {surfaces:?}"
    );
}

#[test]
fn napi_surface_names_the_camel_case_module_exports() {
    let config = minimal_config("node", "");
    let surface = only_surface(&NapiBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&NapiBackend, &config);
    // The Rust item keeps the configured name; napi-rs exports it under the camel form.
    assert_declares(&NapiBackend, &generated, &format!("pub fn {REGISTER_FN}("));
    assert_declares(&NapiBackend, &generated, "js_name = \"removeSamplePlugin\"");
    assert_declares(&NapiBackend, &generated, "js_name = \"clearSamplePlugins\"");
}

#[test]
fn napi_reports_no_register_symbol_without_a_registry_getter() {
    let mut config = minimal_config("node", "");
    config.update_trait_bridge(0, |bridge| bridge.registry_getter = None);

    let surface = only_surface(&NapiBackend, &config);

    assert_eq!(
        surface.register_symbol, None,
        "gen_registration_fn emits nothing without a registry_getter, so no JS export exists"
    );
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
}

#[test]
fn napi_emits_no_bridge_and_reports_no_surface_when_the_target_is_excluded() {
    for excluded in ["node", "napi"] {
        let mut config = minimal_config("node", "");
        config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec![excluded.to_owned()]);

        let surfaces = NapiBackend.trait_bridge_registration_surface(&plugin_api(), &config);
        let generated = generated_text(&NapiBackend, &config);

        assert_eq!(
            surfaces.len(),
            0,
            "`exclude_languages = [\"{excluded}\"]` suppresses the `#[napi]` items, so nothing is \
             left to document; got {surfaces:?}"
        );
        assert!(
            !generated.contains(REGISTER_FN),
            "`exclude_languages = [\"{excluded}\"]` must suppress the registration item too"
        );
        assert!(
            !generated.contains("JsSamplePluginBridge"),
            "`exclude_languages = [\"{excluded}\"]` must suppress the bridge wrapper struct"
        );
    }
}

#[test]
fn wasm_surface_names_the_js_names_stamped_on_the_wasm_bindgen_exports() {
    let config = minimal_config("wasm", "");
    let surface = only_surface(&WasmBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some("installSamplePlugin"));
    assert_eq!(surface.unregister_symbol.as_deref(), Some("removeSamplePlugin"));
    assert_eq!(surface.clear_symbol.as_deref(), Some("clearSamplePlugins"));

    let generated = generated_text(&WasmBackend, &config);
    for js_name in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&WasmBackend, &generated, &format!("js_name = \"{js_name}\""));
    }
}

#[test]
fn wasm_emits_no_bridge_and_reports_no_surface_when_the_target_is_excluded() {
    let mut config = minimal_config("wasm", "");
    config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec!["wasm".to_owned()]);

    let surfaces = WasmBackend.trait_bridge_registration_surface(&plugin_api(), &config);
    let generated = generated_text(&WasmBackend, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "`exclude_languages = [\"wasm\"]` suppresses the `#[wasm_bindgen]` items, so nothing is \
         left to document; got {surfaces:?}"
    );
    for js_name in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert!(
            !generated.contains(js_name),
            "`exclude_languages = [\"wasm\"]` must suppress the `{js_name}` export too"
        );
    }
    assert!(
        !generated.contains("WasmSamplePluginBridge"),
        "`exclude_languages = [\"wasm\"]` must suppress the bridge wrapper struct"
    );
}

#[test]
fn magnus_surface_names_the_module_functions_bound_under_the_configured_names() {
    let config = minimal_config("ruby", "");
    let surface = only_surface(&MagnusBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore.install_sample_plugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore.remove_sample_plugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore.clear_sample_plugins"));

    let generated = generated_text(&MagnusBackend, &config);
    assert_declares(&MagnusBackend, &generated, "define_module(\"SampleCore\")");
    for ruby_name in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert_declares(
            &MagnusBackend,
            &generated,
            &format!("define_module_function(\"{ruby_name}\""),
        );
    }
}

#[test]
fn magnus_binds_no_module_function_when_the_bridged_trait_is_absent_from_the_api_surface() {
    let config = minimal_config("ruby", "");
    let api = api_without_the_trait();

    let surfaces = MagnusBackend.trait_bridge_registration_surface(&api, &config);
    let generated = generated_text_for(&MagnusBackend, &api, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "no trait means `gen_trait_bridge` never ran, so there is no registration API; \
         got {surfaces:?}"
    );
    for ruby_name in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert!(
            !generated.contains(&format!("define_module_function(\"{ruby_name}\"")),
            "`ruby_init` would bind `{ruby_name}` to a `pub fn` no pass emitted"
        );
    }
}

#[test]
fn php_surface_names_the_static_methods_on_the_public_wrapper_class() {
    let config = minimal_config("php", "");
    let surface = only_surface(&PhpBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore::installSamplePlugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore::removeSamplePlugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore::clearSamplePlugins"));

    let generated = generated_public_api_text(&PhpBackend, &config);
    assert_declares(&PhpBackend, &generated, "class SampleCore");
    for method in ["installSamplePlugin", "removeSamplePlugin", "clearSamplePlugins"] {
        assert_declares(&PhpBackend, &generated, &format!("function {method}("));
    }
}

#[test]
fn php_emits_no_wrapper_and_reports_no_surface_when_the_target_is_excluded() {
    let mut config = minimal_config("php", "");
    config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec!["php".to_owned()]);

    let surfaces = PhpBackend.trait_bridge_registration_surface(&plugin_api(), &config);
    let public_api = generated_public_api_text(&PhpBackend, &config);
    let bindings = generated_text(&PhpBackend, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "`exclude_languages = [\"php\"]` suppresses the wrapper methods, so nothing is left to \
         document; got {surfaces:?}"
    );
    assert!(
        !public_api.contains("installSamplePlugin"),
        "`exclude_languages = [\"php\"]` must suppress the public wrapper method"
    );
    assert!(
        !bindings.contains(REGISTER_FN),
        "`exclude_languages = [\"php\"]` must suppress the `…Api` extension method too"
    );
}

#[test]
fn php_emits_no_wrapper_when_the_bridged_trait_is_absent_from_the_api_surface() {
    let config = minimal_config("php", "");
    let api = api_without_the_trait();

    let surfaces = PhpBackend.trait_bridge_registration_surface(&api, &config);
    let public_api = generated_public_api_text_for(&PhpBackend, &api, &config);
    let bindings = generated_text_for(&PhpBackend, &api, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "no trait means `gen_trait_bridge` never ran, so there is no registration API; \
         got {surfaces:?}"
    );
    assert!(
        !public_api.contains("installSamplePlugin"),
        "the public wrapper would call `SampleCoreApi::installSamplePlugin`, which no pass emitted"
    );
    assert!(
        !bindings.contains(REGISTER_FN),
        "the `…Api` extension method would forward to `crate::{REGISTER_FN}`, which no pass emitted"
    );
}

#[test]
fn rustler_surface_names_the_elixir_delegates_on_the_app_module() {
    let config = minimal_config("elixir", "");
    let surface = only_surface(&RustlerBackend, &config);

    assert_eq!(
        surface.register_symbol.as_deref(),
        Some("SampleCore.install_sample_plugin")
    );
    assert_eq!(
        surface.unregister_symbol.as_deref(),
        Some("SampleCore.remove_sample_plugin")
    );
    assert_eq!(surface.clear_symbol.as_deref(), Some("SampleCore.clear_sample_plugins"));

    let generated = generated_public_api_text(&RustlerBackend, &config);
    assert_declares(&RustlerBackend, &generated, "defmodule SampleCore do");
    for func in [REGISTER_FN, UNREGISTER_FN] {
        assert_declares(&RustlerBackend, &generated, &format!("def {func}("));
    }
    // The clear delegate takes no arguments, and Elixir spells a zero-arity `def` without
    // parentheses -- `def clear_sample_plugins do`. ~keep
    assert_declares(&RustlerBackend, &generated, &format!("def {CLEAR_FN} do"));
}

#[test]
fn rustler_reports_nothing_when_the_bridge_excludes_either_spelling_of_the_target() {
    for excluded in ["elixir", "rustler"] {
        let mut config = minimal_config("elixir", "");
        config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec![excluded.to_owned()]);

        let surfaces = RustlerBackend.trait_bridge_registration_surface(&plugin_api(), &config);
        let bindings = generated_text(&RustlerBackend, &config);
        let public_api = generated_public_api_text(&RustlerBackend, &config);

        assert!(
            surfaces.is_empty(),
            "`exclude_languages = [\"{excluded}\"]` suppresses the Elixir delegates, so nothing \
             is left to document; got {surfaces:?}"
        );
        for symbol in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
            assert!(
                !bindings.contains(symbol) && !public_api.contains(symbol),
                "`exclude_languages = [\"{excluded}\"]` leaked `{symbol}` into Rustler output"
            );
        }
        assert!(
            !bindings.contains("visitor_reply"),
            "an excluded-only bridge must not enable Rustler visitor NIF scaffolding"
        );
    }
}

#[test]
fn rustler_emits_no_delegate_when_the_bridged_trait_is_absent_from_the_api_surface() {
    let config = minimal_config("elixir", "");
    let api = api_without_the_trait();

    let surfaces = RustlerBackend.trait_bridge_registration_surface(&api, &config);
    let public_api = generated_public_api_text_for(&RustlerBackend, &api, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "no trait means `native::gen_trait_bridge` never ran, so there is no NIF to delegate to; \
         got {surfaces:?}"
    );
    for func in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert!(
            !public_api.contains(&format!("def {func}")),
            "the `{func}` delegate would call `SampleCore.Native.{func}`, a NIF no pass emitted:\n{public_api}"
        );
    }
}

#[test]
fn kotlin_jvm_reports_no_registration_surface_because_it_emits_none() {
    let config = minimal_config("kotlin", "package = \"io.sample.core\"");

    let surfaces = KotlinBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "`generate_jvm` emits no register/unregister/clear function of its own -- a Kotlin/JVM \
         consumer calls the generated Java bridge class directly -- so there is no Kotlin symbol \
         to document; got {surfaces:?}"
    );
}

#[test]
fn jni_reports_no_registration_surface_because_its_shims_are_not_a_host_api() {
    // `jni` is not standalone: config resolution rejects it unless `kotlin_android` is also
    // enabled, because the shims it emits exist for that target to link against. ~keep
    let config = config_with_bridge(
        "[workspace]\nlanguages = [\"jni\", \"kotlin_android\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.jni]\n",
    );

    let surfaces = JniBackend.trait_bridge_registration_surface(&plugin_api(), &config);

    assert!(
        surfaces.is_empty(),
        "the JNI backend emits `Java_..._nativeRegister*` ABI shims that the Kotlin/Java side \
         links against, not an API a consumer calls; got {surfaces:?}"
    );
}

#[test]
fn extendr_surface_names_the_verbatim_r_functions() {
    let config = minimal_config("r", "");
    let surface = only_surface(&ExtendrBackend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some(REGISTER_FN));
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&ExtendrBackend, &config);
    for r_fn in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
        assert_declares(&ExtendrBackend, &generated, &format!("pub fn {r_fn}("));
    }
}

#[test]
fn extendr_reports_no_register_symbol_without_a_registry_getter() {
    let mut config = minimal_config("r", "");
    config.update_trait_bridge(0, |bridge| bridge.registry_getter = None);

    let surface = only_surface(&ExtendrBackend, &config);

    assert_eq!(
        surface.register_symbol, None,
        "gen_registration_fn emits nothing without a registry_getter, so R receives no such \
         function"
    );
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));
}

#[test]
fn extendr_module_macro_and_surface_agree_about_the_register_function() {
    // `extendr_module!` naming a `fn` no `#[extendr]` item defines is a Rust compile error, so
    // the macro entry and the reported symbol have to appear and disappear together. ~keep
    let getter = "sample_core::plugins::registry::get_sample_plugin_registry";
    for (registry_getter, expected) in [(Some(getter.to_owned()), Some(REGISTER_FN)), (None, None)] {
        let mut config = minimal_config("r", "");
        config.update_trait_bridge(0, |bridge| bridge.registry_getter = registry_getter.clone());

        let surface = only_surface(&ExtendrBackend, &config);
        let generated = generated_text(&ExtendrBackend, &config);

        assert_eq!(surface.register_symbol.as_deref(), expected);
        assert_eq!(
            generated.contains(&format!("    fn {REGISTER_FN};")),
            expected.is_some(),
            "the `extendr_module!` entry must track the reported surface \
             (registry_getter = {registry_getter:?})"
        );
        assert_eq!(
            generated.contains(&format!("pub fn {REGISTER_FN}(")),
            expected.is_some(),
            "the `#[extendr]` item must track the reported surface \
             (registry_getter = {registry_getter:?})"
        );
    }
}

#[test]
fn extendr_reports_nothing_when_the_bridge_excludes_either_spelling_of_the_target() {
    for excluded in ["r", "extendr"] {
        let mut config = minimal_config("r", "");
        config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec![excluded.to_owned()]);

        let surfaces = ExtendrBackend.trait_bridge_registration_surface(&plugin_api(), &config);
        let generated = generated_text(&ExtendrBackend, &config);

        assert!(
            surfaces.is_empty(),
            "`exclude_languages = [\"{excluded}\"]` suppresses the `#[extendr]` items, so \
             nothing is left to document; got {surfaces:?}"
        );
        for symbol in [REGISTER_FN, UNREGISTER_FN, CLEAR_FN] {
            assert!(
                !generated.contains(symbol),
                "`exclude_languages = [\"{excluded}\"]` leaked `{symbol}` into extendr output"
            );
        }
    }
}

/// `alef.toml` may set `[crates.python.stubs]` to opt into `.pyi` generation; the dotted-key form
/// is equivalent TOML to a nested `[crates.python.stubs]` table. ~keep
fn pyo3_config_with_stubs() -> ResolvedCrateConfig {
    minimal_config("python", "stubs.output = \"stubs\"\n")
}

fn generated_pyi_text(config: &ResolvedCrateConfig) -> String {
    Pyo3Backend
        .generate_type_stubs(&plugin_api(), config)
        .unwrap_or_else(|error| panic!("pyo3: stub generation failed: {error}"))
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Positive control: a bridge with `registry_getter` set registers on every PyO3 surface — the
/// `#[pymodule]` body, the reported registration surface, and the `.pyi` stub. This is the
/// baseline the negative tests below must differ from -- the fix must not just "emit nothing,
/// always".
#[test]
fn pyo3_surface_names_the_pymodule_registered_functions() {
    let config = pyo3_config_with_stubs();
    let surface = only_surface(&Pyo3Backend, &config);

    assert_eq!(surface.register_symbol.as_deref(), Some(REGISTER_FN));
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&Pyo3Backend, &config);
    assert_declares(
        &Pyo3Backend,
        &generated,
        &format!("m.add_function(wrap_pyfunction!({REGISTER_FN}, m)?)?;"),
    );
    assert_declares(
        &Pyo3Backend,
        &generated,
        &format!("m.add_function(wrap_pyfunction!(_alef_{UNREGISTER_FN}, m)?)?;"),
    );
    assert_declares(
        &Pyo3Backend,
        &generated,
        &format!("m.add_function(wrap_pyfunction!(_alef_{CLEAR_FN}, m)?)?;"),
    );

    let stub = generated_pyi_text(&config);
    assert!(
        stub.contains(&format!("def {REGISTER_FN}(")),
        "the .pyi stub must declare the register function the pymodule actually exports:\n{stub}"
    );
}

/// The crux regression: `register_fn` without `registry_getter` makes every backend's
/// `gen_registration_fn` write nothing, so the `#[pymodule]` body must not reference the missing
/// `#[pyfunction]` -- doing so is a Rust `cannot find value` compile error, not a missing binding.
/// `unregister_fn`/`clear_fn` need no `registry_getter` and must be unaffected.
#[test]
fn pyo3_reports_no_register_symbol_and_the_pymodule_omits_it_without_a_registry_getter() {
    let mut config = pyo3_config_with_stubs();
    config.update_trait_bridge(0, |bridge| bridge.registry_getter = None);

    let surface = only_surface(&Pyo3Backend, &config);
    assert_eq!(
        surface.register_symbol, None,
        "gen_registration_fn emits no #[pyfunction] without a registry_getter, so no pymodule \
         export exists"
    );
    assert_eq!(surface.unregister_symbol.as_deref(), Some(UNREGISTER_FN));
    assert_eq!(surface.clear_symbol.as_deref(), Some(CLEAR_FN));

    let generated = generated_text(&Pyo3Backend, &config);
    assert!(
        !generated.contains(&format!("wrap_pyfunction!({REGISTER_FN}")),
        "the #[pymodule] body must not wrap a #[pyfunction] no pass emitted:\n{generated}"
    );
    assert_declares(
        &Pyo3Backend,
        &generated,
        &format!("m.add_function(wrap_pyfunction!(_alef_{UNREGISTER_FN}, m)?)?;"),
    );

    let stub = generated_pyi_text(&config);
    assert!(
        !stub.contains(&format!("def {REGISTER_FN}(")),
        "the .pyi stub must not declare a register function the native module never exports:\n{stub}"
    );
}

#[test]
fn pyo3_emits_no_bridge_and_reports_no_surface_when_the_target_is_excluded() {
    for excluded in ["python", "pyo3"] {
        let mut config = pyo3_config_with_stubs();
        config.update_trait_bridge(0, |bridge| bridge.exclude_languages = vec![excluded.to_owned()]);

        let surfaces = Pyo3Backend.trait_bridge_registration_surface(&plugin_api(), &config);
        let generated = generated_text(&Pyo3Backend, &config);

        assert_eq!(
            surfaces.len(),
            0,
            "`exclude_languages = [\"{excluded}\"]` suppresses the pymodule registration, so \
             nothing is left to document; got {surfaces:?}"
        );
        assert!(
            !generated.contains(REGISTER_FN),
            "`exclude_languages = [\"{excluded}\"]` must suppress the register #[pyfunction] too"
        );
        assert!(
            !generated.contains("PySamplePluginBridge"),
            "`exclude_languages = [\"{excluded}\"]` must suppress the bridge wrapper struct"
        );

        let stub = generated_pyi_text(&config);
        assert!(
            !stub.contains(REGISTER_FN),
            "`exclude_languages = [\"{excluded}\"]` must suppress the .pyi stub declaration too"
        );
    }
}

#[test]
fn pyo3_emits_no_bridge_when_the_bridged_trait_is_absent_from_the_api_surface() {
    let config = pyo3_config_with_stubs();
    let api = api_without_the_trait();

    let surfaces = Pyo3Backend.trait_bridge_registration_surface(&api, &config);
    let generated = generated_text_for(&Pyo3Backend, &api, &config);

    assert_eq!(
        surfaces.len(),
        0,
        "no trait means gen_trait_bridge never ran, so there is no pymodule export to document; \
         got {surfaces:?}"
    );
    assert!(
        !generated.contains(REGISTER_FN),
        "the #[pymodule] body would wrap a #[pyfunction] no pass emitted:\n{generated}"
    );
}
