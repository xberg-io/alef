//! TypeScript declaration file (`.d.ts`) generation for NAPI-RS bindings.

use alef_codegen::naming::to_node_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};

pub(super) fn gen_dts(
    api: &ApiSurface,
    prefix: &str,
    exclude_functions: &ahash::AHashSet<String>,
    trait_bridges: &[alef_core::config::TraitBridgeConfig],
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut lines: Vec<String> = header.lines().map(|l| l.to_string()).collect();
    lines.push("/* eslint-disable */".to_string());

    // Collect all declarations: opaque types (classes), plain structs (interfaces), enums, functions.
    // Sort each group alphabetically to produce stable, deterministic output.

    // Opaque types → `export declare class`
    let mut opaque_types: Vec<&TypeDef> = api.types.iter().filter(|t| t.is_opaque).collect();
    opaque_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Plain structs → `export interface`
    let mut plain_types: Vec<&TypeDef> = api.types.iter().filter(|t| !t.is_opaque).collect();
    plain_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Enums → `export declare enum`
    let mut sorted_enums: Vec<&EnumDef> = api.enums.iter().collect();
    sorted_enums.sort_by(|a, b| a.name.cmp(&b.name));

    // Functions → `export declare function`
    // Apply the same filtering as `gen_function`: drop excluded names, and drop
    // sanitized functions unless a trait_bridge can adapt their signature. This
    // keeps the emitted `index.d.ts` declarations in lockstep with the actually
    // exported NAPI functions in `lib.rs`.
    let mut sorted_fns: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| {
            if exclude_functions.contains(&f.name) {
                return false;
            }
            if f.sanitized && crate::trait_bridge::find_bridge_param(f, trait_bridges).is_none() {
                return false;
            }
            true
        })
        .collect();
    sorted_fns.sort_by(|a, b| a.name.cmp(&b.name));

    // Build a merged list of all declarations sorted by their Js-prefixed name so the
    // output is fully alphabetical (matching the committed index.d.ts format).
    enum Decl<'a> {
        Class(&'a TypeDef),
        Interface(&'a TypeDef),
        Enum(&'a EnumDef),
        Function(&'a FunctionDef),
    }

    let mut all_decls: Vec<(String, Decl<'_>)> = Vec::new();
    for t in &opaque_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Class(t)));
    }
    for t in &plain_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Interface(t)));
    }
    for e in &sorted_enums {
        all_decls.push((format!("{prefix}{}", e.name), Decl::Enum(e)));
    }
    for f in &sorted_fns {
        all_decls.push((to_node_name(&f.name), Decl::Function(f)));
    }
    all_decls.sort_by_key(|a| a.0.to_lowercase());

    for (_, decl) in &all_decls {
        lines.push(String::new());
        match decl {
            Decl::Class(typ) => {
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export declare class {prefix}{} {{", typ.name));
                for method in &typ.methods {
                    let js_name = to_node_name(&method.name);
                    let params = dts_params(&method.params, prefix);
                    let ret = dts_return_type(
                        &method.return_type,
                        method.error_type.is_some(),
                        method.is_async,
                        prefix,
                    );
                    lines.extend(format_jsdoc(&method.doc, "  "));
                    if method.is_static {
                        lines.push(format!("  static {js_name}({params}): {ret}"));
                    } else {
                        lines.push(format!("  {js_name}({params}): {ret}"));
                    }
                }
                lines.push("}".to_string());
            }
            Decl::Interface(typ) => {
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export interface {prefix}{} {{", typ.name));
                for field in &typ.fields {
                    let js_name = to_node_name(&field.name);
                    let ts_ty = dts_type(&field.ty, prefix);
                    lines.extend(format_jsdoc(&field.doc, "  "));
                    // Mark a field optional when:
                    //   1. The underlying Rust type is Option<T> (TypeRef::Optional)
                    //   2. The field itself has `optional = true` in the IR (e.g. *Update struct fields)
                    //   3. The parent type has `has_default = true` — the NAPI binding wraps every
                    //      field in Option<T> so callers can omit fields and rely on defaults.
                    let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional || typ.has_default;
                    if is_optional {
                        lines.push(format!("  {js_name}?: {ts_ty}"));
                    } else {
                        lines.push(format!("  {js_name}: {ts_ty}"));
                    }
                }
                lines.push("}".to_string());
            }
            Decl::Enum(e) => {
                let is_data_enum = e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty());
                lines.extend(format_jsdoc(&e.doc, ""));
                if is_data_enum {
                    // Discriminated union: emit a type alias instead of an enum declaration.
                    // Each variant becomes an object literal type with the tag field and its own fields.
                    let tag_field = e.serde_tag.as_deref().unwrap_or("type");
                    let mut member_lines: Vec<String> = Vec::new();
                    for variant in &e.variants {
                        let tag_value = variant
                            .serde_rename
                            .as_deref()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| apply_rename_all(&variant.name, e.serde_rename_all.as_deref()));
                        let mut obj_fields: Vec<String> = vec![format!("{tag_field}: '{tag_value}'")];
                        for field in &variant.fields {
                            let js_name = to_node_name(&field.name);
                            let ts_ty = dts_type(&field.ty, prefix);
                            if matches!(field.ty, TypeRef::Optional(_)) {
                                obj_fields.push(format!("{js_name}?: {ts_ty}"));
                            } else {
                                obj_fields.push(format!("{js_name}: {ts_ty}"));
                            }
                        }
                        member_lines.push(format!("  | {{ {} }}", obj_fields.join("; ")));
                    }
                    lines.push(format!("export type {prefix}{} =", e.name));
                    lines.extend(member_lines);
                } else {
                    lines.push(format!("export declare enum {prefix}{} {{", e.name));
                    for variant in &e.variants {
                        // NAPI string_enum: variant values follow serde_rename_all casing.
                        // Prefer explicit serde_rename, then apply rename_all, then fall back to variant name.
                        let value = variant
                            .serde_rename
                            .as_deref()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| apply_rename_all(&variant.name, e.serde_rename_all.as_deref()));
                        lines.extend(format_jsdoc(&variant.doc, "  "));
                        lines.push(format!("  {} = \"{}\",", variant.name, value));
                    }
                    lines.push("}".to_string());
                }
            }
            Decl::Function(func) => {
                let js_name = to_node_name(&func.name);
                let params = dts_params(&func.params, prefix);
                let ret = dts_return_type(&func.return_type, func.error_type.is_some(), func.is_async, prefix);
                lines.extend(format_jsdoc(&func.doc, ""));
                lines.push(format!("export declare function {js_name}({params}): {ret};"));
            }
        }
    }

    lines.push(String::new());
    lines.join("\n")
}

