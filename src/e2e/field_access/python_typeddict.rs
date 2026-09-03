//! Derives which IR types the pyo3 backend emits as a Python `TypedDict` (subscript access,
//! `result["field"]`) rather than a `@dataclass` / native `#[pyclass]` (attribute access,
//! `result.field`), so the python e2e generator renders accessors that agree with what the
//! backend actually emits.
//!
//! Before this module existed, the python e2e renderer had no way to know which shape the pyo3
//! backend chose for a `[workspace.dto] python_output = "typed-dict"` crate: it always emitted
//! `.field` attribute access, which is `AttributeError: 'dict' object has no attribute 'field'`
//! against a return type the backend actually emitted as a `TypedDict` (a plain `dict` at
//! runtime) under that style.
//!
//! `options.py` no longer has a `TypedDict` rendering path at all
//! (`crate::backends::pyo3::gen_bindings::types::gen_options_py`): every type it publishes
//! renders as `@dataclass`, and a return-position type it does not publish stays the native
//! `#[pyclass]` -- both are attribute access. A real downstream crate's issue #183 was exactly
//! the `TypedDict` branch this module used to detect; the fix removed that branch from the backend
//! rather than teaching this module to route around it, so `typeddict_types` is now always
//! empty and this module no longer needs to ask `is_dataclass_backed_config` at all. It is kept
//! (rather than deleted outright) so a path can still be traced field-by-field for other
//! purposes -- see `record_edge` below -- and so a future rendering path that DOES need
//! subscript access has somewhere to plug back in without re-deriving this module's shape. ~keep
use std::collections::{HashMap, HashSet};

use crate::core::ir::TypeDef;
use crate::e2e::codegen::call_ir::{map_value_named_type, named_type};

use super::types::{PythonMapValueEdges, PythonTypedDictFacts, PythonTypedDictMap};

/// Build the `TypedDict`-membership set (always empty -- see this module's header comment) and
/// `(type, field) -> next type` traversal edges [`PythonTypedDictMap`] needs, by inspecting every
/// `TypeDef` this crate declares. A field is recorded as a traversal edge when its
/// [`named_type`]-resolved type is another `TypeDef` in this crate, exactly as
/// `ir_enum::build_ir_enum_map` and `ir_collection::build_ir_collection_map` do, so a
/// multi-segment path can advance its "current owner type" cursor one segment at a time before
/// asking `is_typeddict` at each link.
///
/// ~keep A MAP-typed field names nothing to [`named_type`] (by design — see
/// [`map_value_named_type`]), so before this it contributed no edge at all and the renderer had no
/// derivable owner for a `extras[key].title` path: it kept the MAP'S OWNER as the cursor, which
/// answers the classification of `title` with the type that owns `extras` rather than the type
/// `extras[key]` actually is. The map's VALUE type is recorded as its own edge so that question
/// has a derived answer instead of a retained one. The two edge sets stay separate because they
/// answer different hops — see [`PythonTypedDictMap`].
pub(super) fn build_python_typeddict_facts(type_defs: &[TypeDef]) -> PythonTypedDictFacts {
    let struct_names: HashSet<&str> = type_defs.iter().map(|t| t.name.as_str()).collect();

    let mut facts = PythonTypedDictFacts::default();

    for type_def in type_defs {
        for field in &type_def.fields {
            record_edge(
                &mut facts.typeddict_map.field_types,
                type_def,
                field,
                named_type(&field.ty),
                &struct_names,
            );
            record_map_value_edge(&mut facts.map_value_edges, type_def, field, &struct_names);
        }
    }

    facts
}

pub(super) fn build_python_typeddict_map(type_defs: &[TypeDef]) -> PythonTypedDictMap {
    build_python_typeddict_facts(type_defs).typeddict_map
}

fn record_map_value_edge(
    edges: &mut PythonMapValueEdges,
    type_def: &TypeDef,
    field: &crate::core::ir::FieldDef,
    struct_names: &HashSet<&str>,
) {
    let Some(named) = map_value_named_type(&field.ty) else {
        return;
    };
    if struct_names.contains(named) {
        record_edge(edges, type_def, field, Some(named), struct_names);
    }
}

