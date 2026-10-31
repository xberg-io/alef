use std::collections::{BTreeMap, HashSet};

use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

#[path = "fixtures/serde_skipped_cache_types.rs"]
mod oracle;

fn extract(source: &str) -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).unwrap()
}

fn configuration() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    config.resolve().unwrap().remove(0)
}

#[test]
fn should_observe_rust_serde_skipped_cache_defaults_and_strict_wire_keys() {
    let valid = [
        serde_json::json!({}),
        serde_json::json!({"enabled_processors": ["keep"], "disabled_processors": ["skip"]}),
        serde_json::json!({"only": ["keep"]}),
    ];
    assert_eq!(valid.len(), 3);
    for input in valid {
        let parsed: oracle::ProcessorOptions = serde_json::from_value(input).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.enabled_set.is_none() && parsed.disabled_set.is_none());
    }
    let mut parsed: oracle::ProcessorOptions = serde_json::from_value(serde_json::json!({"only": ["keep"]})).unwrap();
    parsed.enabled_set = Some(HashSet::from(["cached-allowed".to_owned()]));
    parsed.disabled_set = Some(HashSet::from(["cached-denied".to_owned()]));
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        serde_json::json!({"enabled": true, "enabled_processors": ["keep"], "disabled_processors": null})
    );
    let rejected = [
        "enabled_set",
        "disabled_set",
        "old_enabled_set",
        "old_disabled_set",
        "alien",
    ];
    assert_eq!(rejected.len(), 5);
    for key in rejected {
        let input = serde_json::json!({(key): ["untrusted"]});
        let error = serde_json::from_value::<oracle::ProcessorOptions>(input).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{key}: {error}");
    }
}

fn generated_declaration(api: &ApiSurface) -> String {
    let files = SwiftBackend
        .generate_bindings(api, &configuration())
        .expect("valid strict Rust DTO with fully-skipped public caches must generate");
    let marker = "public struct ProcessorOptions:";
    let matching: Vec<_> = files.iter().filter(|file| file.content.contains(marker)).collect();
    assert_eq!(matching.len(), 1, "actual full backend must emit one public DTO facade");
    let source = &matching[0].content;
    let start = source.find(marker).unwrap();
    let end = start + source[start..].find("\n}").expect("outer generated declaration end") + 2;
    source[start..end].to_owned()
}

fn assert_directional_skip_stays_unsupported() {
    for attribute in ["skip_deserializing", "skip_serializing"] {
        let source = format!(
            concat!(
                "#[derive(Clone, serde::Serialize, serde::Deserialize)] #[serde(deny_unknown_fields)] ",
                "pub struct Directional {{ pub name: String, #[serde({attribute})] pub cache: Option<String> }}"
            ),
            attribute = attribute,
        );
        let api = extract(&source);
        assert_eq!(api.types.len(), 1);
        let error = SwiftBackend
            .generate_bindings(&api, &configuration())
            .expect_err("directional skip must remain unsupported until its distinct wire policy is modeled");
        assert!(error.to_string().contains(attribute), "{error}");
    }
}

fn assert_serialized_keys(declaration: &str) {
    let keys = declaration
        .split("enum CodingKeys:")
        .nth(1)
        .expect("real generated wire keys");
    let cases: Vec<_> = keys
        .split("\n    }")
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("case "))
        .collect();
    assert_eq!(cases.len(), 3, "exactly three actual CodingKey declarations");
    let keys: BTreeMap<_, _> = cases
        .into_iter()
        .map(|case| match case.split_once('=') {
            Some((name, value)) => (
                name.trim(),
                value
                    .trim()
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .expect("explicit string CodingKey wire value"),
            ),
            None => (case, case),
        })
        .collect();
    assert_eq!(
        keys,
        BTreeMap::from([
            ("enabled", "enabled"),
            ("enabledProcessors", "enabled_processors"),
            ("disabledProcessors", "disabled_processors")
        ])
    );
}

#[test]
fn should_generate_strict_dtos_with_public_caches_but_without_skipped_wire_keys() {
    let api = extract(include_str!("fixtures/serde_skipped_cache_types.rs"));
    assert_eq!(api.types.len(), 1);
    let ty = &api.types[0];
    assert_eq!(ty.name, "ProcessorOptions");
    assert!(ty.serde_deserialization.deny_unknown_fields && !ty.has_private_fields);
    assert_eq!(ty.fields.len(), 5);
    assert!(ty.fields.iter().all(|field| !field.binding_excluded));
    assert_eq!(ty.fields.iter().filter(|field| field.serde_skip).count(), 2);
    let declaration = generated_declaration(&api);
    for property in [
        "enabled: Bool",
        "enabledProcessors: [String]?",
        "disabledProcessors: [String]?",
        "enabledSet: [String]?",
        "disabledSet: [String]?",
    ] {
        assert!(declaration.contains(&format!("public let {property}")), "{declaration}");
    }
    assert_serialized_keys(&declaration);
    assert!(declaration.contains("self.enabledSet = nil"), "{declaration}");
    assert!(declaration.contains("self.disabledSet = nil"), "{declaration}");
    assert!(!declaration.contains("old_enabled_set") && !declaration.contains("old_disabled_set"));
    assert_directional_skip_stays_unsupported();
}
