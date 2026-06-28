//! TypeScript declaration file (`.d.ts`) generation for NAPI-RS bindings.

use crate::codegen::naming::{to_node_name, wire_variant_value};
use crate::codegen::shared::{binding_fields, substitute_excluded_types};
use crate::core::config::NodeCapsuleTypeConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Generate the TypeScript declaration file for NAPI-RS bindings.
///
/// `streaming_item_types` maps `"OwnerType.method_name"` (snake_case) to the item type name
/// (unprefixed, e.g. `"ChatCompletionChunk"`). When a class method is identified as a streaming
/// method, its return type is overridden to `Promise<AsyncGenerator<ItemType, void, undefined>>`
/// and a matching iterator class declaration is appended.
pub(super) fn gen_dts(
    api: &ApiSurface,
    prefix: &str,
    exclude_functions: &ahash::AHashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    default_types: &ahash::AHashSet<String>,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut lines: Vec<String> = header.lines().map(|l| l.to_string()).collect();
    lines.push("/* eslint-disable */".to_string());

    // Emit `import type { TypeName } from "module"` for each capsule type.
    // These must appear after the header but before all declarations so TypeScript
    // resolves them before they are referenced in function signatures.
    if !capsule_types.is_empty() {
        // Group by from_module for compact output.
        let mut by_module: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
        for cfg in capsule_types.values() {
            by_module
                .entry(cfg.from_module.as_str())
                .or_default()
                .push(cfg.type_name.as_str());
        }
        for (module, mut names) in by_module {
            names.sort_unstable();
            lines.push(format!("import type {{ {} }} from \"{module}\";", names.join(", ")));
        }
    }

    // Emit JsonValue type definition for serde_json::Value fields.
    // This recursive type supports arbitrary JSON: primitives, arrays, and objects.
    lines.push(String::new());
    lines.push(
        "export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };"
            .to_string(),
    );

    // Collect all declarations: opaque types (classes), plain structs (interfaces), visitor traits (interfaces), enums, functions.
    // Sort each group alphabetically to produce stable, deterministic output.

    // Opaque non-trait types → `export declare class`
    // Skip capsule types — they are not emitted as napi classes.
    let mut opaque_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !capsule_types.contains_key(&t.name))
        .collect();
    opaque_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Plain structs → `export interface`
    let mut plain_types: Vec<&TypeDef> = api.types.iter().filter(|t| !t.is_opaque && !t.is_trait).collect();
    plain_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Visitor traits (opaque or not) → `export interface` (for callback object shape)
    let mut visitor_traits: Vec<&TypeDef> = api.types.iter().filter(|t| t.is_trait).collect();
    visitor_traits.sort_by(|a, b| a.name.cmp(&b.name));

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
            if f.sanitized && crate::backends::napi::trait_bridge::find_bridge_param(f, trait_bridges).is_none() {
                return false;
            }
            true
        })
        .collect();
    sorted_fns.sort_by(|a, b| a.name.cmp(&b.name));

    // Trait-bridge registration functions → `export declare function`
    // For each trait bridge, emit register, unregister, and clear functions.
    let mut trait_bridge_fns: Vec<(String, String, String)> = Vec::new();
    for bridge in trait_bridges {
        // register_{trait_name_lower}
        if let Some(register) = &bridge.register_fn {
            let js_name = crate::codegen::naming::to_node_name(register);
            trait_bridge_fns.push((js_name, format!("impl: {}", bridge.trait_name), "void".to_string()));
        }
        // unregister_{trait_name_lower}
        if let Some(unregister) = &bridge.unregister_fn {
            let js_name = crate::codegen::naming::to_node_name(unregister);
            trait_bridge_fns.push((js_name, "name: string".to_string(), "void".to_string()));
        }
        // clear_{trait_name_lower}s
        if let Some(clear) = &bridge.clear_fn {
            let js_name = crate::codegen::naming::to_node_name(clear);
            trait_bridge_fns.push((js_name, String::new(), "void".to_string()));
        }
    }
    trait_bridge_fns.sort_by(|a, b| a.0.cmp(&b.0));

    // Service entrypoint bridge functions → `export declare function`
    // For each service, emit bridge functions for each entrypoint (run/finalize).
    // The bridge function receives the registrations array and materializes the service.
    let mut service_entrypoint_fns: Vec<(String, String, String)> = Vec::new();
    for service in &api.services {
        for entrypoint in &service.entrypoints {
            let bridge_name = to_node_name(&format!("{}_{}", service.name.to_lowercase(), entrypoint.method));
            // Registrations parameter: Array<[string, any[], (...args: any[]) => any]>
            let registrations_param = "registrations: Array<[string, any[], (...args: any[]) => any]>".to_string();
            // Return type: Promise<void> for async, void for sync
            let return_type = if entrypoint.is_async {
                "Promise<void>".to_string()
            } else {
                "void".to_string()
            };
            service_entrypoint_fns.push((bridge_name, registrations_param, return_type));
        }
    }
    service_entrypoint_fns.sort_by(|a, b| a.0.cmp(&b.0));

    // Build a merged list of all declarations sorted by their Js-prefixed name so the
    // output is fully alphabetical (matching the committed index.d.ts format).
    enum Decl<'a> {
        Class(&'a TypeDef),
        Interface(&'a TypeDef),
        VisitorInterface(&'a TypeDef),
        Enum(&'a EnumDef),
        Function(&'a FunctionDef),
        TraitBridgeFunction {
            name: String,
            params: String,
            return_type: String,
        },
        ServiceEntrypoint {
            name: String,
            params: String,
            return_type: String,
        },
    }

    let mut all_decls: Vec<(String, Decl<'_>)> = Vec::new();
    for t in &opaque_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Class(t)));
    }
    for t in &plain_types {
        all_decls.push((format!("{prefix}{}", t.name), Decl::Interface(t)));
    }
    for t in &visitor_traits {
        all_decls.push((format!("{prefix}{}", t.name), Decl::VisitorInterface(t)));
    }
    for e in &sorted_enums {
        all_decls.push((format!("{prefix}{}", e.name), Decl::Enum(e)));
    }
    for f in &sorted_fns {
        all_decls.push((to_node_name(&f.name), Decl::Function(f)));
    }
    for (name, params, ret) in trait_bridge_fns {
        all_decls.push((
            name.clone(),
            Decl::TraitBridgeFunction {
                name,
                params,
                return_type: ret,
            },
        ));
    }
    for (name, params, ret) in service_entrypoint_fns {
        all_decls.push((
            name.clone(),
            Decl::ServiceEntrypoint {
                name,
                params,
                return_type: ret,
            },
        ));
    }
    all_decls.sort_by_key(|a| a.0.to_lowercase());

    // Deduplicate declarations by name — trait bridges may appear multiple times
    // if trait_bridges config is inadvertently loaded twice.
    all_decls.dedup_by(|a, b| a.0 == b.0);

    // Emit declarations with unprefixed TS names. The Rust structs carry
    // `#[napi(js_name = "Foo")]` so NAPI-RS maps JsFoo → Foo at runtime.
    // Passing empty string to dts_type/dts_params ensures field type references
    // (e.g. `Array<Config>`) are also unprefixed in the generated .d.ts.
    let no_prefix: &str = "";
    let _ = prefix; // prefix is still used in the sort-key strings above
    for (_, decl) in &all_decls {
        lines.push(String::new());
        match decl {
            Decl::Class(typ) => {
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export declare class {} {{", typ.name));
                for method in &typ.methods {
                    let js_name = to_node_name(&method.name);
                    let params = dts_params(&method.params, no_prefix, default_types);
                    // Check if this is a streaming method — if so, override the return type to
                    // Promise<AsyncGenerator<ItemType, void, undefined>> so `for await...of` is
                    // typesafe. The iterator class name follows the PascalCase(method_name)+Iterator
                    // convention emitted by alef-adapters streaming codegen.
                    let streaming_key = format!("{}.{}", typ.name, method.name);
                    let ret = if let Some(item_type) = streaming_item_types.get(&streaming_key) {
                        format!("Promise<AsyncGenerator<{item_type}, void, undefined>>")
                    } else {
                        // Use capsule-aware return type so that methods returning a capsule type
                        // emit the ecosystem type name (e.g. `Language`) rather than the now-
                        // undeclared opaque handle (e.g. `JsLanguage`).
                        dts_return_type_capsule(
                            &method.return_type,
                            method.error_type.is_some(),
                            method.is_async,
                            no_prefix,
                            capsule_types,
                        )
                    };
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
                lines.push(format!("export interface {} {{", typ.name));
                for field in binding_fields(&typ.fields) {
                    let js_name = to_node_name(&field.name);
                    let ts_ty = dts_type(&field.ty, no_prefix);
                    lines.extend(format_jsdoc(&field.doc, "  "));
                    // Mark a field optional when:
                    //   1. The underlying Rust type is Option<T> (TypeRef::Optional)
                    //   2. The field itself has `optional = true` in the IR (e.g. *Update struct fields)
                    //   3. The parent type has `has_default = true` — the NAPI binding wraps every
                    //      field in Option<T> so callers can omit fields and rely on defaults.
                    let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional || typ.has_default;
                    // DTO fields are readonly — callers construct new objects rather than mutating.
                    if is_optional {
                        lines.push(format!("  readonly {js_name}?: {ts_ty}"));
                    } else {
                        lines.push(format!("  readonly {js_name}: {ts_ty}"));
                    }
                }
                lines.push("}".to_string());
            }
            Decl::VisitorInterface(typ) => {
                // Emit visitor trait as a TypeScript interface with optional callback methods.
                // Each method becomes an optional property with a function signature.
                //
                // Types excluded from the binding surface (e.g. `InternalDocument`) are not emitted as
                // `.d.ts` declarations, so substitute them with their JSON marshaling form in method
                // signatures — otherwise the interface references an undefined TS name.
                let excluded: std::collections::HashSet<&str> = api
                    .excluded_type_paths
                    .keys()
                    .map(String::as_str)
                    .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
                    .collect();
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export interface {} {{", typ.name));
                if trait_bridge_requires_plugin_name(typ, trait_bridges) {
                    lines.push("  name(): string".to_string());
                }
                for method in &typ.methods {
                    let js_name = to_node_name(&method.name);
                    if trait_bridge_requires_plugin_name(typ, trait_bridges) && method.name == "name" {
                        continue;
                    }
                    let sub_params: Vec<ParamDef> = method
                        .params
                        .iter()
                        .map(|p| ParamDef {
                            ty: substitute_excluded_types(&p.ty, &excluded),
                            ..p.clone()
                        })
                        .collect();
                    let params = dts_params(&sub_params, no_prefix, default_types);
                    let ret = trait_bridge_dts_return_type(
                        &substitute_excluded_types(&method.return_type, &excluded),
                        method.is_async,
                        no_prefix,
                    );
                    lines.extend(format_jsdoc(&method.doc, "  "));
                    let optional_marker = if method.has_default_impl { "?" } else { "" };
                    lines.push(format!("  {js_name}{optional_marker}({params}): {ret}"));
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
                        let tag_value = wire_variant_value(
                            &variant.name,
                            variant.serde_rename.as_deref(),
                            e.serde_rename_all.as_deref(),
                        );
                        let mut obj_fields: Vec<String> = vec![format!("{tag_field}: '{tag_value}'")];
                        for field in &variant.fields {
                            let js_name = to_node_name(&field.name);
                            let ts_ty = dts_type(&field.ty, no_prefix);
                            if matches!(field.ty, TypeRef::Optional(_)) {
                                obj_fields.push(format!("{js_name}?: {ts_ty}"));
                            } else {
                                obj_fields.push(format!("{js_name}: {ts_ty}"));
                            }
                        }
                        member_lines.push(format!("  | {{ {} }}", obj_fields.join("; ")));
                    }
                    lines.push(format!("export type {} =", e.name));
                    lines.extend(member_lines);
                } else {
                    lines.push(format!("export declare enum {} {{", e.name));
                    for variant in &e.variants {
                        // NAPI string_enum: variant values follow serde_rename_all casing.
                        // Prefer explicit serde_rename, then apply rename_all, then fall back to variant name.
                        let value = wire_variant_value(
                            &variant.name,
                            variant.serde_rename.as_deref(),
                            e.serde_rename_all.as_deref(),
                        );
                        lines.extend(format_jsdoc(&variant.doc, "  "));
                        lines.push(format!("  {} = \"{}\",", variant.name, value));
                    }
                    lines.push("}".to_string());
                }
            }
            Decl::Function(func) => {
                let js_name = to_node_name(&func.name);
                let params = dts_params(&func.params, no_prefix, default_types);
                // When the function returns a capsule type, use the ecosystem type name
                // (e.g. `Language` from `tree-sitter`) instead of the Js-prefixed wrapper.
                let ret = dts_return_type_capsule(
                    &func.return_type,
                    func.error_type.is_some(),
                    func.is_async,
                    no_prefix,
                    capsule_types,
                );
                lines.extend(format_jsdoc(&func.doc, ""));
                lines.push(format!("export declare function {js_name}({params}): {ret};"));
            }
            Decl::TraitBridgeFunction {
                name,
                params,
                return_type,
            } => {
                lines.push(format!("export declare function {name}({params}): {return_type};"));
            }
            Decl::ServiceEntrypoint {
                name,
                params,
                return_type,
            } => {
                lines.push(format!("export declare function {name}({params}): {return_type};"));
            }
        }
    }

    // Emit a class declaration for each streaming iterator struct. These are adapter-generated
    // types (not in api.types) that implement Symbol.asyncIterator via NAPI-RS's AsyncGenerator
    // trait. Each iterator wraps a channel receiver and yields streaming chunks.
    //
    // The iterator class name follows the PascalCase(adapter.name)+Iterator convention from
    // alef-adapters/src/streaming.rs: gen_node_body. The [Symbol.asyncIterator]() method is
    // automatically added by #[napi(async_iterator)] at build time.
    let mut sorted_streaming: Vec<(&String, &String)> = streaming_item_types.iter().collect();
    sorted_streaming.sort_by_key(|(k, _)| k.as_str());
    for (owner_method_key, item_type) in sorted_streaming {
        // Derive the iterator class name: "OwnerType.method_name" → PascalCase(method_name) + "Iterator"
        let method_name = owner_method_key
            .split('.')
            .next_back()
            .unwrap_or(owner_method_key.as_str());
        let iter_class_name = method_name
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                }
            })
            .collect::<String>()
            + "Iterator";
        lines.push(String::new());
        lines.push(format!("export declare class {iter_class_name} {{"));
        lines.push(format!(
            "  next(value?: undefined): Promise<IteratorResult<{item_type}, void>>"
        ));
        lines.push(format!(
            "  [Symbol.asyncIterator](): AsyncGenerator<{item_type}, void, undefined>"
        ));
        lines.push("}".to_string());
    }

    // Emit a class declaration for each error type that has introspection methods.
    // The Rust-side #[napi] struct is named `Js{ErrorName}Info`; the TypeScript
    // declaration strips the Js prefix so it reads `{ErrorName}Info`.
    let mut sorted_errors: Vec<_> = api.errors.iter().filter(|e| !e.methods.is_empty()).collect();
    sorted_errors.sort_by_key(|e| e.name.as_str());
    for error in sorted_errors {
        let class_name = format!("{}Info", error.name);
        lines.push(String::new());
        lines.push(format!("export declare class {class_name} {{"));
        for method in &error.methods {
            let (js_name, ret_type): (&str, &str) = match method.name.as_str() {
                "status_code" => ("statusCode", "number"),
                "is_transient" => ("isTransient", "boolean"),
                "error_type" => ("errorType", "string"),
                _ => continue,
            };
            lines.push(format!("  {js_name}(): {ret_type}"));
        }
        lines.push("}".to_string());
    }

    lines.push(String::new());
    lines.join("\n")
}

fn trait_bridge_requires_plugin_name(typ: &TypeDef, trait_bridges: &[crate::core::config::TraitBridgeConfig]) -> bool {
    trait_bridges
        .iter()
        .any(|bridge| bridge.trait_name == typ.name && bridge.super_trait.as_deref().is_some())
}

/// TypeScript return type for a trait-bridge host interface method.
///
/// The host interface is the type a JS object must satisfy to be registered as a plugin (or used
/// as a visitor). Its method returns are typed natively against the binding's emitted type
/// (`dts_type`) — e.g. a `Doc` return becomes `Doc`, an `Option<Doc>` becomes `Doc | null` — so
/// callers get a precise contract instead of the prior opaque `string`. `()` returns map to
/// `void`. Async methods are wrapped in `Promise<...>`.
fn trait_bridge_dts_return_type(return_type: &TypeRef, is_async: bool, prefix: &str) -> String {
    let base = match return_type {
        TypeRef::Unit => "void".to_string(),
        other => dts_type(other, prefix),
    };
    if is_async { format!("Promise<{base}>") } else { base }
}

/// Format a rustdoc string as JSDoc comment lines with the given `indent` prefix.
///
/// Translates rustdoc Markdown sections (`# Arguments`, `# Returns`,
/// `# Errors`, `# Example`) into JSDoc tags (`@param`, `@returns`,
/// `@throws`, `@example`) via [`crate::codegen::doc_emission::render_jsdoc_sections`].
/// Replaces ` ```rust ` fences with ` ```typescript `.
///
/// Returns an empty `Vec` when `doc` is empty. For a single-line doc, emits
/// `["/** Description */"]`. For multi-line docs, emits the block form:
/// `["/**", " * line1", " * line2", " */"]`, each prefixed by `indent`.
pub(super) fn format_jsdoc(doc: &str, indent: &str) -> Vec<String> {
    let doc = doc.trim();
    if doc.is_empty() {
        return vec![];
    }
    let sections = crate::codegen::doc_emission::parse_rustdoc_sections(doc);
    let rendered = crate::codegen::doc_emission::render_jsdoc_sections(&sections);
    let body = if rendered.trim().is_empty() {
        doc.to_string()
    } else {
        rendered
    };
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() == 1 {
        vec![format!("{indent}/** {} */", lines[0].trim())]
    } else {
        let mut out = Vec::with_capacity(lines.len() + 2);
        out.push(format!("{indent}/**"));
        for line in &lines {
            let trimmed = line.trim_end();
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
            crate::core::ir::PrimitiveType::Bool => "boolean".to_string(),
            crate::core::ir::PrimitiveType::U8
            | crate::core::ir::PrimitiveType::U16
            | crate::core::ir::PrimitiveType::U32
            | crate::core::ir::PrimitiveType::I8
            | crate::core::ir::PrimitiveType::I16
            | crate::core::ir::PrimitiveType::I32
            | crate::core::ir::PrimitiveType::F32
            | crate::core::ir::PrimitiveType::F64 => "number".to_string(),
            // NAPI maps u64/usize/isize to i64 on the Rust side; JS sees it as number.
            crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::I64
            | crate::core::ir::PrimitiveType::Usize
            | crate::core::ir::PrimitiveType::Isize => "number".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
        TypeRef::Bytes => "Uint8Array".to_string(),
        TypeRef::Json => "JsonValue".to_string(),
        TypeRef::Duration => "number".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => format!("{} | null", dts_type(inner, prefix)),
        TypeRef::Vec(inner) => format!("Array<{}>", dts_type(inner, prefix)),
        TypeRef::Map(k, v) => format!("Record<{}, {}>", dts_type(k, prefix), dts_type(v, prefix)),
        TypeRef::Named(name) => format!("{prefix}{name}"),
    }
}

/// Render a list of parameters as a TypeScript parameter string for `.d.ts`.
pub(super) fn dts_params(params: &[ParamDef], prefix: &str, default_types: &ahash::AHashSet<String>) -> String {
    dts_params_with_order(params, prefix, true, default_types)
}

fn dts_params_with_order(
    params: &[ParamDef],
    prefix: &str,
    reorder_for_typescript: bool,
    default_types: &ahash::AHashSet<String>,
) -> String {
    if !reorder_for_typescript {
        let has_required_after = required_after_optional(params, default_types);
        return params
            .iter()
            .enumerate()
            .map(|(idx, p)| dts_param(p, prefix, param_is_optional(p, default_types), !has_required_after[idx]))
            .collect::<Vec<_>>()
            .join(", ");
    }

    // TypeScript requires optional parameters to come after all required parameters (TS1016).
    // If the Rust source has optional params followed by required params (e.g., `lang: Option<&str>`,
    // `code: &str`), we must reorder: required first, then optional, preserving relative order within
    // each group.
    let mut required: Vec<&ParamDef> = Vec::new();
    let mut optional: Vec<&ParamDef> = Vec::new();
    for p in params {
        if param_is_optional(p, default_types) {
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
        .map(|p| dts_param(p, prefix, param_is_optional(p, default_types), true))
        .collect::<Vec<_>>()
        .join(", ")
}

fn dts_param(p: &ParamDef, prefix: &str, is_optional: bool, allow_question_optional: bool) -> String {
    let js_name = to_node_name(&p.name);
    let ts_ty = dts_type(&p.ty, prefix);
    if is_optional && allow_question_optional {
        format!("{js_name}?: {ts_ty} | undefined | null")
    } else if is_optional {
        format!("{js_name}: {ts_ty} | undefined | null")
    } else {
        format!("{js_name}: {ts_ty}")
    }
}

fn param_is_optional(p: &ParamDef, default_types: &ahash::AHashSet<String>) -> bool {
    p.optional
        || p.default.is_some()
        || p.typed_default.is_some()
        || matches!(&p.ty, TypeRef::Named(name) if default_types.contains(name.as_str()))
}

fn required_after_optional(params: &[ParamDef], default_types: &ahash::AHashSet<String>) -> Vec<bool> {
    let mut seen_optional = false;
    let mut result = vec![false; params.len()];
    for (idx, param) in params.iter().enumerate() {
        let is_optional = param_is_optional(param, default_types);
        result[idx] = seen_optional && !is_optional;
        seen_optional |= is_optional;
    }
    result
}

/// Render the TypeScript return type for a function/method in `.d.ts`, substituting
/// the ecosystem type name for capsule-configured types.
///
/// When the return type is a capsule type (e.g. `Language` → `tree-sitter`), emits
/// the type_name from the capsule config (e.g. `Language`) instead of the Js-prefixed
/// wrapper name (e.g. `JsLanguage`). The `import type` line at the top of the file
/// makes that name resolvable.
pub(super) fn dts_return_type_capsule(
    ret: &TypeRef,
    _has_error: bool,
    is_async: bool,
    prefix: &str,
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
) -> String {
    let base = match ret {
        TypeRef::Unit => "void".to_string(),
        TypeRef::Named(name) => {
            if let Some(cfg) = capsule_types.get(name.as_str()) {
                cfg.type_name.clone()
            } else {
                dts_type(ret, prefix)
            }
        }
        other => dts_type(other, prefix),
    };
    if is_async { format!("Promise<{base}>") } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};

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
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
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
        let result = dts_params(&params, "Js", &ahash::AHashSet::new());
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
        let result = dts_params(&params, "Js", &ahash::AHashSet::new());
        assert_eq!(result, "ctx: string, code: string, lang?: string | undefined | null");
    }

    /// All-required params: order must be preserved exactly.
    #[test]
    fn dts_params_all_required_preserves_order() {
        let params = vec![make_param("a", false), make_param("b", false), make_param("c", false)];
        let result = dts_params(&params, "Js", &ahash::AHashSet::new());
        assert_eq!(result, "a: string, b: string, c: string");
    }

    #[test]
    fn dts_params_treats_defaulted_params_as_optional() {
        let mut params = vec![make_param("path", false), make_param("config", false)];
        params[1].default = Some("Default::default()".to_string());
        let result = dts_params(&params, "Js", &ahash::AHashSet::new());
        assert_eq!(
            result, "path: string, config?: string | undefined | null",
            "defaulted params must be optional in generated declarations"
        );
    }

    #[test]
    fn trait_bridge_dts_return_type_wraps_async_methods_in_promise() {
        // Async methods wrap the (now natively-typed) return in Promise<...>; sync methods do not.
        assert_eq!(
            trait_bridge_dts_return_type(&TypeRef::Named("ExtractionResult".to_string()), true, ""),
            "Promise<ExtractionResult>"
        );
        assert_eq!(trait_bridge_dts_return_type(&TypeRef::Unit, true, ""), "Promise<void>");
        assert_eq!(
            trait_bridge_dts_return_type(&TypeRef::Named("ExtractionResult".to_string()), false, ""),
            "ExtractionResult"
        );
    }

    #[test]
    fn plugin_trait_bridge_requires_name_in_typescript_interface() {
        let typ = TypeDef {
            name: "DocumentExtractor".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,

            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        };
        let bridges = vec![crate::core::config::TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        }];
        assert!(trait_bridge_requires_plugin_name(&typ, &bridges));
    }

    #[test]
    fn gen_dts_includes_service_entrypoint_bridge_functions() {
        use crate::core::ir::{EntrypointDef, EntrypointKind, MethodDef, ReceiverKind, ServiceDef};
        let api = ApiSurface {
            crate_name: "test".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![ServiceDef {
                name: "App".to_string(),
                rust_path: "test::App".to_string(),
                constructor: MethodDef {
                    name: "new".to_string(),
                    params: vec![],
                    return_type: TypeRef::Named("App".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    receiver: Some(ReceiverKind::Owned),
                    doc: String::new(),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                configurators: vec![],
                registrations: vec![],
                entrypoints: vec![EntrypointDef {
                    method: "into_router".to_string(),
                    kind: EntrypointKind::Finalize,
                    is_async: true,
                    params: vec![],
                    return_type: TypeRef::Unit,
                    error_type: None,
                    doc: String::new(),
                }],
                doc: String::new(),
                cfg: None,
            }],
            handler_contracts: vec![],
            unsupported_public_items: vec![],
        };
        let dts = gen_dts(
            &api,
            "",
            &ahash::AHashSet::new(),
            &[],
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        // Verify the service entrypoint function appears in the dts
        assert!(
            dts.contains("export declare function appIntoRouter"),
            "dts should declare appIntoRouter bridge function for App.into_router"
        );
        assert!(
            dts.contains("registrations: Array<[string, any[], (...args: any[]) => any]>"),
            "service entrypoint should have registrations parameter"
        );
        assert!(
            dts.contains("Promise<void>"),
            "async into_router entrypoint should return Promise<void>"
        );
    }
}
