use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeRef};

#[test]
fn should_preserve_typed_dart_errors_alongside_checked_input_errors() {
    let api = ApiSurface {
        crate_name: "sample".into(),
        errors: vec![ErrorDef {
            name: "RequestError".into(),
            rust_path: "sample::RequestError".into(),
            variants: vec![ErrorVariant {
                name: "Rejected".into(),
                fields: vec![FieldDef {
                    name: "message".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            exception_base: None,
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "request".into(),
            rust_path: "sample::request".into(),
            params: vec![ParamDef {
                name: "count".into(),
                ty: TypeRef::Primitive(PrimitiveType::U16),
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("RequestError".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = DartBackend
        .generate_bindings(&api, &config.resolve().unwrap()[0])
        .unwrap();
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    assert!(source.contains("Result<(), AlefBridgeError>"), "{source}");
    assert!(source.contains("impl From<String> for AlefBridgeError"), "{source}");
    assert!(
        source.contains("impl From<sample::RequestError> for AlefBridgeError"),
        "{source}"
    );
    assert!(
        source.contains("RequestError_Rejected") || source.contains("RequestError { error:"),
        "{source}"
    );
    assert!(
        !source.contains("sample::request(count as _).map_err(|e| e.to_string())"),
        "{source}"
    );
    verify_generated_bridge_at_runtime(source);
}

fn verify_generated_bridge_at_runtime(source: &str) {
    let mut skipping_attribute = false;
    let generated = source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("#[frb(") {
                skipping_attribute = !trimmed.ends_with(")]");
                return false;
            }
            if skipping_attribute {
                skipping_attribute = !trimmed.ends_with(")]");
                return false;
            }
            !trimmed.starts_with("mod frb_generated;") && !trimmed.contains("use flutter_rust_bridge::")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let fixture = include_str!("fixtures/dart_typed_error_probe.rs");
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("main.rs");
    let executable = temp
        .path()
        .join(format!("bridge-probe{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&file, format!("{generated}\n{fixture}")).unwrap();
    let output = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg(&file)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let output = std::process::Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
