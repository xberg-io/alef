use alef::backends::{jni::JniBackend, kotlin::KotlinBackend, kotlin_android::KotlinAndroidBackend};
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn validator_config(source: &str) -> ResolvedCrateConfig {
    let toml = format!(
        r#"
[workspace]
languages = ["kotlin", "kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
error_type = "ValidatorError"
error_constructor = "demo::ValidatorError::Callback {{ message: {{msg}}, source: {source} }}"

[crates.package_metadata]
repository = "https://github.com/example/demo"
license = "MIT"
authors = ["Example"]

[crates.kotlin]
package = "dev.sample"
ffi_style = "jni"

[crates.kotlin_android]
package = "dev.sample"
namespace = "dev.sample"

[[crates.trait_bridges]]
trait_name = "Validator"
super_trait = "Plugin"
registry_getter = "demo::validators"
register_fn = "register_validator"
"#
    );
    toml::from_str::<NewAlefConfig>(&toml)
        .unwrap()
        .resolve()
        .unwrap()
        .remove(0)
}

fn validator_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Validator".into(),
            rust_path: "demo::Validator".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "validate".into(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("ValidatorError".into()),
                is_async: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated_file(backend: &dyn Backend, api: &ApiSurface, config: &ResolvedCrateConfig, name: &str) -> String {
    backend
        .generate_bindings(api, config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with(name))
        .unwrap_or_else(|| panic!("{backend} must generate {name}", backend = backend.name()))
        .content
}

#[test]
fn python_only_source_constructor_preserves_kotlin_and_jni_diagnostics() {
    let api = validator_api();
    let scoped = validator_config("{python_source}");
    let legacy = validator_config("None");

    for backend in [&KotlinBackend as &dyn Backend, &KotlinAndroidBackend] {
        let file = "ValidatorJniDispatcher.kt";
        let output = generated_file(backend, &api, &scoped, file);
        assert_eq!(output, generated_file(backend, &api, &legacy, file));
        assert!(
            output.contains("mapper.writeValueAsString(mapOf(\"err\" to (t.message ?: t.toString())))"),
            "{} must serialize the original Throwable diagnostic",
            backend.name()
        );
    }

    let output = generated_file(&JniBackend, &api, &scoped, "lib.rs");
    assert!(output.contains("source: None"), "JNI must supply no Python source");
    assert!(
        !output.contains("JniHostException"),
        "JNI must keep the JSON error path"
    );
    assert_eq!(output, generated_file(&JniBackend, &api, &legacy, "lib.rs"));
}

#[test]
fn shared_source_constructor_preserves_jvm_exception_transport() {
    let api = validator_api();
    let config = validator_config("{source}");

    for backend in [&KotlinBackend as &dyn Backend, &KotlinAndroidBackend] {
        let output = generated_file(backend, &api, &config, "ValidatorJniDispatcher.kt");
        assert!(
            output.contains("runBlocking { runCatching { impl.validate("),
            "{} must let callback exceptions cross into JNI",
            backend.name()
        );
        assert!(!output.contains("catch (t: Throwable)"), "Throwable must reach JNI");
    }

    let output = generated_file(&JniBackend, &api, &config, "lib.rs");
    assert!(
        output.contains("e.into_host_source()"),
        "JNI must retain the captured Throwable"
    );
    assert!(
        output.contains("env.throw(&exception.0)"),
        "JNI must rethrow the captured Throwable"
    );
}
