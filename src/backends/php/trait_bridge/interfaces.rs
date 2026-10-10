use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::generators::trait_bridge::is_native_marshalled_struct;
use crate::core::config::TraitBridgeConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use std::collections::HashMap;

/// The PSR-4 class name [`gen_visitor_interface`] declares for `trait_name` -- and therefore the
/// file basename the `GeneratedFile` it backs must be written to.
///
/// Exposed so the call site (`gen_bindings::rust_bindings::generate_bindings`) can name the file
/// from the same formula that names the class, instead of recomputing the suffix independently.
/// The previous call site used `format!("{trait_name}.php")` for every trait bridge, visitor and
/// registration alike, which happened to match [`gen_registration_interface`]'s unsuffixed class
/// name but not this function's `{trait_name}Interface` -- so a visitor interface's file basename
/// never matched the class PHP would find inside it, and no PSR-4 autoloader could resolve the
/// class (alef #485). ~keep
pub fn visitor_interface_class_name(trait_name: &str) -> String {
    format!("{trait_name}Interface")
}

/// PHP type hint for a callback param/return that is a known serde struct: the native
/// `#[php_class]` the runtime bridge now passes/expects. The class lives in the same PHP
/// namespace as the interface, so the bare class name resolves correctly. Returns `None`
/// for types that are not native-marshalled structs.
fn native_struct_php_type(ty: &TypeRef, optional: bool, api: &ApiSurface) -> Option<String> {
    let leaf = match ty {
        TypeRef::Named(n) => n.as_str(),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => n.as_str(),
            _ => return None,
        },
        _ => return None,
    };
    if !is_native_marshalled_struct(leaf, api) {
        return None;
    }
    let is_optional = optional || matches!(ty, TypeRef::Optional(_));
    Some(if is_optional {
        format!("?{leaf}")
    } else {
        leaf.to_string()
    })
}

/// Convert a Rust TypeRef to a PHP type string for interface declarations.
fn rust_type_to_php_type(ty: &TypeRef, _is_ref: bool, optional: bool, _type_paths: &HashMap<String, String>) -> String {
    if matches!(ty, TypeRef::String) {
        if optional {
            return "?string".to_string();
        }
        return "string".to_string();
    }

    if matches!(ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
        if optional {
            return "?bool".to_string();
        }
        return "bool".to_string();
    }

    if let TypeRef::Primitive(prim) = ty {
        match prim {
            crate::core::ir::PrimitiveType::I32
            | crate::core::ir::PrimitiveType::I64
            | crate::core::ir::PrimitiveType::U32
            | crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::Usize => {
                if optional {
                    return "?int".to_string();
                }
                return "int".to_string();
            }
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => {
                if optional {
                    return "?float".to_string();
                }
                return "float".to_string();
            }
            _ => {}
        }
    }

    if optional {
        "?mixed".to_string()
    } else {
        "mixed".to_string()
    }
}

/// Description used for the callback's context parameter when the configured context type is not
/// in the API surface or carries no rustdoc of its own -- deliberately says nothing beyond what
/// the parameter's own name and declared type already say, rather than inventing detail alef
/// cannot source. ~keep
const GENERIC_CONTEXT_PARAM_DOC: &str = "Visitor context information";

/// The `@param` description for the callback's context argument: the context type's own rustdoc
/// summary when it has one.
///
/// The docblock previously spelled this detail out in prose that named a specific context type;
/// when the type names became template variables, the prose was replaced wholesale rather than
/// re-derived, so the generated interface stopped saying anything about what the context carries.
/// Deriving it from the type's own doc restores that without hardcoding any consumer's type. ~keep
fn context_param_doc(api: &ApiSurface, context_type: &str) -> String {
    api.types
        .iter()
        .find(|type_def| type_def.name == context_type)
        .map(|type_def| sanitize_rust_idioms(&type_def.doc, DocTarget::PhpDoc))
        .and_then(|doc| {
            doc.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| GENERIC_CONTEXT_PARAM_DOC.to_string())
}

/// The parenthetical naming the values a callback may return, e.g. ` (Proceed, Halt, or Replace)`.
///
/// Derived from the configured result enum's own variants, so it stays true for any trait bridge
/// instead of restating one crate's variant names the way the hardcoded original did. Empty when
/// the result type resolves to no enum in the API surface: a return alef cannot enumerate gets no
/// invented description. ~keep
fn result_values_suffix(api: &ApiSurface, bridge_cfg: &TraitBridgeConfig) -> String {
    let Some(metadata) = crate::codegen::visitor_result::visitor_result_metadata(api, bridge_cfg) else {
        return String::new();
    };
    let names: Vec<&str> = metadata
        .unit_variants
        .iter()
        .chain(metadata.string_payload_variants.iter())
        .map(|variant| variant.name.as_str())
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [only] => format!(" ({only})"),
        [first, second] => format!(" ({first} or {second})"),
        [leading @ .., last] => format!(" ({}, or {last})", leading.join(", ")),
    }
}

/// The `Result::Variant` expression the interface's default implementations return, for the
/// interface-level docblock -- `None` when the result enum is unresolvable, or when any method
/// lacks a default implementation, in which case the sentence would be false.
///
/// Asked of the trait itself rather than trusted from the caller's own visitor-bridge gate: the
/// gate and this sentence would otherwise be two derivations of one fact. ~keep
fn default_result_expression(
    api: &ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
    trait_type: &TypeDef,
    result_type: &str,
) -> Option<String> {
    if !trait_type.methods.iter().all(|method| method.has_default_impl) {
        return None;
    }
    crate::codegen::visitor_result::visitor_result_metadata(api, bridge_cfg)
        .map(|metadata| crate::codegen::visitor_result::default_result_expr(result_type, &metadata))
}

