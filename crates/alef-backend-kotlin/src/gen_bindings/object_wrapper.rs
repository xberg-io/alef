//! `object {Crate}` namespace, bridge calls, and Kotlin type/enum/error code emission.
//!
//! Contains:
//! - `emit_function` — emits a JVM wrapper function that delegates to the Java `Bridge` alias
//! - `emit_type_with_imports` — emits a Kotlin `data class` or empty `class` for a type
//! - `emit_enum` — emits a Kotlin `enum class` or `sealed class` for an enum
//! - `emit_error_type_with_imports` — emits a `sealed class` hierarchy for an error type
//! - Kotlin type-string helpers (with import collection)

use alef_core::ir::{EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::shared::{kotlin_field_name, to_lower_camel, to_screaming_snake};
use crate::type_map::KotlinMapper;
use alef_codegen::type_mapper::TypeMapper;

// ---------------------------------------------------------------------------
// Type/enum/error emitters (re-exported for gen_mpp)
// ---------------------------------------------------------------------------

pub(crate) fn emit_type_with_imports(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !ty.doc.is_empty() {
        for line in ty.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if ty.fields.is_empty() {
        out.push_str(&format!("class {} {{}}\n", ty.name));
        return;
    }
    out.push_str(&format!("data class {}(\n", ty.name));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&format!("    val {name}: {ty_str}{comma}\n"));
    }
    out.push_str(")\n");
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        for line in en.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&format!("enum class {} {{\n", en.name));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&format!("    {name}{comma}\n"));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&format!("sealed class {} {{\n", en.name));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&format!("    object {} : {}()\n", variant.name, en.name));
            } else {
                out.push_str(&format!("    data class {}(\n", variant.name));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = kotlin_type(&f.ty, f.optional, &mut BTreeSet::new());
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&format!("        val {name}: {ty_str}{comma}\n"));
                }
                out.push_str(&format!("    ) : {}()\n", en.name));
            }
        }
        out.push_str("}\n");
    }
}

pub(crate) fn emit_error_type_with_imports(
    error: &alef_core::ir::ErrorDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    if !error.doc.is_empty() {
        for line in error.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!(
        "sealed class {}(message: String) : Exception(message) {{\n",
        error.name
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&format!(
                "    object {} : {}(\"{}\")\n",
                variant.name,
                error.name,
                variant.message_template.as_deref().unwrap_or(&variant.name)
            ));
        } else {
            out.push_str(&format!("    data class {}(\n", variant.name));
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                // `message` on Throwable subclasses must be `override` because
                // `kotlin.Throwable` declares `open val message: String?`.
                let modifier = if name == "message" { "override " } else { "" };
                let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                out.push_str(&format!("        {modifier}val {name}: {ty_str}{comma}\n"));
            }
            let message_template = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&format!("    ) : {}(\"{message_template}\")\n", error.name));
        }
    }
    out.push_str("}\n");
}

// ---------------------------------------------------------------------------
// Function emitter
// ---------------------------------------------------------------------------

/// Emit a JVM wrapper function body (delegates to Bridge) inside an `object` block.
pub(crate) fn emit_function(
    f: &FunctionDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    _java_package: &str,
) {
    if !f.doc.is_empty() {
        for line in f.doc.lines() {
            out.push_str("    /// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    let params: Vec<String> = f.params.iter().map(|p| format_param_with_imports(p, imports)).collect();
    let return_ty = kotlin_type_with_string_imports(&f.return_type, false, imports);
    let async_kw = if f.is_async { "suspend " } else { "" };
    let func_name_camel = to_lower_camel(&f.name);
    let call_args = f
        .params
        .iter()
        .map(|p| to_lower_camel(&p.name))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&format!(
        "    {async_kw}fun {}({}): {} {{\n",
        func_name_camel,
        params.join(", "),
        return_ty
    ));

    if f.is_async {
        // The Java facade lowers async Rust functions to blocking calls (it
        // awaits the future internally and returns the resolved value, not a
        // CompletionStage). Wrap the call in `withContext(Dispatchers.IO)` so
        // the suspend function yields the calling thread while the JNI call
        // blocks under it.
        out.push_str("        return withContext(Dispatchers.IO) {\n");
        out.push_str(&format!("            Bridge.{}({})\n", func_name_camel, call_args));
        out.push_str("        }\n");
    } else if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&format!("        Bridge.{}({})\n", func_name_camel, call_args));
    } else {
        out.push_str(&format!("        return Bridge.{}({})\n", func_name_camel, call_args));
    }
    out.push_str("    }\n");
}

// ---------------------------------------------------------------------------
// Parameter formatting
// ---------------------------------------------------------------------------

pub(crate) fn format_param_with_imports(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    let ty_str = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
    format!("{}: {}", to_lower_camel(&p.name), ty_str)
}

// ---------------------------------------------------------------------------
// Type-string rendering (String-keyed imports variant — used by JVM + MPP)
// ---------------------------------------------------------------------------

pub(crate) fn kotlin_type_with_string_imports(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<String>) -> String {
    let inner = render_type_ref_with_string_imports(ty, imports);
    if optional { format!("{inner}?") } else { inner }
}

fn render_type_ref_with_string_imports(ty: &TypeRef, imports: &mut BTreeSet<String>) -> String {
    let mapper = KotlinMapper;
    match ty {
        TypeRef::Path => {
            imports.insert("import java.nio.file.Path".to_string());
            mapper.map_type(ty)
        }
        TypeRef::Duration => {
            imports.insert("import kotlin.time.Duration".to_string());
            mapper.map_type(ty)
        }
        TypeRef::Optional(inner) => format!("{}?", render_type_ref_with_string_imports(inner, imports)),
        TypeRef::Vec(inner) => {
            format!("List<{}>", render_type_ref_with_string_imports(inner, imports))
        }
        TypeRef::Map(k, v) => {
            format!(
                "Map<{}, {}>",
                render_type_ref_with_string_imports(k, imports),
                render_type_ref_with_string_imports(v, imports)
            )
        }
        _ => mapper.map_type(ty),
    }
}

// ---------------------------------------------------------------------------
// Type-string rendering (&'static str imports variant — used internally for enums)
// ---------------------------------------------------------------------------

fn kotlin_type(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<&'static str>) -> String {
    let inner = render_type_ref_with_imports(ty, imports);
    if optional { format!("{inner}?") } else { inner }
}

fn render_type_ref_with_imports(ty: &TypeRef, imports: &mut BTreeSet<&'static str>) -> String {
    let mapper = KotlinMapper;
    match ty {
        TypeRef::Path => {
            imports.insert("import java.nio.file.Path");
            mapper.map_type(ty)
        }
        TypeRef::Duration => {
            imports.insert("import kotlin.time.Duration");
            mapper.map_type(ty)
        }
        TypeRef::Optional(inner) => format!("{}?", render_type_ref_with_imports(inner, imports)),
        TypeRef::Vec(inner) => {
            format!("List<{}>", render_type_ref_with_imports(inner, imports))
        }
        TypeRef::Map(k, v) => {
            format!(
                "Map<{}, {}>",
                render_type_ref_with_imports(k, imports),
                render_type_ref_with_imports(v, imports)
            )
        }
        _ => mapper.map_type(ty),
    }
}
