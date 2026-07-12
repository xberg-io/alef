//! Test file and test case rendering for TypeScript e2e tests.

use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{EnumDef, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::escape::{escape_js, expand_fixture_templates, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToUpperCamelCase;

use super::assertions::render_assertion;
use super::json::{json_to_js, json_to_js_camel, json_to_js_multiline, snake_to_camel};
use super::visitors::build_typescript_visitor;

mod args;
mod builders;
mod cache;
mod helpers;
mod http;
mod render;
mod test_case;
#[cfg(test)]
mod tests;
mod visitor;
mod wasm;

pub use render::render_test_file;

pub(in crate::e2e::codegen::typescript::test_file) use args::build_args_and_setup;
pub(in crate::e2e::codegen::typescript::test_file) use builders::{
    rename_napi_serde_tags_to_kind, ts_builder_expression, ts_builder_expression_inner,
};
pub(in crate::e2e::codegen::typescript::test_file) use cache::{
    detect_cache_isolation_needs, emit_cache_isolation_setup,
};
pub(super) use helpers::resolve_node_function_name;
pub(in crate::e2e::codegen::typescript::test_file) use helpers::{
    canonical_ts_type_name, extract_bridge_cleanup, has_bytes_file_reads, has_later_arg_value, has_trait_bridge_args,
    is_typescript_primitive_element_type, strip_setup_metadata, ts_method_helper_import,
};
pub(in crate::e2e::codegen::typescript::test_file) use http::render_http_test_case;
pub(in crate::e2e::codegen::typescript::test_file) use test_case::render_test_case;
pub(in crate::e2e::codegen::typescript::test_file) use visitor::{
    apply_wasm_visitor_arg, node_visitor_args, wasm_visitor_binding,
};
pub(in crate::e2e::codegen::typescript::test_file) use wasm::{
    collect_transitive_nested_types_for_wasm, derive_nested_types_for_wasm, wasm_class_name, wasm_prefixed_wrapped_type,
};
