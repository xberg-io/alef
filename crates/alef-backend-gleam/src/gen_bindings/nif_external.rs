use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, ErrorDef, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;

use crate::type_map::GleamMapper;

use super::helpers::emit_cleaned_gleam_doc;
use super::variant_collision::variant_constructor_name;

pub(crate) fn emit_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<&'static str>) {
    emit_cleaned_gleam_doc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        // Opaque or unit-like — emit a phantom external type
        out.push_str(&format!("pub type {} {{\n  {}\n}}\n", ty.name, ty.name));
        return;
    }
    out.push_str(&format!("pub type {} {{\n  {}(\n", ty.name, ty.name));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = gleam_type(&field.ty, field.optional, imports);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&format!("    {}: {}{}\n", field.name, ty_str, comma));
    }
    out.push_str("  )\n}\n");
}

/// Returns true if a field name represents a positional (tuple) field such as `_0`, `_1`, etc.
/// Gleam constructor arguments do not support labels starting with `_` or numeric labels,
/// so tuple fields must be emitted without a label.
fn is_positional_field(name: &str) -> bool {
    name.starts_with('_') && name[1..].parse::<usize>().is_ok()
}

pub(crate) fn emit_variant_fields(
    fields: &[FieldDef],
    out: &mut String,
    imports: &mut BTreeSet<&'static str>,
) {
    for (idx, field) in fields.iter().enumerate() {
        let ty_str = gleam_type(&field.ty, field.optional, imports);
        let comma = if idx + 1 == fields.len() { "" } else { "," };
        if is_positional_field(&field.name) || field.name.is_empty() {
            // Tuple/positional field: emit as unlabeled argument (e.g. `String`)
            out.push_str(&format!("    {ty_str}{comma}\n"));
        } else {
            out.push_str(&format!("    {}: {ty_str}{comma}\n", field.name));
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
    out.push_str(&format!("pub type {} {{\n", en.name));
    for variant in &en.variants {
        let ctor = variant_constructor_name(&en.name, &variant.name, collisions);
        if variant.fields.is_empty() {
            out.push_str(&format!("  {ctor}\n"));
        } else {
            out.push_str(&format!("  {ctor}(\n"));
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
    out.push_str(&format!("pub type {} {{\n", err.name));
    for variant in &err.variants {
        let ctor = variant_constructor_name(&err.name, &variant.name, collisions);
        if variant.fields.is_empty() {
            out.push_str(&format!("  {ctor}\n"));
        } else {
            out.push_str(&format!("  {ctor}(\n"));
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
    out.push_str(&format!("@external(erlang, \"{nif_module}\", \"{}\")\n", f.name));
    out.push_str(&format!("pub fn {}(", f.name.to_snake_case()));
    let params: Vec<String> = f.params.iter().map(|p| format_param(p, imports)).collect();
    out.push_str(&params.join(", "));
    out.push(')');

    let return_ty = gleam_type(&f.return_type, false, imports);
    let return_str = if let Some(err_ty) = &f.error_type {
        let resolved = resolve_gleam_error_type(err_ty, declared_errors);
        format!("Result({return_ty}, {resolved})")
    } else {
        return_ty
    };
    out.push_str(&format!(" -> {return_str}\n"));
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
