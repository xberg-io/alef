//! Answers "is this field optional?" and "does the result declare this field at all?" against
//! the *exact type the call returns*, instead of by bare field name across the whole crate IR.
//!
//! `FieldResolver::ir_field_sets` has to answer both questions from flat name sets, because it
//! is handed nothing that identifies which type the call under generation actually returns. That
//! forces two compromises it documents honestly and this module removes:
//!
//! * optionality is decided by unanimity — a name counts as optional only when EVERY declaration
//!   of it in the crate is `Option<T>` — so one required twin on an unrelated struct silences the
//!   guard for the declaration that matters;
//! * reachability is decided by existence-anywhere, so a name declared on any type at all reads
//!   as a member of every result.
//!
//! Both are the safe default for a set that cannot tell types apart. Once the call's declared
//! return type is resolved (`codegen::call_ir::resolve_declared_result_type`), neither
//! compromise is needed: [`build_ir_result_field_map`] keys its answers by `(owner_type,
//! field_name)` and the two walkers below advance a type cursor from the root through the IR's
//! own struct graph before answering at the leaf — the same shape `ir_enum` and `ir_collection`
//! already use, and for the same reason.
//!
//! ~keep The optional set is *binding* optionality, not core-crate optionality. A NAPI binding
//! widens every field of a `Default`-implementing type to `Option<T>`, so a field declared
//! `metadata: PageMetadata` in Rust still reaches TypeScript as `readonly metadata?:
//! PageMetadata`; a snippet that renders `result.metadata.title` against it is a `TS18048`.
//! `OptionalityRule` carries which of those rules the target binding applies, and the NAPI arm
//! calls the binding backend's own predicate so the two can never drift.

use std::collections::HashSet;

use crate::backends::go::emission_facts::GoEmissionFacts;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{EnumDef, FieldDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::named_type;

use super::parse::{parse_path, segment_name};
use super::types::IrResultFieldMap;

/// Which "this field may be absent" rule the target language's binding applies.
///
/// A per-language choice rather than one shared answer because the bindings genuinely disagree,
/// and picking either one for everybody breaks the other half: guarding a wasm-bindgen getter
/// that always returns a value adds dead `?.` noise, while not guarding a NAPI `has_default`
/// field is a compile error in the generated snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionalityRule {
    /// Only the field's own declared type decides. Every binding except NAPI.
    DeclaredType,
    /// The NAPI rule, per `backends::napi::gen_bindings::types::napi_field_is_optional`: the
    /// field's own type, OR its owner implementing `Default`.
    Napi,
}

impl OptionalityRule {
    /// The rule the binding generated for `language` applies to its struct fields.
    pub(crate) fn for_language(language: &str) -> Self {
        match language {
            "node" | "typescript" => Self::Napi,
            _ => Self::DeclaredType,
        }
    }

    fn applies_to(self, field: &FieldDef, owner: &TypeDef) -> bool {
        match self {
            Self::DeclaredType => field.optional,
            Self::Napi => crate::backends::napi::napi_field_is_optional(field, owner),
        }
    }
}

/// Build the per-owner-type field facts [`IrResultFieldMap`] answers from.
///
/// `declared_fields` records only fields the binding actually attaches an accessor to
/// ([`binding_fields`], the same predicate every backend emits from), so a `#[serde(skip)]`
/// field is absent here exactly as it is absent from the generated class — a derived accessor
/// for it would not compile.
pub(super) fn build_ir_result_field_map(type_defs: &[TypeDef], rule: OptionalityRule) -> IrResultFieldMap {
    build_ir_result_field_map_with_enums(type_defs, &[], rule)
}

pub(super) fn build_ir_result_field_map_with_enums(
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    rule: OptionalityRule,
) -> IrResultFieldMap {
    let emitted = GoEmissionFacts::new(type_defs, enums, HashSet::new(), HashSet::new());
    build_go_ir_result_field_map(type_defs, enums, rule, &emitted)
}

