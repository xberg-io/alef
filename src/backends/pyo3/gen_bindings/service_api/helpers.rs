use crate::core::ir::{ApiSurface, HandlerContractDef, ServiceDef, TypeRef};
use std::collections::BTreeSet;

pub(super) fn python_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "str".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float".to_owned(),
                _ => "int".to_owned(),
            }
        }
        TypeRef::Bytes => "bytes".to_owned(),
        TypeRef::Optional(inner) => format!("{} | None", python_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("list[{}]", python_type_annotation(inner)),
        TypeRef::Map(k, v) => format!("dict[{}, {}]", python_type_annotation(k), python_type_annotation(v)),
        TypeRef::Unit => "None".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "object".to_owned(),
        TypeRef::Path => "str".to_owned(),
        TypeRef::Duration => "float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
pub(super) fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Recursively collect `Named` type references into `out`.
fn collect_named_types(ty: &TypeRef, out: &mut BTreeSet<String>) {
    match ty {
        TypeRef::Named(n) => {
            out.insert(n.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Walk every `TypeRef` reachable from a service definition's user-facing surface
/// (constructor, configurators, registration metadata, entrypoints).
pub(super) fn collect_service_named_types(service: &ServiceDef, out: &mut BTreeSet<String>) {
    for p in &service.constructor.params {
        collect_named_types(&p.ty, out);
    }
    for m in &service.configurators {
        for p in &m.params {
            collect_named_types(&p.ty, out);
        }
    }
    for r in &service.registrations {
        for p in &r.metadata_params {
            collect_named_types(&p.ty, out);
        }
    }
    for e in &service.entrypoints {
        for p in &e.params {
            collect_named_types(&p.ty, out);
        }
    }
}

/// Collect runtime-referenced type names from registration variants. Variants
/// reference enum classes (e.g. `Method.Get`) inside the wrapper construction,
/// which must be imported at runtime — TYPE_CHECKING is not sufficient.
pub(super) fn collect_variant_runtime_types(service: &ServiceDef, out: &mut BTreeSet<String>) {
    for r in &service.registrations {
        for variant in &r.variants {
            if let Some(wc) = &variant.wrapper_call {
                out.insert(wc.wrapper_type_name.clone());
                for arg in &wc.args {
                    if let crate::core::ir::WrapperConstructorArg::Fixed { value_expr, .. } = arg {
                        let segments: Vec<&str> = value_expr.split("::").collect();
                        if segments.len() >= 2 {
                            out.insert(segments[segments.len() - 2].to_owned());
                        }
                    }
                }
            }
        }
    }
}

/// Emit a Python docstring at the given column indent. Single-line if `text`
/// has no newline; otherwise multi-line with the closing `"""` on its own line
/// and every body line re-indented to match the opening, so the output passes
/// pydocstyle/ruff `D207`/`D209`.
pub(super) fn format_docstring(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    let pad = " ".repeat(indent);
    if !trimmed.contains('\n') {
        return format!("{pad}\"\"\"{trimmed}\"\"\"\n");
    }
    let mut lines = trimmed.lines();
    let first = lines.next().unwrap_or("");
    let mut out = format!("{pad}\"\"\"{first}\n");
    for line in lines {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&pad);
    out.push_str("\"\"\"\n");
    out
}
