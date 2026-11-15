use super::runtime_export_names;
use crate::backends::napi::gen_bindings::type_stubs::esm::gen_esm_wrapper;

const NATIVE_DECLARATIONS: &str = r#"
#[napi(string_enum, js_name = "Mode")]
pub enum JsMode { Ready }
#[napi(js_name = "Worker")]
pub struct JsWorker;
#[napi(object, js_name = "Settings")]
pub struct JsSettings;
#[napi(js_name = "streamItems")]
pub async fn stream_items() {}
#[napi]
pub fn settings_default() {}
#[napi(namespace = "Mode", js_name = "inspect")]
pub fn mode_inspect() {}
impl JsWorker { #[napi] pub fn execute(&self) {} }
"#;

#[test]
fn emitted_native_exports_include_helpers_and_aliases_without_type_only_or_method_names() {
    assert_eq!(
        runtime_export_names(NATIVE_DECLARATIONS).unwrap(),
        ["JsMode", "JsWorker", "Mode", "Worker", "settingsDefault", "streamItems"]
    );
    assert!(runtime_export_names("invalid Rust {{").is_err());
}

#[test]
fn type_stub_generation_exports_actual_dto_method_helpers() {
    use crate::backends::napi::gen_bindings::NapiBackend;
    use crate::core::backend::Backend;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Settings".to_string(),
            rust_path: "sample_core::Settings".to_string(),
            has_default: true,
            methods: vec![MethodDef {
                name: "default".to_string(),
                is_static: true,
                return_type: TypeRef::Named("Settings".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        ..Default::default()
    };
    let generated = NapiBackend.generate_type_stubs(&api, &config).unwrap();
    let esm = generated.iter().find(|file| file.path.ends_with("index.mjs")).unwrap();
    assert!(
        esm.content
            .contains("export const settingsDefault = binding.settingsDefault;")
    );
    assert!(!esm.content.contains("export const JsSettings"));
}

#[test]
fn split_bytes_metadata_factory_is_declared_as_its_runtime_export() {
    use crate::backends::napi::gen_bindings::NapiBackend;
    use crate::core::backend::Backend;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, TypeDef, TypeRef};

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Input".into(),
            rust_path: "sample_core::Input".into(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                optional: true,
                ..Default::default()
            }],
            methods: vec![MethodDef {
                name: "from_json_with_bytes".into(),
                is_static: true,
                return_type: TypeRef::Named("Input".into()),
                error_type: Some("Error".into()),
                params: vec![
                    ParamDef {
                        name: "bytes".into(),
                        ty: TypeRef::Bytes,
                        ..Default::default()
                    },
                    ParamDef {
                        name: "metadata_json".into(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-core".into(),
        ..Default::default()
    };
    let files = NapiBackend.generate_type_stubs(&api, &config).unwrap();
    let declarations = files.iter().find(|file| file.path.ends_with("index.d.ts")).unwrap();
    assert!(
        declarations.content.contains(
            "export declare function inputFromJsonWithBytes(bytes: Uint8Array, metadataJson: string): Input;"
        )
    );
    let esm = files.iter().find(|file| file.path.ends_with("index.mjs")).unwrap();
    assert!(
        esm.content
            .contains("export const inputFromJsonWithBytes = binding.inputFromJsonWithBytes;")
    );
}

#[test]
fn node_conditional_import_and_require_preserve_runtime_helpers_and_undefined_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let node = which::which_in("node", std::env::var_os("PATH"), directory.path()).expect("Node must be installed");
    let package = directory.path().join("node_modules/@sample/native");
    std::fs::create_dir_all(&package).unwrap();
    write_runtime_fixture(&package);
    let probe = directory.path().join("probe.mjs");
    std::fs::write(&probe, include_str!("native_exports_probe.mjs")).unwrap();
    let cases = [
        (vec!["Mode".to_string(), "Worker".to_string()], false),
        (runtime_export_names(NATIVE_DECLARATIONS).unwrap(), true),
    ];
    assert_eq!(cases.len(), 2);
    for (names, expected_success) in cases {
        std::fs::write(package.join("index.mjs"), gen_esm_wrapper(&names, "./index.js")).unwrap();
        let result = std::process::Command::new(&node)
            .arg(&probe)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            expected_success,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if !expected_success {
            assert!(String::from_utf8_lossy(&result.stderr).contains("does not provide an export named"));
        }
    }
}

fn write_runtime_fixture(package: &std::path::Path) {
    let manifest = serde_json::json!({
        "name": "@sample/native",
        "exports": {".": {"import": "./index.mjs", "require": "./index.js"}}
    });
    std::fs::write(package.join("package.json"), manifest.to_string()).unwrap();
    std::fs::write(package.join("index.js"), include_str!("native_exports_fixture.cjs")).unwrap();
    std::fs::write(package.join("binding.cjs"), include_str!("../../templates/binding.cjs")).unwrap();
}
