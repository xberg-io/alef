//! Enum code generators for the Magnus (Ruby) backend, including serde type helpers and variant constructors.

use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use std::collections::HashSet;

/// Preserve untagged record identity as a native enum instead of guessing from a Hash.
///
/// Tagged representations (internal, adjacent, external) are deliberately excluded even when
/// every variant has the same single-named-payload shape: their discriminator key makes Hash
/// reconstruction unambiguous, and `gen_tagged_enum_ruby_classes` (`mod.rs`) already generates a
/// `from_hash` marker module for exactly that case. A native `#[magnus::wrap]` class shares the
/// enum's own name with that marker module, so emitting both is a hard Ruby `TypeError` at
/// require time. ~keep
///
/// The native class is an *additional* ingress form, never the only one: `enum_magnus.rs.jinja`
/// keeps the serde reader as this shape's `TryConvert` fallback. The mirrored enum still derives
/// `Deserialize`, so every variant's own wire form stays a legitimate value for a field of this
/// type, and a caller holding a decoded String or Hash (a fixture, a JSON round trip) has no
/// wrapped instance to offer. ~keep
pub(crate) fn is_native_payload_enum(def: &EnumDef) -> bool {
    def.serde_untagged
        && def.serde_tag.is_none()
        && def.cfg.is_none()
        && !def.variants.is_empty()
        && def.variants.iter().all(|v| {
            v.cfg.is_none()
                && !v.binding_excluded
                && v.is_tuple
                && v.fields.len() == 1
                && !v.fields[0].sanitized
                && !v.fields[0].binding_excluded
                && matches!(v.fields[0].ty, TypeRef::Named(_))
        })
}

/// The variants `enum_def`'s own Magnus wrapper `enum` (rendered by [`gen_enum`] below) actually
/// declares, per the [`enum_variant_declaration`] authority. Shared by `gen_enum` and the two
/// per-variant-constructor generators below: a factory built here emits `Self::<Variant> { .. }`
/// against that SAME wrapper type (not the core dependency's), so a constructor for a variant
/// `gen_enum` dropped is a hard `E0599` (`Self` has no such variant), and a `method!` registration
/// for a dropped constructor is a hard `E0599` too (`method!` resolves the path at compile time).
/// All three must therefore agree on the identical declared set. ~keep
fn declared_enum_variants<'a>(
    enum_def: &'a EnumDef,
    is_host_enum: bool,
    configured_features: Option<&HashSet<&str>>,
) -> Vec<&'a EnumVariant> {
    enum_def
        .variants
        .iter()
        .filter(|v| {
            !matches!(
                enum_variant_declaration(v, is_host_enum, configured_features),
                VariantDeclaration::Drop
            )
        })
        .collect()
}

/// Generate a Magnus enum definition with IntoValue and TryConvert impls.
/// Unit-variant enums are represented as Ruby Symbols for ergonomic Ruby usage.
///
/// `configured_features` is this binding's own configured feature set (see
/// `ConversionConfig::configured_features`'s doc comment), threaded through to
/// [`enum_variant_declaration`] -- the same authority `gen_enum_from_binding_to_core_cfg`/
/// `gen_enum_from_core_to_binding_cfg` already consult for this enum's conversion arms (see
/// `magnus_conv_config` in `gen_bindings::mod`) -- so a FOREIGN cfg-gated variant this binding's
/// own feature set proves unreachable is never declared here either, matching
/// `backends::rustler::gen_bindings::types::gen_enum`. Only the Keep/Drop verdict is used, never
/// the `cfg` a `Keep` carries: like Rustler, a kept variant is always declared unconditionally
/// with no per-variant `#[cfg(...)]` on the declaration -- `enum_variant_declaration` never
/// resolves a host-owned gate to `Drop`, so a host-owned variant is always kept regardless. ~keep
#[cfg(test)]
pub fn gen_enum(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
    types: &[crate::core::ir::TypeDef],
) -> String {
    gen_enum_with_module(
        enum_def,
        core_import,
        configured_features,
        types,
        &crate::backends::magnus::gen_bindings::get_module_name(core_import),
    )
}

