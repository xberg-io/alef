use crate::codegen::doc_emission::{parse_arguments_bullets, parse_rustdoc_sections};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, EnumDef, ParamDef, TypeDef, TypeRef};
use std::collections::HashMap;

use super::bridges::{
    extendr_enum_variant_constructor_registrations, is_flat_data_enum, is_json_passthrough_data_enum,
};
use super::options::find_r_options_type_from_api;
use super::trait_bridge_wrappers::{TraitBridgeFn, collect_excluded_class_types, method_is_excluded_from_impl};

/// Human-readable R type description for a `TypeRef`, used to populate
/// `@param` / `@return` lines in the generated roxygen2 doc blocks. Returns
/// a sentence-cased phrase ending in a period (e.g. "Raw vector of bytes.").
pub(super) fn r_type_description(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Bytes => "Raw vector of bytes.".to_string(),
        TypeRef::String => "Character string.".to_string(),
        TypeRef::Char => "Single-character string.".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "Logical (TRUE/FALSE).".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "Numeric.".to_string(),
            _ => "Integer.".to_string(),
        },
        TypeRef::Optional(inner) => {
            let inner_desc = r_type_description(inner);
            let trimmed = inner_desc.trim_end_matches('.');
            // Lower-case the first letter so "Character string" becomes "character string"
            // after the "Optional " prefix — but only for natural-language descriptions.
            // Named types (e.g. `ExtractionConfig object`) keep their proper-noun casing.
            let body = if matches!(**inner, TypeRef::Named(_)) {
                trimmed.to_string()
            } else {
                match trimmed.chars().next() {
                    Some(c) => {
                        let mut s = c.to_lowercase().collect::<String>();
                        s.push_str(&trimmed[c.len_utf8()..]);
                        s
                    }
                    None => String::new(),
                }
            };
            format!("Optional {body}. Defaults to NULL.")
        }
        TypeRef::Vec(inner) => {
            let inner_desc = r_type_description(inner);
            let trimmed = inner_desc.trim_end_matches('.');
            format!("List of {}.", trimmed.to_lowercase())
        }
        TypeRef::Map(_, _) => "Named list.".to_string(),
        TypeRef::Named(name) => format!("{name} object (list with class attribute)."),
        TypeRef::Path => "File path as character string.".to_string(),
        TypeRef::Unit => "Invisible NULL.".to_string(),
        TypeRef::Json => "JSON-serializable value.".to_string(),
        TypeRef::Duration => "Numeric duration in seconds.".to_string(),
    }
}

/// Convert the first character of `s` to upper-case while leaving the rest untouched.
/// Returns an empty string when `s` is empty.
pub(super) fn title_case_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Append `text` to `block` so multi-line content stays inside the current
/// roxygen tag (`@param` / `@return`). The first line is appended as-is;
/// every subsequent line is prefixed with `#'   ` so R's parser still sees
/// the line as a comment rather than parsing it as code.
pub(super) fn push_roxygen_inline_multiline(block: &mut String, text: &str) {
    let mut lines = text.lines();
    if let Some(first) = lines.next() {
        block.push_str(first.trim_end());
    }
    for line in lines {
        block.push('\n');
        block.push_str("#'   ");
        block.push_str(line.trim_end());
    }
}

