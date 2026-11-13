use crate::test_support::cargo_artifacts::{add_dependency_search_paths, dependency_rlibs};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use quote::ToTokens;

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::e2e::codegen::php::enum_collection_test_support as support;
use crate::e2e::config::E2eConfig;

#[test]
fn ruby_enum_collection_access_matches_actual_producer_wire_names() {
    verify_producer(false);
}

#[test]
#[ignore = "native Ruby proof requires RSpec matchers"]
fn ruby_enum_collection_access_executes_actual_producer_wire_names() {
    verify_producer(true);
}

#[test]
fn ruby_enum_explicit_wire_name_wins_positional_fallback() {
    for (explicit, expected) in [(None, "value"), (Some("declared"), "declared")] {
        let mut surface = support::surface();
        let field = &mut surface
            .enums
            .iter_mut()
            .find(|item| item.name == "Piece")
            .unwrap()
            .variants[0]
            .fields[0];
        field.name = "_0".to_string();
        field.serde_rename = explicit.map(str::to_string);
        let emitted = emitted_piece(&surface);
        let attributes = emitted.variants[0]
            .fields
            .iter()
            .next()
            .unwrap()
            .attrs
            .iter()
            .map(ToTokens::to_token_stream)
            .map(|tokens| tokens.to_string())
            .collect::<String>();
        assert!(attributes.contains(&format!("rename = {expected:?}")), "{attributes}");
        if explicit.is_some() {
            assert!(attributes.contains("alias = \"value\""), "{attributes}");
        }
    }
}

fn verify_producer(native: bool) {
    for (explicit, expected) in [
        (None, "textValue"),
        (Some("wireText"), "wireText"),
        (Some("wire\"\\text"), "wire\"\\text"),
    ] {
        let mut surface = support::surface();
        let piece = surface.enums.iter_mut().find(|item| item.name == "Piece").unwrap();
        piece.rename_all_fields = Some("camelCase".to_string());
        piece.variants[0].fields[0].name = "text_value".to_string();
        piece.variants[0].fields[0].serde_rename = explicit.map(str::to_string);
        let declaration = emitted_piece(&surface);
        let payload = run_producer(&declaration, expected, true).unwrap();
        let rendered = render_assertions(&surface);
        if native {
            support::verify_native_payload("ruby", "rb", &rendered, native_cases(payload, expected));
        }
        let mut broken = declaration;
        let mut removed = 0;
        broken.variants[0].fields.iter_mut().for_each(|field| {
            field.attrs.retain(|attribute| {
                let rename = attribute.to_token_stream().to_string().contains("rename =");
                removed += usize::from(rename);
                !rename
            });
        });
        assert_eq!(removed, 1);
        assert!(run_producer(&broken, expected, false).is_none());
    }
}

fn emitted_piece(surface: &ApiSurface) -> syn::ItemEnum {
    let files = crate::backends::magnus::MagnusBackend
        .generate_bindings(
            surface,
            &ResolvedCrateConfig {
                name: "sample".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    let source = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    syn::parse_file(&source.content)
        .unwrap()
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "Piece" => Some(item),
            _ => None,
        })
        .expect("actual Magnus generated Piece enum")
}

