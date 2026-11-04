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
fn source_manifest_components_may_omit_features() {
    let mut config = component_config("");
    let component = &mut config.crates[0].components[0];
    component.features.clear();
    component.source_manifest = Some("components/crates/x-web/Cargo.toml".into());
    component.producer_dir = Some("components/crates/x-web-component".into());
    config.resolve().unwrap();

    let mut blank = component_config("");
    let component = &mut blank.crates[0].components[0];
    component.features = vec![String::new()];
    component.source_manifest = Some("components/crates/x-web/Cargo.toml".into());
    let error = blank.resolve().unwrap_err().to_string();
    assert!(error.contains("features"), "{error}");
}

#[test]
fn features_default_to_empty_when_omitted() {
    let component: crate::core::config::ComponentConfig = toml::from_str(
        r#"
name = "web"
provides = [{ contract = "ocr", implementation = "x_web::Web" }]
source_manifest = "components/crates/x-web/Cargo.toml"
"#,
    )
    .unwrap();
    assert!(component.features.is_empty());
}

#[test]
fn resolve_accepts_an_explicit_empty_target_list() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = Some(Vec::new());

    let resolved = config.resolve().unwrap().remove(0);
    assert_eq!(resolved.components[0].targets, Some(Vec::new()));
}

#[test]
fn resolve_rejects_malformed_component_target_triples() {
    for bad in [
        "ios",
        "x86_64-Linux-gnu",
        "x86_64--linux",
        "a-b-c-d-e-f",
        "x86 64-unknown-linux",
    ] {
        let mut config = component_config("");
        config.crates[0].components[0].targets = Some(vec![bad.to_string()]);

        let error = config.resolve().unwrap_err().to_string();
        assert!(
            error.contains(&format!("target `{bad}` is not a valid Rust target triple")),
            "{bad}: {error}"
        );
    }
}

#[test]
fn resolve_accepts_component_targets_outside_the_default_matrix() {
    let mut config = component_config("");
    config.crates[0].components[0].targets = Some(vec![
        "aarch64-pc-windows-msvc".to_string(),
        "x86_64-unknown-linux-musl".to_string(),
    ]);

    let resolved = config.resolve().unwrap().remove(0);
    let targets = crate::codegen::component::resolve_component_targets(&resolved, &resolved.components[0]);
    assert_eq!(targets, vec!["aarch64-pc-windows-msvc", "x86_64-unknown-linux-musl"]);
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
    let mut file_url = component_config("");
    file_url.crates[0].component_distribution.as_mut().unwrap().url_template =
        "file:///tmp/dist/components/{artifact}".to_string();
    file_url
        .resolve()
        .expect("file:// url_template is allowed for dev locks");

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
fn resolve_preserves_keyless_component_distribution() {
    let mut empty_keys = component_config("");
    empty_keys.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .public_keys
        .clear();
    empty_keys.crates[0]
        .component_distribution
        .as_mut()
        .unwrap()
        .artifact_name_template = Some("dev-{component}-{target}.tar.gz".into());
    let resolved = empty_keys.resolve().unwrap();
    let distribution = resolved[0].component_distribution.as_ref().unwrap();
    assert!(distribution.public_keys.is_empty());
    assert_eq!(
        distribution.artifact_name_template.as_deref(),
        Some("dev-{component}-{target}.tar.gz")
    );
}

#[test]
fn resolve_rejects_invalid_component_public_keys() {
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

#[test]
fn url_template_requires_only_the_artifact_placeholder() {
    let mut config = component_config("");
    config.crates[0].component_distribution.as_mut().unwrap().url_template =
        "https://github.com/owner/repo/releases/download/v{version}/{artifact}".to_string();
    config
        .resolve()
        .expect("a template with only {artifact} plus version must resolve");

    config.crates[0].component_distribution.as_mut().unwrap().url_template =
        "https://github.com/owner/repo/releases/download/{artifact}".to_string();
    config.resolve().expect("a template with only {artifact} must resolve");
}

#[test]
fn artifact_name_template_is_validated() {
    let set = |template: &str| {
        let mut config = component_config("");
        config.crates[0]
            .component_distribution
            .as_mut()
            .unwrap()
            .artifact_name_template = Some(template.to_string());
        config.resolve().map(|_| ()).map_err(|error| error.to_string())
    };
    set("poly-{component}-{version}-{target}.tar.gz").expect("valid template");
    set("{crate}-{component}-{version}-{fh16}-{ch16}-{target}.tar.gz").expect("default-shaped template");
    assert!(
        set("poly-{component}-{version}.tar.gz")
            .unwrap_err()
            .contains("must contain `{target}`")
    );
    assert!(
        set("poly-{target}.tar.gz")
            .unwrap_err()
            .contains("must contain `{component}`")
    );
    assert!(
        set("{component}-{target}-{bogus}.tar.gz")
            .unwrap_err()
            .contains("unknown placeholder `{bogus}`")
    );
    assert!(
        set("{component}/{target}.tar.gz")
            .unwrap_err()
            .contains("[A-Za-z0-9._-]")
    );
    assert!(set("{component}-{target").unwrap_err().contains("unmatched"));
    assert!(set("{component}-{target}}").unwrap_err().contains("unmatched"));
}

#[test]
fn crate_artifact_name_template_overrides_the_workspace_default() {
    let workspace = ComponentDistributionConfig {
        artifact_name_template: Some("ws-{component}-{target}.tar.gz".into()),
        ..ComponentDistributionConfig::default()
    };
    let krate = ComponentDistributionConfig {
        artifact_name_template: Some("crate-{component}-{target}.tar.gz".into()),
        ..ComponentDistributionConfig::default()
    };
    let merged = ComponentDistributionConfig::merge(Some(&workspace), Some(&krate)).unwrap();
    assert_eq!(
        merged.artifact_name_template.as_deref(),
        Some("crate-{component}-{target}.tar.gz")
    );
    let merged = ComponentDistributionConfig::merge(Some(&workspace), None).unwrap();
    assert_eq!(
        merged.artifact_name_template.as_deref(),
        Some("ws-{component}-{target}.tar.gz")
    );
}

#[test]
fn resolve_allows_components_only_crates_without_languages() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]
languages = []

[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OcrBackend"

[[crates.components]]
name = "tesseract"
provides = [{ contract = "ocr", implementation = "sample_components::TesseractBackend" }]
features = ["ocr-tesseract"]
"#,
    )
    .unwrap();

    let resolved = config.resolve().expect("components-only crate must resolve").remove(0);
    assert!(resolved.languages.is_empty());
    assert_eq!(resolved.components.len(), 1);
}