pub fn gen_enum_with_module(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
    types: &[crate::core::ir::TypeDef],
    module_name: &str,
) -> String {
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let declared_variants: Vec<&EnumVariant> =
        declared_enum_variants(enum_def, is_host_enum, configured_features_set.as_ref());

    let has_data = declared_variants.iter().any(|v| !v.fields.is_empty());
    let first_variant = declared_variants.first().map(|v| v.name.as_str()).unwrap_or("Default");

    // Find the declared variant marked with #[default], or fall back to the first declared
    // variant -- never a variant this declaration itself dropped as unreachable.
    let default = declared_variants
        .iter()
        .find(|v| v.is_default)
        .or(declared_variants.first());
    let default_variant = default.map(|v| v.name.as_str()).unwrap_or(first_variant);

    // variant). When `#[default]` selects a unit variant (e.g. `PageAction::Scrape`)
    let first_variant_default = if has_data {
        match default {
            Some(default) if !default.fields.is_empty() => {
                if emits_tuple_variant(enum_def, default) {
                    let field_defaults: Vec<&str> = default.fields.iter().map(|_| "Default::default()").collect();
                    format!("({})", field_defaults.join(", "))
                } else {
                    let field_defaults: Vec<String> = default
                        .fields
                        .iter()
                        .map(|f| format!("{}: Default::default()", f.name))
                        .collect();
                    format!(" {{ {} }}", field_defaults.join(", "))
                }
            }
            _ => String::new(),
        }
    } else {
        String::new()
    };

    let variants: Vec<minijinja::Value> = declared_variants
        .iter()
        .map(|variant| {
            let flatten_newtype =
                crate::codegen::serde_enum_repr::serde_flattens_newtype_payload(enum_def, variant, types);
            let fields: Vec<minijinja::Value> = variant
                .fields
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let serde_rename = if flatten_newtype {
                        None
                    } else {
                        positional_field_serde_rename(f, idx, &variant.name, variant.fields.len())
                    };
                    minijinja::context! {
                        name => &f.name,
                        field_type => field_type_for_serde(f),
                        flatten_newtype => flatten_newtype,
                        serde_rename => serde_rename,
                    }
                })
                .collect();

            let snake_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            let wire_name = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );

            minijinja::context! {
                name => &variant.name,
                serde_rename => &variant.serde_rename,
                fields => &fields,
                is_tuple => variant.is_tuple,
                emits_as_tuple => emits_tuple_variant(enum_def, variant),
                snake_name => &snake_name,
                wire_name => &wire_name,
                accepted_input_values => accepted_unit_variant_input_spellings(&variant.name, &snake_name, &wire_name),
                typed_newtype => enum_def.serde_tag.is_some() && variant.fields.len() == 1
                    && variant.fields[0].name == "_0"
                    && matches!(&variant.fields[0].ty, TypeRef::Named(name)
                        if types.iter().any(|t| t.name == *name && !t.is_opaque && !t.is_trait)),
                payload_type => variant.fields.first().map(|field| serde_field_type(&field.ty, field.optional)),
                payload_boxed => variant.fields.first().is_some_and(|field| field.is_boxed),
            }
        })
        .collect();

    crate::backends::magnus::template_env::render(
        "enum_magnus.rs.jinja",
        minijinja::context! {
            enum_name => &enum_def.name,
            module_name => module_name,
            native_payload => is_native_payload_enum(enum_def),
            has_data => has_data,
            has_default => enum_def.has_default,
            serde_tag => &enum_def.serde_tag,
            serde_content => &enum_def.serde_content,
            serde_untagged => enum_def.serde_untagged,
            serde_rename_all => &enum_def.serde_rename_all,
            variants => &variants,
            first_variant => first_variant,
            default_variant => default_variant,
            first_variant_default => &first_variant_default,
        },
    )
}

/// Distinct string spellings a unit enum's `TryConvert` accepts for one variant, in priority
/// order: the real serde wire value first (the canonical round-trip spelling now that
/// `IntoValue` emits it), then the always-snake_case symbol Magnus used to emit unconditionally
/// (kept for backward compatibility with existing consumer code), then the verbatim PascalCase
/// Rust name. Deduplicated so `rename_all = "snake_case"` (where all three often coincide)
/// does not produce a Rust "unreachable pattern" warning from repeated match-arm literals.
fn accepted_unit_variant_input_spellings(variant_name: &str, snake_name: &str, wire_name: &str) -> Vec<String> {
    let mut spellings = vec![wire_name.to_string()];
    for candidate in [snake_name, variant_name] {
        if !spellings.iter().any(|existing| existing == candidate) {
            spellings.push(candidate.to_string());
        }
    }
    spellings
}

fn emits_tuple_variant(enum_def: &EnumDef, variant: &crate::core::ir::EnumVariant) -> bool {
    // ~keep Delegates so the enum body emitter and the conversion match arms cannot drift.
    crate::codegen::conversions::helpers::variant_emits_tuple_form(enum_def, variant)
}

