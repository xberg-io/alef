use alef::backends::{kotlin::KotlinBackend, kotlin_android::KotlinAndroidBackend};
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::process::Command;

fn client_source(android: bool) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin", "kotlin_android", "jni", "ffi"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.kotlin]
package = "dev.sample"
ffi_style = "jni"
[crates.kotlin_android]
package = "dev.sample"
namespace = "dev.sample"
[crates.package_metadata]
repository = "https://github.com/example/sample"
license = "MIT"
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "sample".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "DefaultClient".into(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "fetch".into(),
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::Named("Response".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = if android {
        KotlinAndroidBackend.generate_bindings(&api, &config)
    } else {
        KotlinBackend.generate_bindings(&api, &config)
    }
    .unwrap();
    let clients: Vec<_> = files
        .iter()
        .filter(|file| file.path.ends_with("DefaultClient.kt"))
        .collect();
    assert_eq!(clients.len(), 1);
    clients[0].content.clone()
}

fn mapper_expression(source: &str) -> &str {
    source
        .split_once("private val MAPPER = ")
        .expect("generated client has a JSON mapper")
        .1
        .split_once("\n    }")
        .expect("generated companion terminates")
        .0
}

#[test]
fn both_jni_client_targets_accept_serializer_added_response_fields() {
    for android in [false, true] {
        let source = client_source(android);
        assert!(
            mapper_expression(&source).contains("DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false"),
            "JNI response mapper must match Rust's acceptance of additional fields; android={android}"
        );
    }
}

#[test]
#[ignore = "requires Kotlin JVM and Maven access; run explicitly for executable mapper verification"]
fn generated_jni_response_mappers_execute_with_extra_fields_and_reject_wrong_types() {
    for android in [false, true] {
        let source = client_source(android);
        let script = format!(
            r#"@file:DependsOn("com.fasterxml.jackson.module:jackson-module-kotlin:2.22.2")
@file:DependsOn("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:2.22.2")
data class Response(val content: String, val count: Int)
val mapper = {}
val response = mapper.readValue("""{{"role":"assistant","content":"retained","count":7}}""", Response::class.java)
check(response == Response("retained", 7))
val failure = runCatching {{ mapper.readValue("""{{"content":"retained","count":{{"invalid":true}}}}""", Response::class.java) }}.exceptionOrNull()
check(failure is com.fasterxml.jackson.databind.exc.MismatchedInputException)
println("extra field accepted; known values retained; wrong type rejected")
"#,
            mapper_expression(&source),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mapper.main.kts");
        std::fs::write(&path, script).unwrap();
        let output = Command::new("kotlin")
            .arg(path)
            .output()
            .expect("execute installed Kotlin JVM");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "extra field accepted; known values retained; wrong type rejected"
        );
    }
}
