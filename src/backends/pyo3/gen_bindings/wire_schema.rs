//! Generation of `__ALEF_WIRE_*` rename-schema consts for data-enum payload DTO coercion.
//!
//! A data-enum per-variant constructor accepts a public payload (a `@dataclass` or a `dict`) for a
//! config-DTO field and coerces it into the core type via the `__alef_coerce_dto` runtime helper
//! (see [`crate::codegen::generators::PYO3_DTO_COERCE_HELPER`]). A `@dataclass` carries Rust field
//! names, while serde deserializes by wire name, so the helper rewrites the keys using a per-DTO
//! `&[__AlefAlias]` schema emitted here. The schema honors both `#[serde(rename)]` and
//! `#[serde(rename_all)]` (sourced from the centralized [`crate::codegen::naming`] transforms — the
//! same single source of truth the Python `_to_rust_*` converters use) and recurses through nested
//! DTOs, sequences, maps, and optionals so renamed fields at any depth survive the round-trip.

use super::errors::is_dataclass_backed_config;
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::generators::{
    PYO3_DTO_COERCE_HELPER, coercible_payload, collect_all_variant_constructors, data_enum_needs_dto_coercion,
    enum_has_data_variants, pyo3_wire_schema_const_name, variant_constructor_is_reachable,
};
use crate::codegen::naming::wire_field_name;
use crate::codegen::shared::binding_fields;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::{AHashMap, AHashSet};

/// Classify the dataclass-backed config-DTO type names: their public name resolves to a
/// `@dataclass` (via `options.py`), not the compiled `#[pyclass]`. Enum-variant payload fields of
/// these types must accept the public wrapper or a raw dict — coerced into the core type — for
/// parity with struct-field `_to_rust_*` coercion. Native-return types stay compiled and are left
/// untouched. Same source of truth as `gen_init_py`'s import routing.
pub(in crate::backends::pyo3) fn coercible_dto_names<'a>(
    api: &'a ApiSurface,
    config: &ResolvedCrateConfig,
) -> AHashSet<&'a str> {
    let output_style = config.dto.python_output_style();
    let reexported: AHashSet<&str> = config
        .python
        .as_ref()
        .map(|p| p.reexported_types.iter().map(String::as_str).collect())
        .unwrap_or_default();
    api.types
        .iter()
        .filter(|t| is_dataclass_backed_config(t, output_style, &reexported))
        .map(|t| t.name.as_str())
        .collect()
}

/// Emit the data-enum DTO-coercion section for the generated pyo3 module: the runtime coercion
/// helper ([`PYO3_DTO_COERCE_HELPER`]) followed by the per-DTO `__ALEF_WIRE_*` rename-schema consts.
/// Returns an empty string when no data-enum variant constructor needs coercion (or `serde` is
/// unavailable — the helper deserializes the coerced JSON into the core type). The helper and the
/// schema consts are joined the same way `RustFileBuilder` joins items (`"\n\n"`), so emitting this
/// as a single item is byte-identical to emitting them separately.
pub(super) fn emit_dto_coercion_section(
    api: &ApiSurface,
    has_serde: bool,
    coercible: &AHashSet<&str>,
    core_import: &str,
) -> String {
    let needed = has_serde
        && api.enums.iter().any(|e| {
            let is_host_enum = is_host_owned_rust_path(core_import, &e.rust_path);
            data_enum_needs_dto_coercion(e, coercible, is_host_enum)
        });
    if !needed {
        return String::new();
    }
    let schema_consts = gen_wire_schema_consts(api, coercible, core_import);
    if schema_consts.is_empty() {
        PYO3_DTO_COERCE_HELPER.to_string()
    } else {
        format!("{PYO3_DTO_COERCE_HELPER}\n\n{schema_consts}")
    }
}

/// One emitted `__AlefAlias` row.
struct AliasEntry {
    rust: String,
    wire: String,
    kind: &'static str,
    /// Nested schema const name, or `&[]` for a leaf (rename-only) field.
    nested: String,
}

