use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

const REPEATED_GENERATION_COUNT: usize = 32;

fn make_config() -> alef::core::config::ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
"#,
    )
    .expect("test config must parse");
    config.resolve().expect("test config must resolve").remove(0)
}

fn async_vec_function(name: &str, element_name: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        params: Vec::new(),
        return_type: TypeRef::Vec(Box::new(TypeRef::Named(element_name.to_string()))),
        is_async: true,
        doc: String::new(),
        error_type: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        cfg: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        original_rust_path: String::new(),
        return_sanitized: false,
        version: Default::default(),
    }
}

fn generated_swift(api: &ApiSurface) -> String {
    SwiftBackend
        .generate_bindings(api, &make_config())
        .expect("Swift generation must succeed")
        .into_iter()
        .find(|file| {
            file.content
                .contains("extension RustBridge.FirstRecord: @unchecked Sendable")
        })
        .expect("Swift bindings with async return Sendable extensions must be generated")
        .content
}

#[test]
fn async_return_sendable_extensions_are_stable_across_repeated_generation() {
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "sample_core".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![
            async_vec_function("load_second_records", "SecondRecord"),
            async_vec_function("load_first_records", "FirstRecord"),
        ],
        ..ApiSurface::default()
    };

    let expected = generated_swift(&api);
    let first_position = expected
        .find("extension RustBridge.FirstRecord: @unchecked Sendable")
        .expect("FirstRecord Sendable extension must be generated");
    let second_position = expected
        .find("extension RustBridge.SecondRecord: @unchecked Sendable")
        .expect("SecondRecord Sendable extension must be generated");
    assert!(
        first_position < second_position,
        "Sendable extensions must use canonical name order"
    );

    for _ in 1..REPEATED_GENERATION_COUNT {
        assert_eq!(generated_swift(&api), expected);
    }
}