pub(super) fn build_go_ir_result_field_map(
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    rule: OptionalityRule,
    emitted: &GoEmissionFacts<'_>,
) -> IrResultFieldMap {
    let names = GoFieldTypeNames {
        structs: &emitted.structs,
        enums: &emitted.unit_enums,
        passthrough_enums: &emitted.passthrough_enums,
        data_enums: &emitted.data_enums,
        pointer_variant_enums: &emitted.pointer_variant_enums,
    };
    let mut map = IrResultFieldMap::default();
    for type_def in type_defs
        .iter()
        .filter(|definition| emitted.structs.contains(definition.name.as_str()))
    {
        for field in binding_fields(&type_def.fields) {
            record_ir_result_field(&mut map, type_def, field, rule, &names);
        }
    }
    for enum_def in enums
        .iter()
        .filter(|definition| emitted.pointer_variant_enums.contains(definition.name.as_str()))
    {
        record_pointer_variant_enum_fields(&mut map, enum_def, names.structs);
    }
    map
}

/// Extend the map across a struct-shaped tagged-union enum's variant boundary the same way a
/// literal struct field does -- see [`GoEmissionFacts::pointer_variant_enums`].
///
/// Covers both `GoEnumRepresentation::ExternallyTaggedStruct`
/// (`gen_externally_tagged_union_type`) and `GoEnumRepresentation::TupleTaggedStruct`
/// (`gen_tuple_tagged_union_type`): both generators render the same
/// `tagged_union_variant_field.jinja` template and pick a variant's payload field the same way
/// (the first tuple-shaped field narrowed to `TypeRef::Named`), keyed by `wire_variant_value`, so
/// the two can never name a different field for the same variant. Every recorded field is
/// unconditionally a pointer: at most one variant is ever populated, regardless of whether the
/// tag lives outside the payload object or alongside it as a `#[serde(tag = "...")]` field --
/// this is the Go template's own fact, not an inference over an unresolved field. ~keep
fn record_pointer_variant_enum_fields(map: &mut IrResultFieldMap, enum_def: &EnumDef, struct_names: &HashSet<&str>) {
    for variant in &enum_def.variants {
        let Some(field) = variant.fields.first() else {
            continue;
        };
        let TypeRef::Named(struct_type_name) = &field.ty else {
            continue;
        };
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        map.declared_fields
            .entry(enum_def.name.clone())
            .or_default()
            .insert(wire_name.clone());
        map.pointer_fields
            .entry(enum_def.name.clone())
            .or_default()
            .insert(wire_name.clone());
        map.optional_fields
            .entry(enum_def.name.clone())
            .or_default()
            .insert(wire_name.clone());
        if struct_names.contains(struct_type_name.as_str()) {
            map.field_types
                .entry(enum_def.name.clone())
                .or_default()
                .insert(wire_name, struct_type_name.clone());
        }
    }
}

struct GoFieldTypeNames<'a> {
    structs: &'a HashSet<&'a str>,
    enums: &'a HashSet<&'a str>,
    passthrough_enums: &'a HashSet<&'a str>,
    data_enums: &'a HashSet<&'a str>,
    pointer_variant_enums: &'a HashSet<&'a str>,
}

fn record_ir_result_field(
    map: &mut IrResultFieldMap,
    type_def: &TypeDef,
    field: &FieldDef,
    rule: OptionalityRule,
    names: &GoFieldTypeNames<'_>,
) {
    map.declared_fields
        .entry(type_def.name.clone())
        .or_default()
        .insert(field.name.clone());
    if rule.applies_to(field, type_def) {
        map.optional_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
    }
    let go_type = crate::backends::go::go_struct_field_type(
        type_def,
        field,
        names.enums,
        names.passthrough_enums,
        names.data_enums,
        names.structs,
    );
    if go_type.starts_with('*') {
        map.pointer_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
    }
    if let Some(value_ty) = map_value_type(&field.ty)
        && !map_value_is_go_nilable(value_ty, names)
    {
        map.map_scalar_value_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
    }
    if named_type(&field.ty).is_some_and(|name| names.data_enums.contains(name)) {
        map.data_interface_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
    }
    record_ir_result_field_kind(map, type_def, field, names);
}

