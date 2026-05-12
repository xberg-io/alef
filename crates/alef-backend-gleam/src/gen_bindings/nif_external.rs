use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, ErrorDef, FieldDef, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;

use crate::type_map::GleamMapper;

use super::helpers::emit_cleaned_gleam_doc;
use super::variant_collision::variant_constructor_name;

pub(crate) fn emit_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<&'static str>) {
    emit_cleaned_gleam_doc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        // Opaque or unit-like — emit a phantom external type
        out.push_str(&crate::template_env::render(
            "type_opaque.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }
    out.push_str(&crate::template_env::render(
        "type_header.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = gleam_type(&field.ty, field.optional, imports);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&crate::template_env::render(
            "field_labeled.jinja",
            minijinja::context! {
                name => &field.name,
                ty => &ty_str,
                comma => comma,
            },
        ));
    }
    out.push_str("  )\n}\n");
}

/// Returns true if a field name represents a positional (tuple) field such as `_0`, `_1`, etc.
/// Gleam constructor arguments do not support labels starting with `_` or numeric labels,
/// so tuple fields must be emitted without a label.
fn is_positional_field(name: &str) -> bool {
    name.starts_with('_') && name[1..].parse::<usize>().is_ok()
}

pub(crate) fn emit_variant_fields(fields: &[FieldDef], out: &mut String, imports: &mut BTreeSet<&'static str>) {
    for (idx, field) in fields.iter().enumerate() {
        let ty_str = gleam_type(&field.ty, field.optional, imports);
        let comma = if idx + 1 == fields.len() { "" } else { "," };
        if is_positional_field(&field.name) || field.name.is_empty() {
            // Tuple/positional field: emit as unlabeled argument (e.g. `String`)
            out.push_str(&crate::template_env::render(
                "field_positional.jinja",
                minijinja::context! {
                    ty => &ty_str,
                    comma => comma,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "field_labeled.jinja",
                minijinja::context! {
                    name => &field.name,
                    ty => &ty_str,
                    comma => comma,
                },
            ));
        }
    }
}

pub(crate) fn emit_enum(
    en: &EnumDef,
    collisions: &std::collections::HashSet<String>,
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    emit_cleaned_gleam_doc(out, &en.doc, "");
    out.push_str(&crate::template_env::render(
        "enum_header.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    for variant in &en.variants {
        let ctor = variant_constructor_name(&en.name, &variant.name, collisions);
        if variant.fields.is_empty() {
            out.push_str(&crate::template_env::render(
                "variant_simple.jinja",
                minijinja::context! {
                    ctor => &ctor,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "variant_with_fields.jinja",
                minijinja::context! {
                    ctor => &ctor,
                },
            ));
            emit_variant_fields(&variant.fields, out, imports);
            out.push_str("  )\n");
        }
    }
    out.push_str("}\n");
}

pub(crate) fn emit_error_type(
    err: &ErrorDef,
    collisions: &std::collections::HashSet<String>,
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    emit_cleaned_gleam_doc(out, &err.doc, "");
    out.push_str(&crate::template_env::render(
        "error_header.jinja",
        minijinja::context! {
            name => &err.name,
        },
    ));
    for variant in &err.variants {
        let ctor = variant_constructor_name(&err.name, &variant.name, collisions);
        if variant.fields.is_empty() {
            out.push_str(&crate::template_env::render(
                "variant_simple.jinja",
                minijinja::context! {
                    ctor => &ctor,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "variant_with_fields.jinja",
                minijinja::context! {
                    ctor => &ctor,
                },
            ));
            emit_variant_fields(&variant.fields, out, imports);
            out.push_str("  )\n");
        }
    }
    out.push_str("}\n");
}

pub(crate) fn emit_function(
    f: &FunctionDef,
    nif_module: &str,
    declared_errors: &[String],
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    emit_cleaned_gleam_doc(out, &f.doc, "");
    use heck::ToSnakeCase;
    out.push_str(&crate::template_env::render(
        "function_external.jinja",
        minijinja::context! {
            nif_module => nif_module,
            name => &f.name,
        },
    ));
    let snake_name = f.name.to_snake_case();
    let return_ty = gleam_type(&f.return_type, false, imports);
    let return_str = if let Some(err_ty) = &f.error_type {
        let resolved = resolve_gleam_error_type(err_ty, declared_errors);
        format!("Result({return_ty}, {resolved})")
    } else {
        return_ty
    };
    out.push_str(&crate::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            name => &snake_name,
            params => &params_string(f, imports),
            return_type => &return_str,
        },
    ));
}

fn params_string(f: &FunctionDef, imports: &mut BTreeSet<&'static str>) -> String {
    let params: Vec<String> = f.params.iter().map(|p| format_param(p, imports)).collect();
    params.join(", ")
}

/// Map a Rust error type string (e.g. `"anyhow::Error"`, `"KreuzbergError"`)
/// to a Gleam type identifier. Gleam type names cannot contain `::`. If the
/// path's last segment matches a declared error type, use it; otherwise fall
/// back to the first declared error type, or `String` if none are declared.
pub(crate) fn resolve_gleam_error_type(error_type: &str, declared: &[String]) -> String {
    let last = error_type.rsplit("::").next().unwrap_or(error_type);
    if declared.iter().any(|d| d == last) {
        return last.to_string();
    }
    declared.first().cloned().unwrap_or_else(|| "String".to_string())
}

fn format_param(p: &ParamDef, imports: &mut BTreeSet<&'static str>) -> String {
    let ty_str = gleam_type(&p.ty, p.optional, imports);
    format!("{}: {}", p.name, ty_str)
}

pub(crate) fn gleam_type(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<&'static str>) -> String {
    let mapper = GleamMapper;
    let inner = render_type_ref_with_imports(ty, imports, &mapper);
    if optional {
        imports.insert("import gleam/option.{type Option}");
        format!("Option({inner})")
    } else {
        inner
    }
}

fn render_type_ref_with_imports(ty: &TypeRef, imports: &mut BTreeSet<&'static str>, mapper: &GleamMapper) -> String {
    match ty {
        TypeRef::Optional(inner) => {
            imports.insert("import gleam/option.{type Option}");
            format!("Option({})", render_type_ref_with_imports(inner, imports, mapper))
        }
        TypeRef::Vec(inner) => {
            format!("List({})", render_type_ref_with_imports(inner, imports, mapper))
        }
        TypeRef::Map(k, v) => {
            imports.insert("import gleam/dict.{type Dict}");
            format!(
                "Dict({}, {})",
                render_type_ref_with_imports(k, imports, mapper),
                render_type_ref_with_imports(v, imports, mapper)
            )
        }
        _ => mapper.map_type(ty),
    }
}

/// Emit an opaque resource type for a NIF-backed struct with methods.
///
/// Generates:
/// ```gleam
/// pub opaque type DefaultClient {
///   DefaultClient(resource: dynamic.Dynamic)
/// }
/// ```
pub(crate) fn emit_resource_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<&'static str>) {
    imports.insert("import gleam/dynamic");
    emit_cleaned_gleam_doc(out, &ty.doc, "");
    out.push_str(&crate::template_env::render(
        "type_opaque_resource.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));
}

/// Emit an external NIF binding for an instance method on a resource type.
///
/// The NIF entry point name is `{snake_type}_{snake_method}` and the first
/// parameter is `self_: TypeName` (the opaque resource handle).
///
/// Generates:
/// ```gleam
/// @external(erlang, "Elixir.MyModule", "default_client_chat")
/// pub fn chat(self_: DefaultClient, req: ChatRequest) -> Result(ChatResponse, LlmError)
/// ```
pub(crate) fn emit_method(
    method: &MethodDef,
    type_name: &str,
    nif_module: &str,
    declared_errors: &[String],
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    use heck::ToSnakeCase;
    emit_cleaned_gleam_doc(out, &method.doc, "");
    let snake_type = type_name.to_snake_case();
    let snake_method = method.name.to_snake_case();
    let nif_fn_name = format!("{snake_type}_{snake_method}");
    out.push_str(&crate::template_env::render(
        "resource_method_external.jinja",
        minijinja::context! {
            nif_module => nif_module,
            nif_fn_name => &nif_fn_name,
        },
    ));
    let return_ty = gleam_type(&method.return_type, false, imports);
    let return_str = if let Some(err_ty) = &method.error_type {
        let resolved = resolve_gleam_error_type(err_ty, declared_errors);
        format!("Result({return_ty}, {resolved})")
    } else {
        return_ty
    };
    let self_param = format!("self_: {type_name}");
    let rest_params: Vec<String> = method.params.iter().map(|p| format_param(p, imports)).collect();
    let all_params = if rest_params.is_empty() {
        self_param
    } else {
        format!("{self_param}, {}", rest_params.join(", "))
    };
    out.push_str(&crate::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            name => &snake_method,
            params => &all_params,
            return_type => &return_str,
        },
    ));
}
