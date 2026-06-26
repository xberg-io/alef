//! Data-enum payload DTO coercion for the pyo3 backend.
//!
//! A data-enum per-variant constructor (e.g. `EmbeddingModelType.llm(...)`) accepts a public payload
//! — a `@dataclass`, a `dict`, or a list/map of either — for a config-DTO field and coerces it into
//! the core type, rather than demanding the compiled `#[pyclass]` instance. This module owns that
//! concern end to end: the module-level runtime helper ([`PYO3_DTO_COERCE_HELPER`]), the classifier
//! that decides which payload fields are coercible and in what shape ([`coercible_payload`] /
//! [`CoercibleShape`]), the per-DTO rename-schema const naming ([`pyo3_wire_schema_const_name`]),
//! and the per-field init expression the variant-constructor emitter splices in
//! ([`coercible_field_init`]). The variant-constructor emission itself lives in [`super::enums`].

use super::enums::{collect_variant_constructors, enum_has_data_variants};
use crate::core::ir::{EnumDef, TypeRef};
use ahash::AHashSet;

/// Module-level runtime helper coercing a public payload (dataclass / dict / JSON-native value)
/// into a core type via serde. Emitted once per pyo3 module when any data-enum variant constructor
/// has a coercible config-DTO field. Mirrors the struct-field `_to_rust_*` coercion on the Python
/// side so enum-variant payloads accept the same inputs (the public `LlmConfig` dataclass or a
/// `dict`), instead of demanding the compiled `#[pyclass]` instance.
///
/// The helper is non-parameterized (this rule allows a static `const` for non-interpolated code).
/// Per-DTO rename fidelity is supplied at the call site as a runtime `&[__AlefAlias]` schema (the
/// `__ALEF_WIRE_*` module consts emitted by the pyo3 backend), so a dataclass whose fields use
/// `#[serde(rename)]` or `#[serde(rename_all)]` — including nested DTOs, sequences, maps, and
/// optionals — round-trips faithfully. Plain dicts already carry serde wire names and pass straight
/// through without remapping.
pub const PYO3_DTO_COERCE_HELPER: &str = r#"struct __AlefAlias {
    rust: &'static str,
    wire: &'static str,
    kind: __AlefKind,
    nested: &'static [__AlefAlias],
}

enum __AlefKind {
    Leaf,
    Object,
    Seq,
    Map,
}

fn __alef_apply_aliases(value: &mut serde_json::Value, aliases: &[__AlefAlias]) {
    let serde_json::Value::Object(map) = value else {
        return;
    };
    for alias in aliases {
        if !alias.nested.is_empty() {
            if let Some(child) = map.get_mut(alias.rust) {
                match alias.kind {
                    __AlefKind::Object => __alef_apply_aliases(child, alias.nested),
                    __AlefKind::Seq => {
                        if let serde_json::Value::Array(items) = child {
                            for item in items.iter_mut() {
                                __alef_apply_aliases(item, alias.nested);
                            }
                        }
                    }
                    __AlefKind::Map => {
                        if let serde_json::Value::Object(entries) = child {
                            for entry in entries.values_mut() {
                                __alef_apply_aliases(entry, alias.nested);
                            }
                        }
                    }
                    __AlefKind::Leaf => {}
                }
            }
        }
        if alias.rust != alias.wire {
            if let Some(taken) = map.remove(alias.rust) {
                map.insert(alias.wire.to_string(), taken);
            }
        }
    }
}

fn __alef_coerce_dto<T: serde::de::DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, pyo3::types::PyAny>,
    aliases: &[__AlefAlias],
) -> PyResult<T> {
    // A public @dataclass is not directly JSON-serializable: convert it via `dataclasses.asdict`
    // (which emits Rust field names) and rewrite the keys to serde wire names so renamed fields
    // survive. A plain dict / JSON-native value already uses wire names and passes straight through.
    if value.hasattr("__dataclass_fields__")? {
        let as_dict = py.import("dataclasses")?.call_method1("asdict", (value,))?;
        let json_str: String = py.import("json")?.call_method1("dumps", (as_dict,))?.extract()?;
        let mut json_value: serde_json::Value =
            serde_json::from_str(&json_str).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        __alef_apply_aliases(&mut json_value, aliases);
        serde_json::from_value(json_value).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    } else {
        let json_str: String = py.import("json")?.call_method1("dumps", (value,))?.extract()?;
        serde_json::from_str(&json_str).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }
}

fn __alef_coerce_dto_seq<T: serde::de::DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, pyo3::types::PyAny>,
    aliases: &[__AlefAlias],
) -> PyResult<Vec<T>> {
    let items: Vec<Bound<'_, pyo3::types::PyAny>> = value.extract()?;
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        out.push(__alef_coerce_dto(py, item, aliases)?);
    }
    Ok(out)
}