/// The `Map<K, V>` value type `ty` declares, peeling any wrapping `Option<..>` first so a
/// `Map<K, V>` field and an `Option<Map<K, V>>` field answer identically — a nil Go map still
/// safely returns `V`'s zero value on a missing-key read, so the field's own optionality never
/// changes what an indexed read of it can produce. `None` for every other shape.
fn map_value_type(ty: &TypeRef) -> Option<&TypeRef> {
    match ty {
        TypeRef::Map(_, value) => Some(value),
        TypeRef::Optional(inner) => map_value_type(inner),
        _ => None,
    }
}

/// Whether a map's value type `value_ty` is a Go-nilable kind: a pointer (`Optional<T>`), a
/// slice (`Vec<T>`, `Bytes`, `Json` — `json.RawMessage` is slice-backed), another map, or an
/// `interface{}` (a sealed-interface/data-enum `Named` type, or any `Named` type the IR could
/// not resolve to a struct/enum at all, which alef itself renders as `*json.RawMessage`).
/// `false` for every plain Go value kind: bare scalars, `Duration`, `Path`, a resolved struct,
/// or a resolved non-sealed enum — indexing a map of one of those can never produce `nil`.
fn map_value_is_go_nilable(value_ty: &TypeRef, names: &GoFieldTypeNames<'_>) -> bool {
    match value_ty {
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Bytes | TypeRef::Json | TypeRef::Map(_, _) => true,
        TypeRef::Named(name) => {
            names.data_enums.contains(name.as_str())
                || !(names.enums.contains(name.as_str())
                    || names.passthrough_enums.contains(name.as_str())
                    || names.structs.contains(name.as_str()))
        }
        _ => false,
    }
}

fn record_ir_result_field_kind(
    map: &mut IrResultFieldMap,
    type_def: &TypeDef,
    field: &FieldDef,
    names: &GoFieldTypeNames<'_>,
) {
    if type_ref_is_display_safe(&field.ty) {
        map.display_safe_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
    }
    let Some(named) = named_type(&field.ty) else {
        return;
    };
    // A field typed as an `ExternallyTaggedStruct` enum also advances the walk: its own
    // variants are recorded as this enum's pseudo-fields by `record_externally_tagged_enum_variants`,
    // so a path like `metadata.format.excel` can cross `format` the same way it crosses any
    // literal struct field, instead of stopping at `path_crosses_unwalkable_field`.
    let target = if names.structs.contains(named) || names.pointer_variant_enums.contains(named) {
        &mut map.field_types
    } else {
        map.unresolvable_named_fields
            .entry(type_def.name.clone())
            .or_default()
            .insert(field.name.clone());
        return;
    };
    target
        .entry(type_def.name.clone())
        .or_default()
        .insert(field.name.clone(), named.to_string());
}

/// Whether `ty` is a Rust type alef can positively vouch for as implementing `Display`: a bare
/// `String`, `char`, or numeric/`bool` primitive, with no wrapping at all.
///
/// An ALLOWLIST, not the `field_types` denylist-shaped check [`leaf_is_named_type`] makes do
/// with: guessing "safe" wrong here is a per-item snippet line that fails to compile, so every
/// other shape is deliberately refused, including ones that might genuinely implement `Display`
/// in a given crate. `Option<_>` never implements `Display` regardless of what it wraps (unlike
/// [`named_type`]'s peeling, which exists to answer a reachability question, not this one), so it
/// is refused here rather than unwrapped. `Vec<_>`, `Map<_, _>`, `Bytes` (`Vec<u8>`), a `Named`
/// struct/enum (`extract` discards `impl Display` before it reaches the IR, same gap
/// [`leaf_is_named_type`] documents), `Path`, `Json`, `Duration`, and `Unit` are refused for the
/// same reason.
pub(super) fn type_ref_is_display_safe(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Primitive(_))
}