/// A `#[serde(rename = "...")]` for a variant field whose IR name is a synthesized positional
/// name (`_0`, `_1`, ...), or `None` for a genuinely named field (left untouched).
///
/// This USED to be the only fix applied for the common consumer shape of externally-tagged unit
/// variants plus a newtype `Custom(String)`, but
/// renaming `_0` to `value` on a STRUCT-form field only hid the symptom: it still produced
/// `{"custom": {"value": "foo"}}` against serde's real wire for that shape, `{"custom": "foo"}`
/// (confirmed against a real consumer enum). The real fix landed in
/// `codegen::conversions::helpers::variant_emits_tuple_form`
/// (`src/codegen/conversions/helpers/eligibility.rs`), which now emits TUPLE form for
/// externally-tagged newtype variants too (in addition to untagged and adjacently-tagged, which
/// it already covered) -- matching serde's real wire directly, with no field name involved at
/// all. That widening was proven single-consumer-safe before landing: the shared conversion arms
/// in `codegen/conversions/enums.rs` that must stay in lockstep with whatever this predicate says
/// are gated behind `ConversionConfig::binding_tuple_form_for_variants`
/// (`src/codegen/conversions/config.rs`), which only Magnus's own declaration sets `true`
/// (`magnus/gen_bindings/mod.rs`) -- so no other backend's declaration or conversion arms could
/// be affected by the change.
///
/// With that landed, this function is UNREACHABLE for every externally-tagged, untagged, and
/// adjacently-tagged newtype variant the extractor can produce -- [`emits_tuple_variant`] is
/// `true` for all of those now, and the template only calls this function on the struct-form
/// branch. It survives for exactly one residual shape: an INTERNALLY-tagged (`#[serde(tag =
/// "...")]`, no `content`) newtype variant whose payload type IS `Named` and serde WOULD flatten
/// onto the tag object at runtime, but the payload type definition isn't present in the `types`
/// slice this call received. `serde_flattens_newtype_payload` (and this file's own
/// `flatten_newtype`) silently falls back to `false` -- i.e. struct form -- when the payload type
/// is out of reach, and internally-tagged enums are the one representation
/// `variant_emits_tuple_form` deliberately keeps in struct form even when `is_tuple` is true (a
/// flattening newtype payload has no positional slot for tuple form to fill). In that residual
/// case the Rust field is still literally `_0`/`_1`/... in struct form, so it still needs a
/// semantic rename. Renames ONLY the field's serde wire name -- the Rust identifier itself stays
/// `_0`/`_1`/... unchanged, since `codegen::conversions::helpers::enum_arms` (shared by every
/// backend, not just Magnus) pattern-matches and struct-literals on that identifier verbatim;
/// renaming the identifier here without also touching that shared file is a hard compile error
/// (`E0559`/`E0026`). ~keep
fn positional_field_serde_rename(
    field: &FieldDef,
    field_idx: usize,
    variant_name: &str,
    total_fields: usize,
) -> Option<String> {
    let stripped = field.name.strip_prefix('_')?;
    if stripped.is_empty() || !stripped.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    if total_fields == 1 {
        if let TypeRef::Named(type_name) = &field.ty
            && let Some(remainder) = type_name.strip_prefix(variant_name)
        {
            let derived = crate::codegen::naming::pascal_to_snake(remainder);
            if !derived.is_empty() {
                return Some(derived);
            }
        }
        return Some("value".to_string());
    }

    Some(format!("value{field_idx}"))
}

/// Map a field type to a Rust type suitable for serde deserialization in data enums.
/// Helper to recursively map inner TypeRef to serde type strings.
/// For types that need JSON marshalling (Vec<Named>, Map, etc.), returns "String"
/// to indicate they should be JSON-serialized. Otherwise returns the proper type.
fn field_type_for_serde_inner(ty: &TypeRef) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "u8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "u16".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "u64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "usize".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "i8".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "i16".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "i64".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "isize".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Vec(inner) => format!("Vec<{}>", field_type_for_serde_inner(inner)),
        TypeRef::Map(_, _) => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", field_type_for_serde_inner(inner)),
        _ => "String".to_string(),
    }
}

pub(super) fn field_type_for_serde(field: &FieldDef) -> String {
    serde_field_type_with_box(&field.ty, field.optional, field.is_boxed)
}

/// Serde-shaped Rust type for a data-enum field of type `ty` (wrapping in `Option<...>` when
/// `optional`). This is the type the generated `enum {{ name }}` variant declares, so per-variant
/// constructor parameters must use it verbatim — the magnus data enum is binding-shaped, so the
/// constructor assigns parameters into the variant with no core conversion.
pub(super) fn serde_field_type(ty: &TypeRef, optional: bool) -> String {
    serde_field_type_with_box(ty, optional, false)
}