#[test]
fn component_host_resolves_with_default_lock_path() {
    let resolved = component_config("\n[crates.component_host]\noutput = \"crates/host/src/components_gen.rs\"\n")
        .resolve()
        .unwrap()
        .remove(0);
    let host = resolved.component_host.expect("component_host must resolve");
    assert_eq!(host.output, "crates/host/src/components_gen.rs");
    assert_eq!(host.effective_lock_path(), "components.lock.json");
}

#[test]
fn component_host_rejects_invalid_output_and_missing_components() {
    let error = component_config("\n[crates.component_host]\noutput = \"gen.txt\"\n")
        .resolve()
        .unwrap_err()
        .to_string();
    assert!(error.contains("must end in `.rs`"), "{error}");

    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]
languages = ["ffi"]

[crates.component_host]
output = "gen.rs"
"#,
    )
    .unwrap();
    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("component_host requires"), "{error}");
}

#[test]
fn extra_dependencies_parse_from_toml_and_resolve() {
    let mut config = component_config("");
    config.crates[0].components[0].extra_dependencies.insert(
        "pdf-engine".to_string(),
        serde_json::json!({ "version": "2", "features": ["fast"] }),
    );
    let resolved = config.resolve().unwrap().remove(0);
    assert!(resolved.components[0].extra_dependencies.contains_key("pdf-engine"));

    let parsed: crate::core::config::ComponentConfig = toml::from_str(
        r#"
name = "x"
provides = []
features = []
[extra_dependencies]
serde_json = "1"
engine = { version = "2", features = ["a"] }
"#,
    )
    .unwrap();
    assert_eq!(parsed.extra_dependencies.len(), 2);
    assert_eq!(parsed.extra_dependencies["serde_json"], serde_json::json!("1"));
}

