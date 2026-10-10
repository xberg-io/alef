use super::*;
use base64::Engine as _;

fn component_config(extra: &str) -> NewAlefConfig {
    toml::from_str(&format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OcrBackend"

[[crates.components]]
name = "tesseract"
provides = [{{ contract = "ocr", implementation = "sample_components::TesseractBackend" }}]
features = ["ocr-tesseract"]
targets = ["x86_64-unknown-linux-gnu"]

[crates.component_distribution]
url_template = "https://downloads.example.test/{{component}}/{{version}}/{{target}}/{{artifact}}"

[crates.component_distribution.public_keys]
release-2026 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
{extra}
"#
    ))
    .expect("component config should deserialize")
}

#[test]
fn resolve_preserves_component_configuration_and_defaults() {
    let resolved = component_config("").resolve().unwrap().remove(0);

    assert_eq!(resolved.component_contracts.len(), 1);
    assert_eq!(resolved.component_contracts[0].name, "ocr");
    assert_eq!(resolved.component_contracts[0].interface_version, 1);
    assert_eq!(resolved.components.len(), 1);
    assert_eq!(resolved.components[0].provides.len(), 1);
    assert_eq!(resolved.components[0].provides[0].contract, "ocr");
    assert_eq!(
        resolved.components[0].provides[0].implementation,
        "sample_components::TesseractBackend"
    );
    assert_eq!(resolved.components[0].features, ["ocr-tesseract"]);
    assert!(!resolved.components[0].default_features);
    assert_eq!(
        resolved.components[0].targets,
        Some(vec!["x86_64-unknown-linux-gnu".to_string()])
    );
    assert!(resolved.components[0].bundled_on.is_empty());
    assert_eq!(
        resolved
            .component_distribution
            .unwrap()
            .public_keys
            .get("release-2026")
            .map(String::as_str),
        Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
    );
}

#[test]
fn resolve_allows_components_without_remote_distribution() {
    let mut config = component_config("");
    config.crates[0].component_distribution = None;

    let resolved = config.resolve().unwrap().remove(0);
    assert_eq!(resolved.components.len(), 1);
    assert!(resolved.component_distribution.is_none());
}

#[test]
fn resolve_rejects_duplicate_component_contract_names() {
    let config = component_config(
        r#"
[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OtherOcrBackend"
"#,
    );

    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("duplicate component contract `ocr`"), "{error}");
}

#[test]
fn resolve_rejects_duplicate_component_names() {
    let config = component_config(
        r#"
[[crates.components]]
name = "tesseract"
provides = [{ contract = "ocr", implementation = "sample_components::OtherBackend" }]
features = ["ocr-other"]
targets = ["aarch64-apple-darwin"]
"#,
    );

    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("duplicate component `tesseract`"), "{error}");
}

#[test]
fn resolve_rejects_component_with_no_provides_entries() {
    let config = component_config("");
    let mut config = config;
    config.crates[0].components[0].provides.clear();

    let error = config.resolve().unwrap_err().to_string();
    assert!(
        error.contains("must declare at least one entry in `provides`"),
        "{error}"
    );
}

#[test]
fn resolve_rejects_a_component_providing_the_same_contract_twice() {
    let mut config = component_config("");
    let duplicate = config.crates[0].components[0].provides[0].clone();
    config.crates[0].components[0].provides.push(duplicate);

    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("provides contract `ocr` more than once"), "{error}");
}

#[test]
fn resolve_rejects_invalid_component_identifiers_and_interface_version() {
    let mut invalid_contract_name = component_config("");
    invalid_contract_name.crates[0].component_contracts[0].name = "bad/name".to_string();
    let error = invalid_contract_name.resolve().unwrap_err().to_string();
    assert!(error.contains("contract name `bad/name`"), "{error}");

    let mut invalid_component_name = component_config("");
    invalid_component_name.crates[0].components[0].name = "bad name".to_string();
    let error = invalid_component_name.resolve().unwrap_err().to_string();
    assert!(error.contains("component name `bad name`"), "{error}");

    let mut zero_interface_version = component_config("");
    zero_interface_version.crates[0].component_contracts[0].interface_version = 0;
    let error = zero_interface_version.resolve().unwrap_err().to_string();
    assert!(error.contains("interface_version must be greater than zero"), "{error}");
}