fn serde_field_type_with_box(ty: &TypeRef, optional: bool, is_boxed: bool) -> String {
    let base = field_type_for_serde_inner(ty);
    let base = if is_boxed && matches!(ty, TypeRef::Named(_)) {
        format!("Box<{base}>")
    } else {
        base
    };
    if optional { format!("Option<{base}>") } else { base }
}

/// Generate per-variant singleton constructors for a data enum.
///
/// For a data enum `Shape { Circle { radius }, Rect { width, height } }`, emits an `impl Shape`
/// block with one constructor per data-carrying struct variant so Ruby callers write
/// `Shape.circle(radius)` / `Shape.rect(width, height)` instead of building a raw Hash. Each
/// constructor builds the serde-shaped variant directly (`Self::Circle { radius }`).
///
/// Variant selection (skipping unit/tuple/`binding_excluded` variants) is shared with pyo3 and
/// rustler via `collect_all_variant_constructors`. Unlike `collect_variant_constructors`, this does
/// not yield to a same-named `impl` method: no backend forwards that hand-written method into the
/// generated binding, so yielding to it used to drop the constructor entirely with nothing to
/// replace it. The Rust function name is `_factory_<snake>` to avoid colliding with the variant
/// accessor of the same snake_case name; Ruby registers it under the bare snake name via
/// `define_singleton_method`.
///
/// `core_import`/`configured_features` narrow the constructor set to [`declared_enum_variants`] --
/// the SAME set `gen_enum` above declares for the wrapper `enum` this constructor's `Self::<Variant>`
/// literal references. Without this, a FOREIGN cfg-gated variant `gen_enum` already drops left its
/// constructor still emitting `Self::Rect { .. }` for a variant the wrapper enum no longer has: a
/// hard `E0599`, not a warning.
///
/// Returns an empty string when no variant qualifies (no empty `impl` block).
pub fn gen_data_enum_variant_constructors(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> String {
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let declared_features = configured_features_set.as_ref();
    let declared: Vec<&EnumVariant> = declared_enum_variants(enum_def, is_host_enum, declared_features);
    let declared_names: HashSet<&str> = declared.iter().map(|v| v.name.as_str()).collect();

    let constructors: Vec<_> = crate::codegen::generators::collect_all_variant_constructors(enum_def)
        .into_iter()
        .filter(|ctor| declared_names.contains(ctor.variant_name))
        .collect();
    if constructors.is_empty() {
        return String::new();
    }

    let rendered: Vec<minijinja::Value> = constructors
        .iter()
        .map(|ctor| {
            let params = ctor
                .params
                .iter()
                .map(|p| format!("{}: {}", p.name, serde_field_type(&p.ty, p.optional)))
                .collect::<Vec<_>>()
                .join(", ");
            let field_inits = ctor
                .params
                .iter()
                .zip(ctor.boxed.iter())
                .map(|(p, is_boxed)| {
                    if *is_boxed && matches!(p.ty, TypeRef::Named(_)) {
                        if p.optional {
                            format!("{}.map(Box::new)", p.name)
                        } else {
                            format!("Box::new({})", p.name)
                        }
                    } else {
                        p.name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            minijinja::context! {
                rust_fn_name => format!("_factory_{}", ctor.snake_name),
                variant_name => ctor.variant_name,
                params => params,
                field_inits => field_inits,
            }
        })
        .collect();

    crate::backends::magnus::template_env::render(
        "enum_variant_constructor.rs.jinja",
        minijinja::context! {
            enum_name => &enum_def.name,
            constructors => rendered,
        },
    )
}

/// Ruby method names of the per-variant constructors generated for `enum_def`, paired with their
/// Rust function names and arity. Used by module-init to register `define_singleton_method`s.
///
/// Must resolve `core_import`/`configured_features` identically to
/// [`gen_data_enum_variant_constructors`] above: registering a `method!(Shape::_factory_rect, ..)`
/// path for a constructor that function no longer emits is a hard `E0599` at the registration
/// site, not a missing Ruby method.
pub fn data_enum_variant_constructor_registrations(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> Vec<(String, String, i32)> {
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let declared_features = configured_features_set.as_ref();
    let declared: Vec<&EnumVariant> = declared_enum_variants(enum_def, is_host_enum, declared_features);
    let declared_names: HashSet<&str> = declared.iter().map(|v| v.name.as_str()).collect();

    crate::codegen::generators::collect_all_variant_constructors(enum_def)
        .into_iter()
        .filter(|ctor| declared_names.contains(ctor.variant_name))
        .map(|ctor| {
            let arity = ctor.params.len() as i32;
            (ctor.snake_name.clone(), format!("_factory_{}", ctor.snake_name), arity)
        })
        .collect()
}
