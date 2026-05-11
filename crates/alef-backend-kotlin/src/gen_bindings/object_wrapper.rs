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

use super::helpers::emit_cleaned_kdoc;
use super::shared::{kotlin_field_name, to_lower_camel, to_screaming_snake};
use crate::type_map::KotlinMapper;
use alef_codegen::type_mapper::TypeMapper;

// ---------------------------------------------------------------------------
// Type/enum/error emitters (re-exported for gen_mpp)
// ---------------------------------------------------------------------------

pub(crate) fn emit_type_with_imports(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    emit_cleaned_kdoc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        out.push_str(&crate::template_env::render(
            "empty_class.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }
    out.push_str(&crate::template_env::render(
        "data_class_header.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&crate::template_env::render(
            "class_field.jinja",
            minijinja::context! {
                name => &name,
                type => &ty_str,
                comma => comma,
            },
        ));
    }
    out.push_str(")\n");
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    emit_cleaned_kdoc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::template_env::render(
            "enum_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&crate::template_env::render(
                "enum_variant.jinja",
                minijinja::context! {
                    name => name,
                    comma => comma,
                },
            ));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&crate::template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&crate::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "variant_data_class_header.jinja",
                    minijinja::context! {
                        name => &variant.name,
                    },
                ));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = kotlin_type(&f.ty, f.optional, &mut BTreeSet::new());
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&crate::template_env::render(
                        "variant_class_field.jinja",
                        minijinja::context! {
                            name => &name,
                            type => &ty_str,
                            comma => comma,
                        },
                    ));
                }
                out.push_str(&crate::template_env::render(
                    "variant_close.jinja",
                    minijinja::context! {
                        parent_name => &en.name,
                    },
                ));
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
    emit_cleaned_kdoc(out, &error.doc, "");
    out.push_str(&crate::template_env::render(
        "error_sealed_class_header.jinja",
        minijinja::context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&crate::template_env::render(
                "error_object_variant.jinja",
                minijinja::context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => variant.message_template.as_deref().unwrap_or(&variant.name),
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "variant_data_class_header.jinja",
                minijinja::context! {
                    name => &variant.name,
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                // `message` on Throwable subclasses must be `override` because
                // `kotlin.Throwable` declares `open val message: String?`.
                let modifier = if name == "message" { "override " } else { "" };
                let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                out.push_str(&crate::template_env::render(
                    "error_field.jinja",
                    minijinja::context! {
                        modifier => modifier,
                        name => &name,
                        type => &ty_str,
                        comma => comma,
                    },
                ));
            }
            let message_template = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&crate::template_env::render(
                "error_variant_close.jinja",
                minijinja::context! {
                    parent_name => &error.name,
                    message => message_template,
                },
            ));
        }
    }
    out.push_str("}\n");
}

// ---------------------------------------------------------------------------
// Function emitter
// ---------------------------------------------------------------------------

/// Emit a JVM wrapper function body (delegates to Bridge) inside an `object` block.
///
/// `client_type_names` lists struct types that have a hand-written Kotlin
/// wrapper class (see [`super::emit_jvm_client_class`]). Functions returning
/// those types must wrap the raw Java result in the Kotlin class so the public
/// type matches the wrapper's signature.
pub(crate) fn emit_function(
    f: &FunctionDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    _java_package: &str,
    client_type_names: &std::collections::HashSet<&str>,
) {
    emit_cleaned_kdoc(out, &f.doc, "    ");
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

    // Detect a client-type return so we can wrap the Java result in its Kotlin
    // companion (e.g. `LiterLlm.createClient(...)` returns Java `DefaultClient`
    // but the Kotlin facade must hand back the coroutine-friendly Kotlin
    // `DefaultClient` wrapper).
    let returns_client_type = match &f.return_type {
        TypeRef::Named(n) => client_type_names.contains(n.as_str()),
        _ => false,
    };

    out.push_str(&crate::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            name => func_name_camel,
            params => params.join(", "),
            return_type => return_ty,
        },
    ));
    out.push('\n');

    if f.is_async {
        // The Java facade lowers async Rust functions to blocking calls (it
        // awaits the future internally and returns the resolved value, not a
        // CompletionStage). Wrap the call in `withContext(Dispatchers.IO)` so
        // the suspend function yields the calling thread while the JNI call
        // blocks under it.
        if returns_client_type {
            let wrapper = return_ty.trim_end_matches('?');
            out.push_str(&format!(
                "        return withContext(Dispatchers.IO) {{ {wrapper}(Bridge.{func_name_camel}({call_args})) }}\n"
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "bridge_call_with_dispatch.jinja",
                minijinja::context! {
                    name => func_name_camel,
                    args => call_args,
                },
            ));
            out.push('\n');
        }
    } else if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&crate::template_env::render(
            "bridge_call_unit.jinja",
            minijinja::context! {
                name => func_name_camel,
                args => call_args,
            },
        ));
        out.push('\n');
    } else if returns_client_type {
        let wrapper = return_ty.trim_end_matches('?');
        out.push_str(&format!(
            "        return {wrapper}(Bridge.{func_name_camel}({call_args}))\n"
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "bridge_call_return.jinja",
            minijinja::context! {
                name => func_name_camel,
                args => call_args,
            },
        ));
        out.push('\n');
    }
    out.push_str("    }\n");
}

// ---------------------------------------------------------------------------
// Parameter formatting
// ---------------------------------------------------------------------------

pub(crate) fn format_param_with_imports(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    let ty_str = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
    // Optional params get a `= null` default so callers can drop them via
    // named-argument syntax (e.g. `createClient(apiKey = "x", baseUrl = "y")`)
    // without having to spell out every nullable downstream argument.
    let default = if p.optional { " = null" } else { "" };
    format!("{}: {}{}", to_lower_camel(&p.name), ty_str, default)
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
