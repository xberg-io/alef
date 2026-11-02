use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};

fn fixture_api() -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, include_str!("fixtures/serde_policy_inventory.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).unwrap();
    assert_eq!(api.types.len(), 4, "actual public source structs");
    assert_eq!(
        api.errors.len(),
        1,
        "thiserror source routes to errors, not ordinary enums"
    );
    assert_eq!(api.errors[0].name, "AlefWireKey");
    assert_eq!(api.enums.len(), 1);
    assert_eq!(api.enums[0].name, "Mode");
    assert!(api.enums[0].has_serde);
    assert!(!api.types.iter().find(|ty| ty.name == "AlefWireKey1").unwrap().has_serde);
    let owner = api.types.iter().find(|ty| ty.name == "Owner").unwrap();
    assert!(owner.serde_deserialization.deny_unknown_fields);
    assert_eq!(owner.fields[0].ty, TypeRef::Named("Mode".to_owned()));
    api
}

fn configuration(exclusions: &[&str]) -> ResolvedCrateConfig {
    let encoded = serde_json::to_string(exclusions).unwrap();
    let source = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\n\
         sources = [\"src/lib.rs\"]\n[crates.swift]\nexclude_fields = {encoded}\n"
    );
    let parsed: NewAlefConfig = toml::from_str(&source).unwrap();
    let resolved = parsed.resolve().unwrap().remove(0);
    assert_eq!(
        resolved.swift.as_ref().unwrap().exclude_fields,
        exclusions.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>()
    );
    resolved
}

fn generated_swift(api: &ApiSurface) -> String {
    let files = SwiftBackend.generate_bindings(api, &configuration(&[])).unwrap();
    let source: Vec<_> = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .map(|file| file.content.as_str())
        .collect();
    assert!(!source.is_empty(), "actual Swift output must exist");
    source.join("\n")
}

fn declaration<'a>(source: &'a str, marker: &str) -> &'a str {
    assert_eq!(
        source.matches(marker).count(),
        1,
        "unique actual generated declaration {marker}"
    );
    let start = source.find(marker).unwrap();
    let end = start + source[start..].find("\n}").expect("outer declaration closing brace") + 2;
    &source[start..end]
}

#[test]
fn should_use_canonical_field_keys_for_non_snake_case_source_exclusions() {
    let api = fixture_api();
    let strict = api.types.iter().find(|ty| ty.name == "Strict").unwrap();
    assert!(strict.serde_deserialization.deny_unknown_fields);
    assert_eq!(strict.fields.len(), 1);
    assert_eq!(
        strict.fields[0].name, "cacheDir",
        "retain actual Rust spelling before canonicalization"
    );
    assert!(!strict.fields[0].binding_excluded);
    assert!(
        generated_swift(&api).contains("public struct Strict:"),
        "unexcluded source generates"
    );
    let result = SwiftBackend.generate_bindings(&api, &configuration(&["Strict.cache_dir"]));
    let error = result.expect_err("canonical qualified exclusions must reject the unexposed strict field");
    let message = error.to_string();
    assert!(
        message.contains("Strict") && message.contains("unexposed fields"),
        "{message}"
    );
}

fn emitted_public_names(source: &str) -> Vec<&'static str> {
    for marker in [
        "public enum AlefWireKey: Swift.Error",
        "public typealias AlefWireKey1 = RustBridge.AlefWireKey1",
        "public struct AlefWireKey2:",
        "public enum Mode",
        "public struct Owner:",
        "public struct Strict:",
    ] {
        assert!(
            source.contains(marker),
            "actual emitted public inventory missing {marker}"
        );
    }
    vec!["AlefWireKey", "AlefWireKey1", "AlefWireKey2", "Mode", "Owner", "Strict"]
}

#[test]
fn should_reserve_emitted_errors_aliases_and_dtos_for_private_decoder_keys() {
    let source = generated_swift(&fixture_api());
    let public_names = emitted_public_names(&source);
    let error = declaration(&source, "public enum AlefWireKey: Swift.Error");
    assert!(error.contains("case validation(message: String, source: String)"));
    let initializer = declaration(&source, "internal extension Owner {");
    assert!(
        initializer.contains("throw AlefWireKey.validation("),
        "the actual unit-enum FFI conversion uses the public error, so helper shadowing affects real code"
    );
    let owner = declaration(&source, "public struct Owner:");
    let helper = owner
        .lines()
        .find(|line| line.trim_start().starts_with("private ") && line.contains(": CodingKey"))
        .expect("actual emitted decoder key type");
    let helper_name = helper.split(':').next().unwrap().split_whitespace().last().unwrap();
    assert!(
        !public_names.contains(&helper_name),
        "private decoder key {helper_name} shadows actual emitted public names {public_names:?}"
    );
}
