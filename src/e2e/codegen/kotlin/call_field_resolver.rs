//! Per-call `FieldResolver` construction for `test_method.rs`.
//!
//! Extracted out of `test_method.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow. Behavior is otherwise unchanged from the
//! block this replaces, plus `with_ir_result_fields`/`with_anchored_optional_paths`: without
//! `with_ir_result_fields`, `FieldResolver`'s `ir_result_field_map` keeps its default `root_type:
//! None`, which makes `with_anchored_optional_paths` an unconditional no-op (it early-returns on
//! an unresolved root) regardless of what paths it is given. With both wired in, an
//! `Option<Vec<T>>` segment field reached through an array-projected path (e.g.
//! `entries[0].sections`) — which never matches `with_ir_fields`'s bare-name-only optional set
//! once the path crosses more than one segment — resolves correctly, and
//! `with_anchored_optional_paths` materializes that IR-anchored answer for this fixture's own
//! assertion paths into the lookup set the per-segment accessor renderer consults, mirroring
//! `presentation.rs`'s existing use of it.

use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::HashSet;

/// Inputs for [`build_call_field_resolver`], grouped to keep the function under the
/// too-many-parameters limit without changing any of the values passed through.
pub(super) struct CallFieldResolverInputs<'a> {
    pub e2e_config: &'a E2eConfig,
    pub call_config: &'a CallConfig,
    pub fixture: &'a Fixture,
    pub lang: &'a str,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub functions: &'a [crate::core::ir::FunctionDef],
}

pub(super) fn build_call_field_resolver(inputs: CallFieldResolverInputs<'_>) -> FieldResolver {
    let CallFieldResolverInputs {
        e2e_config,
        call_config,
        fixture,
        lang,
        type_defs,
        enums,
        functions,
    } = inputs;
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_root_type = resolve_declared_result_type(call_config, lang, CallIr { functions, type_defs });
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    // The Kotlin (JVM) target asserts against the Java facade, so its `.getValue()` exists exactly
    // when the Java binding backend's own `emits_get_value` says so — a different predicate from
    // the all-fieldless-variants gate kotlin_android's `toWire()` has, since Java still folds an
    // externally-tagged data enum down to a plain `enum`. Wiring the same set Java's own e2e
    // generator wires is what keeps the two from disagreeing. ~keep
    .with_java_wrapper_enum_names(
        enums
            .iter()
            .filter(|enum_def| !crate::backends::java::gen_bindings::emits_get_value(enum_def))
            .map(|enum_def| enum_def.name.clone())
            .collect(),
    )
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}
