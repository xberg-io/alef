use crate::codegen::conversions::helpers::type_discovery::field_references_excluded_type;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Build the set of types that can have core→binding From safely generated.
/// More permissive than binding→core: allows sanitized fields (uses format!("{:?}"))
/// and accepts data enums (data discarded with `..` in match arms).
///
/// `excluded_field_types` lists type names that the calling backend excludes from
/// its binding surface (e.g. wasm `exclude_types`). Fields whose type appears in
/// this list are skipped in the binding struct AND the From impl, so they cannot
/// make a parent type non-convertible. Pass `&[]` from backends that have no
/// such exclusions.
pub fn core_to_binding_convertible_types(surface: &ApiSurface, excluded_field_types: &[String]) -> AHashSet<String> {
    let convertible_enums: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| can_generate_enum_conversion_from_core(e))
        .map(|e| e.name.as_str())
        .collect();

    let opaque_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    let data_enum_names: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    let (enum_paths, type_paths) = build_rust_path_maps(surface);

    let mut convertible: AHashSet<String> = surface
        .types
        .iter()
        .filter(|t| !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = convertible.iter().cloned().collect();
        let mut known: AHashSet<&str> = convertible.iter().map(|s| s.as_str()).collect();
        known.extend(&opaque_type_names);
        known.extend(&data_enum_names);
        let mut to_remove = Vec::new();
        for type_name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *type_name) {
                let ok = binding_fields(&typ.fields).all(|f| {
                    if f.sanitized {
                        true
                    } else if !excluded_field_types.is_empty()
                        && field_references_excluded_type(&f.ty, excluded_field_types)
                    {
                        true
                    } else if field_has_path_mismatch(f, &enum_paths, &type_paths) {
                        false
                    } else {
                        is_field_convertible(&f.ty, &convertible_enums, &known)
                    }
                });
                if !ok {
                    to_remove.push(type_name.clone());
                }
            }
        }
        for name in to_remove {
            if convertible.remove(&name) {
                changed = true;
            }
        }
    }
    convertible
}

/// Build the set of types that can have binding→core From safely generated.
/// Strict: excludes types with sanitized fields (lossy conversion).
/// This is transitive: a type is convertible only if all its Named field types
/// are also convertible (or are enums with From/Into support).
pub fn convertible_types(surface: &ApiSurface) -> AHashSet<String> {
    let convertible_enums: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| can_generate_enum_conversion(e))
        .map(|e| e.name.as_str())
        .collect();

    let _all_type_names: AHashSet<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();

    let default_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.has_default)
        .map(|t| t.name.as_str())
        .collect();

    let mut convertible: AHashSet<String> = surface
        .types
        .iter()
        .filter(|t| !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let opaque_type_names: AHashSet<&str> = surface
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    let data_enum_names: AHashSet<&str> = surface
        .enums
        .iter()
        .filter(|e| e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    let (enum_paths, type_paths) = build_rust_path_maps(surface);

    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = convertible.iter().cloned().collect();
        let mut known: AHashSet<&str> = convertible.iter().map(|s| s.as_str()).collect();
        known.extend(&opaque_type_names);
        known.extend(&data_enum_names);
        let mut to_remove = Vec::new();
        for type_name in &snapshot {
            if let Some(typ) = surface.types.iter().find(|t| t.name == *type_name) {
                let ok = binding_fields(&typ.fields).all(|f| {
                    if f.sanitized {
                        sanitized_field_has_default(&f.ty, &default_type_names)
                    } else if field_has_path_mismatch(f, &enum_paths, &type_paths) {
                        false
                    } else {
                        is_field_convertible(&f.ty, &convertible_enums, &known)
                    }
                });
                if !ok {
                    to_remove.push(type_name.clone());
                }
            }
        }
        for name in to_remove {
            if convertible.remove(&name) {
                changed = true;
            }
        }
    }
    convertible
}

/// Check if a sanitized field's type can produce a valid `Default::default()` expression.
/// Primitive types, strings, collections, Options, and Named types with `has_default` are fine.
/// Named types without `has_default` are not — generating `Default::default()` for them would
/// fail to compile.
fn sanitized_field_has_default(ty: &TypeRef, default_types: &AHashSet<&str>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Optional(_) => true,
        TypeRef::Vec(_) => true,
        TypeRef::Map(_, _) => true,
        TypeRef::Named(name) => {
            if is_tuple_type_name(name) {
                true
            } else {
                default_types.contains(name.as_str())
            }
        }
    }
}