fn render_assertions(surface: &ApiSurface) -> String {
    let resolver = support::resolver(surface, "ruby").with_ruby_hash_serialized_enum_names(
        super::super::enum_variant_access::hash_serialized_enum_names(&surface.enums),
    );
    let mut rendered = String::new();
    for mut assertion in support::assertions() {
        assertion.field = assertion.field.map(|field| field.replace(".text", ".text_value"));
        super::super::assertions::render_assertion(
            &mut rendered,
            &assertion,
            "result",
            &resolver,
            false,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
    }
    assert_eq!(rendered.matches("expect(").count(), 3, "{rendered}");
    rendered
}

fn native_cases(payload: serde_json::Value, wire_key: &str) -> [(serde_json::Value, &'static str); 6] {
    let mut wrong_count = payload.clone();
    wrong_count.as_array_mut().unwrap().pop();
    let mut wrong_first = payload.clone();
    wrong_first[0][wire_key] = serde_json::json!("bad");
    let mut wrong_second = payload.clone();
    wrong_second[1][wire_key] = serde_json::json!("bad");
    [
        (payload, "PASS:3"),
        (wrong_count, "ASSERTION_FAILURE:1:"),
        (wrong_first, "ASSERTION_FAILURE:2:"),
        (wrong_second, "ASSERTION_FAILURE:3:"),
        (serde_json::json!("ab"), "expected enum collection payload"),
        (
            serde_json::json!({"first":1,"second":2}),
            "expected enum collection payload",
        ),
    ]
}

fn run_producer(declaration: &syn::ItemEnum, wire_key: &str, success: bool) -> Option<serde_json::Value> {
    let directory = tempfile::tempdir().unwrap();
    let source = crate::e2e::template_env::render(
        "ruby/enum_wire_producer_probe.rs.jinja",
        crate::alef_context! { declaration => declaration.to_token_stream().to_string(),
        wire_key => format!("{wire_key:?}") },
    );
    let path = directory.path().join("probe.rs");
    let binary = directory.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&path, source).unwrap();
    let compiler = which::which_in("rustc", std::env::var_os("PATH"), directory.path()).unwrap();
    let mut command = std::process::Command::new(&compiler);
    command
        .arg("--edition=2024")
        .arg(path)
        .arg("-o")
        .arg(&binary)
        .current_dir(directory.path());
    for name in ["serde", "serde_json"] {
        let library = library(name, &compiler, directory.path());
        command.arg("--extern").arg(format!("{name}={}", library.display()));
    }
    let compiled = add_dependency_search_paths(&mut command).output().unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let output = std::process::Command::new(binary)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if !success {
        assert!(String::from_utf8_lossy(&output.stderr).contains("canonical wire field missing"));
        return None;
    }
    Some(serde_json::from_slice(&output.stdout).unwrap())
}

fn library(name: &str, compiler: &Path, directory: &Path) -> PathBuf {
    select_library(&dependency_rlibs(name), compiler, directory)
}

fn select_library(candidates: &[PathBuf], compiler: &Path, directory: &Path) -> PathBuf {
    assert!(!candidates.is_empty(), "no dependency libraries found");
    let mut failures = Vec::new();
    for candidate in candidates {
        let output = dependency_probe(candidate, compiler, directory);
        if output.status.success() {
            return candidate.clone();
        }
        failures.push(format!(
            "{}: {}",
            candidate.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    panic!("no compiler-compatible dependency library: {}", failures.join("\n"));
}

fn dependency_probe(library: &Path, compiler: &Path, directory: &Path) -> std::process::Output {
    let source = directory.join("dependency_probe.rs");
    std::fs::write(&source, "extern crate probe_dependency;\n").unwrap();
    add_dependency_search_paths(&mut std::process::Command::new(compiler))
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--crate-name=alef_dependency_probe",
            "--emit=metadata",
        ])
        .arg(source)
        .arg("--extern")
        .arg(format!("probe_dependency={}", library.display()))
        .arg("-o")
        .arg(directory.join("dependency_probe.rmeta"))
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn dependency_probe_rejects_invalid_candidates_before_selecting_a_library() {
    let directory = tempfile::tempdir().unwrap();
    let compiler = which::which_in("rustc", std::env::var_os("PATH"), directory.path()).unwrap();
    let valid = library("serde_json", &compiler, directory.path());
    let invalid = directory.path().join("libserde_json-invalid.rlib");
    std::fs::write(&invalid, "invalid Rust library metadata").unwrap();
    let rejection = dependency_probe(&invalid, &compiler, directory.path());
    assert!(
        !rejection.status.success(),
        "invalid dependency metadata must fail compilation"
    );
    assert!(String::from_utf8_lossy(&rejection.stderr).contains("libserde_json-invalid.rlib"));
    let candidates = [invalid, valid.clone()];
    assert_eq!(candidates.len(), 2);
    assert_eq!(select_library(&candidates, &compiler, directory.path()), valid);
}