/// Record `type_def.field -> resolved` in `edges`, when `resolved` names a `TypeDef` this crate
/// declares. A name the crate does not declare is dropped rather than recorded, so the cursor
/// never advances to a type nothing else in the map can answer questions about.
fn record_edge(
    edges: &mut HashMap<String, HashMap<String, String>>,
    type_def: &TypeDef,
    field: &crate::core::ir::FieldDef,
    resolved: Option<&str>,
    struct_names: &HashSet<&str>,
) {
    let Some(named) = resolved else { return };
    if !struct_names.contains(named) {
        return;
    }
    edges
        .entry(type_def.name.clone())
        .or_default()
        .insert(field.name.clone(), named.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeRef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn return_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields,
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        }
    }

    /// `ParseOutput { metadata: Metadata }`, `Metadata { title: String }`, both `is_return_type`.
    /// Neither is ever classified as `TypedDict` -- see this module's header comment
    /// (a real downstream crate's issue #183).
    fn type_defs() -> Vec<TypeDef> {
        vec![
            return_type(
                "ParseOutput",
                vec![field("metadata", TypeRef::Named("Metadata".to_string()))],
            ),
            return_type("Metadata", vec![field("title", TypeRef::String)]),
        ]
    }

    /// REGRESSION (a real downstream crate's issue #183): a return-position type is never classified as
    /// `TypedDict`, so the python e2e generator never renders subscript access (`result["field"]`)
    /// for one -- it always renders attribute access, agreeing with the native `#[pyclass]` the
    /// pyo3 backend actually returns and with `_native.pyi`'s declared shape.
    #[test]
    fn a_return_type_is_never_classified_as_typeddict() {
        let map = build_python_typeddict_map(&type_defs());
        assert!(map.typeddict_types.is_empty());
    }

    #[test]
    fn a_named_field_is_recorded_as_a_traversal_edge_regardless_of_typeddict_classification() {
        let map = build_python_typeddict_map(&type_defs());
        assert_eq!(
            map.field_types.get("ParseOutput").and_then(|f| f.get("metadata")),
            Some(&"Metadata".to_string())
        );
    }

    fn map_field_type_defs(value: TypeRef) -> Vec<TypeDef> {
        vec![
            return_type("ParseOutput", vec![field("extras", value)]),
            return_type("Metadata", vec![field("title", TypeRef::String)]),
        ]
    }

    fn string_map(value: TypeRef) -> TypeRef {
        TypeRef::Map(Box::new(TypeRef::String), Box::new(value))
    }

    /// `extras: HashMap<String, Metadata>` records the map's VALUE type as a map-value edge, so
    /// `extras[key].title` has a derivable owner for `title`.
    ///
    /// Reverting the fix drops the edge entirely (`named_type` names nothing for a map), leaving
    /// the internal map-value namespace empty and the renderer with nothing to advance to.
    #[test]
    fn a_map_valued_field_records_the_value_type_as_a_map_value_edge() {
        let facts =
            build_python_typeddict_facts(&map_field_type_defs(string_map(TypeRef::Named("Metadata".to_string()))));
        assert_eq!(
            facts
                .map_value_edges
                .get("ParseOutput")
                .and_then(|fields| fields.get("extras"))
                .map(String::as_str),
            Some("Metadata")
        );
    }

    /// The map-value edge does NOT also land in `field_types`: a plain field hop onto `extras`
    /// yields a `dict`, not a `Metadata`, and only the key-access segment may advance. ~keep
    #[test]
    fn a_map_valued_field_records_no_plain_field_edge() {
        let map = build_python_typeddict_map(&map_field_type_defs(string_map(TypeRef::Named("Metadata".to_string()))));
        assert_eq!(map.field_types.get("ParseOutput").and_then(|f| f.get("extras")), None);
    }

    /// CONTROL: `Option<T>` and `Vec<T>` fields keep recording exactly the plain `field_types`
    /// edge they always did, and gain no map-value edge — the shared `named_type` behaviour
    /// `ir_enum`/`ir_collection` also depend on is untouched by this change.
    #[test]
    fn optional_and_vec_named_fields_keep_their_plain_edge_and_gain_no_map_value_edge() {
        let named = || TypeRef::Named("Metadata".to_string());
        for wrapped in [
            TypeRef::Optional(Box::new(named())),
            TypeRef::Vec(Box::new(named())),
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(named())))),
        ] {
            let facts = build_python_typeddict_facts(&map_field_type_defs(wrapped));
            assert_eq!(
                facts
                    .typeddict_map
                    .field_types
                    .get("ParseOutput")
                    .and_then(|f| f.get("extras")),
                Some(&"Metadata".to_string()),
                "an Option/Vec of a named type is still a plain traversal edge"
            );
            assert!(
                facts.map_value_edges.is_empty(),
                "an Option/Vec of a named type is not a map and traverses no key access"
            );
        }
    }

    /// A map whose values name no `TypeDef` this crate declares records no edge at all — the
    /// documented "the IR cannot judge this hop" answer, distinct from a recorded non-`TypedDict`
    /// target.
    #[test]
    fn a_map_of_scalars_records_no_map_value_edge() {
        let facts = build_python_typeddict_facts(&map_field_type_defs(string_map(TypeRef::String)));
        assert!(facts.map_value_edges.is_empty());
    }
}
