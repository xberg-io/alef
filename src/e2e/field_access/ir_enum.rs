//! Derives enum-field classification from the crate's own IR instead of trusting a
//! hand-written `alef.toml` `fields_enum` list to have enumerated every enum-typed result
//! field.
//!
//! Before this module existed, `FieldResolver::is_enum` answered purely from the
//! author-declared `fields_enum` set (`E2eConfig::effective_fields_enum`). A consumer that
//! never populated `fields_enum` got `false` for every field, so the Rust e2e generator
//! emitted `<field>.to_string()` for enum-typed fields — a compile error whenever the enum
//! does not implement `Display` (only `Debug` is a safe assumption for an arbitrary enum).
//!
//! The fix has to be type-driven, not name-driven: the same crate can declare `kind: String`
//! on one struct and `kind: SomeEnum` on another, so a bare-field-name rule would misclassify
//! one of them regardless of which way it defaults. [`build_ir_enum_map`] therefore keys its
//! answer by `(owner_type, field_name)`, and [`is_enum_path`] only trusts that answer once it
//! has walked the field path from a known root type through the IR's own struct graph to the
//! exact type that owns the leaf segment.
use std::collections::{HashMap, HashSet};

use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::call_ir::named_type;

use super::parse::{parse_path, segment_name};
use super::types::PathSegment;
use super::types::{IrEnumMap, TaggedEnumWire};

/// Build the `(type, field) -> is-enum` / `(type, field) -> next type` maps [`IrEnumMap`]
/// needs, by inspecting every field of every `TypeDef` this crate declares.
///
/// A field's declared type resolves through [`named_type`] — the same `Option`/`Vec` unwrapper
/// `CallIr` already uses for parameter and return types (`Box<T>` fields carry the unboxed
/// named type directly in the IR, so no separate unwrap is needed for them). When the
/// resolved name matches a real `EnumDef`, the field is recorded as enum-typed on its owner.
/// When it instead matches another `TypeDef` — a struct the path can keep traversing into —
/// it is recorded as a traversal edge so multi-segment paths like `choices[0].finish_reason`
/// can advance their type cursor one segment at a time. A field whose resolved name is
/// neither (a primitive, or an external/opaque type the IR did not resolve) lands in neither
/// map, and a path through it answers `false` in [`is_enum_path`] — the same safe default an
/// unconfigured `fields_enum` entry already had.
pub(super) fn build_ir_enum_map(type_defs: &[TypeDef], enums: &[EnumDef]) -> IrEnumMap {
    let enum_names: HashSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
    let struct_names: HashSet<&str> = type_defs.iter().map(|t| t.name.as_str()).collect();

    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut enum_fields: HashMap<String, HashSet<String>> = HashMap::new();
    let mut enum_field_types: HashMap<String, HashMap<String, String>> = HashMap::new();

    for type_def in type_defs {
        for field in &type_def.fields {
            let Some(named) = named_type(&field.ty) else {
                continue;
            };
            if enum_names.contains(named) {
                enum_fields
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone());
                enum_field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            } else if struct_names.contains(named) {
                field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            }
        }
    }

    let (variant_payload_types, variant_payload_is_collection) = build_variant_payload_types(enums);

    IrEnumMap {
        field_types,
        enum_fields,
        enum_field_types,
        variant_payload_types,
        variant_payload_is_collection,
        tagged_enum_wire: build_tagged_enum_wire(enums),
        data_carrying_enum_names: data_carrying_enum_names(enums),
        enum_wire_variants: build_enum_wire_variants(enums),
        root_type: None,
    }
}

/// The enums with at least one data-carrying variant, keyed by IR name.
///
/// The predicate is the negation of the `variants.iter().all(|v| v.fields.is_empty())` test the
/// Dart (`backends::dart::gen_bindings::wire_value::flat_wire_enums`), Kotlin
/// (`backends::kotlin::gen_bindings::object_wrapper::enums::emit_enum`) and Swift
/// (`backends::swift::gen_bindings::enums::emit_enum`) binding backends each apply before emitting
/// a scalar, string-lowerable representation. Asking it here, from the same IR those backends read,
/// is what keeps an assertion generator from appending a lowering accessor to a union the binding
/// rendered with a payload instead. ~keep
fn data_carrying_enum_names(enums: &[EnumDef]) -> HashSet<String> {
    enums
        .iter()
        .filter(|enum_def| enum_def.variants.iter().any(|variant| !variant.fields.is_empty()))
        .map(|enum_def| enum_def.name.clone())
        .collect()
}

