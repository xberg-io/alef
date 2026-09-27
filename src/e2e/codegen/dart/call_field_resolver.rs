//! Per-call `FieldResolver` construction for `test_case.rs`.
//!
//! Extracted out of `test_case.rs`, which sits at the file-size ratchet's frozen ceiling
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
//! existing use of it. ~keep Before this fix, `dart_length_expr`'s `FieldResolver::is_optional`
//! check silently fell back to the config-only `fields_optional` set for any consumer whose
//! `alef.toml` never listed the field by hand, so a leaf `Option<Vec<T>>` field known only
//! through the IR emitted a bare `.length` against a nullable `List<T>?` (dart analyzer:
//! "potentially null").

use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::{DartFirstClassMap, FieldResolver};
use crate::e2e::fixture::Fixture;
use std::collections::{HashMap, HashSet};

/// Inputs for [`build_call_field_resolver`], grouped to keep the function under the
/// too-many-parameters limit without changing any of the values passed through. Mirrors the
/// identically-named struct already used by `csharp/call_field_resolver.rs`,
/// `php/call_field_resolver.rs`, and `kotlin/call_field_resolver.rs`.
pub(super) struct CallFieldResolverInputs<'a> {
    pub e2e_config: &'a E2eConfig,
    pub call_config: &'a CallConfig,
    pub fixture: &'a Fixture,
    pub lang: &'a str,
    pub dart_first_class_map: &'a DartFirstClassMap,
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
        dart_first_class_map,
        type_defs,
        enums,
        functions,
    } = inputs;
    // Merge per-language enum_fields from the Dart override into the effective enum set so that
    // fields like "status" (BatchStatus on BatchObject) are treated as enum-typed even when
    // they are not globally listed in fields_enum (they are context-dependent — BatchStatus on
    // BatchObject but plain String on ResponseObject). `with_ir_enum_map` below then rescues
    // every enum-typed field this config never mentions at all, anchored at the call's declared
    // Rust return type. ~keep
    let mut effective_enum_fields: HashSet<String> = e2e_config.effective_fields_enum(call_config).clone();
    if let Some(overrides) = call_config.overrides.get(lang) {
        effective_enum_fields.extend(overrides.enum_fields.keys().cloned());
    }
    let call_root_type = resolve_declared_result_type(call_config, lang, CallIr { functions, type_defs });

    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    FieldResolver::new_with_dart_first_class(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &HashMap::new(),
        dart_first_class_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_dart_root_type(super::dart_call_result_type(call_config).or_else(|| dart_first_class_map.root_type.clone()))
    .with_enum_fields(effective_enum_fields)
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    // Mirrors csharp.rs/kotlin's identical `with_ir_collection_map` wiring: without it, a
    // collection field with no per-element path anywhere in the fixture suite (nothing ever
    // indexes into it — e.g. a recursive `Option<Vec<DataNode>> Children`) has no
    // `fields_array` config signal at all, so `FieldResolver::is_collection_root` always
    // returned false regardless of what `assertions.rs` checks it for. ~keep
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()))
}

#[cfg(test)]
#[path = "call_field_resolver/tests.rs"]
mod tests;
