use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::test_support::{spawn_from_stable_dir, toolchain};

fn sample_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample".into(),
        enums: vec![EnumDef {
            name: "Preference".into(),
            rust_path: "sample::Preference".into(),
            has_serde: true,
            serde_rename_all: Some("lowercase".into()),
            variants: ["Low", "None", "NoneValue", "Default", "RDFa"]
                .map(|name| EnumVariant {
                    name: name.into(),
                    ..Default::default()
                })
                .to_vec(),
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "Request".into(),
            rust_path: "sample::Request".into(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "preference".into(),
                ty: TypeRef::Named("Preference".into()),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn should_name_swift_cases_without_optional_none_ambiguity_or_new_collisions() {
    use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
    use crate::core::config::Language;
    for (rust, expected) in [
        ("None", "none_"),
        ("NoneValue", "noneValue"),
        ("Low", "low"),
        ("Default", "`default`"),
        ("RDFa", "rdfa"),
    ] {
        assert_eq!(
            public_host_identifier(Language::Swift, PublicIdentifierKind::EnumVariant, rust),
            expected
        );
    }
}

fn declaration<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source.find(marker).expect("generated declaration");
    let opening = start + source[start..].find('{').expect("declaration body");
    let mut depth = 0;
    for (offset, character) in source[opening..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &source[start..=opening + offset];
        }
    }
    panic!("unterminated generated declaration");
}

fn generated_program() -> String {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = SwiftBackend
        .generate_bindings(&sample_api(), &config.resolve().unwrap()[0])
        .unwrap();
    let sources: Vec<_> = files
        .iter()
        .filter(|file| file.content.contains("public enum Preference:"))
        .collect();
    assert_eq!(sources.len(), 1);
    let source = &sources[0].content;
    let enum_declaration = declaration(source, "public enum Preference:");
    let request_declaration = declaration(source, "public struct Request:");
    let selected_case = enum_declaration
        .lines()
        .find_map(|line| {
            let (name, wire) = line.trim().strip_prefix("case ")?.split_once('=')?;
            (wire.trim() == "\"none\"").then(|| name.trim().to_owned())
        })
        .unwrap_or_else(|| {
            assert!(enum_declaration.lines().any(|line| line.trim() == "case none"));
            "none".to_owned()
        });
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/optional_enum_none.swift.jinja"))
        .unwrap();
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! {
            enum_declaration => enum_declaration,
            request_declaration => request_declaration,
            selected_case => selected_case,
        })
        .unwrap()
}

#[test]
fn should_preserve_a_selected_none_variant_in_an_optional_swift_field() {
    let Some(swift) = toolchain::SWIFT.open() else { return };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.swift");
    std::fs::write(&path, generated_program()).unwrap();
    let output = spawn_from_stable_dir(swift).arg(path).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    assert_eq!(lines[0], "none", "selecting the generated case must retain a value");
    let encoded: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(encoded["preference"], "none");
    assert_eq!(lines[2], "none");
    assert_eq!(lines[3], "true");
}
