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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn doc_content<'a>(files: &'a [crate::core::backend::GeneratedFile], slug: &str) -> &'a str {
    let expected_name = format!("{slug}.md");
    files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == expected_name)
        })
        .map(|file| file.content.as_str())
        .unwrap_or_else(|| panic!("missing generated doc file for {slug}"))
}

mod enum_default_syntax;
mod function_dedup;
mod generate_docs;
mod generated_stage;
mod headings;
mod java_exception_agreement;
mod language_pages;
mod markdown_canonical;
mod markdown_quality;
mod mut_param_writeback;
mod optional_enum_default;
mod rust_reference;
mod rustdoc_fence_attributes;
mod shared_docs;
mod snippet_build_dependency_removed;
mod strict_attribution;
mod strict_bail_order;
mod unknown_headings;
mod wasm_untagged_enum_docs;
