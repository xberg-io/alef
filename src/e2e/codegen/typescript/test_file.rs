//! Test file and test case rendering for TypeScript e2e tests.

use crate::codegen::naming::underscore_camel_case;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{EnumDef, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::escape::{escape_js, expand_fixture_templates, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToUpperCamelCase;

use super::assertions::render_assertion_with_streaming_item_type;
use super::json::{js_object_key, json_to_js, json_to_js_camel, json_to_js_camel_with_types, json_to_js_multiline};
use super::visitors::build_typescript_visitor;

mod args;
mod builders;
mod bytes;
mod cache;
#[cfg(test)]
mod call_arity_tests;
#[cfg(test)]
mod handle_config_typing_tests;
mod handle_values;
#[cfg(test)]
mod handle_values_tests;
mod helpers;
mod http;
#[cfg(test)]
mod json_object_field_agreement_tests;
#[cfg(test)]
mod loop_binding_tests;
#[cfg(test)]
mod node_enum_import_tests;
#[cfg(test)]
mod optional_segment_len_tests;
mod render;
#[cfg(test)]
mod resolver_metadata_wiring_tests;
#[cfg(test)]
mod result_enum_import_invariant_tests;
mod snippet;
#[cfg(test)]
mod stream_adapter_item_tests;
#[cfg(test)]
mod tagged_union_wiring_tests;
mod test_case;
#[cfg(test)]
mod tests;
mod visitor;
#[cfg(test)]
mod void_not_error_call_tests;
mod wasm;
#[cfg(test)]
mod wasm_enum_import_tests;
#[cfg(test)]
mod wasm_enum_member_agreement_tests;
#[cfg(test)]
mod wasm_handle_config_transitive_import_tests;
#[cfg(test)]
mod wasm_optional_chain_tests;
#[cfg(test)]
mod wasm_options_type_import_prefix_tests;
#[cfg(test)]
mod wasm_snippet_prefix_tests;
#[cfg(test)]
mod wasm_trait_bridge_import_tests;

pub use render::render_test_file;
pub(crate) use snippet::{SnippetContext, render_node_snippet_body, render_snippet_body};

pub(in crate::e2e::codegen::typescript::test_file) use args::build_args_and_setup;
pub(in crate::e2e::codegen::typescript::test_file) use builders::{
    node_enum_string_literal, node_typed_value_expression, rename_napi_serde_tags_to_kind, ts_builder_expression,
    ts_builder_expression_inner, wasm_scalar_value_expression,
};
pub(in crate::e2e::codegen::typescript::test_file) use bytes::ts_bytes_value_expression;
pub(in crate::e2e::codegen::typescript::test_file) use cache::{
    detect_cache_isolation_needs, emit_cache_isolation_setup,
};
pub(in crate::e2e::codegen::typescript::test_file) use handle_values::{
    HandleConfigContext, build_handle_config_value, collect_used_handle_config_types,
};
pub(super) use helpers::resolve_node_function_name;
pub(in crate::e2e::codegen::typescript::test_file) use helpers::{
    canonical_ts_type_name, enum_field_key, extract_bridge_cleanup, has_bytes_file_reads, has_later_arg_value,
    has_trait_bridge_args, is_typescript_primitive_element_type, resolve_enum_type, resolve_js_function_name,
    strip_setup_metadata, ts_method_helper_import,
};
pub(in crate::e2e::codegen::typescript::test_file) use http::render_http_test_case;
pub(in crate::e2e::codegen::typescript::test_file) use test_case::render_test_case;
pub(in crate::e2e::codegen::typescript::test_file) use visitor::{
    apply_wasm_visitor_arg, node_visitor_args, wasm_visitor_binding,
};
pub(in crate::e2e::codegen::typescript::test_file) use wasm::{
    collect_transitive_nested_types_for_wasm, derive_nested_types_for_wasm, wasm_class_name, wasm_prefixed_wrapped_type,
};
