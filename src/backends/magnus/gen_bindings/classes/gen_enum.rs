//! Enum code generators for the Magnus (Ruby) backend, including serde type helpers and variant constructors.

use crate::core::ir::{EnumDef, FieldDef, TypeRef};

/// Generate a Magnus enum definition with IntoValue and TryConvert impls.
/// Unit-variant enums are represented as Ruby Symbols for ergonomic Ruby usage.
pub fn gen_enum(enum_def: &EnumDef) -> String {
    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    let first_variant = enum_def.variants.first().map(|v| v.name.as_str()).unwrap_or("Default");

    // Find the variant marked with #[default], or fall back to first_variant
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.is_default)
        .map(|v| v.name.as_str())
        .unwrap_or(first_variant);

    // variant). When `#[default]` selects a unit variant (e.g. `PageAction::Scrape`)
    let first_variant_default = if has_data {
        let default = enum_def
            .variants
            .iter()
            .find(|v| v.is_default)
            .unwrap_or_else(|| enum_def.variants.first().unwrap());
        if default.fields.is_empty() {
            String::new()
        } else if enum_def.serde_untagged && default.is_tuple {
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
    } else {
        String::new()
    };

    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|variant| {
            let fields: Vec<minijinja::Value> = variant
                .fields
                .iter()
                .map(|f| {
                    minijinja::context! {
                        name => &f.name,
                        field_type => field_type_for_serde(f),
                    }
                })
                .collect();

            minijinja::context! {
                name => &variant.name,
                serde_rename => &variant.serde_rename,
                fields => &fields,
                is_tuple => variant.is_tuple,
                snake_name => crate::codegen::naming::pascal_to_snake(&variant.name),
            }
        })
        .collect();

    crate::backends::magnus::template_env::render(
        "enum_magnus.rs.jinja",
        minijinja::context! {
            enum_name => &enum_def.name,
            has_data => has_data,
            serde_tag => &enum_def.serde_tag,
            serde_untagged => enum_def.serde_untagged,
            serde_rename_all => &enum_def.serde_rename_all,
            variants => &variants,
            first_variant => first_variant,
            default_variant => default_variant,
            first_variant_default => &first_variant_default,
        },
    )
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
/// Variant selection (skipping unit/tuple/`binding_excluded` variants and yielding to a hand-written
/// `impl` method of the same name) is shared with pyo3 via `collect_variant_constructors`. The Rust
/// function name is `_factory_<snake>` to avoid colliding with the variant accessor of the same
/// snake_case name; Ruby registers it under the bare snake name via `define_singleton_method`.
///
/// Returns an empty string when no variant qualifies (no empty `impl` block).
pub fn gen_data_enum_variant_constructors(enum_def: &EnumDef) -> String {
    let constructors = crate::codegen::generators::collect_variant_constructors(enum_def);
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
pub fn data_enum_variant_constructor_registrations(enum_def: &EnumDef) -> Vec<(String, String, i32)> {
    crate::codegen::generators::collect_variant_constructors(enum_def)
        .into_iter()
        .map(|ctor| {
            let arity = ctor.params.len() as i32;
            (ctor.snake_name.clone(), format!("_factory_{}", ctor.snake_name), arity)
        })
        .collect()
}
