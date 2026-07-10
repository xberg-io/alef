use super::shared::{render_deser_line, render_preamble, resolve_core_type_path};
use crate::core::ir::{ParamDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Build the deserialization preamble for `Option<String>` JSON params that
/// correspond to default-typed core types, and for `TypeRef::Json` params that
/// need String → serde_json::Value conversion. Returns an empty string when no
/// param needs JSON deserialization.
pub(super) fn build_default_deser_preamble(
    params: &[ParamDef],
    default_types: &AHashSet<String>,
    core_import: &str,
    has_error: bool,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    for p in params {
        if let TypeRef::Named(n) = &p.ty {
            if default_types.contains(n) {
                let core_ty = resolve_core_type_path(n, types_by_name, core_import);
                let line = if has_error {
                    render_deser_line("default_deser_with_error.rs.jinja", &p.name, &core_ty)
                } else {
                    render_deser_line("default_deser_without_error.rs.jinja", &p.name, &core_ty)
                };
                lines.push(line);
                if p.is_ref && p.is_mut {
                    lines.push(format!("let mut {}_mut = {}_core.unwrap_or_default();", p.name, p.name));
                }
            }
        } else if matches!(&p.ty, TypeRef::Json) {
            if p.optional {
                lines.push(format!(
                    "let {name}_json: Option<serde_json::Value> = {name}.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());",
                    name = p.name
                ));
            } else {
                let mut_keyword = if p.is_mut { "mut " } else { "" };
                lines.push(format!(
                    "let {mut_keyword}{name}_json: serde_json::Value = serde_json::from_str::<serde_json::Value>(&{name}).unwrap_or_default();",
                    name = p.name,
                    mut_keyword = mut_keyword
                ));
            }
        }
    }
    render_preamble(&lines)
}
