// Test module: diagnostic output to stdout/stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::*;

fn gate_function(name: &str, cfg: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: cfg.map(str::to_string),
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn gated_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            gate_function("always_on", None),
            gate_function("count_tokens", Some(r#"feature = "tokenizer""#)),
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn config_with_features(features: &[&str]) -> ResolvedCrateConfig {
    let features_line = if features.is_empty() {
        String::new()
    } else {
        let joined = features
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("features = [{joined}]\n")
    };
    let toml_str = format!(
        "[workspace]\nlanguages = [\"zig\"]\n\n[[crates]]\nname = \"demo\"\nsources = [\"src/lib.rs\"]\n{features_line}",
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_str).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn zig_module_content(files: &[alef::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".zig"))
        .expect("zig module generated")
        .content
}

/// Regression: a `#[cfg(feature = "tokenizer")]`-gated FFI export must not appear in the
/// generated Zig module when the configured Zig feature set doesn't enable that feature.
/// `@cImport` compiles the C header verbatim and Zig resolves declared externs at
/// comptime/link time, so an unconditionally emitted call into a symbol the linked FFI library
/// never compiled is a build-time failure, not a graceful runtime error.
#[test]
fn zig_omits_gated_function_when_feature_disabled() {
    let backend = ZigBackend;
    let config = config_with_features(&[]);

    let files = backend.generate_bindings(&gated_api(), &config).unwrap();
    let content = zig_module_content(&files);

    assert!(
        content.contains("pub fn always_on("),
        "ungated function must still be emitted, got:\n{content}"
    );
    assert!(
        !content.contains("pub fn count_tokens("),
        "gated function must be omitted when its feature is disabled, got:\n{content}"
    );
    assert!(
        !content.contains("demo_count_tokens"),
        "gated function's native symbol must not be referenced when its feature is disabled, got:\n{content}"
    );
}

/// Companion to the above: when the configured Zig feature set does enable the gate, the
/// function must be emitted normally.
#[test]
fn zig_includes_gated_function_when_feature_enabled() {
    let backend = ZigBackend;
    let config = config_with_features(&["tokenizer"]);

    let files = backend.generate_bindings(&gated_api(), &config).unwrap();
    let content = zig_module_content(&files);

    assert!(
        content.contains("pub fn always_on("),
        "ungated function must be emitted, got:\n{content}"
    );
    assert!(
        content.contains("pub fn count_tokens("),
        "gated function must be emitted when its feature is enabled, got:\n{content}"
    );
}
