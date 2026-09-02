mod attributes;
mod enum_variants;
mod field_types;
mod fields;
mod reexport_map;
pub(crate) mod result_alias_scope;
mod rustdoc;

#[cfg(test)]
mod tests;

pub(crate) use attributes::{
    extract_alef_error_code, extract_binding_exclusion_reason, extract_cfg_condition, extract_error_message_template,
    extract_field_binding_exclusion_reason, extract_serde_container_conversion, extract_serde_rename_all,
    extract_serde_rename_all_fields, extract_serde_skip, extract_serde_skip_serializing_if, extract_version_annotation,
    has_cfg_attribute, has_container_serde_default, has_derive, has_field_attr, is_pub, is_test_gated,
    is_thiserror_enum,
};
pub(crate) use enum_variants::extract_enum_variant;
pub(crate) use field_types::{
    build_rust_path, detect_core_wrapper, extract_field_type_rust_path, syn_type_is_boxed, unwrap_optional,
};
pub(crate) use fields::extract_field;
pub(crate) use reexport_map::{ReexportKind, collect_reexport_map};
pub(crate) use result_alias_scope::{ResultModuleContextGuard, resolve_result_alias_scope};
pub(crate) use rustdoc::extract_doc_comments;

#[cfg(test)]
pub use rustdoc::normalize_rustdoc;