#[test]
fn resolve_rejects_component_with_unknown_contract() {
    let mut config = component_config("");
    config.crates[0].components[0].provides[0].contract = "missing".to_string();

    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("references unknown contract `missing`"), "{error}");
}

#[test]
fn resolve_rejects_invalid_component_rust_paths() {
    let mut invalid_trait = component_config("");
    invalid_trait.crates[0].component_contracts[0].trait_path = "OcrBackend".to_string();
    let error = invalid_trait.resolve().unwrap_err().to_string();
    assert!(error.contains("trait_path `OcrBackend`"), "{error}");

    let mut invalid_implementation = component_config("");
    invalid_implementation.crates[0].components[0].provides[0].implementation = "sample::bad-path".to_string();
    let error = invalid_implementation.resolve().unwrap_err().to_string();
    assert!(error.contains("implementation `sample::bad-path`"), "{error}");
}

#[test]
fn resolve_rejects_empty_component_features() {
    let mut empty_features = component_config("");
    empty_features.crates[0].components[0].features.clear();
    let error = empty_features.resolve().unwrap_err().to_string();
    assert!(error.contains("must declare non-empty features"), "{error}");
}

#[test]
fn resolve_accepts_an_explicit_empty_target_list() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = Some(Vec::new());

    let resolved = config.resolve().unwrap().remove(0);
    assert_eq!(resolved.components[0].targets, Some(Vec::new()));
}

#[test]
fn resolve_rejects_component_targets_the_v1_loader_cannot_load() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = Some(vec!["aarch64-apple-ios".to_string()]);

    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("unsupported v1 target `aarch64-apple-ios`"), "{error}");
}

#[test]
fn resolve_rejects_a_target_listed_in_both_targets_and_bundled_on() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = Some(vec!["x86_64-unknown-linux-gnu".to_string()]);
    config.crates[0].components[0].bundled_on = vec!["x86_64-unknown-linux-gnu".to_string()];

    let error = config.resolve().unwrap_err().to_string();
    assert!(
        error.contains("target `x86_64-unknown-linux-gnu` is listed in both `targets` and `bundled_on`"),
        "{error}"
    );
}

#[test]
fn default_component_targets_come_from_the_crate_targets_table_minus_bundled_on() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = None;
    config.crates[0].components[0].bundled_on = vec!["aarch64-apple-darwin".to_string()];
    config.crates[0].targets.insert("mac_intel".to_string(), false);

    let resolved = config.resolve().unwrap().remove(0);
    let targets = crate::codegen::component::resolve_component_targets(&resolved, &resolved.components[0]);
    assert!(!targets.iter().any(|target| target == "x86_64-apple-darwin"));
    assert!(!targets.iter().any(|target| target == "aarch64-apple-darwin"));
    assert!(targets.iter().any(|target| target == "x86_64-unknown-linux-gnu"));
}

#[test]
fn resolve_rejects_invalid_component_distribution_url_template() {
    let mut insecure = component_config("");
    insecure.crates[0].component_distribution.as_mut().unwrap().url_template =
        "http://downloads.test/{component}/{version}/{target}/{artifact}".to_string();
    let error = insecure.resolve().unwrap_err().to_string();
    assert!(error.contains("must use HTTPS"), "{error}");

    let mut missing_placeholder = component_config("");
    missing_placeholder.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .url_template = "https://downloads.test/{component}/{version}/{target}".to_string();
    let error = missing_placeholder.resolve().unwrap_err().to_string();
    assert!(error.contains("must contain `{artifact}`"), "{error}");

    let mut missing_host = component_config("");
    missing_host.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .url_template = "https:///{component}/{version}/{target}/{artifact}".to_string();
    let error = missing_host.resolve().unwrap_err().to_string();
    assert!(error.contains("must use HTTPS"), "{error}");
}