/// Walk `path` from `map.root_type` through the IR struct graph and answer whether the leaf
/// segment is optional on the exact type that owns it.
///
/// `false` — never "unknown" — for an unresolved root, an unrecognized segment, or an unpopulated
/// map. Every one of those is the pre-anchoring answer for a field with no `fields_optional`
/// entry, so this is purely additive: it can only turn a `false` into a `true` when the IR
/// positively confirms the leaf is optional on the type the path reaches. Mirrors
/// `ir_collection::is_collection_path`.
pub(super) fn is_optional_path(map: &IrResultFieldMap, path: &str) -> bool {
    optionality_at_path(map, path).unwrap_or(false)
}

/// Return the binding's authoritative optionality when `path` resolves from the anchored root.
/// `None` means the IR cannot answer and callers may fall back to authored configuration.
pub(super) fn optionality_at_path(map: &IrResultFieldMap, path: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let (owner, leaf) = walk_to_owner_from(map, root, path)?;
    Some(
        map.optional_fields
            .get(owner)
            .is_some_and(|fields| fields.contains(&leaf)),
    )
}

pub(super) fn pointer_at_path(map: &IrResultFieldMap, path: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let (owner, leaf) = walk_to_owner_from(map, root, path)?;
    Some(
        map.pointer_fields
            .get(owner)
            .is_some_and(|fields| fields.contains(&leaf)),
    )
}

/// Whether `path`'s leaf is a `Map<K, V>` field (see [`map_value_type`]) whose value type `V`
/// is a plain, never-nil Go value kind, per [`map_value_is_go_nilable`]. `None` when the IR
/// cannot resolve `path` at all — callers must not treat that as "no" (a plain string value)
/// on an unresolved path, since that would wrongly strip a nil guard the IR never actually
/// vouched for.
pub(super) fn map_value_is_scalar_at_path(map: &IrResultFieldMap, path: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let (owner, leaf) = walk_to_owner_from(map, root, path)?;
    Some(
        map.map_scalar_value_fields
            .get(owner)
            .is_some_and(|fields| fields.contains(&leaf)),
    )
}

pub(super) fn data_interface_at_path(map: &IrResultFieldMap, path: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let (owner, leaf) = walk_to_owner_from(map, root, path)?;
    Some(
        map.data_interface_fields
            .get(owner)
            .is_some_and(|fields| fields.contains(&leaf)),
    )
}

/// Walk `path` from a known IR owner instead of the call result root. Tagged-union renderers use
/// this after narrowing a variant to its payload type. ~keep
pub(super) fn is_optional_path_from(map: &IrResultFieldMap, root: &str, path: &str) -> bool {
    let Some((owner, leaf)) = walk_to_owner_from(map, root, path) else {
        return false;
    };
    map.optional_fields
        .get(owner)
        .is_some_and(|fields| fields.contains(&leaf))
}

/// Whether `path`'s leaf segment is declared with a type this map cannot vouch for as
/// implementing `Display`: it resolves, after peeling `Option`/`Vec`, to a `Named` type from
/// the crate's own IR.
///
/// `extract` discards every `impl Display for X` before it reaches the IR (`Display` is one of
/// `STD_TRAITS`, dropped alongside `Debug`/`Clone`/etc. in
/// `extract::extractor::functions::impl_blocks`), so alef has no record of which IR types
/// genuinely implement it. `field_types` already carries exactly the fact needed to be
/// conservative about that gap: it is populated only for fields whose declared type unwraps to
/// a `Named` type ([`named_type`](crate::e2e::codegen::call_ir::named_type)), i.e. a struct or
/// enum this crate defines — the shape `println!("{}", ...)` fails to compile against unless
/// the type happens to derive/implement `Display` by hand. A scalar leaf (`String`, a numeric
/// primitive, `char`) never appears in `field_types`, so it reads as safe here, matching every
/// std type `display: true` was written for.
///
/// `false` — never "unsafe" — for an unresolved root, an unrecognized segment, or an unpopulated
/// map, mirroring [`is_optional_path`]'s fallback: caller must already default the flag to "no
/// warning" for a fixture with no IR in scope, so this cannot regress those.
pub(super) fn leaf_is_named_type(map: &IrResultFieldMap, path: &str) -> bool {
    let Some((owner, leaf)) = walk_to_owner(map, path) else {
        return false;
    };
    map.field_types
        .get(owner)
        .is_some_and(|fields| fields.contains_key(&leaf))
}

