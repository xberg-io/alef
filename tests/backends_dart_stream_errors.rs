#[path = "support/mise_task.rs"]
mod mise_task;
#[path = "backends_dart_stream_errors/native.rs"]
mod native;

use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, Language, NewAlefConfig};
use alef::core::ir::ApiSurface;
use std::path::Path;

const CORE: &str = include_str!("fixtures/dart_stream_errors/core.rs");

fn bridge(root: &Path, known_error: bool) -> String {
    bridge_for_error(root, known_error, "StreamError")
}

fn bridge_for_error(root: &Path, known_error: bool, error_type: &str) -> String {
    bridge_with_skip(root, known_error, error_type, false)
}

fn bridge_with_skip(root: &Path, known_error: bool, error_type: &str, skip_dart: bool) -> String {
    bridge_with_options(root, known_error, error_type, skip_dart, None)
}

fn bridge_with_options(
    root: &Path,
    known_error: bool,
    error_type: &str,
    skip_dart: bool,
    owner_cfg: Option<&str>,
) -> String {
    let input = root.join("lib.rs");
    std::fs::write(&input, CORE).unwrap();
    let mut api: ApiSurface = alef::extract::extractor::extract(&[&input], "stream-sample", "1.0.0", None).unwrap();
    let client = api.types.iter_mut().find(|ty| ty.name == "Client").unwrap();
    client.cfg = owner_cfg.map(str::to_owned);
    client.is_opaque = true;
    client.is_clone = true;
    for method in &mut client.methods {
        method.sanitized = true;
    }
    assert_eq!(api.errors.len(), 1);
    assert!(
        api.errors[0].rust_path.contains("errors::StreamError")
            || api.errors[0].original_rust_path.contains("errors::StreamError"),
        "{:?} {:?}",
        api.errors[0].rust_path,
        api.errors[0].original_rust_path
    );
    api.errors[0].variants.retain(|variant| variant.name != "FutureFailure");
    if !known_error {
        api.errors[0].binding_excluded = true;
    }
    let mut config = configuration(error_type);
    if skip_dart {
        for adapter in &mut config.adapters {
            adapter.skip_languages.push("dart".into());
        }
    }
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    assert!(config.languages.contains(&Language::Dart));
    files
        .into_iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content
}

fn configuration(error_type: &str) -> alef::core::config::ResolvedCrateConfig {
    let parsed: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"stream-sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let mut config = parsed.resolve().unwrap().remove(0);
    config.adapters = [
        ("records", "Chunk"),
        ("scalar_records", "u64"),
        ("optional_records", "Option<u64>"),
        ("vector_records", "Vec<u32>"),
        ("float_records", "Vec<f32>"),
        ("unit_records", "()"),
    ]
    .into_iter()
    .map(|(name, item)| AdapterConfig {
        name: name.into(),
        owner_type: Some("Client".into()),
        pattern: AdapterPattern::Streaming,
        core_path: name.into(),
        item_type: Some(item.into()),
        error_type: Some(error_type.into()),
        params: if name == "records" {
            vec![AdapterParam {
                name: "mode".into(),
                ty: "u32".into(),
                optional: false,
            }]
        } else {
            vec![]
        },
        returns: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: vec![],
    })
    .collect();
    config
}

#[test]
fn streaming_errors_use_the_same_typed_codec_as_non_stream_calls() {
    let root = tempfile::tempdir().unwrap();
    let source = bridge(root.path(), true);
    assert!(source.contains("StreamSink<AlefStreamClientRecords>)"), "{source}");
    assert_eq!(
        source.matches("sink.add(AlefStreamClientRecords::from(error))").count(),
        2
    );
    assert!(!source.contains("sink.add_error(error.to_string())"));
    syn::parse_file(&source).expect("the entire emitted bridge parses as Rust");
}

#[test]
fn excluded_stream_errors_keep_the_string_fallback_codec() {
    let root = tempfile::tempdir().unwrap();
    let source = bridge(root.path(), false);
    assert!(source.contains("StreamSink<Chunk>)"), "{source}");
    assert_eq!(source.matches("sink.add_error(error.to_string())").count(), 12);
    assert!(!source.contains("sink.add_error(AlefBridgeError::from(error))"));
}

#[test]
fn qualified_errors_keep_the_typed_codec_and_unresolved_errors_keep_the_fallback() {
    let root = tempfile::tempdir().unwrap();
    let known = bridge_for_error(root.path(), true, "stream_sample::StreamError");
    assert!(known.contains("StreamSink<AlefStreamClientRecords>)"), "{known}");
    let nested = bridge_for_error(root.path(), true, "stream_sample::errors::StreamError");
    assert!(nested.contains("StreamSink<AlefStreamClientRecords>)"), "{nested}");
    let unresolved = bridge_for_error(root.path(), true, "external::StreamError");
    assert!(unresolved.contains("StreamSink<Chunk>)"), "{unresolved}");
    assert_eq!(unresolved.matches("sink.add_error(error.to_string())").count(), 12);
}

#[test]
fn skipped_dart_adapters_do_not_expose_sink_methods_or_transport_types() {
    let root = tempfile::tempdir().unwrap();
    let source = bridge_with_skip(root.path(), true, "StreamError", true);
    assert!(!source.contains("pub fn records("), "{source}");
    assert!(!source.contains("pub enum AlefStream"), "{source}");
    syn::parse_file(&source).unwrap();
}

#[test]
fn transport_items_inherit_the_widened_owner_cfg() {
    use quote::ToTokens;
    let root = tempfile::tempdir().unwrap();
    let cfg = r#"all(feature = "streaming", not(all(target_os = "android", target_arch = "x86_64")))"#;
    let source = bridge_with_options(root.path(), true, "StreamError", false, Some(cfg));
    let parsed = syn::parse_file(&source).unwrap();
    let mut checked = 0;
    for item in parsed.items {
        let (name, attrs) = match &item {
            syn::Item::Mod(item) => (item.ident.to_string(), &item.attrs),
            syn::Item::Use(item) => (item.tree.to_token_stream().to_string(), &item.attrs),
            syn::Item::Enum(item) => (item.ident.to_string(), &item.attrs),
            syn::Item::Impl(item) => (item.self_ty.to_token_stream().to_string(), &item.attrs),
            syn::Item::Fn(item) => (item.sig.ident.to_string(), &item.attrs),
            _ => continue,
        };
        if !name.contains("alef_stream_")
            && !name.contains("AlefStream")
            && !name.contains("alef_encode_stream_")
            && !name.contains("alef_decode_stream_")
        {
            continue;
        }
        let cfg = attrs.iter().find(|attr| attr.path().is_ident("cfg")).expect(&name);
        let cfg = cfg.to_token_stream().to_string();
        assert!(cfg.contains("streaming"), "{name}: {cfg}");
        assert!(!cfg.contains("android"), "{name}: {cfg}");
        checked += 1;
    }
    assert_eq!(checked, 41);
    native::assert_disabled_transport_compiles(root.path(), &source);
}
