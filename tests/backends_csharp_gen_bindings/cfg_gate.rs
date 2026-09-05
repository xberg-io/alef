// Test module: diagnostic output to stdout/stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

fn gate_function(name: &str, cfg: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
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
        crate_name: "test-lib".to_string(),
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
        "[workspace]\nlanguages = [\"csharp\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n{features_line}[crates.ffi]\nprefix = \"test\"\nerror_style = \"last_error\"\n[crates.csharp]\n",
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_str).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn native_methods_content(files: &[alef::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NativeMethods.cs"))
        .expect("NativeMethods.cs generated")
        .content
}

/// Regression: a `#[cfg(feature = "tokenizer")]`-gated FFI export must not produce a `DllImport`
/// declaration in `NativeMethods.cs` when the configured C# feature set doesn't enable that
/// feature. `DllImport` resolves lazily, so an unconditionally emitted stub for a symbol the
/// linked FFI library never compiled would compile cleanly and only throw
/// `EntryPointNotFoundException` at runtime — the failure mode this filter exists to prevent.
#[test]
fn csharp_omits_gated_function_when_feature_disabled() {
    let backend = CsharpBackend;
    let config = config_with_features(&[]);

    let files = backend.generate_bindings(&gated_api(), &config).unwrap();
    let content = native_methods_content(&files);

    assert!(
        content.contains("test_always_on"),
        "ungated function's DllImport must still be emitted, got:\n{content}"
    );
    assert!(
        !content.contains("test_count_tokens"),
        "gated function's DllImport must be omitted when its feature is disabled, got:\n{content}"
    );
}

/// Companion to the above: when the configured C# feature set does enable the gate, the
/// `DllImport` declaration must be emitted normally.
#[test]
fn csharp_includes_gated_function_when_feature_enabled() {
    let backend = CsharpBackend;
    let config = config_with_features(&["tokenizer"]);

    let files = backend.generate_bindings(&gated_api(), &config).unwrap();
    let content = native_methods_content(&files);

    assert!(
        content.contains("test_always_on"),
        "ungated function's DllImport must be emitted, got:\n{content}"
    );
    assert!(
        content.contains("test_count_tokens"),
        "gated function's DllImport must be emitted when its feature is enabled, got:\n{content}"
    );
}