/// Check if a specific type is in the convertible set.
pub fn can_generate_conversion(typ: &TypeDef, convertible: &AHashSet<String>) -> bool {
    convertible.contains(&typ.name)
}

/// Whether a backend emits `impl From<core::T> for T` for `typ`.
///
/// `core_to_binding` must be the set returned by [`core_to_binding_convertible_types`] for the
/// same surface and the same `excluded_field_types` the caller passes to its own type emitter —
/// this predicate deliberately takes the set rather than recomputing it, so a caller cannot pair
/// the right question with the wrong set. Trait definitions are excluded because no backend emits
/// a struct wrapper, and therefore no conversion, for one.
///
/// This is the single predicate shared by the site that *emits* the impl and every site that
/// *calls* it (`core_value.into()`). A caller that re-derives its own eligibility rule can emit a
/// `.into()` against a `From` impl that was never generated — the exact drift this exists to make
/// impossible. ~keep
pub fn core_to_binding_from_impl_emitted(typ: &TypeDef, core_to_binding: &AHashSet<String>) -> bool {
    !typ.is_trait && can_generate_conversion(typ, core_to_binding)
}

/// Whether the pyo3 backend gives `typ` a `from_json` staticmethod. Requires all three,
/// independently necessary, conditions: `typ` itself derives `serde::Deserialize`
/// (`TypeDef::has_serde` — without it there is no `Deserialize` impl to parse into), the
/// binding crate has `serde` + `serde_json` available (`crate_has_serde` — without it neither
/// `serde_json::from_str` nor the derive macros compile), and `typ` is in the core<->binding
/// convertible set (opaque types and types with inconvertible fields are excluded).
///
/// This is the single predicate shared by pyo3's raw-text `#[pymethods]` injection, its `.pyi`
/// stub declaration, and the e2e python snippet emitter's `from_json()` call-site selection —
/// call it from all three instead of re-checking the conditions separately, so the compiled
/// extension, its stub, and the doc snippets that call `from_json()` can never drift apart. ~keep
pub fn pyo3_from_json_eligible(typ: &TypeDef, crate_has_serde: bool, convertible_types: &AHashSet<String>) -> bool {
    crate_has_serde && typ.has_serde && convertible_types.contains(&typ.name)
}

pub(crate) fn is_field_convertible(
    ty: &TypeRef,
    convertible_enums: &AHashSet<&str>,
    known_types: &AHashSet<&str>,
) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_field_convertible(inner, convertible_enums, known_types),
        TypeRef::Map(k, v) => {
            is_field_convertible(k, convertible_enums, known_types)
                && is_field_convertible(v, convertible_enums, known_types)
        }
        TypeRef::Named(name) if is_tuple_type_name(name) => true,
        TypeRef::Named(name) => convertible_enums.contains(name.as_str()) || known_types.contains(name.as_str()),
    }
}

/// Check if a field's `type_rust_path` is compatible with the known type/enum rust_paths.
///
/// When a struct field has a `type_rust_path` that differs from the `rust_path` of the
/// enum or type with the same short name, the `.into()` conversion will fail because
/// the `From` impl targets a different type. This detects such mismatches.
fn field_has_path_mismatch(
    field: &FieldDef,
    enum_rust_paths: &AHashMap<&str, &str>,
    type_rust_paths: &AHashMap<&str, &str>,
) -> bool {
    let name = match &field.ty {
        TypeRef::Named(n) => n.as_str(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => n.as_str(),
            _ => return false,
        },
        _ => return false,
    };

    if let Some(field_path) = &field.type_rust_path {
        if let Some(enum_path) = enum_rust_paths.get(name)
            && !paths_compatible(field_path, enum_path)
        {
            return true;
        }
        if let Some(type_path) = type_rust_paths.get(name)
            && !paths_compatible(field_path, type_path)
        {
            return true;
        }
    }
    false
}