/// Build the roxygen2 doc block for a free R wrapper function.
///
/// The block carries a title line (derived from the first line of `doc`, or
/// the function name as a fallback), optional description paragraphs, one
/// `@param` per parameter, an `@return`, and the `@export` tag. Every output
/// line is prefixed with `#'` — callers prepend the block directly above the
/// `name <- function(...) ...` definition.
/// Build one roxygen2 doc line describing a single trait-callback method the host backend must
/// implement: `name(arg: Type, ...) -> ReturnType`. Param types that are native-marshalled
/// structs are tagged `(native object)` so the host knows it receives a binding external pointer
/// rather than a JSON string. Used to emit a typed host-interface contract on `register_<trait>`.
fn trait_method_doc_line(
    method: &crate::core::ir::MethodDef,
    native_structs: &std::collections::HashSet<String>,
) -> String {
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = doc_type_name(&p.ty);
            if matches!(&p.ty, TypeRef::Named(n) if native_structs.contains(n)) {
                format!("{}: {} (native object)", p.name.trim_start_matches('_'), ty)
            } else {
                format!("{}: {}", p.name.trim_start_matches('_'), ty)
            }
        })
        .collect();
    let ret = doc_type_name(&method.return_type);
    format!("`{}({}) -> {}`", method.name, args.join(", "), ret)
}

/// Render a TypeRef as a short, R-facing type label for roxygen docs (not Rust syntax).
fn doc_type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "void".to_string(),
        TypeRef::String | TypeRef::Char => "character".to_string(),
        TypeRef::Bytes => "raw".to_string(),
        TypeRef::Path => "character".to_string(),
        TypeRef::Primitive(_) => "numeric".to_string(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Optional(inner) => format!("{} or NULL", doc_type_name(inner)),
        TypeRef::Vec(inner) => format!("list of {}", doc_type_name(inner)),
        TypeRef::Map(_, _) => "named list".to_string(),
        TypeRef::Json => "character".to_string(),
        TypeRef::Duration => "numeric".to_string(),
    }
}

