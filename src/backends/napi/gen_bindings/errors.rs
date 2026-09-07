//! TypeScript declaration file (`.d.ts`) generation for NAPI-RS bindings.

use super::enums;
use super::types::{opaque_instance_method_is_dropped, opaque_static_method_is_dropped};
use crate::codegen::naming::ts_property_key::ts_property_key;
use crate::codegen::naming::{node_type_name, to_node_name, wire_variant_value};
use crate::codegen::shared::{binding_fields, substitute_excluded_types};
use crate::core::config::NodeCapsuleTypeConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Generate the TypeScript declaration file for NAPI-RS bindings.
///
/// `streaming_item_types` maps `"OwnerType.method_name"` (snake_case) to the item type name
/// (unprefixed, e.g. `"ChatCompletionChunk"`). When a class method is identified as a streaming
/// method, its return type is overridden to `Promise<AsyncGenerator<ItemType, void, undefined>>`
/// and a matching iterator class declaration is appended.
// Each parameter is an independent slice of the generation input with no shared owner to group
// them under; bundling them into a struct would add a type whose only purpose is to satisfy the
// arity lint, and every call site would construct it inline anyway. ~keep
#[allow(
    clippy::too_many_arguments,
    reason = "independent codegen inputs with no natural grouping"
)]
pub(super) fn gen_dts(
    api: &ApiSurface,
    prefix: &str,
    exclude_functions: &ahash::AHashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    default_types: &ahash::AHashSet<String>,
    adapter_bodies: &crate::adapters::AdapterBodies,
    core_import: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut lines: Vec<String> = header.lines().map(|l| l.to_string()).collect();
    lines.push("/* eslint-disable */".to_string());

    if !capsule_types.is_empty() {
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

    lines.push(String::new());
    lines.push(
        "export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };"
            .to_string(),
    );

    let mut opaque_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !capsule_types.contains_key(&t.name))
        .collect();
    opaque_types.sort_by(|a, b| a.name.cmp(&b.name));

    // Same name sets `gen_opaque_struct_methods` (`types.rs`) builds at its call site — passed to
    // the two `opaque_*_method_is_dropped` predicates below so a `Decl::Class` method is declared
    // here only when the binding actually generates a wrapper for it.
    let opaque_type_names: ahash::AHashSet<String> = opaque_types.iter().map(|t| t.name.clone()).collect();
    let capsule_type_names: ahash::AHashSet<String> = capsule_types.keys().cloned().collect();

    let mut plain_types: Vec<&TypeDef> = api.types.iter().filter(|t| !t.is_opaque && !t.is_trait).collect();
    plain_types.sort_by(|a, b| a.name.cmp(&b.name));

    let mut visitor_traits: Vec<&TypeDef> = api.types.iter().filter(|t| t.is_trait).collect();
    visitor_traits.sort_by(|a, b| a.name.cmp(&b.name));

    let mut sorted_enums: Vec<&EnumDef> = api.enums.iter().collect();
    sorted_enums.sort_by(|a, b| a.name.cmp(&b.name));

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

    let mut trait_bridge_fns: Vec<(String, String, String)> = Vec::new();
    for bridge in trait_bridges {
        if let Some(register) = &bridge.register_fn {
            let js_name = crate::codegen::naming::to_node_name(register);
            trait_bridge_fns.push((js_name, format!("impl: {}", bridge.trait_name), "void".to_string()));
        }
        if let Some(unregister) = &bridge.unregister_fn {
            let js_name = crate::codegen::naming::to_node_name(unregister);
            trait_bridge_fns.push((js_name, "name: string".to_string(), "void".to_string()));
        }
        if let Some(clear) = &bridge.clear_fn {
            let js_name = crate::codegen::naming::to_node_name(clear);
            trait_bridge_fns.push((js_name, String::new(), "void".to_string()));
        }
    }
    trait_bridge_fns.sort_by(|a, b| a.0.cmp(&b.0));

    let mut service_entrypoint_fns: Vec<(String, String, String)> = Vec::new();
    for service in &api.services {
        for entrypoint in &service.entrypoints {
            let bridge_name = to_node_name(&format!("{}_{}", service.name.to_lowercase(), entrypoint.method));
            let registrations_param = "registrations: Array<[string, any[], (...args: any[]) => any]>".to_string();
            let return_type = if entrypoint.is_async {
                "Promise<void>".to_string()
            } else {
                "void".to_string()
            };
            service_entrypoint_fns.push((bridge_name, registrations_param, return_type));
        }
    }
    service_entrypoint_fns.sort_by(|a, b| a.0.cmp(&b.0));

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

    all_decls.dedup_by(|a, b| a.0 == b.0);

    // `#[napi(js_name = "Foo")]` so NAPI-RS maps JsFoo → Foo at runtime; every declared name and
    // every reference to it below goes through `node_type_name` so the two can never diverge.
    for (_, decl) in &all_decls {
        lines.push(String::new());
        match decl {
            Decl::Class(typ) => {
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export declare class {} {{", node_type_name(&typ.name)));
                // `gen_opaque_struct_methods` (`types.rs`) silently drops a method that can't
                // cross into a `#[napi]` wrapper — never registering it in the `#[napi]` impl
                // block — for the exact reasons these two predicates check. Calling them here
                // (rather than re-deriving the condition) is what keeps `index.d.ts` from
                // promising a method the compiled extension does not export. ~keep
                let declared_methods = typ.methods.iter().filter(|method| {
                    if method.receiver.is_some() {
                        !opaque_instance_method_is_dropped(
                            method,
                            &typ.name,
                            adapter_bodies,
                            &capsule_type_names,
                            &opaque_type_names,
                        )
                    } else {
                        !opaque_static_method_is_dropped(method, &typ.name, adapter_bodies)
                    }
                });
                for method in declared_methods {
                    let js_name = to_node_name(&method.name);
                    let params = dts_params(&method.params, default_types);
                    let streaming_key = format!("{}.{}", typ.name, method.name);
                    let ret = if let Some(item_type) = streaming_item_types.get(&streaming_key) {
                        format!("Promise<AsyncGenerator<{item_type}, void, undefined>>")
                    } else {
                        dts_return_type_capsule(
                            &method.return_type,
                            method.error_type.is_some(),
                            method.is_async,
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
                lines.push(format!("export interface {} {{", node_type_name(&typ.name)));
                for field in binding_fields(&typ.fields) {
                    let js_name = to_node_name(&field.name);
                    let ts_ty = dts_type(&field.ty);
                    lines.extend(format_jsdoc(&field.doc, "  "));
                    let is_optional = super::types::napi_field_is_optional(field, typ);
                    if is_optional {
                        lines.push(format!("  readonly {js_name}?: {ts_ty}"));
                    } else {
                        lines.push(format!("  readonly {js_name}: {ts_ty}"));
                    }
                }
                lines.push("}".to_string());
            }
            Decl::VisitorInterface(typ) => {
                let excluded: std::collections::HashSet<&str> = api
                    .excluded_type_paths
                    .keys()
                    .map(String::as_str)
                    .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
                    .collect();
                lines.extend(format_jsdoc(&typ.doc, ""));
                lines.push(format!("export interface {} {{", node_type_name(&typ.name)));
                if trait_bridge_requires_plugin_name(typ, trait_bridges) {
                    lines.push("  name(): string".to_string());
                    lines.push("  version?(): string".to_string());
                    lines.push("  initialize?(): void".to_string());
                    lines.push("  shutdown?(): void".to_string());
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
                    let params = dts_params(&sub_params, default_types);
                    let ret = trait_bridge_dts_return_type(
                        &substitute_excluded_types(&method.return_type, &excluded),
                        method.is_async,
                    );
                    lines.extend(format_jsdoc(&method.doc, "  "));
                    let optional_marker = if method.has_default_impl { "?" } else { "" };
                    lines.push(format!("  {js_name}{optional_marker}({params}): {ret}"));
                }
                lines.push("}".to_string());
            }
            Decl::Enum(e) => {
                // `enums::is_tagged_data_enum` is the SAME authority `gen_enum` (the compiled
                // `#[napi]` type that actually executes at runtime) routes through. Re-deriving
                // this as `e.serde_tag.is_some()` here previously missed the case `gen_enum` also
                // treats as a tagged object: a default (externally tagged, no
                // `#[serde(tag/content/untagged)]`) enum that carries a payload variant -- that
                // combination has no `serde_tag`, so the old check declared it `export declare
                // enum Foo { ... }` while the compiled struct behind it was `{ type, ...fields }`
                // -- a `.d.ts` and a runtime shape for the same type that disagreed. Asking the
                // shared predicate instead of re-deriving it is what keeps them in lockstep. ~keep
                let is_data_enum = enums::is_tagged_data_enum(e);
                // Same host/foreign verdict `enums::gen_enum` reaches for the emitted Rust enum,
                // from the same authority, so the overlay and the wrapper agree on which crate
                // owns a cfg-gated variant's feature. ~keep
                let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &e.rust_path);
                let ts_name = node_type_name(&e.name);
                lines.extend(format_jsdoc(&e.doc, ""));
                if is_data_enum && e.serde_content.is_some() {
                    // Adjacent tagging (`#[serde(tag, content)]`): each variant serializes as its
                    // own `{ tag: 'value'; content: T }`, so a discriminated union of per-variant
                    // shapes matches the wire format exactly. (~keep)
                    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(e);
                    let mut member_lines: Vec<String> = Vec::new();
                    for variant in &e.variants {
                        let tag_value = wire_variant_value(
                            &variant.name,
                            variant.serde_rename.as_deref(),
                            e.serde_rename_all.as_deref(),
                        );
                        let mut obj_fields: Vec<String> = vec![format!("{tag_field}: '{tag_value}'")];
                        for field in &variant.fields {
                            let js_name = if crate::codegen::conversions::is_tuple_variant(&variant.fields) {
                                e.serde_content
                                    .as_deref()
                                    .expect("adjacent content is present")
                                    .to_string()
                            } else {
                                to_node_name(&field.name)
                            };
                            let ts_ty = dts_type(&field.ty);
                            if matches!(field.ty, TypeRef::Optional(_)) {
                                obj_fields.push(format!("{js_name}?: {ts_ty}"));
                            } else {
                                obj_fields.push(format!("{js_name}: {ts_ty}"));
                            }
                        }
                        member_lines.push(format!("  | {{ {} }}", obj_fields.join("; ")));
                    }
                    lines.push(format!("export type {ts_name} ="));
                    lines.extend(member_lines);
                    lines.push(format!("export declare const {ts_name}: {{"));
                    for variant in &e.variants {
                        if let Some(field) = variant.fields.first() {
                            lines.push(format!(
                                "  {}({}: {}): {ts_name};",
                                variant.name,
                                e.serde_content.as_deref().expect("adjacent content is present"),
                                dts_type(&field.ty),
                            ));
                        } else {
                            lines.push(format!("  readonly {}: {ts_name};", variant.name));
                        }
                    }
                    lines.push("};".to_string());
                } else if is_data_enum && e.variants.iter().any(|v| !v.fields.is_empty()) {
                    lines.extend(internal_tagged_union_dts_lines(e, ts_name));
                } else if is_data_enum {
                    // Internal tagging, every variant a unit variant: `{"kind":"A"}` carries no
                    // payload fields to differentiate, so a single object with a union-valued tag
                    // says the same thing as a per-variant union without the redundant repetition.
                    // (~keep)
                    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(e);
                    let tag_values: Vec<String> = e
                        .variants
                        .iter()
                        .map(|v| {
                            format!(
                                "'{}'",
                                wire_variant_value(&v.name, v.serde_rename.as_deref(), e.serde_rename_all.as_deref())
                            )
                        })
                        .collect();
                    lines.push(format!(
                        "export type {ts_name} = {{ {tag_field}: {} }};",
                        tag_values.join(" | ")
                    ));
                } else if e.serde_untagged && e.variants.iter().any(|v| !v.fields.is_empty()) {
                    // `#[serde(untagged)]`: each variant serializes as its own bare shape, with no
                    // discriminant and no wrapper object — the napi glue already reflects this by
                    // passing the value through as opaque `serde_json::Value`
                    // (`gen_untagged_data_enum_as_value_wrapper`), so the `.d.ts` union is the only
                    // place the real per-variant shapes can be expressed. (~keep)
                    lines.push(format!("export type {ts_name} ="));
                    for variant in &e.variants {
                        lines.push(format!("  | {}", untagged_variant_dts_type(e, variant)));
                    }
                } else if enums::is_variant_untagged_string_enum(e) {
                    // Every data-carrying variant is individually `#[serde(untagged)]` while the
                    // container keeps default external tagging: unit variants still serialize as
                    // their own bare (cased) name, so they stay literal union members exactly like
                    // an ordinary string enum -- unlike the container-level-untagged branch above,
                    // they are NOT `null`. Each untagged data variant serializes as its own bare
                    // payload with no discriminant; when that payload is a single `String` it
                    // widens the union with `(string & {})`, the standard TypeScript idiom that
                    // accepts any string while keeping autocomplete for the known literals. A
                    // non-`String` payload falls back to its own `dts_type`, unwrapped, since the
                    // widening idiom only helps when the wider type and the literals share a
                    // primitive. (~keep)
                    let mut members =
                        enums::variant_untagged_string_enum_literal_values(e, is_host_enum, configured_features);
                    for variant in e.variants.iter().filter(|v| !v.fields.is_empty()) {
                        // Only a SINGLE-field variant serializes as that field's bare value. An
                        // untagged multi-field tuple variant is a JSON array and a struct-shaped
                        // one a JSON object, so naming the first field's type there would declare
                        // a type the runtime never produces -- emit `unknown` rather than a
                        // confident lie. (~keep)
                        let field_ty = match variant.fields.as_slice() {
                            [only] => Some(dts_type(&only.ty)),
                            _ => None,
                        };
                        members.push(match field_ty.as_deref() {
                            Some("string") => "(string & {})".to_string(),
                            Some(other) => other.to_string(),
                            None => "unknown".to_string(),
                        });
                    }
                    lines.push(format!("export type {ts_name} = {};", members.join(" | ")));
                } else {
                    lines.push(format!("export declare enum {ts_name} {{"));
                    // `wire_variant_value` computes the *serde* JSON wire name, but a plain enum
                    // here is emitted by `gen_enum` as `#[napi(string_enum = "...")]`, whose
                    // runtime value comes from napi-derive-backend's own `convert_case`-based
                    // case transform — a different algorithm that disagrees with serde's for
                    // identifiers with a letter-to-digit boundary (`Bm25` -> serde's helper gives
                    // `"bm25"`, napi's actual runtime value is `"bm_25"`).
                    // `declared_string_enum_variants` (`enums.rs`) is the canonical derivation of
                    // both that napi-side value AND of which variants the `#[napi(string_enum)]`
                    // wrapper actually declares, so ask it instead of re-deriving either here.
                    // Asking it for membership too is what keeps this overlay from advertising a
                    // foreign cfg-gated variant `gen_enum` proved unreachable and omitted from the
                    // emitted Rust enum — a value a consumer could name in TypeScript and never
                    // construct at runtime. Fall back to the serde name over every variant only
                    // for enum shapes it declines to classify as a string enum, preserving prior
                    // behavior for those. (~keep)
                    let declared = enums::declared_string_enum_variants(e, is_host_enum, configured_features);
                    let members: Vec<(&EnumVariant, String)> = declared.unwrap_or_else(|| {
                        e.variants
                            .iter()
                            .map(|variant| {
                                let value = wire_variant_value(
                                    &variant.name,
                                    variant.serde_rename.as_deref(),
                                    e.serde_rename_all.as_deref(),
                                );
                                (variant, value)
                            })
                            .collect()
                    });
                    for (variant, value) in members {
                        lines.extend(format_jsdoc(&variant.doc, "  "));
                        lines.push(format!("  {} = \"{}\",", variant.name, value));
                    }
                    lines.push("}".to_string());
                }
            }
            Decl::Function(func) => {
                let js_name = to_node_name(&func.name);
                let params = dts_params(&func.params, default_types);
                let ret = dts_return_type_capsule(
                    &func.return_type,
                    func.error_type.is_some(),
                    func.is_async,
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

    // automatically added by #[napi(async_iterator)] at build time.
    let mut sorted_streaming: Vec<(&String, &String)> = streaming_item_types.iter().collect();
    sorted_streaming.sort_by_key(|(k, _)| k.as_str());
    for (owner_method_key, item_type) in sorted_streaming {
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

    // The Rust-side #[napi] struct is named `Js{ErrorName}Info`; the TypeScript
    let mut sorted_errors: Vec<_> = api.errors.iter().filter(|e| !e.methods.is_empty()).collect();
    sorted_errors.sort_by_key(|e| e.name.as_str());
    for error in sorted_errors {
        let class_name = format!("{}Info", error.name);
        lines.push(String::new());
        lines.push(format!("export declare class {class_name} {{"));
        // `code` is always present — it doesn't depend on which introspection methods the
        // error type implements (see `gen_napi_error_class`). (~keep)
        lines.push("  code(): number".to_string());
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
fn trait_bridge_dts_return_type(return_type: &TypeRef, is_async: bool) -> String {
    let base = match return_type {
        TypeRef::Unit => "void".to_string(),
        other => dts_type(other),
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
    let sanitized =
        crate::codegen::doc_emission::sanitize_rust_idioms(doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    let doc = sanitized.trim();
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
///
/// `TypeRef::Named` resolves through `node_type_name` — the same function every declaration
/// site in `gen_dts` uses for the type's own name — so a reference to a type can never disagree
/// with how that type declared itself.
pub(super) fn dts_type(ty: &TypeRef) -> String {
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
        TypeRef::Optional(inner) => format!("{} | null", dts_type(inner)),
        TypeRef::Vec(inner) => format!("Array<{}>", dts_type(inner)),
        TypeRef::Map(k, v) => format!("Record<{}, {}>", dts_type(k), dts_type(v)),
        TypeRef::Named(name) => node_type_name(name).to_string(),
    }
}

/// `.d.ts` lines for an internally-tagged enum (`#[serde(tag = "...")]`) with at least one
/// data-bearing variant: each variant serializes to its own flat object on the wire —
/// `{"type":"basic","username":"...","password":"..."}` — with no other keys present, so a
/// discriminated union of per-variant shapes matches the wire format exactly and gives callers
/// real narrowing plus required fields. The compiled napi struct behind this still stores every
/// variant's fields as one flattened `Option<T>` bag (`gen_tagged_enum_as_object`), but a
/// constructed instance only ever populates its own variant's fields, so the union type is a
/// faithful (if narrower) view of what a caller actually receives — the same relationship the
/// adjacent-tagging branch relies on. Field naming reuses `tagged_enum_field_js_name` so a
/// newtype variant's synthetic `_0` field still gets its variant-derived name, not a bare `0` —
/// e.g. `Message::User(UserMessage)` renders as `{ role: 'user'; user: UserMessage }`, not a
/// flattened `{ role: 'user'; content: string }`.
///
/// Exposed at `pub(crate)` (re-exported from `backends::napi`) so the TypeScript e2e snippet
/// generator can typecheck a generated snippet's object literal against the exact union this
/// function produces, rather than against a hand-guessed copy of it — see
/// `e2e::codegen::typescript::test_file::builders`'s `node_tagged_enum_*` cross-generator
/// tests. ~keep
pub(crate) fn internal_tagged_union_dts_lines(e: &EnumDef, ts_name: &str) -> Vec<String> {
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(e);
    let mut lines = vec![format!("export type {ts_name} =")];
    for variant in &e.variants {
        let tag_value = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            e.serde_rename_all.as_deref(),
        );
        let mut obj_fields: Vec<String> = vec![format!("{tag_field}: '{tag_value}'")];
        for field in &variant.fields {
            let js_name = enums::tagged_enum_field_js_name(variant, field);
            let ts_ty = dts_type(&field.ty);
            if matches!(field.ty, TypeRef::Optional(_)) {
                obj_fields.push(format!("{js_name}?: {ts_ty}"));
            } else {
                obj_fields.push(format!("{js_name}: {ts_ty}"));
            }
        }
        lines.push(format!("  | {{ {} }}", obj_fields.join("; ")));
    }
    lines
}

/// TypeScript shape of one variant of an `untagged` enum, as it actually appears on the wire:
/// a newtype variant serializes as its inner value, a multi-field tuple variant as a TS tuple,
/// a struct variant as its own object, and a unit variant as `null`. There is no discriminant —
/// serde distinguishes untagged variants structurally at deserialize time. (~keep)
fn untagged_variant_dts_type(enum_def: &EnumDef, variant: &EnumVariant) -> String {
    if variant.fields.is_empty() {
        return "null".to_string();
    }
    if variant.is_tuple {
        if variant.fields.len() == 1 {
            return dts_type(&variant.fields[0].ty);
        }
        let elems: Vec<String> = variant.fields.iter().map(|f| dts_type(&f.ty)).collect();
        return format!("[{}]", elems.join(", "));
    }
    let fields: Vec<String> = variant
        .fields
        .iter()
        .map(|field| {
            // WIRE names, not host names. An untagged data enum is the one napi shape whose
            // value never passes through a `#[napi(object)]` struct: `gen_enum` routes it to
            // `gen_untagged_data_enum_as_value_wrapper`, a `#[serde(transparent)]` newtype over
            // `serde_json::Value`, and the generated `impl From<Js{Enum}> for core::{Enum}` is
            // `serde_json::from_value(val.0)` straight into the CORE type. The keys that
            // deserializer accepts are therefore the core type's serde names -- there is no
            // `js_name` anywhere on this path to make `to_node_name` true.
            //
            // Declaring `to_node_name(&field.name)` here made this the exact defect shape the
            // `.d.ts`/runtime enum-shape parity tests exist to catch, one layer deeper: a core
            // field `max_chars` was declared `maxChars`, and `from_value` on the object a caller
            // wrote against that declaration falls through to `unwrap_or_default()` -- the wrong
            // variant, silently, with no error to trace back.
            //
            // The container rule for a struct variant's field names is `rename_all_fields`, NOT
            // the enum's `serde_rename_all` (which cases VARIANT names) -- the same two-namespace
            // split `backends::go::gen_bindings::types::field_shape::go_data_enum_variant_field`
            // already honors, which is the sibling backend that got this right. ~keep
            //
            // A wire name is not an identifier and cannot be interpolated bare: kebab-case from
            // `#[serde(rename_all = "kebab-case")]` parses as a subtraction, so it must go
            // through `ts_property_key`, the one renderer `backends::wasm::gen_bindings::ts_union`
            // also uses -- two emitters describing the same runtime object must not disagree
            // about when a key needs quoting. ~keep
            let wire_name = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                enum_def.rename_all_fields.as_deref(),
            );
            let key = ts_property_key(&wire_name);
            let ts_ty = dts_type(&field.ty);
            if matches!(field.ty, TypeRef::Optional(_)) {
                format!("{key}?: {ts_ty}")
            } else {
                format!("{key}: {ts_ty}")
            }
        })
        .collect();
    format!("{{ {} }}", fields.join("; "))
}

/// Render a list of parameters as a TypeScript parameter string for `.d.ts`.
pub(super) fn dts_params(params: &[ParamDef], default_types: &ahash::AHashSet<String>) -> String {
    dts_params_with_order(params, true, default_types)
}

fn dts_params_with_order(
    params: &[ParamDef],
    reorder_for_typescript: bool,
    default_types: &ahash::AHashSet<String>,
) -> String {
    if !reorder_for_typescript {
        let has_required_after = required_after_optional(params, default_types);
        return params
            .iter()
            .enumerate()
            .map(|(idx, p)| dts_param(p, param_is_optional(p, default_types), !has_required_after[idx]))
            .collect::<Vec<_>>()
            .join(", ");
    }

    let mut required: Vec<&ParamDef> = Vec::new();
    let mut optional: Vec<&ParamDef> = Vec::new();
    for p in params {
        if param_is_optional(p, default_types) {
            optional.push(p);
        } else {
            required.push(p);
        }
    }
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
        .map(|p| dts_param(p, param_is_optional(p, default_types), true))
        .collect::<Vec<_>>()
        .join(", ")
}

fn dts_param(p: &ParamDef, is_optional: bool, allow_question_optional: bool) -> String {
    let js_name = to_node_name(&p.name);
    let ts_ty = dts_type(&p.ty);
    if is_optional && allow_question_optional {
        format!("{js_name}?: {ts_ty} | undefined | null")
    } else if is_optional {
        format!("{js_name}: {ts_ty} | undefined | null")
    } else {
        format!("{js_name}: {ts_ty}")
    }
}

/// [`param_is_optional`] against a raw `ApiSurface::types` slice, for callers outside this
/// backend that hold `type_defs` rather than the prebuilt `Default`-implementing name set.
///
/// The e2e snippet generator has to know whether a call it renders may end its argument list
/// early, and that is decided by whichever `.d.ts` the snippet is compiled against — this one for
/// node. Delegating rather than restating keeps the widening rule in one place: a snippet that
/// omits an argument the emitted declaration marks required is `TS2554` in the validator. ~keep
pub(crate) fn napi_param_is_optional(param: &ParamDef, type_defs: &[TypeDef]) -> bool {
    let default_types: ahash::AHashSet<String> = type_defs
        .iter()
        .filter(|type_def| type_def.has_default)
        .map(|type_def| type_def.name.clone())
        .collect();
    param_is_optional(param, &default_types)
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
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
) -> String {
    let base = match ret {
        TypeRef::Unit => "void".to_string(),
        TypeRef::Named(name) => {
            if let Some(cfg) = capsule_types.get(name.as_str()) {
                cfg.type_name.clone()
            } else {
                dts_type(ret)
            }
        }
        other => dts_type(other),
    };
    if is_async { format!("Promise<{base}>") } else { base }
}

#[cfg(test)]
mod tests;