/// Check if two rust paths refer to the same type.
///
/// Handles re-exports: `crate::module::Type` and `crate::Type` are compatible
/// when they share the same crate root and type name (the type is re-exported).
fn paths_compatible(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a_norm = a.replace('-', "_");
    let b_norm = b.replace('-', "_");
    if a_norm == b_norm {
        return true;
    }
    if a_norm.ends_with(&b_norm) || b_norm.ends_with(&a_norm) {
        return true;
    }
    let a_root = a_norm.split("::").next().unwrap_or("");
    let b_root = b_norm.split("::").next().unwrap_or("");
    let a_name = a_norm.rsplit("::").next().unwrap_or("");
    let b_name = b_norm.rsplit("::").next().unwrap_or("");
    a_root == b_root && a_name == b_name
}

/// Build maps of name -> rust_path for enums and types in the API surface.
fn build_rust_path_maps(surface: &ApiSurface) -> (AHashMap<&str, &str>, AHashMap<&str, &str>) {
    let enum_paths: AHashMap<&str, &str> = surface
        .enums
        .iter()
        .map(|e| (e.name.as_str(), e.rust_path.as_str()))
        .collect();
    let type_paths: AHashMap<&str, &str> = surface
        .types
        .iter()
        .map(|t| (t.name.as_str(), t.rust_path.as_str()))
        .collect();
    (enum_paths, type_paths)
}

/// Check if an enum can have From/Into safely generated (both directions).
/// All enums are allowed — data variants use Default::default() for non-simple fields
/// in the binding→core direction.
pub fn can_generate_enum_conversion(enum_def: &EnumDef) -> bool {
    !enum_def.variants.is_empty()
}

/// Check if an enum can have core→binding From safely generated.
/// This is always possible: unit variants map 1:1, data variants discard data with `..`.
pub fn can_generate_enum_conversion_from_core(enum_def: &EnumDef) -> bool {
    !enum_def.variants.is_empty()
}

/// Returns true if fields represent a tuple variant (positional: _0, _1, ...).
pub fn is_tuple_variant(fields: &[FieldDef]) -> bool {
    !fields.is_empty()
        && fields[0]
            .name
            .strip_prefix('_')
            .is_some_and(|rest: &str| rest.chars().all(|c: char| c.is_ascii_digit()))
}

/// Returns true if serde represents `variant` in tuple form `Variant(T)` rather than
/// struct form `Variant { _0: T }`.
///
/// Covers all four serde enum representations explicitly, one arm each, rather than
/// letting a case fall out by accident:
/// - **Untagged** (`#[serde(untagged)]`): tuple form -- `Variant(T)` serializes as the bare
///   value of `T`.
/// - **Adjacently tagged** (`tag` + `content`): tuple form -- `{"<tag>": "Variant", "<content>":
///   T}`.
/// - **Internally tagged** (`tag`, no `content`): struct form, and ONLY struct form -- a
///   newtype payload here must flatten its own fields onto the tag object at the top level
///   (`serde` requires the payload be a map), so there is no positional slot to fill; tuple
///   form would be a different kind of wrong (the payload has no shape to hold it).
/// - **Externally tagged** (the default: no `tag`, no `content`, not untagged): tuple form --
///   `Variant(T)` serializes as `{"Variant": T}`, the payload directly under the tag key, with
///   no extra nesting. (Confirmed against a real consumer enum of this shape, whose actual wire
///   is `{"custom": "foo"}`, not `{"custom": {"_0": "foo"}}` -- the
///   struct-form shape this predicate used to imply for this exact case.)
///
/// A backend whose enum body emitter follows serde here must use this same predicate for its
/// conversion match arms, or the definition and the `From` impls disagree in shape and rustc
/// rejects them with E0559 / E0769.
///
/// Project-agnostic on purpose: the emitter and the conversion layer must not each
/// carry their own copy of this rule. ~keep
pub fn variant_emits_tuple_form(enum_def: &EnumDef, variant: &EnumVariant) -> bool {
    if !variant.is_tuple {
        return false;
    }
    if enum_def.serde_untagged || enum_def.serde_content.is_some() {
        return true;
    }
    // Internally tagged (`tag` set, no `content`) is the one representation that must stay
    // struct form: a newtype payload flattens onto the tag object, so no positional slot exists.
    enum_def.serde_tag.is_none()
}