/// Whether the call's result type declares `path`'s FIRST segment as a binding-visible field.
///
/// `None` when nothing was anchored — no resolved root type, or a root type this map has no
/// fields for (an opaque handle, an enum, a type from outside the extracted surface). Callers
/// must treat `None` as "no answer" and fall back, exactly as `TargetParams::IrAbsent` does;
/// reading it as rejection would empty out every snippet whose result type is not a plain struct.
///
/// Only the first segment is judged. A deeper segment can legitimately walk into a type this map
/// does not carry (a map value, a `serde_json::Value`, a foreign type), and rejecting those would
/// discard real, compiling accessors to close a hole that only ever opened at the root. ~keep
pub(super) fn root_declares_first_segment(map: &IrResultFieldMap, first_segment: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    let declared = map.declared_fields.get(root)?;
    Some(declared.contains(first_segment))
}

/// Whether the call's result type declares EVERY segment of `path`, walking the IR struct graph
/// from the root the same way [`walk_to_owner`] does.
///
/// [`root_declares_first_segment`] judges the root step only, which leaves a derived accessor free
/// to invent any deeper segment it likes: a snippet showed `result.document.document_structure`
/// against a `document` type declaring only `nodes`, because `document` itself was a real field
/// and nothing looked further. This walks on.
///
/// `None` — no answer, caller falls back — for every state where the IR genuinely cannot judge:
/// an unresolved root, a type this map carries no fields for, a `length`/`count` pseudo-segment,
/// and (the load-bearing one) a prefix segment whose declared type is not a struct in this map at
/// all. That last case is a map value, a `serde_json::Value`, a primitive, or a type from outside
/// the extracted surface — reachable, spellable, and unjudgeable — so it keeps the conservatism
/// [`root_declares_first_segment`] documents rather than discarding real accessors. Only a segment
/// the IR positively knows the owner of, and positively does not find, answers `Some(false)`. ~keep
pub(super) fn root_declares_path(map: &IrResultFieldMap, path: &str) -> Option<bool> {
    let root = map.root_type.as_deref()?;
    type_declares_path(map, root, path)
}

/// The same walk [`root_declares_path`] does, starting from an explicit `owner_type` instead of
/// `map.root_type` — for a caller that has already resolved a different anchor `root_declares_path`
/// cannot itself express, e.g. a tagged-union variant's payload type once
/// [`path_crosses_unwalkable_field`] has been overridden by a `fields_method_calls` entry that
/// names how to cross that exact union. Shares every fallback `root_declares_path` documents
/// (`None` on an unresolvable prefix segment, `Some(false)` only on a positively-undeclared one).
pub(super) fn type_declares_path(map: &IrResultFieldMap, owner_type: &str, path: &str) -> Option<bool> {
    let segments = parse_path(path);
    let (last, prefix) = segments.split_last()?;

    let mut owner = owner_type;
    for segment in prefix {
        let name = segment_name(segment)?;
        if !map.declared_fields.get(owner)?.contains(name) {
            return Some(false);
        }
        owner = map.field_types.get(owner)?.get(name)?.as_str();
    }
    Some(map.declared_fields.get(owner)?.contains(segment_name(last)?))
}

