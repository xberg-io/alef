use crate::core::ir::{DefaultValue, EnumDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;

use super::render_type::{format_param, render_type};

/// Returns `true` if the parameter is a config type that should be made optional in Dart.
///
/// Parameters named `config` whose named type has a Rust `Default` implementation AND
/// for which alef can synthesize a complete Dart constructor expression are made
/// optional (named) in the Dart wrapper. Both conditions are required: FRB-generated DTOs use
/// `required` named parameters for every field, so a bare `Type()` constructor only
/// compiles when alef can emit a value for every field. When alef cannot synthesize a
/// default (e.g. a field whose type lacks a known zero value), the config param stays
/// required in the wrapper signature — otherwise the `config ?? Type()` fallback emits
/// dart that fails to compile.
fn is_optional_config_param(p: &crate::core::ir::ParamDef, type_defs: &[TypeDef], enums: &[EnumDef]) -> bool {
    let TypeRef::Named(name) = &p.ty else {
        return false;
    };
    if p.name != "config" {
        return false;
    }
    if !type_defs.iter().any(|ty| ty.name == *name && ty.has_default) {
        return false;
    }
    default_expression_for_named_type(name, type_defs, enums).is_some()
}

pub(super) fn emit_function(
    f: &FunctionDef,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    if !f.doc.is_empty() {
        let doc_lines: Vec<String> = f.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "  ",
                lines => doc_lines,
            },
        ));
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&template_env::render(
            "function_throws_annotation.jinja",
            minijinja::context! {
                error_ty => error_ty.as_str(),
            },
        ));
    }

    let fn_name = dart_safe_ident(&f.name.to_lower_camel_case());

    // Find the optional config param if present, and determine its type.
    let config_param = f.params.iter().find(|p| is_optional_config_param(p, type_defs, enums));
    let config_default = config_param.and_then(|p| match &p.ty {
        TypeRef::Named(n) => {
            default_expression_for_named_type(n, type_defs, enums).map(|default| (n.as_str(), default))
        }
        _ => None,
    });

    // Build the dart wrapper parameter list. If the function has a config param
    // with a synthesizable default, include it as an optional named parameter.
    //
    // For all other functions, emit required (non-optional) params as positional and
    // optional params inside a `{...}` named-parameter block. This matches the natural
    // Dart calling convention `createClient('key', baseUrl: ...)` and mirrors the
    // underlying FRB binding which is itself named-only.
    let params_str = if let Some((cfg_type, _)) = &config_default {
        let required_params: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p, type_defs, enums))
            .map(|p| format_param(p, imports))
            .collect();
        let config_sig = format!("{{{cfg_type}? config}}");
        if required_params.is_empty() {
            config_sig
        } else {
            format!("{}, {config_sig}", required_params.join(", "))
        }
    } else {
        let required: Vec<String> = f
            .params
            .iter()
            .filter(|p| !p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        let optional: Vec<String> = f
            .params
            .iter()
            .filter(|p| p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        match (required.is_empty(), optional.is_empty()) {
            (true, true) => String::new(),
            (false, true) => required.join(", "),
            (true, false) => format!("{{{}}}", optional.join(", ")),
            (false, false) => format!("{}, {{{}}}", required.join(", "), optional.join(", ")),
        }
    };

    // FRB bridge functions use Dart named parameters (required keyword).
    // Call them with `name: value` named-argument syntax.
    let call_args_str = if let Some((_, default_expr)) = &config_default {
        let non_config: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p, type_defs, enums))
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect();
        let config_arg = format!("config: config ?? {default_expr}");
        if non_config.is_empty() {
            config_arg
        } else {
            format!("{}, {config_arg}", non_config.join(", "))
        }
    } else {
        f.params
            .iter()
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // FRB v2 wraps ALL Rust functions as `Future<T>` in Dart, including sync ones.
    // Therefore all wrapper methods must be `async` and `await` the bridge call.
    {
        let return_ty = if matches!(f.return_type, TypeRef::Unit) {
            "Future<void>".to_string()
        } else {
            format!("Future<{}>", render_type(&f.return_type, imports))
        };
        out.push_str(&template_env::render(
            "function_signature_async.jinja",
            minijinja::context! {
                return_ty => return_ty,
                fn_name => fn_name.as_str(),
                params => params_str.as_str(),
            },
        ));
        out.push_str(&template_env::render(
            "function_await_return.jinja",
            minijinja::context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        out.push_str("  }\n");
    }
}

fn default_expression_for_named_type(name: &str, type_defs: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    let ty = type_defs.iter().find(|ty| ty.name == name && ty.has_default)?;
    let fields: Vec<String> = ty
        .fields
        .iter()
        .filter(|field| !field.binding_excluded)
        .map(|field| {
            let field_name = dart_safe_ident(&field.name.to_lower_camel_case());
            let value = default_expression_for_field(field, type_defs, enums)?;
            Some(format!("{field_name}: {value}"))
        })
        .collect::<Option<Vec<_>>>()?;

    if fields.is_empty() {
        Some(format!("{name}()"))
    } else {
        Some(format!("{name}({})", fields.join(", ")))
    }
}

fn default_expression_for_field(
    field: &crate::core::ir::FieldDef,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    if let Some(default) = &field.typed_default {
        return render_default_value(&field.ty, default, type_defs, enums);
    }
    zero_value_for_type(&field.ty, type_defs, enums)
}

fn render_default_value(
    ty: &TypeRef,
    default: &DefaultValue,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    match default {
        DefaultValue::BoolLiteral(value) => Some(value.to_string()),
        DefaultValue::StringLiteral(value) => Some(format!("'{}'", escape_dart_string(value))),
        DefaultValue::IntLiteral(value) => Some(value.to_string()),
        DefaultValue::FloatLiteral(value) => Some(value.to_string()),
        DefaultValue::EnumVariant(variant) => render_enum_variant_default(ty, variant, enums),
        DefaultValue::Empty => zero_value_for_type(ty, type_defs, enums),
        DefaultValue::None => Some("null".to_string()),
    }
}

fn zero_value_for_type(ty: &TypeRef, type_defs: &[TypeDef], enums: &[EnumDef]) -> Option<String> {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => Some("false".to_string()),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => Some("0.0".to_string()),
        TypeRef::Primitive(_) => Some("0".to_string()),
        TypeRef::String | TypeRef::Char | TypeRef::Path => Some("''".to_string()),
        TypeRef::Bytes => Some("Uint8List(0)".to_string()),
        TypeRef::Vec(inner) => Some(empty_vec_literal(inner)),
        TypeRef::Map(_, _) | TypeRef::Json => Some("{}".to_string()),
        TypeRef::Optional(_) | TypeRef::Unit => Some("null".to_string()),
        TypeRef::Duration => Some("Duration.zero".to_string()),
        TypeRef::Named(name) => {
            if let Some(default) = default_enum_variant(name, enums) {
                render_enum_variant_default(ty, default, enums)
            } else {
                default_expression_for_named_type(name, type_defs, enums)
            }
        }
    }
}

/// Empty-`Vec` default that matches the FRB-mapped Dart type.
///
/// Alef's `gen_rust_crate` widens every Rust integer to `i64` and every float
/// to `f64` in the FRB-facing mirror struct (see `gen_rust_crate::mirror`),
/// matching FRB's own widening behavior. FRB then maps `Vec<i64>` →
/// `Int64List` and `Vec<f64>` → `Float64List` in the Dart class. `Vec<u8>` is
/// a special case (kept as `Vec<u8>` for byte buffers, mapped to `Uint8List`).
///
/// A plain `[]` literal is `List<dynamic>` and fails to satisfy the FRB ctor's
/// typed-list parameter, so we emit the typed-list constructor matching the
/// widened FRB type. Non-primitive element types (Strings, named structs,
/// nested Vecs, etc.) stay as `List<T>` in FRB and accept `[]`.
fn empty_vec_literal(inner: &TypeRef) -> String {
    match inner {
        TypeRef::Primitive(PrimitiveType::U8) => "Uint8List(0)".to_string(),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => "Float64List(0)".to_string(),
        TypeRef::Primitive(
            PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64,
        ) => "Int64List(0)".to_string(),
        _ => "[]".to_string(),
    }
}

fn render_enum_variant_default(ty: &TypeRef, variant: &str, enums: &[EnumDef]) -> Option<String> {
    let TypeRef::Named(name) = ty else {
        return None;
    };
    let variant_name = dart_safe_ident(&variant.to_lower_camel_case());
    let enum_def = enums.iter().find(|e| e.name == *name)?;
    let enum_variant = enum_def.variants.iter().find(|v| v.name == variant)?;
    // Flat Dart enums (all variants are unit variants) are emitted as `enum Foo { a, b }`.
    // Their variants are accessed as `Foo.a` with no call parens.
    // Tagged enums (any variant has fields) become `@freezed sealed class Foo` in Dart,
    // where every variant — including unit ones — is a `const factory` constructor and
    // requires `()` to invoke it.  Without the parens the expression is a function
    // tear-off (`OutputFormat Function()`), not an `OutputFormat` value.
    let is_flat_enum = enum_def.variants.iter().all(|v| v.fields.is_empty());
    if is_flat_enum && enum_variant.fields.is_empty() {
        Some(format!("{name}.{variant_name}"))
    } else {
        Some(format!("{name}.{variant_name}()"))
    }
}

fn default_enum_variant<'a>(name: &str, enums: &'a [EnumDef]) -> Option<&'a str> {
    enums
        .iter()
        .find(|e| e.name == name)
        .and_then(|e| e.variants.iter().find(|v| v.is_default))
        .map(|v| v.name.as_str())
}