#[test]
fn resolve_rejects_invalid_or_overriding_extra_dependencies() {
    for (name, spec, expected) in [
        ("bad name", serde_json::json!("1"), "must be a Cargo package name"),
        ("futures", serde_json::json!("1"), "would override"),
        ("alef_component_abi", serde_json::json!("1"), "would override"),
        ("sample", serde_json::json!("1"), "would override"),
        ("ok", serde_json::json!(3), "version string or a table"),
    ] {
        let mut config = component_config("");
        config.crates[0].components[0]
            .extra_dependencies
            .insert(name.to_string(), spec);
        let error = config.resolve().unwrap_err().to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn generated_schema_contains_extra_dependencies() {
    let schema = crate::core::config::alef_config_schema("test").unwrap();
    assert!(serde_json::to_string(&schema).unwrap().contains("extra_dependencies"));
}

#[test]
fn source_manifest_and_producer_dir_default_to_unset() {
    let resolved = component_config("").resolve().unwrap().remove(0);
    assert_eq!(resolved.components[0].source_manifest, None);
    assert_eq!(resolved.components[0].producer_dir, None);
    let value = serde_json::to_value(&resolved.components[0]).unwrap();
    assert!(value.get("source_manifest").is_none() && value.get("producer_dir").is_none());
}

#[test]
fn source_manifest_and_producer_dir_parse_and_resolve() {
    let parsed: crate::core::config::ComponentConfig = toml::from_str(
        r#"
name = "web"
provides = []
features = []
source_manifest = "components/crates/x-web/Cargo.toml"
producer_dir = "components/crates/x-web-component"
"#,
    )
    .unwrap();
    assert_eq!(
        parsed.source_manifest.as_deref(),
        Some("components/crates/x-web/Cargo.toml")
    );
    assert_eq!(
        parsed.producer_dir.as_deref(),
        Some("components/crates/x-web-component")
    );

    let mut config = component_config("");
    config.crates[0].components[0].source_manifest = Some("../other/Cargo.toml".into());
    config.crates[0].components[0].producer_dir = Some("components/out".into());
    let resolved = config.resolve().unwrap().remove(0);
    assert_eq!(
        resolved.components[0].source_manifest.as_deref(),
        Some("../other/Cargo.toml")
    );
    assert_eq!(resolved.components[0].producer_dir.as_deref(), Some("components/out"));
}

#[test]
fn resolve_rejects_invalid_source_manifest_and_producer_dir() {
    for (manifest, dir, expected) in [
        (Some("/abs/Cargo.toml"), None, "relative path"),
        (Some("a\\b/Cargo.toml"), None, "relative path"),
        (Some(""), None, "must not be empty"),
        (Some("crates/x"), None, "must point at a `Cargo.toml`"),
        (Some("crates/x/Cargo.lock"), None, "must point at a `Cargo.toml`"),
        (None, Some("../out"), "must not contain `..`"),
        (None, Some("/abs/out"), "relative path"),
        (None, Some("  "), "must not be empty"),
    ] {
        let mut config = component_config("");
        config.crates[0].components[0].source_manifest = manifest.map(str::to_string);
        config.crates[0].components[0].producer_dir = dir.map(str::to_string);
        let error = config.resolve().unwrap_err().to_string();
        assert!(error.contains(expected), "{manifest:?} {dir:?}: {error}");
    }
}

#[test]
fn resolve_rejects_source_manifest_for_go_producers() {
    let mut config = component_config("");
    let component = &mut config.crates[0].components[0];
    component.producer_language = crate::core::config::ComponentProducerLanguage::Go;
    component.provides[0].implementation = "NewTesseract".into();
    component.source_manifest = Some("crates/x/Cargo.toml".into());
    let error = config.resolve().unwrap_err().to_string();
    assert!(error.contains("cannot declare `source_manifest`"), "{error}");
}

#[test]
fn rebase_paths_roots_both_fields_at_the_config_directory() {
    let mut component = component_config("").crates.remove(0).components.remove(0);
    component.source_manifest = Some("components/crates/x-web/Cargo.toml".into());
    component.producer_dir = Some("components/crates/x-web-component".into());

    let mut unchanged = component.clone();
    unchanged.rebase_paths(std::path::Path::new(""));
    assert_eq!(unchanged, component);

    component.rebase_paths(std::path::Path::new(".."));
    assert_eq!(
        component.source_manifest.as_deref(),
        Some("../components/crates/x-web/Cargo.toml")
    );
    assert_eq!(
        component.producer_dir.as_deref(),
        Some("../components/crates/x-web-component")
    );

    let mut unset = component_config("").crates.remove(0).components.remove(0);
    unset.rebase_paths(std::path::Path::new(".."));
    assert_eq!((unset.source_manifest, unset.producer_dir), (None, None));
}

#[test]
fn generated_schema_contains_source_manifest_and_producer_dir() {
    let schema = serde_json::to_string(&crate::core::config::alef_config_schema("test").unwrap()).unwrap();
    assert!(schema.contains("source_manifest") && schema.contains("producer_dir"));
}

fn resolve_error_with(edit: impl FnOnce(&mut NewAlefConfig)) -> String {
    let mut config = component_config("");
    edit(&mut config);
    config.resolve().unwrap_err().to_string()
}

fn second_component(config: &mut NewAlefConfig, producer_dir: Option<&str>) {
    let mut other = config.crates[0].components[0].clone();
    other.name = "other".into();
    other.producer_dir = producer_dir.map(str::to_string);
    config.crates[0].components.push(other);
}

#[test]
fn resolve_rejects_dot_segments_in_producer_dir() {
    for dir in ["./out", "a/./b", "a/."] {
        let error = resolve_error_with(|config| config.crates[0].components[0].producer_dir = Some(dir.into()));
        assert!(error.contains("must not contain `.` segments"), "{dir}: {error}");
    }
}

#[test]
fn resolve_normalises_trailing_slashes_in_producer_dir() {
    let mut config = component_config("");
    config.crates[0].components[0].producer_dir = Some("components/out/".into());
    config.resolve().expect("trailing slash is accepted");
}

#[test]
fn resolve_rejects_duplicate_producer_dirs_including_trailing_slash_spellings() {
    let error = resolve_error_with(|config| {
        config.crates[0].components[0].producer_dir = Some("components/out".into());
        second_component(config, Some("components/out/"));
    });
    assert!(error.contains("resolve to the same producer directory"), "{error}");
}

#[test]
fn resolve_rejects_an_explicit_producer_dir_equal_to_another_components_default() {
    let error = resolve_error_with(|config| {
        config.crates[0].components[0].producer_dir = Some("crates/sample-other-component".into());
        second_component(config, None);
    });
    assert!(
        error.contains("resolve to the same producer directory `crates/sample-other-component`"),
        "{error}"
    );
}

#[test]
fn resolve_rejects_producer_dir_that_is_an_ancestor_of_or_inside_the_core_crate() {
    for dir in [
        "crates/sample",
        "crates",
        "crates/sample/components/out",
        "crates/sample/",
    ] {
        let error = resolve_error_with(|config| {
            config.crates[0].sources = vec!["crates/sample/src/lib.rs".into()];
            config.crates[0].components[0].producer_dir = Some(dir.into());
        });
        assert!(error.contains("core crate directory `crates/sample`"), "{dir}: {error}");
    }
}

#[test]
fn producer_dir_beside_the_core_crate_is_accepted() {
    let mut config = component_config("");
    config.crates[0].sources = vec!["crates/sample/src/lib.rs".into()];
    config.crates[0].components[0].producer_dir = Some("crates/sample-web-component".into());
    config
        .resolve()
        .expect("a sibling directory does not overlap the core crate");
}

#[test]
fn root_flat_core_crate_only_claims_its_src_directory() {
    for dir in ["src", "src/out"] {
        let error = resolve_error_with(|config| config.crates[0].components[0].producer_dir = Some(dir.into()));
        assert!(error.contains("core crate directory `src`"), "{dir}: {error}");
    }
    let mut config = component_config("");
    config.crates[0].components[0].producer_dir = Some("components/out".into());
    config.resolve().expect("a directory away from src is accepted");
}

#[test]
fn resolve_rejects_producer_dir_equal_to_the_source_manifest_directory() {
    let error = resolve_error_with(|config| {
        let component = &mut config.crates[0].components[0];
        component.source_manifest = Some("components/x-web/Cargo.toml".into());
        component.producer_dir = Some("components/x-web/".into());
    });
    assert!(error.contains("must not overlap source_manifest directory"), "{error}");
}

#[test]
fn resolve_rejects_default_producer_dir_overlapping_a_source_manifest() {
    let error = resolve_error_with(|config| {
        config.crates[0].components[0].source_manifest = Some("crates/sample-tesseract-component/Cargo.toml".into());
    });
    assert!(error.contains("source_manifest"), "{error}");
}

#[test]
fn resolve_rejects_producer_dirs_overlapping_another_crates_source_manifest() {
    let error = resolve_error_with(|config| {
        config.crates[0].components[0].producer_dir = Some("implementations/web/build".into());
        let mut sibling = config.crates[0].clone();
        sibling.name = "other".into();
        sibling.components[0].producer_dir = None;
        sibling.components[0].source_manifest = Some("implementations/web/Cargo.toml".into());
        config.crates.push(sibling);
    });
    assert!(error.contains("implementations/web/Cargo.toml"), "{error}");
}
