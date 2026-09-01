//! Per-call `FieldResolver` construction for the Go e2e generator.
//!
//! Split out of `test_function.rs` to keep result-shape resolution focused.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

#[derive(Clone)]
pub(in crate::e2e::codegen::go) struct GoCrateResolverFacts {
    ir_reachable_fields: std::collections::HashSet<String>,
    ir_known_excluded_fields: std::collections::HashSet<String>,
    ir_optional_fields: std::collections::HashSet<String>,
    ir_result_fields: crate::e2e::field_access::IrResultFieldMap,
    ir_collection_fields: crate::e2e::field_access::IrCollectionMap,
    reserved_type_names: std::collections::HashSet<String>,
}

impl GoCrateResolverFacts {
    pub(in crate::e2e::codegen::go) fn new(
        type_defs: &[TypeDef],
        enums: &[EnumDef],
        config: &ResolvedCrateConfig,
    ) -> Self {
        let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) =
            FieldResolver::ir_field_sets(type_defs);
        let emission = crate::backends::go::emission_facts::GoEmissionFacts::from_config(type_defs, enums, config);
        let ir_result_fields = FieldResolver::go_ir_result_field_facts_from_emission(type_defs, &emission);
        let ir_collection_fields = FieldResolver::ir_collection_fields(type_defs);
        let reserved_type_names = emission
            .structs
            .iter()
            .chain(emission.opaque.iter())
            .chain(emission.enums.iter())
            .map(|name| crate::codegen::naming::go_type_name(name))
            .collect();
        Self {
            ir_reachable_fields,
            ir_known_excluded_fields,
            ir_optional_fields,
            ir_result_fields,
            ir_collection_fields,
            reserved_type_names,
        }
    }

    pub(super) fn reserved_type_names(&self) -> &std::collections::HashSet<String> {
        &self.reserved_type_names
    }
}

pub(super) const LANG: &str = "go";

pub(in crate::e2e::codegen::go) fn fixture_has_go_callable(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    if fixture.is_http_test() {
        return false;
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if call.skip_languages.iter().any(|language| language == LANG) {
        return false;
    }
    let override_config = call.overrides.get(LANG).or_else(|| e2e_config.call.overrides.get(LANG));
    if override_config
        .and_then(|config| config.client_factory.as_deref())
        .is_some()
    {
        return true;
    }
    let function = override_config
        .and_then(|config| config.function.as_deref())
        .filter(|function| !function.is_empty())
        .unwrap_or(call.function.as_str());
    !function.is_empty()
}

/// Build the field resolver for one call, anchored at the call's declared Rust return type.
///
/// Anchoring `with_ir_result_fields` mirrors the rust/python/java/csharp/elixir generators and is
/// purely additive: `result_field_oracle_knows` only ever REFUSES what it positively knows the
/// root type lacks, so an unresolved root leaves every anchored answer disabled. ~keep
///
/// ~keep `with_ir_collection_map` mirrors csharp/dart/java/kotlin/php's identical wiring, which Go
/// alone was missing: without it `FieldResolver::is_array` can answer `true` only from the
/// hand-authored `fields_array` config (`array_fields`), never from the IR, so any `Option<Vec<T>>`
/// (or plain `Vec<T>`) result field a consumer never listed there reads as "not an array". That
/// silently mis-set `is_array_for_len`/`is_slice` in `assertion_field_shape.rs`, which combined
/// with a resolution gap in `target_field_is_pointer` produced the wrong `.unwrap_or(is_optional &&
/// !is_array_for_len)` guess for e.g. `Chunks []Chunk` — `len(*result.Chunks)` against a plain
/// slice, a Go compile error (`cannot indirect ... variable of type []xberg.Chunk`).
pub(super) fn build_call_field_resolver_with_facts(
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    functions: &[FunctionDef],
    type_defs: &[TypeDef],
    facts: &GoCrateResolverFacts,
) -> FieldResolver {
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        LANG,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_ir_collection_map(facts.ir_collection_fields.clone(), call_root_type.clone())
    .with_ir_result_fields(facts.ir_result_fields.clone(), call_root_type)
    .with_ir_fields(
        facts.ir_reachable_fields.clone(),
        facts.ir_known_excluded_fields.clone(),
        facts.ir_optional_fields.clone(),
    )
    .with_result_is_byte_payload(call_config.effective_result_is_bytes(LANG))
}