/// Generate a PHP interface stub definition for the trait.
/// This allows PHP users to implement the interface and pass their implementation to functions.
pub fn gen_visitor_interface(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    namespace: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> String {
    let interface_name = visitor_interface_class_name(&bridge_cfg.trait_name);
    let context_type = bridge_cfg.context_type.as_deref().unwrap_or("mixed");
    let result_type = bridge_cfg.result_type.as_deref().unwrap_or("mixed");
    let context_doc = context_param_doc(api, context_type);
    let result_values = result_values_suffix(api, bridge_cfg);
    let default_result_expr = default_result_expression(api, bridge_cfg, trait_type, result_type);
    let mut out = String::with_capacity(2048);

    out.push_str("<?php\n\n");
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("declare(strict_types=1);\n\n");
    out.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        crate::alef_context! { namespace => namespace },
    ));
    out.push('\n');

    out.push_str(&crate::backends::php::template_env::render(
        "php_visitor_interface_start.jinja",
        crate::alef_context! {
            interface_name => &interface_name,
            default_result_expr => &default_result_expr,
        },
    ));
    out.push('\n');

    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        if named_type_name(&method.return_type) != bridge_cfg.result_type.as_deref() {
            continue;
        }

        let name = &method.name;

        let mut method_params_parts = Vec::new();
        let mut param_docs = Vec::new();

        for p in &method.params {
            let is_ctx_param = match &p.ty {
                TypeRef::Named(n) => Some(n.as_str()) == bridge_cfg.context_type.as_deref(),
                _ => false,
            };
            if is_ctx_param {
                continue;
            }

            let php_type = rust_type_to_php_type(&p.ty, p.is_ref, p.optional, type_paths);
            method_params_parts.push(format!("{} ${}", php_type, p.name));

            let doc = format!("     * @param {} ${}", php_type, p.name);
            param_docs.push(doc);
        }

        let method_params = method_params_parts.join(", ");

        let param_docs_str = if param_docs.is_empty() {
            String::new()
        } else {
            format!("\n{}", param_docs.join("\n"))
        };

        let doc_lines = if !method.doc.is_empty() {
            let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
            sanitized.lines().next().unwrap_or("").to_string()
        } else {
            format!("Handle for {} callback", name)
        };

        out.push_str(&crate::backends::php::template_env::render(
            "php_visitor_interface_method.jinja",
            crate::alef_context! {
                method_name => name,
                method_params => &method_params,
                doc_lines => &doc_lines,
                param_docs => &param_docs_str,
                context_type => context_type,
                context_doc => &context_doc,
                result_type => result_type,
                result_values => &result_values,
            },
        ));
        out.push('\n');
    }

    out.push_str("}\n");

    out
}

pub(super) fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => named_type_name(inner),
        _ => None,
    }
}

/// Generate a PHP interface stub definition for a registration-style trait bridge.
/// These bridges allow PHP users to implement the interface and register their implementation.
pub fn gen_registration_interface(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    namespace: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> String {
    let interface_name = &bridge_cfg.trait_name;
    let mut out = String::with_capacity(2048);

    out.push_str("<?php\n\n");
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str("declare(strict_types=1);\n\n");
    out.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        crate::alef_context! { namespace => namespace },
    ));
    out.push('\n');

    out.push_str(&crate::backends::php::template_env::render(
        "php_interface_start.jinja",
        crate::alef_context! {
            interface_name => interface_name,
        },
    ));
    out.push('\n');

    let (required, optional): (Vec<&crate::core::ir::MethodDef>, Vec<&crate::core::ir::MethodDef>) =
        trait_type.methods.iter().partition(|m| !m.has_default_impl);
    if !optional.is_empty() {
        let names: Vec<&str> = optional.iter().map(|m| m.name.as_str()).collect();
        out.push_str(&format!(
            "    // Optional methods the bridge calls when the class defines them (the
    // trait's Rust default behavior applies otherwise): {}.
    // The lifecycle hooks initialize()/shutdown() are likewise optional.
",
            names.join(", ")
        ));
    }

    for method in required {
        let name = &method.name;

        let mut method_params_parts = Vec::new();
        let mut param_docs = Vec::new();

        for p in &method.params {
            let php_type = native_struct_php_type(&p.ty, p.optional, api)
                .unwrap_or_else(|| rust_type_to_php_type(&p.ty, p.is_ref, p.optional, type_paths));
            method_params_parts.push(format!("{} ${}", php_type, p.name));

            let doc = format!("     * @param {} ${}", php_type, p.name);
            param_docs.push(doc);
        }

        let method_params = method_params_parts.join(", ");

        let return_type = native_struct_php_type(&method.return_type, false, api)
            .unwrap_or_else(|| rust_type_to_php_type(&method.return_type, false, false, type_paths));

        let param_docs_str = if param_docs.is_empty() {
            String::new()
        } else {
            format!("\n{}", param_docs.join("\n"))
        };

        let doc_lines = if !method.doc.is_empty() {
            let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
            sanitized.lines().next().unwrap_or("").to_string()
        } else {
            format!("Trait method: {}", name)
        };

        out.push_str(&crate::backends::php::template_env::render(
            "php_interface_method.jinja",
            crate::alef_context! {
                method_name => name,
                method_params => &method_params,
                return_type => &return_type,
                doc_lines => &doc_lines,
                param_docs => &param_docs_str,
            },
        ));
        out.push('\n');
    }

    out.push_str("}\n");

    out
}
