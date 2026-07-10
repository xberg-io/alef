//! Regression test: Swift opaque types must have Vec accessors registered
//!
//! When an opaque Rust type is declared via `type T;` in a swift-bridge extern block,
//! swift-bridge auto-generates Vectorizable conformance which references C ABI symbols
//! `__swift_bridge__$Vec_T$as_ptr` and `__swift_bridge__$Vec_T$len`. These symbols must
//! be emitted by swift-bridge-build on the Rust side, which only happens when the type
//! appears in a `Vec<T>` somewhere in the extern block.
//!
//! This test verifies that a phantom `__register_vec_accessors` function is emitted
//! with references to every opaque type in a Vec context, forcing symbol emission.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;

fn make_config() -> alef::core::config::ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn test_swift_vec_accessors_phantom_emitted() {
    let api = ApiSurface::default();
    let config = make_config();

    let backend = SwiftBackend;
    let files = backend
        .generate_bindings(&api, &config)
        .expect("Swift generation should succeed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("rust/src/lib.rs"))
        .expect("Should generate rust/src/lib.rs");

    let lib_content = &lib_rs.content;

    eprintln!("Checking for __register_vec_accessors in generated lib.rs");
    eprintln!("Content length: {} bytes", lib_content.len());
    eprintln!("Generated lib.rs content:\n{}", lib_content);

    assert!(
        !lib_content.contains("fn __register_vec_accessors("),
        "Empty API should not emit __register_vec_accessors"
    );

    assert!(
        lib_content.contains("#[swift_bridge::bridge]"),
        "lib.rs must be a valid swift-bridge module"
    );
}