pub(super) fn r_roxygen_block(func_name: &str, doc: &str, params: &[ParamDef], return_type: &TypeRef) -> String {
    let mut block = String::with_capacity(256);
    let trimmed_doc = doc.trim();
    // Parse the rustdoc into sections so `# Arguments` / `# Returns` / `# Errors` /
    // `# Example` are surfaced as native roxygen2 tags instead of being emitted as raw
    // markdown headings in the description body.
    let sections = parse_rustdoc_sections(trimmed_doc);
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (func_name.to_string(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    // Build a name → description map from the `# Arguments` bullets, if any.
    // Falls back to the type-based description when no entry is present.
    let mut param_docs: HashMap<String, String> = HashMap::new();
    if let Some(args_body) = sections.arguments.as_deref() {
        for (name, desc) in parse_arguments_bullets(args_body) {
            if !desc.is_empty() {
                param_docs.insert(name, desc);
            }
        }
    }
    for param in params {
        block.push_str("#' @param ");
        block.push_str(&param.name);
        block.push(' ');
        if let Some(desc) = param_docs.get(&param.name) {
            push_roxygen_inline_multiline(&mut block, desc);
            if !desc.trim_end().ends_with('.') {
                block.push('.');
            }
        } else {
            block.push_str(&r_type_description(&param.ty));
        }
        block.push('\n');
    }
    block.push_str("#' @return ");
    if let Some(ret) = sections.returns.as_deref() {
        let ret = ret.trim();
        push_roxygen_inline_multiline(&mut block, ret);
        if !ret.ends_with('.') {
            block.push('.');
        }
    } else {
        block.push_str(&r_type_description(return_type));
    }
    block.push('\n');
    if let Some(err) = sections.errors.as_deref() {
        block.push_str("#'\n#' @section Errors:\n");
        for line in err.trim().lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    block.push_str("#' @export\n");
    block
}

/// Build a one-line description for a struct field, derived from the field's
/// `doc` comment. Falls back to the field name when the IR carries no docs.
///
/// R's roxygen2 `@field` tag is single-line per field; multi-paragraph rustdoc
/// must be collapsed. We take the first paragraph (lines up to the first blank
/// line), trim, and join with a single space.
pub(super) fn r_field_one_liner(field_name: &str, doc: &str) -> String {
    let trimmed = doc.trim();
    if trimmed.is_empty() {
        return field_name.to_string();
    }
    let paragraph: Vec<&str> = trimmed
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect();
    if paragraph.is_empty() {
        field_name.to_string()
    } else {
        let mut result = paragraph.join(" ");
        // Enforce 120-char line limit for roxygen2. The format is:
        // #' @field <field_name> <description>
        // which is 10 + len(field_name) + 1 + len(description) = 120 max
        // So description can be at most 109 - len(field_name).
        let max_desc_len = 109_usize.saturating_sub(field_name.len());
        if result.len() > max_desc_len {
            result.truncate(max_desc_len);
            // Remove trailing partial words by finding the last space.
            if let Some(last_space) = result.rfind(' ') {
                result.truncate(last_space);
            }
        }
        result
    }
}

/// Build the roxygen2 doc block for a class env (one per registered struct).
///
/// Layout: title (first line of `typ.doc`, falling back to the class name),
/// optional description body, one `#' @field <name> <description>` per public
/// field, and the `#' @export` tag. The block is prepended to the class env
/// definition via the `r_type_class_env.jinja` template.
pub(super) fn r_class_roxygen_block(typ: &TypeDef) -> String {
    let mut block = String::with_capacity(256);
    let sections = parse_rustdoc_sections(typ.doc.trim());
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (typ.name.clone(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let rname = field.name.trim_start_matches('_');
        block.push_str("#' @field ");
        block.push_str(rname);
        block.push(' ');
        block.push_str(&r_field_one_liner(rname, &field.doc));
        block.push('\n');
    }
    block.push_str("#' @export\n");
    block
}

/// Build the roxygen2 doc block for a flat data enum class env.
///
/// Like `r_class_roxygen_block` but uses enum variants as fields — the flat
/// representation exposes one scalar field per variant (see
/// [`is_flat_data_enum`]). For JSON-passthrough enums (`is_json_passthrough_data_enum`),
/// the `@field` list is omitted because callers interact with the opaque
/// `__inner` JSON blob rather than typed variant fields.
pub(super) fn r_enum_roxygen_block(enum_def: &EnumDef, include_variants_as_fields: bool) -> String {
    let mut block = String::with_capacity(256);
    let sections = parse_rustdoc_sections(enum_def.doc.trim());
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (enum_def.name.clone(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if include_variants_as_fields {
        for variant in &enum_def.variants {
            block.push_str("#' @field ");
            block.push_str(&variant.name);
            block.push(' ');
            block.push_str(&r_field_one_liner(&variant.name, &variant.doc));
            block.push('\n');
        }
    }
    block.push_str("#' @export\n");
    block
}

/// Generate `extendr-wrappers.R` — the R-side bindings for every `#[extendr]` symbol
/// registered in the generated `extendr_module!` macro.
///
/// The output mirrors what `rextendr::document()` would produce at package-development
/// time, but is written directly from the alef IR so it is always present at install time.
///
/// Layout:
///   1. Free-function wrappers: `name <- function(...) .Call("wrap__name", ..., PACKAGE = "<pkg>")`.
///      Exported via `#' @export` (paired with explicit `export(name)` lines in NAMESPACE).
///   2. One `<TypeName> <- new.env(parent = emptyenv())` block per registered class, with:
///      • static methods bound as `Type$method <- function(...) .Call("wrap__Type__method", ...)`,
///      • instance methods bound as `Type$method <- function(...) .Call("wrap__Type__method", self, ...)`,
///      • dispatch operators (`$.Type`, `[[.Type`) so callers can write `instance$method(...)`.
pub(super) fn gen_extendr_wrappers_r(
    api: &ApiSurface,
    package_name: &str,
    input_type_names: &ahash::AHashSet<String>,
    trait_bridge_fns: &[TraitBridgeFn],
    r_exclude_functions: &ahash::AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(8 * 1024);
    out.push_str("# Generated by extendr: Do not edit by hand\n");
    out.push_str("#\n");
    out.push_str("# This file is regenerated by alef on every `alef generate` run.\n");
    out.push_str("# It mirrors the output of `rextendr::document()` and binds every\n");
    out.push_str("# wrap__<symbol> entry registered in extendr_module! to an R-callable\n");
    out.push_str("# function or class env.\n\n");

    out.push_str(&crate::backends::extendr::template_env::render(
        "r_use_dyn_lib.jinja",
        minijinja::context! { package_name => package_name },
    ));
    out.push_str("NULL\n\n");

    // Names emitted by the trait-bridge generator; skip them in the free-function
    // pass so the wrapper file does not define the same R wrapper twice.
    let bridge_fn_names: ahash::AHashSet<&str> = trait_bridge_fns.iter().map(|tb| tb.name.as_str()).collect();

    // Free functions. Only functions registered in `extendr_module!` get a `wrap__` symbol, so
    // skip feature-gated wrappers that are not always compiled in the binding crate — otherwise the
    // R wrapper would `.Call("wrap__X")` a symbol that does not exist (see `super::always_registered`).
    for func in &api.functions {
        if bridge_fn_names.contains(func.name.as_str()) {
            continue;
        }
        if r_exclude_functions.contains(&func.name) {
            continue;
        }
        if !super::always_registered(func.cfg.as_deref()) {
            continue;
        }
        let params: Vec<String> = func.params.iter().map(|p| sanitize_r_param_name(&p.name)).collect();
        let params_sig = r_wrapper_params_signature(&func.params, api);
        let mut call_args = vec![format!("\"wrap__{}\"", func.name)];
        for p in &params {
            call_args.push(p.clone());
        }
        call_args.push(format!("PACKAGE = \"{package_name}\""));
        let call_args_str = call_args.join(", ");

        let roxygen_block = r_roxygen_block(&func.name, &func.doc, &func.params, &func.return_type);

        out.push_str(&crate::backends::extendr::template_env::render(
            "r_free_function_wrapper.jinja",
            minijinja::context! {
                func_name => &func.name,
                params_sig => params_sig,
                call_args_str => call_args_str,
                roxygen_block => roxygen_block,
            },
        ));
    }

    // Trait-bridge functions (register_<trait> / unregister_<trait> / clear_<trait>).
    // These are synthesised from `[trait_bridges]` in alef.toml rather than parsed from
    // Rust source, so they are absent from `api.functions` but are still registered in
    // `extendr_module!` (see `collect_trait_bridge_functions`). Without these R wrappers
    // callers cannot reach the `wrap__register_<trait>` entry points.
    //
    // The R-side surface is intentionally minimal: `register_<trait>` accepts an R object
    // (typically a named list of closures), `unregister_<trait>` accepts a name string,
    // `clear_<trait>` accepts nothing. Type checking happens on the Rust side via extendr's
    // `Robj` introspection — R's dynamic typing makes static signatures unnecessary.
    for bridge_fn in trait_bridge_fns {
        let params_sig = bridge_fn.params.join(", ");
        let mut call_args = vec![format!("\"wrap__{}\"", bridge_fn.name)];
        for p in &bridge_fn.params {
            call_args.push(p.clone());
        }
        call_args.push(format!("PACKAGE = \"{package_name}\""));
        let call_args_str = call_args.join(", ");

        // Hand-crafted roxygen block — we cannot reuse `r_roxygen_block` because
        // trait-bridge functions are not represented as `FunctionDef` in the IR.
        let kind = if bridge_fn.name.starts_with("register_") {
            "register"
        } else if bridge_fn.name.starts_with("unregister_") {
            "unregister"
        } else if bridge_fn.name.starts_with("clear_") {
            "clear"
        } else {
            ""
        };
        // For `register_<trait>`, emit a typed host-interface contract: one doc line per
        // callback method the host backend must implement, naming each param's struct type and
        // the return type. This is R's equivalent of the typed plugin Protocol other bindings
        // emit — it documents the shape the registered named list must satisfy. Params whose
        // type is a native-marshalled struct are flagged so the host knows it receives a native
        // binding object (external pointer) rather than a JSON string.
        let method_docs: Vec<String> = if kind == "register" {
            bridges
                .iter()
                .find(|b| b.register_fn.as_deref() == Some(bridge_fn.name.as_str()))
                .and_then(|b| api.types.iter().find(|t| t.is_trait && t.name == b.trait_name))
                .map(|trait_def| {
                    let native =
                        crate::backends::extendr::trait_bridge::native_marshalled_extendr_struct_params(trait_def, api);
                    trait_def
                        .methods
                        .iter()
                        .filter(|m| !m.binding_excluded)
                        .map(|m| trait_method_doc_line(m, &native))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let roxygen_block = crate::backends::extendr::template_env::render(
            "r_trait_bridge_roxygen.jinja",
            minijinja::context! {
                name => &bridge_fn.name,
                kind => kind,
                method_docs => method_docs,
            },
        );

        out.push_str(&crate::backends::extendr::template_env::render(
            "r_free_function_wrapper.jinja",
            minijinja::context! {
                func_name => &bridge_fn.name,
                params_sig => params_sig,
                call_args_str => call_args_str,
                roxygen_block => roxygen_block,
            },
        ));
    }

    // Collect S3 method pairs once — used both for per-type forwarder emission below and
    // for the trailing generic block at the end of the file.
    let s3_pairs = collect_s3_methods(api, trait_bridge_fns, bridges);
    let s3_pairs_by_type: ahash::AHashMap<String, Vec<String>> = {
        let mut map: ahash::AHashMap<String, Vec<String>> = ahash::AHashMap::new();
        for (method_name, type_name) in &s3_pairs {
            map.entry(type_name.clone()).or_default().push(method_name.clone());
        }
        map
    };

    // Class env blocks. One per non-trait, non-extendr-incompatible type — matching the
    // set registered in `extendr_module! { impl Type; ... }`.
    let excluded = collect_excluded_class_types(api, bridges);
    for typ in &api.types {
        if typ.is_trait || excluded.contains(&typ.name) {
            continue;
        }

        let class_roxygen = r_class_roxygen_block(typ);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => &typ.name,
                roxygen_block => class_roxygen,
            },
        ));

        // Emit method bindings. Skip methods that are filtered out of the Rust impl
        // block — they have no `wrap__Type__method` symbol.
        for method in &typ.methods {
            if method_is_excluded_from_impl(method, api, bridges) {
                continue;
            }
            let params: Vec<String> = method.params.iter().map(|p| sanitize_r_param_name(&p.name)).collect();
            let params_sig = if method.is_static {
                params.join(", ")
            } else if params.is_empty() {
                "self".to_string()
            } else {
                format!("self, {}", params.join(", "))
            };
            let mut call_args = vec![format!(
                "\"wrap__{type_name}__{method_name}\"",
                type_name = typ.name,
                method_name = method.name,
            )];
            if !method.is_static {
                call_args.push("self".to_string());
            }
            for p in &params {
                call_args.push(p.clone());
            }
            call_args.push(format!("PACKAGE = \"{package_name}\""));
            let call_args_str = call_args.join(", ");

            out.push_str(&crate::backends::extendr::template_env::render(
                "r_method_binding.jinja",
                minijinja::context! {
                    type_name => &typ.name,
                    method_name => &method.name,
                    params_sig => params_sig,
                    call_args_str => call_args_str,
                },
            ));
        }

        // Synthetic from_json static factory: generated for every has_default non-opaque struct.
        // The Rust impl block adds `pub fn from_json(json: String) -> Result<Self>` which extendr
        // registers as `wrap__TypeName__from_json`. We emit the R wrapper here since from_json is
        // not part of the IR and gen_extendr_wrappers_r would otherwise skip it.
        if typ.has_default && !typ.fields.is_empty() && input_type_names.contains(&typ.name) {
            out.push_str(&crate::backends::extendr::template_env::render(
                "r_from_json_factory.jinja",
                minijinja::context! {
                    type_name => &typ.name,
                    package_name => package_name,
                },
            ));
        }

        // Dispatch operators: `instance$method` and `instance[["method"]]` resolve via
        // the class env. The dispatcher captures `self` so instance methods see it.
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => &typ.name },
        ));

        // S3 method forwarders: `is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)`.
        // Lets callers write `is_valid(meta)` instead of the env-class form `meta$is_valid()`,
        // hiding the extendr implementation detail behind idiomatic R generic dispatch.
        if let Some(method_names) = s3_pairs_by_type.get(&typ.name) {
            for method_name in method_names {
                out.push_str(&crate::backends::extendr::template_env::render(
                    "r_s3_method.jinja",
                    minijinja::context! { name => method_name, type_name => &typ.name },
                ));
            }
        }
    }

    // Flat data enum class env blocks — data enums are registered as structs in
    // extendr_module! and therefore need the same dispatch operator setup so R can
    // access fields via `instance$field_name`.
    for e in &api.enums {
        if !is_flat_data_enum(e) {
            continue;
        }
        let type_name = &e.name;
        let enum_roxygen = r_enum_roxygen_block(e, true);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => type_name,
                roxygen_block => enum_roxygen,
            },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
    }

    // Unit enum wrapper functions — simple enums with no data variants that are not
    // registered as extendr classes. Emit a function that returns the default variant.
    // R callers can write `ProcessingStage()` to get the default variant.
    for e in &api.enums {
        if is_flat_data_enum(e) || is_json_passthrough_data_enum(e) {
            continue;
        }
        // Only emit for unit enums (no data in any variant)
        let is_unit_enum = e.variants.iter().all(|v| v.fields.is_empty());
        if !is_unit_enum {
            continue;
        }

        let enum_name = &e.name;

        // Emit a simple wrapper function that returns the default variant as a list with class attribute.
        // This mirrors how structs are constructed via `TypeName$default()` in R.
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_unit_enum_wrapper.jinja",
            minijinja::context! { enum_name => enum_name },
        ));
    }

    // JSON-passthrough data enum class env blocks — these enums are also
    // registered as structs in extendr_module! with `default` and `from_json`
    // static methods. Emit the class env + method bindings + dispatchers so R
    // callers can write `EnumType$from_json("...")`.
    for e in &api.enums {
        if !is_json_passthrough_data_enum(e) {
            continue;
        }
        let type_name = &e.name;
        let enum_roxygen = r_enum_roxygen_block(e, false);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => type_name,
                roxygen_block => enum_roxygen,
            },
        ));
        // `default` and `from_json` are emitted by `gen_extendr_json_passthrough_enum_struct`
        // as `pub fn` items in the `#[extendr] impl` block, so extendr registers them as
        // `wrap__<EnumType>__default` and `wrap__<EnumType>__from_json`. Bind them in R
        // by name so callers can use `EnumType$default()` / `EnumType$from_json(json)`.
        for method_name in ["default", "from_json"] {
            let params_sig = if method_name == "from_json" { "json" } else { "" };
            let mut call_args = vec![format!("\"wrap__{type_name}__{method_name}\"")];
            if method_name == "from_json" {
                call_args.push("json".to_string());
            }
            call_args.push(format!("PACKAGE = \"{package_name}\""));
            let call_args_str = call_args.join(", ");
            out.push_str(&crate::backends::extendr::template_env::render(
                "r_method_binding.jinja",
                minijinja::context! {
                    type_name => type_name,
                    method_name => method_name,
                    params_sig => params_sig,
                    call_args_str => call_args_str,
                },
            ));
        }
        // Per-variant constructors: `<Name>$<snake> <- function(<params>)
        // .Call("wrap__<Name>___factory_<snake>", <params>, PACKAGE = ...)`. The Rust fn is
        // `_factory_<snake>` (disambiguated like pyo3/magnus); R surfaces it under the snake name.
        for (r_name, rust_fn, param_names) in extendr_enum_variant_constructor_registrations(e) {
            let r_params: Vec<String> = param_names.iter().map(|p| sanitize_r_param_name(p)).collect();
            let params_sig = r_params.join(", ");
            let mut call_args = vec![format!("\"wrap__{type_name}__{rust_fn}\"")];
            call_args.extend(r_params.iter().cloned());
            call_args.push(format!("PACKAGE = \"{package_name}\""));
            let call_args_str = call_args.join(", ");
            out.push_str(&crate::backends::extendr::template_env::render(
                "r_method_binding.jinja",
                minijinja::context! {
                    type_name => type_name,
                    method_name => r_name,
                    params_sig => params_sig,
                    call_args_str => call_args_str,
                },
            ));
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
    }

    // S3 generics: one `name <- function(x, ...) UseMethod("name")` per unique instance
    // method name across every emitted class. Emit last so all class methods they dispatch
    // over are already defined in source order.
    for generic_name in unique_s3_generic_names(&s3_pairs) {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_s3_generic.jinja",
            minijinja::context! { name => generic_name },
        ));
    }

    out
}