#[test]
fn resolve_rejects_invalid_component_public_keys() {
    let mut empty_keys = component_config("");
    empty_keys.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .public_keys
        .clear();
    let error = empty_keys.resolve().unwrap_err().to_string();
    assert!(error.contains("at least one public key"), "{error}");

    let mut invalid_key = component_config("");
    invalid_key.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .public_keys
        .insert("release".to_string(), "not-base64".to_string());
    let error = invalid_key.resolve().unwrap_err().to_string();
    assert!(error.contains("must be a PEM or base64-encoded"), "{error}");

    let mut wrong_length = component_config("");
    wrong_length.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .public_keys
        .insert("release".to_string(), "YQ==".to_string());
    let error = wrong_length.resolve().unwrap_err().to_string();
    assert!(error.contains("must be a PEM or base64-encoded"), "{error}");
}

/// Regression: this validation used to accept `base64::STANDARD` or
/// `base64::STANDARD_NO_PAD`, but `alef-component-runtime`'s loader and `alef`'s artifact
/// verification accepted only `base64::STANDARD`, so an unpadded key would validate here
/// and then fail to load or verify. All three now share one decoder.
#[test]
fn resolve_accepts_unpadded_base64_component_public_key() {
    let mut config = component_config("");
    config.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .public_keys
        .insert(
            "release-unpadded".to_string(),
            base64::engine::general_purpose::STANDARD_NO_PAD.encode([0u8; 32]),
        );

    let resolved = config.resolve().unwrap().remove(0);
    assert!(
        resolved
            .component_distribution
            .unwrap()
            .public_keys
            .contains_key("release-unpadded")
    );
}

#[test]
fn workspace_component_distribution_merges_field_by_field_with_crate_override() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi"]

[component_distribution]
url_template = "https://workspace.example.test/{component}/{version}/{target}/{artifact}"

[component_distribution.public_keys]
workspace-key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OcrBackend"

[[crates.components]]
name = "tesseract"
provides = [{ contract = "ocr", implementation = "sample_components::TesseractBackend" }]
features = ["ocr-tesseract"]
targets = ["x86_64-unknown-linux-gnu"]

[crates.component_distribution.public_keys]
crate-key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#,
    )
    .unwrap();

    let resolved = config.resolve().unwrap().remove(0);
    let distribution = resolved.component_distribution.unwrap();
    // The crate table declares no url_template of its own, so the workspace default survives.
    assert_eq!(
        distribution.url_template,
        "https://workspace.example.test/{component}/{version}/{target}/{artifact}"
    );
    // Public keys merge: the workspace key is kept, the crate key is added.
    assert_eq!(distribution.public_keys.len(), 2);
    assert!(distribution.public_keys.contains_key("workspace-key"));
    assert!(distribution.public_keys.contains_key("crate-key"));
}

#[test]
fn crate_component_distribution_url_template_overrides_workspace_default() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi"]

[component_distribution]
url_template = "https://workspace.example.test/{component}/{version}/{target}/{artifact}"

[component_distribution.public_keys]
workspace-key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OcrBackend"

[[crates.components]]
name = "tesseract"
provides = [{ contract = "ocr", implementation = "sample_components::TesseractBackend" }]
features = ["ocr-tesseract"]
targets = ["x86_64-unknown-linux-gnu"]

[crates.component_distribution]
url_template = "https://crate.example.test/{component}/{version}/{target}/{artifact}"

[crates.component_distribution.public_keys]
crate-key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#,
    )
    .unwrap();

    let resolved = config.resolve().unwrap().remove(0);
    let distribution = resolved.component_distribution.unwrap();
    assert_eq!(
        distribution.url_template,
        "https://crate.example.test/{component}/{version}/{target}/{artifact}"
    );
    assert!(distribution.public_keys.contains_key("workspace-key"));
    assert!(distribution.public_keys.contains_key("crate-key"));
}

#[test]
fn generated_schema_contains_component_configuration() {
    let schema = crate::core::config::alef_config_schema("test").unwrap();
    let rendered = serde_json::to_string(&schema).unwrap();

    for expected in [
        "component_contracts",
        "components",
        "component_distribution",
        "interface_version",
        "implementation",
        "public_keys",
        "provides",
        "bundled_on",
    ] {
        assert!(rendered.contains(expected), "schema is missing `{expected}`");
    }
}
