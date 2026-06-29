use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::docs::test_helpers::{
    make_field, make_function, make_method, make_minimal_api, make_param, make_test_config,
};

fn config_from_toml(toml_str: &str) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(toml_str).expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

fn empty_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

mod generate_docs;
mod generated_stage;
mod headings;
mod language_pages;
mod markdown_quality;
mod shared_docs;