/// Build `enum_wire_variants[enum_name][wire value] -> Rust variant identifier` — the reverse of
/// [`build_tagged_enum_wire`]'s per-variant map, for EVERY enum rather than only internally
/// tagged ones, and restricted to variants a serde rename actually moves off their identifier.
///
/// An entry is recorded only when all three hold, so that a lookup hit is unambiguous evidence
/// that the wire spelling and the Rust spelling disagree:
///
/// * the variant's wire value differs from its identifier (no rename means nothing to reconcile);
/// * no other variant of the same enum produces that same wire value;
/// * the wire value is not itself the identifier of some other variant of the same enum — such a
///   value would be a valid answer on both surfaces at once, and translating it would silently
///   redirect the assertion to a different variant.
///
/// Every excluded case leaves the caller with a `None`, i.e. its pre-existing behaviour. ~keep
fn build_enum_wire_variants(enums: &[EnumDef]) -> HashMap<String, HashMap<String, String>> {
    let mut per_enum = HashMap::new();
    for enum_def in enums {
        let identifiers: HashSet<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
        let mut by_wire: HashMap<String, String> = HashMap::new();
        let mut ambiguous: HashSet<String> = HashSet::new();
        for variant in &enum_def.variants {
            let wire = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            if wire == variant.name || identifiers.contains(wire.as_str()) {
                continue;
            }
            if by_wire.insert(wire.clone(), variant.name.clone()).is_some() {
                ambiguous.insert(wire);
            }
        }
        for wire in &ambiguous {
            by_wire.remove(wire);
        }
        if !by_wire.is_empty() {
            per_enum.insert(enum_def.name.clone(), by_wire);
        }
    }
    per_enum
}

fn build_tagged_enum_wire(enums: &[EnumDef]) -> HashMap<String, TaggedEnumWire> {
    enums
        .iter()
        .filter_map(|enum_def| {
            let tag = enum_def.serde_tag.clone()?;
            let variants = enum_def
                .variants
                .iter()
                .map(|variant| {
                    let wire = crate::codegen::naming::wire_variant_value(
                        &variant.name,
                        variant.serde_rename.as_deref(),
                        enum_def.serde_rename_all.as_deref(),
                    );
                    (variant.name.clone(), wire)
                })
                .collect();
            let content = enum_def.serde_content.clone();
            Some((enum_def.name.clone(), TaggedEnumWire { tag, variants, content }))
        })
        .collect()
}

/// Builds `variant_payload_types[enum][variant] -> (field_name, payload_type_name)` for every
/// tagged-union variant that carries exactly one field whose type resolves to a `Named` IR
/// type (through `Option`/`Vec` unwrapping, via [`named_type`]), alongside
/// `variant_payload_is_collection[enum]` — the subset of those variants whose payload field is
/// itself `Vec`-typed (`Variant(Vec<Item>)`) rather than a struct that merely wraps one
/// (`Variant(Payload)`); `named_type` unwraps `Vec` the same way it unwraps `Option`, so the
/// first map alone cannot distinguish the two shapes. A variant with zero or several fields has
/// no single payload type to record, so it is left out of both — callers asking for it get
/// `None`/`false` and must fall back to their own unimplemented-shape handling rather than
/// receive a misleading answer for one of several fields.
/// `variant_payload_types[enum][variant] -> (field_name, payload_type_name)` — see
/// [`IrEnumMap::variant_payload_types`] for the field this feeds.
type VariantPayloadTypeMap = HashMap<String, HashMap<String, (String, String)>>;

/// `variant_payload_is_collection[enum] -> variant names` — see
/// [`IrEnumMap::variant_payload_is_collection`] for the field this feeds.
type VariantPayloadCollectionMap = HashMap<String, HashSet<String>>;

