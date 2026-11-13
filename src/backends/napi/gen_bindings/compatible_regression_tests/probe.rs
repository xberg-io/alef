//! Native Rust probes of real emitted conversion items; no Node ABI qualification.

use crate::core::ir::ApiSurface;
use crate::test_support::cargo_artifacts::{add_dependency_search_paths, dependency_rlibs};
use quote::ToTokens;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(super) fn extract(source: &str) -> ApiSurface {
    let directory = tempfile::tempdir().expect("core source fixture");
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, source).expect("write core source");
    crate::extract::extractor::extract(&[path.as_path()], "test_lib", "0.1.0", None).expect("extract actual core API")
}

fn strip_napi_attributes(attributes: &mut Vec<syn::Attribute>) {
    attributes.retain(|attribute| !attribute.path().is_ident("napi"));
}

fn projected_item(item: syn::Item) -> Option<syn::Item> {
    match item {
        syn::Item::Struct(mut item) => {
            strip_napi_attributes(&mut item.attrs);
            for field in &mut item.fields {
                strip_napi_attributes(&mut field.attrs);
            }
            Some(syn::Item::Struct(item))
        }
        syn::Item::Enum(mut item) => {
            strip_napi_attributes(&mut item.attrs);
            for variant in &mut item.variants {
                strip_napi_attributes(&mut variant.attrs);
                for field in &mut variant.fields {
                    strip_napi_attributes(&mut field.attrs);
                }
            }
            Some(syn::Item::Enum(item))
        }
        syn::Item::Impl(item)
            if item.trait_.as_ref().is_some_and(|(path, _)| {
                path.segments
                    .last()
                    .is_some_and(|segment| matches!(segment.ident.to_string().as_str(), "From" | "TryFrom"))
            }) =>
        {
            Some(syn::Item::Impl(item))
        }
        syn::Item::Fn(item) if item.sig.ident.to_string().starts_with("__alef_wire") => Some(syn::Item::Fn(item)),
        _ => None,
    }
}

pub(super) fn conversion_source(generated: &str, core_source: &str, driver: &str) -> String {
    let parsed = syn::parse_file(generated).expect("the complete generated Rust must parse");
    let items: Vec<_> = parsed.items.into_iter().filter_map(projected_item).collect();
    let implementations = items.iter().filter(|item| matches!(item, syn::Item::Impl(_))).count();
    assert!(implementations >= 2, "both real conversion directions must be selected");
    let declarations = items
        .iter()
        .map(ToTokens::to_token_stream)
        .map(|tokens| tokens.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    format!("use std::collections::HashMap;\nmod test_lib {{\n{core_source}\n}}\n{declarations}\n{driver}\n")
}

fn compile(directory: &Path, source: &Path, binary: &Path, serde: &Path, json: &Path) -> Output {
    add_dependency_search_paths(&mut Command::new("rustc"))
        .current_dir(directory)
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("serde={}", serde.display()))
        .arg("--extern")
        .arg(format!("serde_json={}", json.display()))
        .arg(source)
        .arg("-o")
        .arg(binary)
        .output()
        .expect("execute committed rustc")
}

fn compatible_dependencies(directory: &Path) -> (PathBuf, PathBuf) {
    let source = directory.join("dependency_control.rs");
    let binary = directory.join(format!("dependency_control{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&source, r#"#[derive(serde::Serialize, serde::Deserialize)] struct Control { value: serde_json::Value } fn main() { let value = Control { value: serde_json::json!({"ok": true}) }; assert_eq!(serde_json::to_value(value).unwrap()["value"]["ok"], true); }"#).expect("write positive dependency control");
    let mut last_error = String::new();
    for serde in dependency_rlibs("serde") {
        for json in dependency_rlibs("serde_json") {
            let compiled = compile(directory, &source, &binary, &serde, &json);
            if compiled.status.success() {
                let control = Command::new(&binary)
                    .current_dir(directory)
                    .output()
                    .expect("execute positive dependency control");
                assert!(control.status.success(), "{}", String::from_utf8_lossy(&control.stderr));
                return (serde, json);
            }
            last_error = String::from_utf8_lossy(&compiled.stderr).into_owned();
        }
    }
    panic!("positive serde dependency control did not compile: {last_error}");
}

pub(super) fn run(source: &str) -> String {
    let directory = tempfile::tempdir().expect("native generated conversion probe");
    let (serde, json) = compatible_dependencies(directory.path());
    let path = directory.path().join("generated.rs");
    let binary = directory
        .path()
        .join(format!("generated{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&path, source).expect("write actual generated conversion source");
    let result = compile(directory.path(), &path, &binary, &serde, &json);
    assert!(
        result.status.success(),
        "actual emitted conversions failed to compile:\n{}\nSOURCE:\n{source}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(binary)
        .current_dir(directory.path())
        .output()
        .expect("run emitted conversions");
    assert!(
        result.status.success(),
        "actual conversion round trip failed:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).expect("JSON probe output is UTF-8")
}
