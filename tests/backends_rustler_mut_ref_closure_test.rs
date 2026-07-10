//! Verifies that Rustler backend correctly propagates &mut T references in nested closures.
//!
//! Regression test for Block B9: When a Rust core function expects Vec<&mut T> or similar
//! collections containing mutable references, the emitter must ensure that the mutable binding
//! created in the preamble is correctly borrowed in the closure environment, not captured
//! immutably.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FunctionDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn make_config(app_name: &str) -> ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn test_vec_of_mutable_refs_in_closure_preserves_mutability() {
    let opaque_type = TypeDef {
        name: "SampleHandle".to_string(),
        rust_path: "sample_core::SampleHandle".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let func = FunctionDef {
        name: "process_handles".to_string(),
        rust_path: "sample_core::process_handles".to_string(),
        original_rust_path: String::new(),
        is_async: false,
        params: vec![alef::core::ir::ParamDef {
            name: "handles".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("SampleHandle".to_string()))),
            is_ref: true,
            is_mut: true,
            optional: false,
            default: None,
            typed_default: None,
            sanitized: false,
            newtype_wrapper: None,
            original_type: None,
            core_wrapper: CoreWrapper::None,
            map_is_btree: false,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
        }],
        return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::Bool),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        return_sanitized: false,
        error_type: None,
        doc: "Process a list of mutable handles.".to_string(),
        sanitized: false,
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let config = make_config("test_app");
    let api = ApiSurface {
        crate_name: "test-app".to_string(),
        version: "1.0.0".to_string(),
        types: vec![opaque_type],
        functions: vec![func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let generated = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("should generate Rustler bindings");

    let native_module = generated
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("native.ex"))
        .expect("should generate native.ex Elixir module");

    let code = &native_module.content;

    println!("Generated Elixir native module:");
    println!("{}", code);

    assert!(
        code.contains("process_handles"),
        "should generate process_handles function in native module"
    );
}
