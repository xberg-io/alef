use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, DefaultValue, TypeRef};

fn surface() -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BlockDetails { pub vendor: String }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct Response {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_details: Option<BlockDetails>,
    #[serde(default)]
    pub retry_enabled: Option<bool>,
}
"#,
    )
    .unwrap();
    alef::extract::extractor::extract(&[&path], "fixture", "1.0.0", None).unwrap()
}

fn config() -> ResolvedCrateConfig {
    let raw: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["dart"]
[[crates]]
name = "fixture"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    raw.resolve().unwrap().remove(0)
}

fn render(surface: &ApiSurface) -> anyhow::Result<String> {
    Ok(alef::backends::dart::DartBackend
        .generate_bindings(surface, &config())?
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n"))
}

#[test]
fn optional_named_and_boolean_extensions_default_to_null_after_extraction() {
    let surface = surface();
    let response = surface.types.iter().find(|ty| ty.name == "Response").unwrap();
    let details = response
        .fields
        .iter()
        .find(|field| field.name == "block_details")
        .unwrap();
    assert_eq!(details.ty, TypeRef::Named("BlockDetails".into()));
    assert!(details.optional);
    assert_eq!(details.typed_default, Some(DefaultValue::Empty));
    let mirror = render(&surface).unwrap();
    for name in ["blockDetails", "retryEnabled"] {
        assert_eq!(mirror.matches(&format!(r#"["Response","{name}","null"]"#)).count(), 1);
    }
    assert!(mirror.contains("pub block_details: Option<BlockDetails>"), "{mirror}");
    assert!(!mirror.contains("#[frb(default = false)]"), "{mirror}");
    let dart = "class Response { const Response({required this.url, required this.blockDetails, required this.retryEnabled,}); }";
    let rewritten = alef::backends::dart::apply_constructor_defaults(dart, &mirror).unwrap();
    assert!(rewritten.contains("this.blockDetails = null,"), "{rewritten}");
    assert!(rewritten.contains("this.retryEnabled = null,"), "{rewritten}");
    assert!(rewritten.contains("required this.url"), "{rewritten}");
}

#[test]
fn explicit_optional_boolean_default_preserves_false() {
    let mut surface = surface();
    let response = surface.types.iter_mut().find(|ty| ty.name == "Response").unwrap();
    response.fields.retain(|field| field.name != "block_details");
    response
        .fields
        .iter_mut()
        .find(|field| field.name == "retry_enabled")
        .unwrap()
        .typed_default = Some(DefaultValue::BoolLiteral(false));
    let mirror = render(&surface).unwrap();
    assert!(mirror.contains(r#"["Response","retryEnabled","false"]"#), "{mirror}");
    assert!(mirror.contains("#[frb(default = false)]"), "{mirror}");
}

#[test]
fn unsupported_required_named_default_is_still_rejected() {
    let mut surface = surface();
    let response = surface.types.iter_mut().find(|ty| ty.name == "Response").unwrap();
    response
        .fields
        .iter_mut()
        .find(|field| field.name == "block_details")
        .unwrap()
        .optional = false;
    let error = render(&surface).unwrap_err().to_string();
    assert!(
        error.contains("Response.block_details") && error.contains("BlockDetails"),
        "{error}"
    );
}

#[test]
fn generated_defaults_allow_legacy_and_explicit_nullable_dart_construction() {
    let mirror = render(&surface()).unwrap();
    let source = r#"
class BlockDetails { final String vendor; const BlockDetails(this.vendor); }
class Response {
  final String url;
  final BlockDetails? blockDetails;
  final bool? retryEnabled;
  const Response({required this.url, required this.blockDetails, required this.retryEnabled,});
}
void main() {
  const legacy = Response(url: 'https://example.test');
  if (legacy.blockDetails != null || legacy.retryEnabled != null) throw StateError('legacy defaults');
  const explicit = Response(url: 'https://example.test', blockDetails: BlockDetails('probe'), retryEnabled: false);
  if (explicit.blockDetails?.vendor != 'probe' || explicit.retryEnabled != false) throw StateError('explicit values');
}
"#;
    let rewritten = alef::backends::dart::apply_constructor_defaults(source, &mirror).unwrap();
    assert_eq!(rewritten.matches("= null,").count(), 2);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.dart");
    std::fs::write(&path, rewritten).unwrap();
    let output = std::process::Command::new("dart").arg(&path).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn optional_boolean_empty_default_is_null_without_frb_false_attribute() {
    let mut surface = surface();
    let response = surface.types.iter_mut().find(|ty| ty.name == "Response").unwrap();
    response.fields.retain(|field| field.name != "block_details");
    let mirror = render(&surface).unwrap();
    assert!(mirror.contains(r#"["Response","retryEnabled","null"]"#), "{mirror}");
    assert!(!mirror.contains("#[frb(default = false)]"), "{mirror}");
}