/// Emit the `__ALEF_WIRE_*` rename-schema consts for every coercible DTO reachable from a data-enum
/// variant-constructor payload field (transitively through nested coercible DTO fields). Returns an
/// empty string when no coercion is in play. Cyclic type graphs are broken at back-edges (the deeper
/// occurrence references `&[]`) so the emitted consts never form a const-evaluation cycle.
pub(super) fn gen_wire_schema_consts(
    api: &ApiSurface,
    coercible_dto_names: &AHashSet<&str>,
    core_import: &str,
) -> String {
    if coercible_dto_names.is_empty() {
        return String::new();
    }
    let types: AHashMap<&str, &TypeDef> = api.types.iter().map(|t| (t.name.as_str(), t)).collect();

    let mut seeds: Vec<String> = Vec::new();
    for e in &api.enums {
        if !enum_has_data_variants(e) {
            continue;
        }
        let is_host_enum = is_host_owned_rust_path(core_import, &e.rust_path);
        let reachable_ctors = collect_all_variant_constructors(e).into_iter().filter(|ctor| {
            e.variants
                .iter()
                .find(|v| v.name == ctor.variant_name)
                .is_some_and(|v| variant_constructor_is_reachable(v, is_host_enum))
        });
        for ctor in reachable_ctors {
            for p in &ctor.params {
                if let Some((dto, _)) = coercible_payload(&p.ty, coercible_dto_names)
                    && !seeds.iter().any(|s| s == dto)
                {
                    seeds.push(dto.to_string());
                }
            }
        }
    }
    if seeds.is_empty() {
        return String::new();
    }

    let mut built: Vec<(String, Vec<AliasEntry>)> = Vec::new();
    let mut done: AHashSet<String> = AHashSet::new();
    for seed in &seeds {
        build_type(
            seed,
            &types,
            coercible_dto_names,
            &mut Vec::new(),
            &mut done,
            &mut built,
        );
    }

    built.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (const_name, entries) in &built {
        let rendered_entries: Vec<minijinja::Value> = entries
            .iter()
            .map(|e| {
                minijinja::context! {
                    rust => &e.rust,
                    wire => &e.wire,
                    kind => e.kind,
                    nested => &e.nested,
                }
            })
            .collect();
        out.push_str(&crate::backends::pyo3::template_env::render(
            "pyo3_wire_schema_const.jinja",
            minijinja::context! {
                const_name => const_name,
                entries => rendered_entries,
            },
        ));
        out.push('\n');
    }
    out
}

/// Build (and memoize) the schema const for `type_name`, recursing into nested coercible DTOs.
/// `path` tracks the current DFS ancestry so a back-edge to a type still being built is broken
/// (`&[]`) — guaranteeing the const-reference graph stays acyclic.
fn build_type(
    type_name: &str,
    types: &AHashMap<&str, &TypeDef>,
    coercible: &AHashSet<&str>,
    path: &mut Vec<String>,
    done: &mut AHashSet<String>,
    built: &mut Vec<(String, Vec<AliasEntry>)>,
) {
    if done.contains(type_name) {
        return;
    }
    let Some(typ) = types.get(type_name) else {
        done.insert(type_name.to_string());
        built.push((pyo3_wire_schema_const_name(type_name), Vec::new()));
        return;
    };
    done.insert(type_name.to_string());
    path.push(type_name.to_string());

    let rename_all = typ.serde_rename_all.as_deref();
    let mut entries: Vec<AliasEntry> = Vec::new();
    for field in binding_fields(&typ.fields) {
        let wire = wire_field_name(&field.name, field.serde_rename.as_deref(), rename_all);
        // `rust` is looked up against the dict `dataclasses.asdict(value)` returns, so it has to be
        // the *emitted* attribute name, not the Rust one — the dataclass field for `global` is
        // `global_` (`types.rs`, via `python_ident`). This is also what decides whether a field
        // needs an alias entry at all: a keyword field's attribute name differs from its wire name
        // even when no `serde(rename)` is involved, so comparing the wire name against the raw Rust
        // name would drop the entry and silently ship `{"global_": ...}` on the wire. ~keep
        let python_attr = crate::core::keywords::python_ident(&field.name);
        match coercible_payload(&field.ty, coercible) {
            Some((dto_name, shape)) => {
                let nested = if path.iter().any(|p| p == dto_name) {
                    "&[]".to_string()
                } else {
                    build_type(dto_name, types, coercible, path, done, built);
                    pyo3_wire_schema_const_name(dto_name)
                };
                entries.push(AliasEntry {
                    rust: python_attr,
                    wire,
                    kind: shape.wire_kind(),
                    nested,
                });
            }
            None => {
                if wire != python_attr {
                    entries.push(AliasEntry {
                        rust: python_attr,
                        wire,
                        kind: "Leaf",
                        nested: "&[]".to_string(),
                    });
                }
            }
        }
    }

    path.pop();
    built.push((pyo3_wire_schema_const_name(type_name), entries));
}

#[cfg(test)]
mod tests;
