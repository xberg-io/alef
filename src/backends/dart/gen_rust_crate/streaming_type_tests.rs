use crate::backends::dart::DartBackend;
use crate::core::backend::Backend;
use crate::core::config::{AdapterConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, ErrorDef, FunctionDef};

fn assert_stream_type_matches_public_binding(rust: &str, dart: &str) {
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..Default::default()
    };
    let ty = syn::parse_str::<syn::Type>(rust).unwrap();
    let api = ApiSurface {
        crate_name: "sample".into(),
        version: "1.0.0".into(),
        functions: vec![FunctionDef {
            name: "sample_value".into(),
            rust_path: "sample::sample_value".into(),
            return_type: crate::extract::type_resolver::resolve_type(&ty),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let public = files
        .iter()
        .find(|file| file.path.ends_with("lib/src/sample.dart"))
        .expect("the public Dart wrapper was generated");
    let signature = format!("Future<{dart}> sampleValue()");
    assert!(public.content.contains(&signature), "{rust}: {}", public.content);
    let native = stream_transport(rust, &config);
    let annotation = format!("dart_type = \"{dart}\"");
    assert_eq!(native.matches(&annotation).count(), 2, "{rust}: {native}");
}

fn stream_transport(item_type: &str, config: &ResolvedCrateConfig) -> String {
    let adapter: AdapterConfig = serde_json::from_value(serde_json::json!({
        "name": "events", "pattern": "streaming", "core_path": "events",
        "owner_type": "SampleEngine", "item_type": item_type, "error_type": "SampleError",
    }))
    .unwrap();
    let error: ErrorDef = serde_json::from_value(serde_json::json!({
        "name": "SampleError", "rust_path": "sample::SampleError", "variants": [], "doc": "",
    }))
    .unwrap();
    let mut generated = String::new();
    super::emit_transport(&mut generated, &adapter, &error, config, "");
    syn::parse_file(&generated).expect("the streaming transport is valid Rust syntax");
    generated
}

#[test]
fn usize_stream_codec_matches_public_scalar_binding() {
    assert_stream_type_matches_public_binding("usize", "int");
}

#[test]
fn usize_stream_codec_matches_public_nested_binding() {
    for (rust, dart) in [
        ("Option<usize>", "int?"),
        ("Vec<Option<usize>>", "List<int?>"),
        ("HashMap<usize, Option<usize>>", "Map<int, int?>"),
    ] {
        assert_stream_type_matches_public_binding(rust, dart);
    }
}

#[test]
fn usize_stream_codec_matches_public_vector_binding() {
    for (rust, dart) in [
        ("Vec<usize>", "Int64List"),
        ("Option<Vec<usize>>", "Int64List?"),
        ("Vec<Vec<usize>>", "List<Int64List>"),
    ] {
        assert_stream_type_matches_public_binding(rust, dart);
    }
}

#[test]
fn u64_stream_codec_preserves_public_bigint_binding() {
    for (rust, dart) in [
        ("u64", "BigInt"),
        ("Option<u64>", "BigInt?"),
        ("Vec<u64>", "Uint64List"),
        ("Vec<Option<u64>>", "List<BigInt?>"),
        ("HashMap<usize, u64>", "Map<int, BigInt>"),
    ] {
        assert_stream_type_matches_public_binding(rust, dart);
    }
}
