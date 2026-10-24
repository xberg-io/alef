use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "sample".into(),
        version: "1.0.0".into(),
        types: vec![packet_type()],
        enums: vec![body_enum()],
        ..Default::default()
    }
}

fn packet_type() -> TypeDef {
    let named = || TypeRef::Named("BodyValue".into());
    TypeDef {
        name: "Packet".into(),
        rust_path: "sample::Packet".into(),
        has_serde: true,
        fields: vec![
            FieldDef {
                name: "required".into(),
                ty: named(),
                ..Default::default()
            },
            FieldDef {
                name: "nullable".into(),
                ty: TypeRef::Optional(Box::new(named())),
                ..Default::default()
            },
            FieldDef {
                name: "legacy".into(),
                ty: named(),
                optional: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn body_enum() -> EnumDef {
    EnumDef {
        name: "BodyValue".into(),
        rust_path: "sample::BodyValue".into(),
        serde_untagged: true,
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "PlainValue".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Count".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn configuration(projected: bool) -> NewAlefConfig {
    let text_types = if projected {
        "untagged_union_text_types = [\"BodyValue\"]"
    } else {
        ""
    };
    let config: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["swift"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
{text_types}
[crates.swift]
package_name = "Sample"
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_display_as_text = ["required", "nullable", "legacy"]
[crates.e2e.call]
function = "fetch"
module = "Sample"
result_var = "result"

"#
    ))
    .unwrap();
    config
}

fn generated(projected: bool) -> Vec<alef::core::GeneratedFile> {
    let config = configuration(projected);
    SwiftBackend
        .generate_bindings(&surface(), &config.resolve().unwrap().remove(0))
        .unwrap()
}

fn swift(files: &[alef::core::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path.ends_with("Sample.swift"))
        .unwrap()
        .content
}

#[test]
fn configured_text_getters_keep_returning_plain_text_and_the_struct_reads_the_accessor() {
    let files = generated(true);
    let source = swift(&files);
    for name in ["required", "nullable", "legacy"] {
        let assignment = source
            .lines()
            .find(|line| line.contains(&format!("self.{name} =")) && line.contains("rb."))
            .unwrap();
        assert!(assignment.contains(&format!("try rb.{name}Value()")), "{assignment}");
        assert!(!assignment.contains("plainValue(field0:"), "{assignment}");
        assert!(!assignment.contains("JSONDecoder"), "{assignment}");
    }
    let rust = &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    assert!(rust.contains("self.0.required.to_string()"));
    assert!(
        rust.contains("self.0.nullable.as_ref().map(|v| v.to_string())"),
        "{}",
        rust.lines()
            .filter(|line| line.contains("nullable") || line.contains("serde_json::to_string"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(rust.contains("self.0.legacy.as_ref().map(|v| v.to_string())"));
    let files = generated(false);
    let source = swift(&files);
    assert!(
        source
            .lines()
            .any(|line| line.contains("self.required = try JSONDecoder().decode"))
    );
}

fn assertion_lines(projected: bool, property: bool, expected: &str, optional_only: bool) -> String {
    use alef::e2e::codegen::{E2eCodegen, swift::SwiftE2eCodegen};
    use alef::e2e::fixture::FixtureGroup;
    let mut api = surface();
    api.types[0].is_opaque = !property;
    api.types[0].has_serde = property;
    api.functions.push(alef::core::ir::FunctionDef {
        name: "fetch".into(),
        rust_path: "sample::fetch".into(),
        return_type: TypeRef::Named("Packet".into()),
        ..Default::default()
    });
    let config = configuration(projected);
    let fields = if optional_only {
        vec!["nullable", "legacy"]
    } else {
        vec!["required", "nullable", "legacy"]
    };
    let fixture = assertion_fixture(&fields, expected);
    let files = SwiftE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "smoke".into(),
                fixtures: vec![fixture],
            }],
            config.crates[0].e2e.as_ref().unwrap(),
            &config.clone().resolve().unwrap().remove(0),
            &api.types,
            &api.enums,
            &api.functions,
            &[],
        )
        .unwrap();
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("SmokeTests.swift"))
        .unwrap()
        .content;
    let lines: Vec<_> = source.lines().filter(|line| line.contains("XCTAssertEqual(")).collect();
    assert_eq!(lines.len(), fields.len(), "{source}");
    lines.join("\n")
}

fn assertion_fixture(fields: &[&str], expected: &str) -> alef::e2e::fixture::Fixture {
    use alef::e2e::fixture::{Assertion, Fixture};
    Fixture {
        id: "text_projection".into(),
        assertions: fields
            .iter()
            .map(|name| Assertion {
                assertion_type: "equals".into(),
                field: Some((*name).into()),
                value: Some(serde_json::json!(expected)),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn configured_opaque_text_assertions_consume_plain_text() {
    let source = assertion_lines(true, false, "hello", false);
    assert!(!source.contains("JSONDecoder"), "{source}");
    assert!(source.contains("result.required().toString()"), "{source}");
    assert!(source.contains("result.nullable()?.toString()"), "{source}");
    assert!(source.contains("result.legacy()?.toString()"), "{source}");
    let structural = assertion_lines(false, false, "hello", false);
    assert!(structural.contains("JSONDecoder"));
    assert!(structural.contains("result.nullable().toString()"), "{structural}");
    assert!(!structural.contains("result.nullable()?.toString()"), "{structural}");
    assert!(assertion_lines(true, true, "hello", false).contains("result.required.text()"));
}

fn block(source: &str, signature: &str) -> String {
    let start = source.find(signature).unwrap_or_else(|| panic!("missing {signature}"));
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth == 1 => return source[start..=open + offset].to_string(),
            '}' => depth -= 1,
            _ => {}
        }
    }
    panic!("unterminated {signature}")
}

fn native_source() -> String {
    let projected = generated(true);
    let source = swift(&projected);
    let structural = generated(false);
    let structural = swift(&structural);
    let rows: Vec<_> = ["", "ordinary text", "\"quoted\"", "{\"key\":1}", " leading whitespace"]
        .iter()
        .map(|value| {
            serde_json::json!({ "literal": serde_json::to_string(value).unwrap(),
            "opaque": assertion_lines(true, false, value, false),
            "property": assertion_lines(true, true, value, false),
            "structural": assertion_lines(false, false, value, false) })
        })
        .collect();
    assert_eq!(rows.len(), 5);
    let mut environment = minijinja::Environment::new();
    alef::template::configure_env(&mut environment);
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    environment
        .add_template(
            "probe",
            include_str!("fixtures/swift_text_projection/probe.swift.jinja"),
        )
        .unwrap();
    environment.get_template("probe").unwrap().render(alef::alef_context! {
        enum_declaration => block(source, "public enum BodyValue:"),
        text_extension => block(source, "extension BodyValue {"),
        packet_declaration => block(source, "public struct Packet:"),
        packet_initializer => block(source, "init(_ rb: RustBridge.PacketRef)"),
        structural_declaration => block(structural, "public struct Packet:").replace("struct Packet:", "struct StructuralPacket:"),
        structural_initializer => block(structural, "init(_ rb: RustBridge.PacketRef)").replace("RustBridge.PacketRef", "RustBridge.StructuralPacketRef"),
        rows => rows, nil_opaque => assertion_lines(true, false, "", true),
        nil_property => assertion_lines(true, true, "", true),
        nil_structural => assertion_lines(false, false, "", true),
    }).unwrap()
}

#[test]
#[ignore = "requires Swift compiler and XCTest runtime"]
fn generated_text_wrappers_and_assertions_preserve_exact_values() {
    let directory = tempfile::Builder::new().prefix("alef-swift-text-").tempdir().unwrap();
    let root = directory.path();
    let source = native_source();
    assert_eq!(
        source.matches("XCTAssert").count(),
        90,
        "exercise all expected native checks"
    );
    std::fs::write(root.join("probe.swift"), source).unwrap();
    std::fs::write(
        root.join("Taskfile.yml"),
        include_str!("fixtures/swift_text_projection/Taskfile.yml"),
    )
    .unwrap();
    let compiled = native_task(root, "build", &[]);
    assert!(compiled.success, "{root:?}\n{}\n{}", compiled.stdout, compiled.stderr);
    let good = native_task(root, "run", &[]);
    assert!(
        good.success && format!("{}\n{}", good.stdout, good.stderr).contains("Executed 1 test, with 0 failures"),
        "{}\n{}",
        good.stdout,
        good.stderr
    );
    assert!(good.stdout.contains("swift_text_projection_checks=90"));
    let wrong = native_task(root, "run", &["MODE=wrong-text"]);
    assert!(!wrong.success, "wrong text must fail an actual generated assertion");
    assert!(
        format!("{}\n{}", wrong.stdout, wrong.stderr).contains("XCTAssertEqual failed"),
        "{}\n{}",
        wrong.stdout,
        wrong.stderr
    );
}

fn native_task(root: &std::path::Path, task: &str, args: &[&str]) -> alef::snippets::validators::CapturedStreams {
    let executable = which::which_in("task", std::env::var_os("PATH"), root).expect("Task executable");
    let mut command = std::process::Command::new(executable);
    command
        .arg(task)
        .args(args)
        .current_dir(root)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("CARGO_TARGET_DIR", root.join("target"));
    alef::snippets::validators::run_command_streams(&mut command, 180).expect("run native Swift fixture")
}

fn fixture_template(source: &str, context: minijinja::Value) -> String {
    let mut environment = minijinja::Environment::new();
    alef::template::configure_env(&mut environment);
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    environment.add_template("fixture", source).unwrap();
    environment.get_template("fixture").unwrap().render(context).unwrap()
}

fn write_rust_fixture(root: &std::path::Path, projected: bool) -> String {
    write_native_dependencies(root);
    std::fs::create_dir_all(root.join("bridge/src")).unwrap();
    let files = generated(projected);
    let manifest = files
        .iter()
        .find(|file| file.path.ends_with("rust/Cargo.toml"))
        .unwrap();
    let manifest: toml::Value = toml::from_str(&manifest.content).unwrap();
    let bridge_version = manifest["dependencies"]["swift-bridge"].as_str().unwrap();
    let manifest = fixture_template(
        include_str!("fixtures/swift_text_projection/bridge_Cargo.toml.jinja"),
        alef::alef_context! { bridge_version => bridge_version },
    );
    std::fs::write(root.join("bridge/Cargo.toml"), manifest).unwrap();
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    let probe = fixture_template(
        include_str!("fixtures/swift_text_projection/getters.rs.jinja"),
        alef::alef_context! { projected => projected },
    );
    [source.as_str(), probe.as_str()].join("\n")
}

fn write_native_dependencies(root: &std::path::Path) {
    for (name, source) in [
        (
            "core/Cargo.toml",
            include_str!("fixtures/swift_text_projection/core_Cargo.toml"),
        ),
        (
            "core/src/lib.rs",
            include_str!("fixtures/swift_text_projection/core.rs"),
        ),
        (
            "ffi/Cargo.toml",
            include_str!("fixtures/swift_text_projection/ffi_Cargo.toml"),
        ),
        ("ffi/src/lib.rs", include_str!("fixtures/swift_text_projection/ffi.rs")),
        (
            "Taskfile.yml",
            include_str!("fixtures/swift_text_projection/Taskfile.yml"),
        ),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
}

#[test]
#[ignore = "requires Cargo and the swift-bridge macro"]
fn generated_rust_text_getters_match_bridge_declarations() {
    let directory = tempfile::Builder::new()
        .prefix("alef-swift-text-rust-")
        .tempdir()
        .unwrap();
    let root = directory.path();
    for projected in [true, false] {
        let source = write_rust_fixture(root, projected);
        if projected {
            let signature = "fn nullable(&self) -> Option<String>;";
            assert_eq!(
                source.matches(signature).count(),
                1,
                "one actual extern getter declaration"
            );
            let broken = source.replacen(signature, "fn nullable(&self) -> String;", 1);
            std::fs::write(root.join("bridge/src/lib.rs"), broken).unwrap();
            let rejected = native_task(root, "rust-check", &[]);
            assert!(!rejected.success, "mismatched actual extern getter must fail");
            assert!(
                rejected.stderr.contains("mismatched types") && rejected.stderr.contains("E0308"),
                "{}\n{}",
                rejected.stdout,
                rejected.stderr
            );
        }
        std::fs::write(root.join("bridge/src/lib.rs"), source).unwrap();
        let compiled = native_task(root, "rust-test", &[]);
        assert!(compiled.success, "{root:?}\n{}\n{}", compiled.stdout, compiled.stderr);
        assert!(compiled.stdout.contains("1 passed; 0 failed"), "{}", compiled.stdout);
    }
}

#[test]
fn text_projection_without_a_string_variant_reports_the_enum() {
    let mut api = surface();
    api.enums[0].variants.remove(0);
    let config = configuration(true).resolve().unwrap().remove(0);
    let error = SwiftBackend.generate_bindings(&api, &config).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("text-projected enum BodyValue has no single-String variant")
    );
}

fn rust_source(files: &[alef::core::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content
}

#[test]
fn configured_text_fields_expose_the_whole_value_through_a_public_accessor() {
    let files = generated(true);
    let source = swift(&files);
    let extension = block(source, "extension RustBridge.PacketRef {");
    assert!(
        extension.contains("public func requiredValue() throws -> BodyValue {"),
        "{extension}"
    );
    assert!(
        extension.contains("public func nullableValue() throws -> BodyValue? {"),
        "{extension}"
    );
    assert!(
        extension.contains("public func legacyValue() throws -> BodyValue? {"),
        "{extension}"
    );
    assert!(
        extension.contains("try JSONDecoder().decode(BodyValue.self, from: Data(requiredJson().toString().utf8))"),
        "{extension}"
    );
    assert!(
        extension.contains(
            "try nullableJson().map { try JSONDecoder().decode(BodyValue.self, from: Data($0.toString().utf8)) }"
        ),
        "{extension}"
    );
    let rust = rust_source(&files);
    assert!(rust.contains("fn required_json(&self) -> String;"), "{rust}");
    assert!(rust.contains("fn nullable_json(&self) -> Option<String>;"), "{rust}");
    assert!(rust.contains("fn legacy_json(&self) -> Option<String>;"), "{rust}");
    assert!(rust.contains("serde_json::to_string(&self.0.required)"), "{rust}");
    assert!(
        rust.contains("self.0.legacy.as_ref().map(|v| serde_json::to_string(v)"),
        "{rust}"
    );
    let text = block(source, "public enum BodyValue:");
    assert!(text.contains("case plainValue(field0: String)"), "{text}");
}

#[test]
fn text_fields_keep_the_text_getter_the_reply_text_depends_on() {
    let files = generated(true);
    let rust = rust_source(&files);
    assert!(rust.contains("self.0.required.to_string()"), "{rust}");
    assert!(rust.contains("fn required(&self) -> String;"), "{rust}");
}

#[test]
fn fields_outside_the_text_types_get_no_value_accessor() {
    let files = generated(false);
    let source = swift(&files);
    assert!(!source.contains("extension RustBridge.PacketRef"), "{source}");
    assert!(!source.contains("requiredValue"), "{source}");
    assert!(!rust_source(&files).contains("required_json"));
}

#[test]
fn the_struct_form_carries_the_whole_value_not_only_its_text() {
    let files = generated(true);
    let source = swift(&files);
    let init = block(source, "internal extension Packet {");
    assert!(init.contains("self.required = try rb.requiredValue()"), "{init}");
    assert!(init.contains("self.nullable = try rb.nullableValue()"), "{init}");
    assert!(init.contains("self.legacy = try rb.legacyValue()"), "{init}");
    assert!(!init.contains("BodyValue.text("), "{init}");
}
