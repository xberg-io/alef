//! Per-call `FieldResolver` construction for `test_method.rs`.
//!
//! Extracted out of `test_method.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow. Behavior is otherwise unchanged from the
//! block this replaces, plus `with_ir_result_fields`/`with_anchored_optional_paths`: without
//! `with_ir_result_fields`, `FieldResolver`'s `ir_result_field_map` keeps its default `root_type:
//! None`, which makes `with_anchored_optional_paths` an unconditional no-op (it early-returns on
//! an unresolved root) regardless of what paths it is given — the same gap kotlin's identical
//! module documents and wires around. With both wired in, an `Option<Vec<T>>` segment field
//! reached through an array-projected path (e.g. `entries[0].sections`) — which never matches
//! `with_ir_fields`'s bare-name-only optional set once the path crosses more than one segment —
//! resolves correctly, and `with_anchored_optional_paths` materializes that IR-anchored answer
//! for this fixture's own assertion paths into the same lookup set, mirroring `presentation.rs`'s
//! existing use of it.

use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::{FieldResolver, PhpGetterMap};
use crate::e2e::fixture::Fixture;
use std::collections::{HashMap, HashSet};

/// Inputs for [`build_call_field_resolver`], grouped to keep the function under the
/// too-many-parameters limit without changing any of the values passed through.
pub(super) struct CallFieldResolverInputs<'a> {
    pub e2e_config: &'a E2eConfig,
    pub call_config: &'a CallConfig,
    pub fixture: &'a Fixture,
    pub lang: &'a str,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub functions: &'a [crate::core::ir::FunctionDef],
    pub per_call_getter_map: &'a PhpGetterMap,
}

pub(super) fn build_call_field_resolver(inputs: CallFieldResolverInputs<'_>) -> FieldResolver {
    let CallFieldResolverInputs {
        e2e_config,
        call_config,
        fixture,
        lang,
        type_defs,
        functions,
        per_call_getter_map,
    } = inputs;
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = resolve_declared_result_type(call_config, lang, CallIr { functions, type_defs });
    FieldResolver::new_with_php_getters(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
        &HashMap::new(),
        per_call_getter_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}

#[cfg(test)]
#[path = "call_field_resolver/tests.rs"]
mod tests;