fn escape_dart_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::PrimitiveType;

    #[test]
    fn empty_vec_of_integer_primitive_uses_int64list_ctor() {
        // Alef widens every Rust integer to i64 in the FRB-facing mirror
        // (see backends/dart/gen_rust_crate/mirror.rs), and FRB then maps
        // Vec<i64> → Int64List. Bytes (Vec<u8>) is the lone special case.
        let widened_to_int64 = [
            PrimitiveType::U16,
            PrimitiveType::U32,
            PrimitiveType::U64,
            PrimitiveType::I8,
            PrimitiveType::I16,
            PrimitiveType::I32,
            PrimitiveType::I64,
        ];
        for prim in widened_to_int64 {
            let prim_dbg = format!("{prim:?}");
            let got = empty_vec_literal(&TypeRef::Primitive(prim));
            assert_eq!(got, "Int64List(0)", "Vec<{prim_dbg}> empty default");
        }
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::U8)),
            "Uint8List(0)"
        );
    }

    #[test]
    fn empty_vec_of_float_primitive_uses_float64list_ctor() {
        // Alef widens f32 → f64 in the mirror; FRB maps Vec<f64> → Float64List.
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::F32)),
            "Float64List(0)"
        );
        assert_eq!(
            empty_vec_literal(&TypeRef::Primitive(PrimitiveType::F64)),
            "Float64List(0)"
        );
    }

    #[test]
    fn empty_vec_of_string_or_named_stays_list_literal() {
        assert_eq!(empty_vec_literal(&TypeRef::String), "[]");
        assert_eq!(empty_vec_literal(&TypeRef::Named("Foo".to_string())), "[]");
        assert_eq!(empty_vec_literal(&TypeRef::Vec(Box::new(TypeRef::String))), "[]");
    }

    #[test]
    fn bytes_default_is_typed_uint8list() {
        assert_eq!(
            zero_value_for_type(&TypeRef::Bytes, &[], &[]),
            Some("Uint8List(0)".to_string())
        );
    }
}
