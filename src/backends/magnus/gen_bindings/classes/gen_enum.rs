//! Enum code generators for the Magnus (Ruby) backend, including serde type helpers and variant constructors.

use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use std::collections::HashSet;

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
pub fn gen_enum(enum_def: &EnumDef, core_import: &str, configured_features: Option<&[String]>) -> String {
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
            let fields: Vec<minijinja::Value> = variant
                .fields
                .iter()
                .map(|f| {
                    minijinja::context! {
                        name => &f.name,
                        field_type => field_type_for_serde(f),
                        flatten_newtype => variant.is_tuple
                            && variant.fields.len() == 1
                            && enum_def.serde_tag.is_some()
                            && enum_def.serde_content.is_none()
                            && !enum_def.serde_untagged,
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
            }
        })
        .collect();

    crate::backends::magnus::template_env::render(
        "enum_magnus.rs.jinja",
        minijinja::context! {
            enum_name => &enum_def.name,
            has_data => has_data,
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
    serde_field_type(&field.ty, field.optional)
}

/// Serde-shaped Rust type for a data-enum field of type `ty` (wrapping in `Option<...>` when
/// `optional`). This is the type the generated `enum {{ name }}` variant declares, so per-variant
/// constructor parameters must use it verbatim — the magnus data enum is binding-shaped, so the
/// constructor assigns parameters into the variant with no core conversion.
pub(super) fn serde_field_type(ty: &TypeRef, optional: bool) -> String {
    let base = field_type_for_serde_inner(ty);
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
                .map(|p| p.name.as_str())
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