/// Sanitize a Rust parameter name for use in R code.
/// R identifiers cannot start with underscore, so we strip any leading underscore.
pub(super) fn sanitize_r_param_name(name: &str) -> String {
    name.trim_start_matches('_').to_string()
}

pub(super) fn r_wrapper_params_signature(params: &[ParamDef], api: &ApiSurface) -> String {
    let default_types: ahash::AHashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.has_default)
        .map(|t| t.name.as_str())
        .collect();
    params
        .iter()
        .map(|p| {
            let sanitized_name = sanitize_r_param_name(&p.name);
            if let TypeRef::Named(name) = &p.ty
                && default_types.contains(name.as_str())
            {
                format!("{} = {}$default()", sanitized_name, name)
            } else if p.optional || matches!(p.ty, TypeRef::Optional(_)) {
                format!("{} = NULL", sanitized_name)
            } else {
                sanitized_name
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Collect S3 (method_name, type_name) pairs for instance methods.
///
/// Instance methods get idiomatic R S3 wrappers — `is_valid(meta)` instead of `meta$is_valid()`
/// — so callers don't have to think about the env-class implementation detail. Static methods
/// (factories like `from_json`, `default`) are intentionally excluded: they're accessed
/// directly off the class env (`Type$from_json(json)`) and don't need a generic.
///
/// Method names that collide with free functions or trait-bridge functions are skipped to
/// avoid clobbering them with a generic that calls `UseMethod`.
pub(super) fn collect_s3_methods(
    api: &ApiSurface,
    trait_bridge_fns: &[TraitBridgeFn],
    bridges: &[TraitBridgeConfig],
) -> Vec<(String, String)> {
    let excluded_types = collect_excluded_class_types(api, bridges);
    let mut reserved: ahash::AHashSet<String> = api.functions.iter().map(|f| f.name.clone()).collect();
    for bridge_fn in trait_bridge_fns {
        reserved.insert(bridge_fn.name.clone());
    }

    let mut pairs: Vec<(String, String)> = Vec::new();
    for typ in &api.types {
        if typ.is_trait || excluded_types.contains(&typ.name) {
            continue;
        }
        for method in &typ.methods {
            if method.is_static || method_is_excluded_from_impl(method, api, bridges) {
                continue;
            }
            if reserved.contains(&method.name) {
                continue;
            }
            pairs.push((method.name.clone(), typ.name.clone()));
        }
    }
    pairs
}

/// Unique generic names (sorted for deterministic emission) from a list of S3 method pairs.
pub(super) fn unique_s3_generic_names(pairs: &[(String, String)]) -> Vec<String> {
    let mut names: Vec<String> = pairs.iter().map(|(name, _)| name.clone()).collect();
    names.sort();
    names.dedup();
    names
}

/// Generate `NAMESPACE` from the alef IR.
///
/// Lists every free function and every class dispatch operator (`$.Type`, `[[.Type`)
/// emitted by `gen_extendr_wrappers_r`. Without explicit `export()` entries, R loads
/// the wrapper file but treats the symbols as internal — calling code receives
/// `could not find function`.
pub(super) fn gen_namespace(
    api: &ApiSurface,
    package_name: &str,
    trait_bridge_fns: &[TraitBridgeFn],
    r_exclude_functions: &ahash::AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(2 * 1024);
    out.push_str("# Generated by alef — do not edit.\n\n");
    // NAMESPACE requires the bare `useDynLib(...)` directive. The roxygen2 form
    // (`#' @useDynLib ...`) only takes effect when present in `.R` source files
    // processed by roxygen2 — emitting it directly into NAMESPACE leaves the
    // shared library unloaded and every `.Call` site fails at runtime.
    out.push_str(&crate::backends::extendr::template_env::render(
        "r_namespace_use_dyn_lib.jinja",
        minijinja::context! { package_name => package_name },
    ));
    out.push('\n');

    // Names emitted by the trait-bridge generator; skip them in the free-function
    // export pass to avoid duplicate `export(...)` lines in NAMESPACE.
    let bridge_fn_names: ahash::AHashSet<&str> = trait_bridge_fns.iter().map(|tb| tb.name.as_str()).collect();

    for func in &api.functions {
        if bridge_fn_names.contains(func.name.as_str()) {
            continue;
        }
        if r_exclude_functions.contains(&func.name) {
            continue;
        }
        // Skip feature-gated functions: their R wrapper is not emitted (no `wrap__` symbol), so an
        // `export()` entry would reference an undefined function. Mirrors `gen_extendr_wrappers_r`.
        if !super::always_registered(func.cfg.as_deref()) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &func.name },
        ));
    }

    // Trait-bridge functions need explicit NAMESPACE exports so that callers can use
    // them directly (e.g. `sample_core::register_text_backend(...)`). Without an `export()`
    // entry, R restricts the wrapper to internal-only visibility and `:: ` lookups fail.
    for bridge_fn in trait_bridge_fns {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &bridge_fn.name },
        ));
    }

    // Export the options helper function if an options-like input type exists.
    if find_r_options_type_from_api(api).is_some() {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => "conversion_options" },
        ));
    }

    let excluded = collect_excluded_class_types(api, bridges);
    for typ in &api.types {
        if typ.is_trait || excluded.contains(&typ.name) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &typ.name },
        ));
    }

    // Flat data enums are registered as classes in extendr_module! and need exports too.
    for e in &api.enums {
        if !is_flat_data_enum(e) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &e.name },
        ));
    }

    // JSON-passthrough data enums also need NAMESPACE exports for the class
    // env and the `$`/`[[` dispatch operators.
    for e in &api.enums {
        if !is_json_passthrough_data_enum(e) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &e.name },
        ));
    }

    // S3 generics emitted in extendr-wrappers.R are exposed through the same NAMESPACE.
    // Without `export(name)` + `S3method(name, Type)` entries R loads the wrappers but
    // refuses to dispatch — `is_valid(meta)` raises `could not find function "is_valid"`.
    let s3_pairs = collect_s3_methods(api, trait_bridge_fns, bridges);
    for generic_name in unique_s3_generic_names(&s3_pairs) {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &generic_name },
        ));
    }
    for (method_name, type_name) in &s3_pairs {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method_named.jinja",
            minijinja::context! { method_name => method_name, type_name => type_name },
        ));
    }

    out
}
