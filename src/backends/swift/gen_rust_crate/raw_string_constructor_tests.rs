use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = include_str!("raw_string_constructor_fixture.rs.txt");
const CONSUMER: &str = include_str!("raw_string_constructor_consumer.rs.txt");

fn extracted_api(directory: &Path) -> ApiSurface {
    let source = directory.join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let mut api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert_eq!(api.types.len(), 3);
    assert_eq!(api.types.iter().filter(|owner| owner.binding_excluded).count(), 1);
    api.types.retain(|owner| !owner.binding_excluded);
    crate::cli::pipeline::sanitize_binding_projection(&mut api);
    assert_eq!(api.types.len(), 2);
    for owner in &api.types {
        assert_eq!(owner.fields.len(), 8);
        assert!(owner.has_serde);
        assert_eq!(owner.has_default, owner.name == "DefaultText");
        let names = HashSet::new();
        assert_eq!(
            super::extern_block::has_constructor_extern(
                owner,
                &HashSet::new(),
                &names,
                super::default_construction::EnumKinds::new(&names, &names),
                false,
            ),
            owner.has_default,
            "only the Default fixture exposes an actual wrapper constructor",
        );
        for (index, field) in owner.fields.iter().enumerate() {
            assert_eq!(field.optional, index % 2 == 1);
            let expected = match index {
                2 | 3 => TypeRef::Json,
                4 | 5 => TypeRef::Path,
                _ => TypeRef::String,
            };
            assert_eq!(field.ty, expected, "{}.{}", owner.name, field.name);
            assert_eq!(field.sanitized, index >= 6, "{}.{}", owner.name, field.name);
        }
    }
    api
}

fn emitted_wrappers(api: &ApiSurface) -> String {
    let paths = build_type_path_lookup(api);
    let names = HashSet::new();
    let excluded = HashSet::new();
    api.types
        .iter()
        .map(|owner| {
            super::wrappers::emit_type_wrapper(
                owner,
                "sample_core",
                &paths,
                &names,
                &names,
                &names,
                &names,
                &names,
                &excluded,
                &names,
                &names,
                &names,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn direct_field_initializers(api: &ApiSurface) -> String {
    let owner = api.types.iter().find(|owner| owner.name == "DirectText").unwrap();
    assert!(!owner.has_default);
    let names = HashSet::new();
    let fields = super::default_construction::emit_direct_field_inits(
        owner,
        &build_type_path_lookup(api),
        super::default_construction::EnumKinds::new(&names, &names),
        &names,
        &HashSet::new(),
        &names,
    );
    assert_eq!(
        fields.len(),
        8,
        "the internal direct helper must initialize every source field"
    );
    fields.join(",\n")
}

fn dependency_library(name: &str) -> PathBuf {
    let executable = std::env::current_exe().unwrap();
    std::fs::read_dir(executable.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .is_some_and(|file| file.starts_with(&format!("lib{name}-")) && file.ends_with(".rlib"))
        })
        .max_by_key(|path| path.metadata().unwrap().modified().unwrap())
        .unwrap_or_else(|| panic!("the running test must have a compiled {name} dependency"))
}

fn runtime_rows(kind: &str) -> Vec<serde_json::Value> {
    let directory = tempfile::tempdir().unwrap();
    let api = extracted_api(directory.path());
    let wrappers = emitted_wrappers(&api);
    assert_eq!(CONSUMER.matches("__DIRECT_FIELD_INITS__").count(), 1);
    let consumer = CONSUMER.replace("__DIRECT_FIELD_INITS__", &direct_field_initializers(&api));
    let source = ["mod sample_core {", SOURCE, "}", &wrappers, &consumer].join("\n");
    let source_path = directory.path().join("probe.rs");
    let binary = directory.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source_path, source).unwrap();
    let serde = dependency_library("serde");
    let serde_json = dependency_library("serde_json");
    let compiled = Command::new("rustc")
        .current_dir(directory.path())
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("serde={}", serde.display()))
        .arg("--extern")
        .arg(format!("serde_json={}", serde_json.display()))
        .arg("-L")
        .arg(format!("dependency={}", serde_json.parent().unwrap().display()))
        .arg(&source_path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = Command::new(binary).arg(kind).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    serde_json::from_slice(&run.stdout).unwrap()
}

fn assert_literal_rows(kind: &str) {
    let cases = [
        None,
        Some(""),
        Some("ordinary text"),
        Some("null"),
        Some("true"),
        Some("false"),
        Some("123"),
        Some(r#""quoted""#),
        Some(r#"{"key":"value"}"#),
        Some("[]"),
        Some("עברית e\u{301} 界 🦀"),
        Some("before\0after"),
        Some(r#""\u0061""#),
        Some("line\nback\\slash"),
    ];
    let rows = runtime_rows(kind);
    assert_eq!(rows.len(), 14, "every literal must traverse the emitted {kind} path");
    for (index, (row, input)) in rows.iter().zip(cases).enumerate() {
        let expected = serde_json::json!({
            "text": input.unwrap_or_default(),
            "optional_text": input,
            "json": {"kind": "literal", "text": "null"},
            "optional_json": {"kind": "literal", "text": "null"},
            "path": "samples/界.txt",
            "optional_path": "samples/界.txt",
            "projected": {"marker": "projection"},
            "optional_projected": {"marker": "projection"},
        });
        assert_eq!(
            row, &expected,
            "emitted {kind} path changed literal row {index}: {input:?}"
        );
    }
}

#[test]
fn default_constructor_preserves_native_string_literals_and_projected_controls() {
    assert_literal_rows("default");
}

#[test]
fn internal_direct_field_initializers_preserve_native_string_literals_and_projected_controls() {
    assert_literal_rows("internal_direct_field_inits");
}