/// Returns true if a TypeDef represents a newtype struct (single unnamed field `_0`).
pub fn is_newtype(typ: &TypeDef) -> bool {
    typ.fields.len() == 1 && typ.fields[0].name == "_0"
}

/// Returns true if a type name looks like a tuple (starts with `(`).
/// Tuple types are passthrough — no conversion needed.
pub(crate) fn is_tuple_type_name(name: &str) -> bool {
    name.starts_with('(')
}

/// Returns true for `Vec<Named>` whose element is a real (non-tuple) named type. As the element
/// of an outer `Vec`, it needs a per-inner-element `.into()`; a bare `.collect()` only converts
/// the outer container and leaves the inner `Vec<CoreT>` where `Vec<BindingT>` is expected.
pub(crate) fn is_vec_of_named(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n)))
}

/// Check if a type has any sanitized fields (binding→core conversion is lossy).
pub fn has_sanitized_fields(typ: &TypeDef) -> bool {
    binding_fields(&typ.fields).any(|f| f.sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuple_variant() -> EnumVariant {
        EnumVariant {
            name: "Custom".to_string(),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Untagged (`#[serde(untagged)]`): tuple form -- the payload serializes as the bare value.
    #[test]
    fn untagged_enum_newtype_variant_emits_tuple_form() {
        let enum_def = EnumDef {
            serde_untagged: true,
            ..Default::default()
        };
        assert!(variant_emits_tuple_form(&enum_def, &tuple_variant()));
    }

    /// Adjacently tagged (`tag` + `content`): tuple form -- `{"<tag>": "Custom", "<content>": T}`.
    #[test]
    fn adjacently_tagged_enum_newtype_variant_emits_tuple_form() {
        let enum_def = EnumDef {
            serde_tag: Some("type".to_string()),
            serde_content: Some("value".to_string()),
            ..Default::default()
        };
        assert!(variant_emits_tuple_form(&enum_def, &tuple_variant()));
    }

    /// Internally tagged (`tag`, no `content`): the ONE representation that must stay struct
    /// form -- a newtype payload flattens its own fields onto the tag object, so there is no
    /// positional slot for tuple form to fill. This is the negative case a careless widening of
    /// the predicate breaks: before this test existed, the predicate covered exactly untagged
    /// and adjacently-tagged, and simply adding "also true when `serde_tag.is_none()`" without
    /// this case pinned would have been indistinguishable, on the type signature alone, from
    /// "also true whenever `serde_tag` is anything" -- which would wrongly flip this case too.
    #[test]
    fn internally_tagged_enum_newtype_variant_does_not_emit_tuple_form() {
        let enum_def = EnumDef {
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        assert!(!variant_emits_tuple_form(&enum_def, &tuple_variant()));
    }

    /// Externally tagged (the serde default: no `tag`, no `content`, not untagged): tuple form
    /// -- `Custom(String)` serializes as `{"custom": "foo"}`, the payload directly under the tag
    /// key with no extra nesting. This is the representation the predicate used to omit
    /// entirely (returning struct form, i.e. `{"custom": {"_0": "foo"}}` once a backend rendered
    /// it), which was the root cause behind Magnus's Ruby bindings exposing the synthesized
    /// positional field name `_0` on the wire for `EntityCategory::Custom(String)` and similar
    /// real xberg core enums.
    #[test]
    fn externally_tagged_enum_newtype_variant_emits_tuple_form() {
        let enum_def = EnumDef::default();
        assert!(variant_emits_tuple_form(&enum_def, &tuple_variant()));
    }

    /// A unit variant (no fields, `is_tuple: false`) must never emit tuple form under any
    /// representation -- guards against a fixture mistake in the four cases above ever
    /// silently passing because `is_tuple` was left `false`.
    #[test]
    fn unit_variant_never_emits_tuple_form() {
        let unit_variant = EnumVariant::default();
        for enum_def in [
            EnumDef::default(),
            EnumDef {
                serde_untagged: true,
                ..Default::default()
            },
            EnumDef {
                serde_tag: Some("type".to_string()),
                ..Default::default()
            },
        ] {
            assert!(!variant_emits_tuple_form(&enum_def, &unit_variant));
        }
    }
}