fn __alef_coerce_dto_map<T: serde::de::DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, pyo3::types::PyAny>,
    aliases: &[__AlefAlias],
) -> PyResult<T> {
    // Build a wire-keyed JSON object from the mapping's items (each value coerced like a DTO via
    // the object helper), then deserialize the whole map into the core type.
    let items: Vec<(String, Bound<'_, pyo3::types::PyAny>)> = value.call_method0("items")?.extract()?;
    let mut object = serde_json::Map::with_capacity(items.len());
    for (key, val) in &items {
        object.insert(key.clone(), __alef_coerce_dto(py, val, aliases)?);
    }
    serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}"#;

/// The module-const identifier holding the `&[__AlefAlias]` rename schema for a coercible DTO
/// (`LlmConfig` → `__ALEF_WIRE_LLM_CONFIG`). Shared between the pyo3 backend (which emits the const)
/// and the variant-constructor call site (which references it) so there is a single naming path.
pub fn pyo3_wire_schema_const_name(type_name: &str) -> String {
    use heck::ToShoutySnakeCase;
    format!("__ALEF_WIRE_{}", type_name.to_shouty_snake_case())
}

/// The JSON shape a coercible payload field takes, selecting which runtime coercion helper the
/// generated factory calls.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CoercibleShape {
    /// A single DTO (`Named` / `Optional<Named>`) → `__alef_coerce_dto`.
    Object,
    /// A list of DTOs (`Vec<Named>` / `Optional<Vec<Named>>`) → `__alef_coerce_dto_seq`.
    Seq,
    /// A string-keyed map of DTOs (`Map<_, Named>` / `Optional<Map<_, Named>>`) → `__alef_coerce_dto_map`.
    Map,
}

impl CoercibleShape {
    /// The `__AlefKind` variant name this shape recurses as in a `__ALEF_WIRE_*` rename schema.
    pub fn wire_kind(self) -> &'static str {
        match self {
            CoercibleShape::Object => "Object",
            CoercibleShape::Seq => "Seq",
            CoercibleShape::Map => "Map",
        }
    }
}

/// Resolve the coercible config-DTO a variant payload field carries and the JSON shape of its value.
/// `Optional` is transparent. Returns `None` when the field is not (or does not contain) a coercible
/// DTO — those keep the compiled-instance `.into()` path. The returned `&str` is the DTO type name,
/// used to select its `__ALEF_WIRE_*` rename schema. Single source of truth for both the
/// variant-constructor call site and the pyo3 backend's rename-schema generation.
pub fn coercible_payload<'a>(ty: &'a TypeRef, coercible: &AHashSet<&str>) -> Option<(&'a str, CoercibleShape)> {
    let named_if_coercible = |t: &'a TypeRef| match t {
        TypeRef::Named(n) if coercible.contains(n.as_str()) => Some(n.as_str()),
        _ => None,
    };
    match ty {
        TypeRef::Optional(inner) => coercible_payload(inner, coercible),
        TypeRef::Vec(inner) => named_if_coercible(inner).map(|n| (n, CoercibleShape::Seq)),
        TypeRef::Map(_, value) => named_if_coercible(value).map(|n| (n, CoercibleShape::Map)),
        TypeRef::Named(_) => named_if_coercible(ty).map(|n| (n, CoercibleShape::Object)),
        _ => None,
    }
}

/// True when any generated variant constructor of `enum_def` has a coercible config-DTO payload
/// field — i.e. the [`PYO3_DTO_COERCE_HELPER`] runtime helper must be emitted into the module.
pub fn data_enum_needs_dto_coercion(enum_def: &EnumDef, coercible_dto_names: &AHashSet<&str>) -> bool {
    if !enum_has_data_variants(enum_def) {
        return false;
    }
    collect_variant_constructors(enum_def).iter().any(|c| {
        c.params
            .iter()
            .any(|p| coercible_payload(&p.ty, coercible_dto_names).is_some())
    })
}

/// Build the struct-literal init expression for a coercible config-DTO payload field. The param
/// arrives as `&Bound<PyAny>` (or `Option<&Bound<PyAny>>` when optional/promoted) and is routed
/// through the shape-appropriate `__alef_coerce_dto*` helper, whose target core type is resolved by
/// inference from the variant literal slot — so the core type path is never named (non-re-exported
/// core DTOs work unchanged). The `dto` type name selects the per-DTO `&[__AlefAlias]` rename schema
/// const so the dataclass round-trips with full serde-rename fidelity (a `Seq`/`Map` field applies
/// it to each element/value).
///
/// `promoted` is true when the binding signature widened a non-optional core field to `Option<T>`
/// because it follows an optional param; the coerced value is unwrapped to the field default.
pub(crate) fn coercible_field_init(
    name: &str,
    dto: &str,
    shape: CoercibleShape,
    optional: bool,
    promoted: bool,
) -> String {
    let schema = pyo3_wire_schema_const_name(dto);
    let helper = match shape {
        CoercibleShape::Object => "__alef_coerce_dto",
        CoercibleShape::Seq => "__alef_coerce_dto_seq",
        CoercibleShape::Map => "__alef_coerce_dto_map",
    };
    if optional {
        // Genuinely-optional core field: keep the `Option`, coercing the inner value when present.
        format!("{name}.map(|v| {helper}(py, v, {schema})).transpose()?")
    } else if promoted {
        // Core field is `T` but the param arrived as `Option<&Bound>`: coerce then unwrap to default.
        format!("{name}.map(|v| {helper}(py, v, {schema})).transpose()?.unwrap_or_default()")
    } else {
        format!("{helper}(py, {name}, {schema})?")
    }
}