fn build_variant_payload_types(enums: &[EnumDef]) -> (VariantPayloadTypeMap, VariantPayloadCollectionMap) {
    let mut variant_payload_types: HashMap<String, HashMap<String, (String, String)>> = HashMap::new();
    let mut variant_payload_is_collection: HashMap<String, HashSet<String>> = HashMap::new();
    for enum_def in enums {
        for variant in &enum_def.variants {
            let [only_field] = variant.fields.as_slice() else {
                continue;
            };
            let Some(named) = named_type(&only_field.ty) else {
                continue;
            };
            variant_payload_types
                .entry(enum_def.name.clone())
                .or_default()
                .insert(variant.name.clone(), (only_field.name.clone(), named.to_string()));
            if super::ir_collection::is_vec_type(&only_field.ty) {
                variant_payload_is_collection
                    .entry(enum_def.name.clone())
                    .or_default()
                    .insert(variant.name.clone());
            }
        }
    }
    (variant_payload_types, variant_payload_is_collection)
}

/// Walk `map.field_types` from `root` through `prefix`, returning the owner type the path's
/// last segment lands on — or `None` if any segment names something the IR does not recognize
/// as a field on the current owner. Shared by [`is_enum_path`] and [`enum_type_at_path`] so the
/// two answer from the exact same walk and can never disagree about which type a path reaches.
fn resolve_owner<'a>(map: &'a IrEnumMap, root: &'a str, prefix: &[PathSegment]) -> Option<&'a str> {
    let mut owner = root;
    for segment in prefix {
        let name = segment_name(segment)?;
        let next = map.field_types.get(owner).and_then(|fields| fields.get(name))?;
        owner = next.as_str();
    }
    Some(owner)
}

/// Walk `path` from `map.root_type` through `map.field_types`, answering whether the leaf
/// segment's declared type (per [`build_ir_enum_map`]) is a real IR enum.
///
/// Returns `false` — never "unknown" — whenever the root type is unresolved, a segment names
/// something the IR does not recognize as a field on the current owner type, or `map` was
/// never populated. Every one of those is the pre-existing behaviour for a field with no
/// `fields_enum` entry, so this is purely additive: it can only turn a `false` into a `true`
/// when the IR positively confirms the leaf is enum-typed on the exact type the path reaches.
pub(super) fn is_enum_path(map: &IrEnumMap, path: &str) -> bool {
    let Some(root) = map.root_type.as_deref() else {
        return false;
    };
    let segments = parse_path(path);
    let Some((last, prefix)) = segments.split_last() else {
        return false;
    };
    let Some(owner) = resolve_owner(map, root, prefix) else {
        return false;
    };
    let Some(name) = segment_name(last) else {
        return false;
    };
    map.enum_fields.get(owner).is_some_and(|fields| fields.contains(name))
}

/// Resolve the concrete IR enum type name backing `path`'s leaf segment, walking the same
/// `map.field_types` chain as [`is_enum_path`]. Returns `None` under the exact same
/// "unknown" conditions `is_enum_path` returns `false` for; callers that need to know *which*
/// enum a positively-classified field resolves to (not just that it is one) use this instead
/// of re-walking the path themselves.
pub(super) fn enum_type_at_path(map: &IrEnumMap, path: &str) -> Option<String> {
    let root = map.root_type.as_deref()?;
    enum_type_at_path_from(map, root, path)
}

/// The same walk [`enum_type_at_path`] does, starting from an explicit `owner` type instead of
/// `map.root_type` -- for a caller that has already crossed into a tagged-union variant's own
/// payload type and needs to resolve a SECOND enum-typed field declared on it (a union nested
/// inside another union's payload), which is not `map.root_type` and has no other way to anchor
/// this walk. [`enum_type_at_path`] delegates here with `map.root_type` so both callers share one
/// walk and can never disagree about which type a path reaches. ~keep
pub(super) fn enum_type_at_path_from(map: &IrEnumMap, owner: &str, path: &str) -> Option<String> {
    let segments = parse_path(path);
    let (last, prefix) = segments.split_last()?;
    let owner = resolve_owner(map, owner, prefix)?;
    let name = segment_name(last)?;
    map.enum_field_types
        .get(owner)
        .and_then(|fields| fields.get(name))
        .cloned()
}
