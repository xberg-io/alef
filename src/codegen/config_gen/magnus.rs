use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};

use super::shared::{constructor_fields, default_value_for_field_in_type, use_unwrap_or_default};

const MAGNUS_MAX_ARITY: usize = 15;

/// Generate a Magnus (Ruby) kwargs constructor for a type with `has_default`.
///
/// For types with <=15 fields, generates a positional `Option<T>` parameter constructor.
/// For types with >15 fields (exceeding Magnus arity limit), generates a hash-based constructor
/// using `RHash` that extracts fields by name, applying defaults for missing keys.
pub fn gen_magnus_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let _ = MAGNUS_MAX_ARITY;
    gen_magnus_hash_constructor(typ, type_mapper)
}

/// Wrap a type string for use as a type-path prefix in Rust.
///
/// Types containing `<` (generics like `Vec<String>`, `Option<T>`) cannot be used as
/// `Vec<String>::try_convert(v)` — that's a parse error. They must use the UFCS form
/// `<Vec<String>>::try_convert(v)` instead. Simple names like `String`, `bool` can use
/// `String::try_convert(v)` directly.
fn as_type_path_prefix(type_str: &str) -> String {
    if type_str.contains('<') {
        format!("<{type_str}>")
    } else {
        type_str.to_string()
    }
}

/// Build the `Some(v) => ...` arm shared by every field kind: convert the present Ruby value
/// and, on failure, raise a `TypeError` naming the field instead of silently discarding the
/// error. `kwargs.get` already tells the two cases apart (`None` = "not provided", `Some(v)` =
/// "provided, must convert") — this expression is only ever reached for the latter, so a
/// conversion failure here is always a genuine bad value, never an absent one. ~keep
fn try_convert_or_raise(field_name: &str, type_prefix: &str) -> String {
    format!(
        "{type_prefix}::try_convert(v).map_err(|e| magnus::Error::new(unsafe {{ magnus::Ruby::get_unchecked() }}.exception_type_error(), format!(\"invalid value for `{field_name}`: {{}}\", e)))?"
    )
}

/// Generate a hash-based Magnus constructor for types with many fields.
/// Accepts `(kwargs: RHash)` and extracts each field by symbol name, applying defaults.
fn gen_magnus_hash_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let fields: Vec<_> = constructor_fields(typ)
        .map(|field| {
            let is_optional = field_is_optional_in_rust(field);
            let effective_inner_ty = match &field.ty {
                TypeRef::Optional(inner) if is_optional => inner.as_ref(),
                ty => ty,
            };
            let inner_type = type_mapper(effective_inner_ty);
            let type_prefix = as_type_path_prefix(&inner_type);

            let has_callable_default = matches!(
                &field.typed_default,
                Some(DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_))
            );

            let try_convert = try_convert_or_raise(&field.name, &type_prefix);

            let assignment = if is_optional {
                format!(
                    "match kwargs.get(ruby.to_symbol(\"{}\")) {{ Some(v) => Some({}), None => None }},",
                    field.name, try_convert
                )
            } else if use_unwrap_or_default(field) {
                format!(
                    "match kwargs.get(ruby.to_symbol(\"{}\")) {{ Some(v) => {}, None => Default::default() }},",
                    field.name, try_convert
                )
            } else if matches!(effective_inner_ty, TypeRef::Named(_))
                && !matches!(&field.typed_default, Some(DefaultValue::EnumVariant(_)))
                && !has_callable_default
            {
                // Magnus-wrapped structs (`#[magnus::wrap]`) never implement
                format!(
                    "match kwargs.get(ruby.to_symbol(\"{}\")) {{ Some(v) => {}, None => return Err(magnus::Error::new(unsafe {{ magnus::Ruby::get_unchecked() }}.exception_arg_error(), \"missing required field: {}\")) }},",
                    field.name, try_convert, field.name
                )
            } else {
                let default_str = if inner_type == "String" {
                    if let Some(DefaultValue::EnumVariant(variant)) = &field.typed_default {
                        use heck::ToSnakeCase;
                        format!("\"{}\".to_string()", variant.to_snake_case())
                    } else {
                        default_value_for_field_in_type(field, "rust", typ)
                    }
                } else {
                    default_value_for_field_in_type(field, "rust", typ)
                };
                // A `#[serde(default = "path")]` function returns the field's own core type
                // (serde's contract). When that type is `Named`, Magnus mirrors it into its own
                // `#[magnus::wrap]` struct of the same short name but a distinct Rust type from
                // the core one, so the call's return value needs `.into()` to become the type
                // this field actually holds — otherwise the assignment is an E0308 mismatch.
                let default_expr = if has_callable_default && matches!(effective_inner_ty, TypeRef::Named(_)) {
                    format!("{default_str}.into()")
                } else {
                    default_str
                };
                format!(
                    "match kwargs.get(ruby.to_symbol(\"{}\")) {{ Some(v) => {}, None => {} }},",
                    field.name, try_convert, default_expr
                )
            };

            minijinja::context! {
                name => field.name.clone(),
                assignment => assignment,
                // The field's own gate: a field whose type is itself conditionally compiled
                // (e.g. `Option<SparseEmbedding>` where `SparseEmbedding` carries its own
                // `#[cfg(feature = "...")]`) must repeat that gate on this constructor's
                // per-field initializer, mirroring the same gate now attached to the field's
                // own declaration in `struct_def.rs.jinja` -- the two must agree, or one of
                // them references a type/field the other has already compiled out. ~keep
                cfg => field.cfg.as_deref(),
            }
        })
        .collect();

    crate::codegen::template_env::render(
        "config_gen/magnus_hash_constructor.jinja",
        minijinja::context! {
            fields => fields,
        },
    )
}

/// Returns true if the generated Rust field type is already `Option<T>`.
/// This covers both:
/// - Fields with `optional: true` (the Rust field type becomes `Option<inner_type>`)
/// - Fields whose `TypeRef` is explicitly `Optional(_)` (rare, for nested Option types)
fn field_is_optional_in_rust(field: &FieldDef) -> bool {
    field.optional || matches!(&field.ty, TypeRef::Optional(_))
}
