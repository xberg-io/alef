use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use std::process::Command;

fn config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]
[[crates]]
name = "fixture"
sources = ["src/lib.rs"]
[crates.java]
package = "fixture"
[crates.ffi]
prefix = "fixture"
"#,
    )
    .unwrap();
    config.resolve().unwrap().remove(0)
}

fn surface() -> ApiSurface {
    serde_json::from_value(serde_json::json!({
        "crate_name": "fixture", "version": "0.1.0", "types": [], "functions": [], "enums": [],
        "errors": [
            {"name": "RequestError", "rust_path": "fixture::RequestError", "doc": "", "variants": [
                {"name": "TooManyRedirects", "error_code": 100, "doc": "", "message_template": "loop", "is_unit": true}]},
            {"name": "PolicyError", "exception_base": "RequestError", "rust_path": "fixture::PolicyError",
             "doc": "", "variants": [
                {"name": "TooManyRedirects", "error_code": 101, "doc": "", "message_template": "loop", "is_unit": true}]}
        ]
    }))
    .unwrap()
}

fn dispatch(source: &str) -> &str {
    let start = source
        .find("    static FixtureRsException lastErrorException(")
        .unwrap();
    let opening = source[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (offset, character) in source[opening..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..opening + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("generated dispatch has no closing brace")
}

#[test]
fn generated_dispatch_preserves_shared_variant_native_codes_and_legacy_constructors() {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("fixture");
    std::fs::create_dir(&package).unwrap();
    let mut sources = Vec::new();
    for file in JavaBackend.generate_bindings(&surface(), &config()).unwrap() {
        let name = file.path.file_name().unwrap().to_string_lossy();
        if name.ends_with("Exception.java") || name == "NativeLib.java" {
            let content = if name == "NativeLib.java" {
                [
                    "package fixture;\nclass NativeLib {\n",
                    dispatch(&file.content),
                    "\n}\n",
                ]
                .concat()
            } else {
                file.content
            };
            let path = package.join(name.as_ref());
            std::fs::write(&path, content).unwrap();
            sources.push(path);
        }
    }
    assert_eq!(sources.len(), 9, "eight emitted exceptions and the actual dispatch");
    let probe = package.join("CodeProbe.java");
    std::fs::write(&probe, include_str!("fixtures/java_native_error_code_probe.java")).unwrap();
    sources.push(probe);
    let compile = Command::new("javac").args(&sources).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let output = Command::new("java")
        .args(["-cp", directory.path().to_str().unwrap(), "fixture.CodeProbe"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "CODE_CASES=5 LEGACY_CONSTRUCTORS=3"
    );
}