/// Format a rustdoc string as JSDoc comment lines with the given `indent` prefix.
///
/// Returns an empty `Vec` when `doc` is empty. For a single-line doc, emits
/// `["/** Description */"]`. For multi-line docs, emits the block form:
/// `["/**", " * line1", " * line2", " */"]`, each prefixed by `indent`.
pub(super) fn format_jsdoc(doc: &str, indent: &str) -> Vec<String> {
    let doc = doc.trim();
    if doc.is_empty() {
        return vec![];
    }
    let lines: Vec<&str> = doc.lines().collect();
    if lines.len() == 1 {
        vec![format!("{indent}/** {} */", lines[0].trim())]
    } else {
        let mut out = Vec::with_capacity(lines.len() + 2);
        out.push(format!("{indent}/**"));
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                out.push(format!("{indent} *"));
            } else {
                out.push(format!("{indent} * {trimmed}"));
            }
        }
        out.push(format!("{indent} */"));
        out
    }
}

/// Map an IR `TypeRef` to its TypeScript equivalent for `.d.ts` generation.
pub(super) fn dts_type(ty: &TypeRef, prefix: &str) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "boolean".to_string(),
            alef_core::ir::PrimitiveType::U8
            | alef_core::ir::PrimitiveType::U16
            | alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::I8
            | alef_core::ir::PrimitiveType::I16
            | alef_core::ir::PrimitiveType::I32
            | alef_core::ir::PrimitiveType::F32
            | alef_core::ir::PrimitiveType::F64 => "number".to_string(),
            // NAPI maps u64/usize/isize to i64 on the Rust side; JS sees it as number.
            alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::I64
            | alef_core::ir::PrimitiveType::Usize
            | alef_core::ir::PrimitiveType::Isize => "number".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
        TypeRef::Bytes => "Uint8Array".to_string(),
        TypeRef::Json => "unknown".to_string(),
        TypeRef::Duration => "number".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => format!("{} | undefined | null", dts_type(inner, prefix)),
        TypeRef::Vec(inner) => format!("Array<{}>", dts_type(inner, prefix)),
        TypeRef::Map(k, v) => format!("Record<{}, {}>", dts_type(k, prefix), dts_type(v, prefix)),
        TypeRef::Named(name) => format!("{prefix}{name}"),
    }
}