/// Whether `path` walks PAST a segment that is a real, declared field but whose type this map
/// cannot advance through as a struct — the shape a tagged-union field has: a field like `format`
/// can be a genuine member of its owner type while its own type is an enum, which is never
/// entered into `field_types` (only struct-typed fields are, per [`build_ir_result_field_map`]),
/// so a path like `metadata.format.variant.detail` has nowhere left to walk after `format` yet
/// two more segments to go.
///
/// [`root_declares_path`] cannot answer this itself: it treats a declared-but-unresolvable prefix
/// segment as `None` ("no answer") on purpose, for the primitive/map/foreign-type cases where that
/// conservatism is correct. This asks the narrower, positive question those cases can't — was the
/// segment DECLARED, yet had no further hop, with path left to walk past it — which a foreign or
/// primitive-typed field never has (there is nothing to have "no further hop" from if the field
/// itself is the path's last segment). Unlike `is_enum_path` (`ir_enum` module), this needs no
/// `EnumDef` list wired in: any field the IR cannot advance through as a struct answers the same
/// way, whether the reason is a tagged union, a map value, or a `serde_json::Value` — all of which
/// share the one fact that matters here, that `accessor()` cannot walk a plain field access past
/// them either. ~keep
pub(super) fn path_crosses_unwalkable_field(map: &IrResultFieldMap, path: &str) -> bool {
    let Some(root) = map.root_type.as_deref() else {
        return false;
    };
    let segments = parse_path(path);
    let Some((_last, prefix)) = segments.split_last() else {
        return false;
    };

    let mut owner = root;
    for segment in prefix {
        let Some(name) = segment_name(segment) else {
            return false;
        };
        let Some(declared) = map.declared_fields.get(owner) else {
            return false;
        };
        if !declared.contains(name) {
            // An unknown field is `root_declares_path`'s concern (it answers `Some(false)` for
            // it directly), not this one's -- conflating the two would report "crosses" for a
            // path that never reached a real field to cross.
            return false;
        }
        // A segment naming another user type the IR would not walk into as a struct is the
        // positive "crosses" answer. A segment absent from `field_types` but ALSO absent from
        // `unresolvable_named_fields` names a scalar, `serde_json::Value`, or other opaque type
        // with no `Named` resolution at all -- unjudgeable, not unwalkable, so the walk must
        // still abstain (`false`) for it rather than reject a legitimate map/JSON traversal.
        if map
            .unresolvable_named_fields
            .get(owner)
            .is_some_and(|fields| fields.contains(name))
        {
            return true;
        }
        match map.field_types.get(owner).and_then(|fields| fields.get(name)) {
            Some(next) => owner = next.as_str(),
            None => return false,
        }
    }
    false
}

/// The `(owner_type, leaf_field_name)` a path resolves to, walking every prefix segment through
/// `field_types`. `None` when the root is unresolved or any segment names something the IR does
/// not recognize as a field on the type reached so far.
fn walk_to_owner<'a>(map: &'a IrResultFieldMap, path: &str) -> Option<(&'a str, String)> {
    let root = map.root_type.as_deref()?;
    walk_to_owner_from(map, root, path)
}

fn walk_to_owner_from<'a>(map: &'a IrResultFieldMap, root: &'a str, path: &str) -> Option<(&'a str, String)> {
    let segments = parse_path(path);
    let (last, prefix) = segments.split_last()?;

    let mut owner = root;
    for segment in prefix {
        let name = segment_name(segment)?;
        owner = map.field_types.get(owner)?.get(name)?.as_str();
    }
    Some((owner, segment_name(last)?.to_string()))
}

#[cfg(test)]
mod display_safe_field_tests {
    use super::*;
    use crate::core::ir::PrimitiveType;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// Table-driven allowlist check: only a bare `String`, `char`, or numeric/`bool` primitive is
    /// vouched for. Every wrapped or opaque shape — including `Option<String>`, which never
    /// implements `Display` no matter what it wraps — is refused, matching the task's explicit
    /// allowlist-over-denylist requirement rather than trying to unwrap toward a "real" leaf type.
    #[test]
    fn type_ref_is_display_safe_only_for_bare_scalars() {
        let cases: &[(&str, TypeRef, bool)] = &[
            ("string", TypeRef::String, true),
            ("char", TypeRef::Char, true),
            ("bool", TypeRef::Primitive(PrimitiveType::Bool), true),
            ("i32", TypeRef::Primitive(PrimitiveType::I32), true),
            ("f64", TypeRef::Primitive(PrimitiveType::F64), true),
            (
                "option_of_string_is_unsafe",
                TypeRef::Optional(Box::new(TypeRef::String)),
                false,
            ),
            (
                "vec_of_string_is_unsafe",
                TypeRef::Vec(Box::new(TypeRef::String)),
                false,
            ),
            (
                "nested_vec_of_string_is_unsafe",
                TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
                false,
            ),
            (
                "map_is_unsafe",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
            ),
            ("bytes_is_unsafe", TypeRef::Bytes, false),
            ("named_is_unsafe", TypeRef::Named("Widget".to_string()), false),
            ("path_is_unsafe", TypeRef::Path, false),
            ("json_is_unsafe", TypeRef::Json, false),
            ("duration_is_unsafe", TypeRef::Duration, false),
            ("unit_is_unsafe", TypeRef::Unit, false),
        ];
        for (name, ty, expected) in cases {
            assert_eq!(
                type_ref_is_display_safe(ty),
                *expected,
                "case `{name}` expected display-safe={expected}"
            );
        }
    }