/// Render a list of parameters as a TypeScript parameter string for `.d.ts`.
pub(super) fn dts_params(params: &[ParamDef], prefix: &str) -> String {
    // TypeScript requires optional parameters to come after all required parameters (TS1016).
    // If the Rust source has optional params followed by required params (e.g., `lang: Option<&str>`,
    // `code: &str`), we must reorder: required first, then optional, preserving relative order within
    // each group.
    let mut required: Vec<&ParamDef> = Vec::new();
    let mut optional: Vec<&ParamDef> = Vec::new();
    for p in params {
        if p.optional {
            optional.push(p);
        } else {
            required.push(p);
        }
    }
    // If no reordering is needed (already ordered), use original order to avoid churn.
    let ordered: Vec<&ParamDef> = if params
        .iter()
        .zip(required.iter().chain(optional.iter()))
        .all(|(a, b)| std::ptr::eq(a as *const ParamDef, *b as *const ParamDef))
    {
        params.iter().collect()
    } else {
        required.into_iter().chain(optional).collect()
    };
    ordered
        .iter()
        .map(|p| {
            let js_name = to_node_name(&p.name);
            let ts_ty = dts_type(&p.ty, prefix);
            if p.optional {
                format!("{js_name}?: {ts_ty} | undefined | null")
            } else {
                format!("{js_name}: {ts_ty}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the TypeScript return type for a function/method in `.d.ts`.
///
/// Async functions return `Promise<T>`. Functions that can error still return `T`
/// (NAPI throws JS exceptions on error, so the `.d.ts` signature just shows the success type).
pub(super) fn dts_return_type(ret: &TypeRef, _has_error: bool, is_async: bool, prefix: &str) -> String {
    let base = match ret {
        TypeRef::Unit => "void".to_string(),
        other => dts_type(other, prefix),
    };
    if is_async { format!("Promise<{base}>") } else { base }
}

/// Apply a serde `rename_all` rule to a PascalCase variant name, returning the serialized string.
///
/// NAPI `string_enum` serializes variant names using the same rule as serde's `rename_all`.
/// When a variant has no explicit `serde_rename`, the enum-level `rename_all` applies.
pub(super) fn apply_rename_all(variant_name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => {
            // PascalCase → snake_case: insert underscore before each uppercase letter (after the first)
            let mut out = String::with_capacity(variant_name.len() + 4);
            for (i, c) in variant_name.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('_');
                }
                out.extend(c.to_lowercase());
            }
            out
        }
        Some("camelCase") => {
            // PascalCase → camelCase: lowercase the first character only
            let mut chars = variant_name.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
            }
        }
        Some("kebab-case") => {
            let mut out = String::with_capacity(variant_name.len() + 4);
            for (i, c) in variant_name.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('-');
                }
                out.extend(c.to_lowercase());
            }
            out
        }
        Some("SCREAMING_SNAKE_CASE") => {
            let mut out = String::with_capacity(variant_name.len() + 4);
            for (i, c) in variant_name.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('_');
                }
                out.extend(c.to_uppercase());
            }
            out
        }
        Some("lowercase") => variant_name.to_lowercase(),
        Some("UPPERCASE") => variant_name.to_uppercase(),
        // PascalCase and unknown rules: use the variant name as-is
        _ => variant_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{ParamDef, TypeRef};

    fn make_param(name: &str, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }
    }

    /// TypeScript TS1016: required parameter must not follow optional parameter.
    /// A visitor method like `visit_code_block(ctx, lang?: Option<str>, code: str)`
    /// must be reordered to `visit_code_block(ctx, code, lang?)` in the `.d.ts`.
    #[test]
    fn dts_params_reorders_required_after_optional() {
        let params = vec![
            make_param("ctx", false),
            make_param("lang", true),
            make_param("code", false),
        ];
        let result = dts_params(&params, "Js");
        // Required params (ctx, code) must precede optional param (lang)
        let ctx_pos = result.find("ctx:").expect("ctx not found");
        let code_pos = result.find("code:").expect("code not found");
        let lang_pos = result.find("lang?:").expect("lang? not found");
        assert!(ctx_pos < lang_pos, "ctx should come before lang?: {result}");
        assert!(code_pos < lang_pos, "code should come before lang?: {result}");
    }

    /// When params are already in valid order (all required before all optional),
    /// the output must be unchanged — no unnecessary reordering.
    #[test]
    fn dts_params_preserves_already_valid_order() {
        let params = vec![
            make_param("ctx", false),
            make_param("code", false),
            make_param("lang", true),
        ];
        let result = dts_params(&params, "Js");
        assert_eq!(result, "ctx: string, code: string, lang?: string | undefined | null");
    }

    /// All-required params: order must be preserved exactly.
    #[test]
    fn dts_params_all_required_preserves_order() {
        let params = vec![make_param("a", false), make_param("b", false), make_param("c", false)];
        let result = dts_params(&params, "Js");
        assert_eq!(result, "a: string, b: string, c: string");
    }
}