    /// The builder wires the allowlist result into `display_safe_fields`, keyed by owner type —
    /// the shape [`super::super::resolver::display_safety`] reads directly.
    #[test]
    fn build_ir_result_field_map_populates_display_safe_fields_per_owner_type() {
        let type_defs = vec![TypeDef {
            name: "Table".to_string(),
            fields: vec![
                field("name", TypeRef::String),
                field("cells", TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))))),
            ],
            ..TypeDef::default()
        }];
        let map = build_ir_result_field_map(&type_defs, OptionalityRule::DeclaredType);
        assert!(map.display_safe_fields.get("Table").is_some_and(|f| f.contains("name")));
        assert!(
            !map.display_safe_fields
                .get("Table")
                .is_some_and(|f| f.contains("cells"))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef};

    fn field(name: &str, ty: crate::core::ir::TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// `Envelope { metadata: Metadata }`, `Metadata { format: VariantInfo, title: String }`, and
    /// `VariantInfo` is deliberately absent from `type_defs` — a tagged union (or any other type
    /// the IR's own struct graph does not carry) looks identical here: declared, but with no
    /// further hop.
    fn type_defs_with_unresolvable_variant_field() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![field(
                    "metadata",
                    crate::core::ir::TypeRef::Named("Metadata".to_string()),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".to_string(),
                fields: vec![
                    field("format", crate::core::ir::TypeRef::Named("VariantInfo".to_string())),
                    field("title", crate::core::ir::TypeRef::String),
                ],
                ..TypeDef::default()
            },
        ]
    }

    fn anchored_map(type_defs: &[TypeDef]) -> IrResultFieldMap {
        let mut map = build_ir_result_field_map(type_defs, OptionalityRule::DeclaredType);
        map.root_type = Some("Envelope".to_string());
        map
    }

    #[test]
    fn a_path_continuing_past_a_declared_but_unwalkable_field_crosses() {
        let map = anchored_map(&type_defs_with_unresolvable_variant_field());
        assert!(path_crosses_unwalkable_field(&map, "metadata.format.variant.detail"));
    }

    /// The control: a path that stops AT the unwalkable field, rather than past it, is exactly
    /// what `root_declares_path` already renders fine — this check must not fire for it.
    #[test]
    fn a_path_stopping_at_the_unwalkable_field_does_not_cross() {
        let map = anchored_map(&type_defs_with_unresolvable_variant_field());
        assert!(!path_crosses_unwalkable_field(&map, "metadata.format"));
    }

    /// A field the IR CAN walk through (a real struct-to-struct edge) must never be flagged,
    /// or every ordinary nested path in the suite would be rejected.
    #[test]
    fn a_path_through_a_real_struct_field_does_not_cross() {
        let map = anchored_map(&type_defs_with_unresolvable_variant_field());
        assert!(!path_crosses_unwalkable_field(&map, "metadata.title"));
    }

    /// An unknown segment is a different question (`root_declares_path` already answers `Some(false)`
    /// for it) — this check must stay silent rather than double-report it.
    #[test]
    fn a_path_through_an_undeclared_segment_does_not_cross() {
        let map = anchored_map(&type_defs_with_unresolvable_variant_field());
        assert!(!path_crosses_unwalkable_field(&map, "not_a_real_field.anything"));
    }

    /// The critical negative control: a field declared with NO `Named` type at all (a scalar, or
    /// `serde_json::Value`) must stay permissive, exactly like `root_declares_path` already does
    /// for it. An earlier version of this check conflated "no `field_types` entry" with "crosses
    /// an unwalkable field", which is true for a tagged union but NOT for a JSON/map value one
    /// entry (`document.payload.anything`) already derives an accessor through on purpose. This
    /// pins the fix `unresolvable_named_fields` makes: only a field that positively names ANOTHER
    /// user type must be flagged, never a field with no `Named` resolution whatsoever.
    #[test]
    fn a_path_through_a_field_with_no_named_type_at_all_does_not_cross() {
        let type_defs = vec![TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field("payload", crate::core::ir::TypeRef::Json)],
            ..TypeDef::default()
        }];
        let map = anchored_map(&type_defs);
        assert!(!path_crosses_unwalkable_field(&map, "payload.anything"));
    }

    #[test]
    fn no_anchored_root_never_crosses() {
        let mut map = build_ir_result_field_map(
            &type_defs_with_unresolvable_variant_field(),
            OptionalityRule::DeclaredType,
        );
        map.root_type = None;
        assert!(!path_crosses_unwalkable_field(&map, "metadata.format.variant.detail"));
    }

    /// Regression for a real downstream crate's `SheetCount` defect: PR #305 taught this walk to cross an
    /// *externally* tagged union's variant boundary (`GoEnumRepresentation::ExternallyTaggedStruct`)
    /// but left out an internally tagged one -- `#[serde(tag = "format_type")]`, which Go
    /// classifies as `GoEnumRepresentation::TupleTaggedStruct` and renders through the identical
    /// one-pointer-per-variant struct template. `pointer_at_path` silently stopped one segment
    /// early (`Metadata.format` had no further hop in `field_types`), so
    /// `FieldResolver::target_field_is_pointer` returned `None` and the go `greater_than`/
    /// `less_than` family compared the raw `*uint32` pointer against an untyped int: `invalid
    /// operation: mismatched types *uint32 and untyped int`.
    #[test]
    fn pointer_at_path_crosses_an_internally_tagged_tuple_struct_enum_variant() {
        let excel_metadata = TypeDef {
            name: "ExcelMetadata".to_string(),
            fields: vec![FieldDef {
                name: "sheet_count".to_string(),
                ty: crate::core::ir::TypeRef::Optional(Box::new(crate::core::ir::TypeRef::Primitive(
                    crate::core::ir::PrimitiveType::U32,
                ))),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![field(
                "format",
                crate::core::ir::TypeRef::Optional(Box::new(crate::core::ir::TypeRef::Named(
                    "FormatMetadata".to_string(),
                ))),
            )],
            ..TypeDef::default()
        };
        let envelope = TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field(
                "metadata",
                crate::core::ir::TypeRef::Named("Metadata".to_string()),
            )],
            ..TypeDef::default()
        };
        let format_metadata = crate::core::ir::EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![crate::core::ir::EnumVariant {
                name: "Excel".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: crate::core::ir::TypeRef::Named("ExcelMetadata".to_string()),
                    ..FieldDef::default()
                }],
                ..crate::core::ir::EnumVariant::default()
            }],
            ..crate::core::ir::EnumDef::default()
        };

        let type_defs = vec![envelope, metadata, excel_metadata];
        let enums = vec![format_metadata];

        let emitted = GoEmissionFacts::new(&type_defs, &enums, HashSet::new(), HashSet::new());
        assert!(
            emitted.pointer_variant_enums.contains("FormatMetadata"),
            "an internally tagged (`#[serde(tag = ...)]`) tuple-struct union must classify as a \
             pointer-variant enum exactly like an externally tagged one does"
        );

        let mut map = build_ir_result_field_map_with_enums(&type_defs, &enums, OptionalityRule::DeclaredType);
        map.root_type = Some("Envelope".to_string());

        assert_eq!(
            pointer_at_path(&map, "metadata.format.excel.sheet_count"),
            Some(true),
            "the walk must cross the internally tagged `format.excel` variant boundary and \
             report `sheet_count` as a pointer, or the go comparison-assertion emitter derefs \
             nothing"
        );
    }
}
