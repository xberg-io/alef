use minijinja::context;

use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::generators::trait_bridge::is_native_marshalled_struct;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use std::collections::HashMap;

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
    // String reference or optional string ref → PHP string (nullable if optional)
    if matches!(ty, TypeRef::String) {
        if optional {
            return "?string".to_string();
        }
        return "string".to_string();
    }

    // Boolean type
    if matches!(ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
        if optional {
            return "?bool".to_string();
        }
        return "bool".to_string();
    }

    // Numeric types → int or float
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

    // Default: untyped (mixed)
    if optional {
        "?mixed".to_string()
    } else {
        "mixed".to_string()
    }
}

/// Generate a PHP interface stub definition for the trait.
/// This allows PHP users to implement the interface and pass their implementation to functions.
pub fn gen_visitor_interface(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    namespace: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let interface_name = format!("{}Interface", bridge_cfg.trait_name);
    let context_type = bridge_cfg.context_type.as_deref().unwrap_or("mixed");
    let result_type = bridge_cfg.result_type.as_deref().unwrap_or("mixed");
    let mut out = String::with_capacity(2048);

    // PHP file header with declare(strict_types=1)
    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");
    out.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => namespace },
    ));
    out.push('\n');

    // Interface declaration header
    out.push_str(&crate::backends::php::template_env::render(
        "php_visitor_interface_start.jinja",
        context! {
            interface_name => &interface_name,
        },
    ));
    out.push('\n');

    // Generate each interface method
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        if named_type_name(&method.return_type) != bridge_cfg.result_type.as_deref() {
            continue;
        }

        let name = &method.name;

        // Build method signature parameters (excluding self and only PHP-visible ones)
        let mut method_params_parts = Vec::new();
        let mut param_docs = Vec::new();

        for p in &method.params {
            // Skip the context parameter - it's internal to the bridge
            let is_ctx_param = match &p.ty {
                TypeRef::Named(n) => Some(n.as_str()) == bridge_cfg.context_type.as_deref(),
                _ => false,
            };
            if is_ctx_param {
                continue;
            }

            // Convert Rust type to PHP type
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

        // Get docstring from method, sanitized for PHP target
        let doc_lines = if !method.doc.is_empty() {
            let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
            sanitized.lines().next().unwrap_or("").to_string()
        } else {
            format!("Handle for {} callback", name)
        };

        out.push_str(&crate::backends::php::template_env::render(
            "php_visitor_interface_method.jinja",
            context! {
                method_name => name,
                method_params => &method_params,
                doc_lines => &doc_lines,
                param_docs => &param_docs_str,
                context_type => context_type,
                result_type => result_type,
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

    // PHP file header with declare(strict_types=1)
    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");
    out.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => namespace },
    ));
    out.push('\n');

    // Interface declaration header
    out.push_str(&crate::backends::php::template_env::render(
        "php_interface_start.jinja",
        context! {
            interface_name => interface_name,
        },
    ));
    out.push('\n');

    // Generate each interface method (trait_type.methods already includes super-trait methods)
    for method in &trait_type.methods {
        let name = &method.name;

        // Build method signature parameters
        let mut method_params_parts = Vec::new();
        let mut param_docs = Vec::new();

        for p in &method.params {
            // Known serde structs are typed as their native PHP class (matching the native object
            // the runtime bridge now passes); everything else uses the scalar/mixed mapping.
            let php_type = native_struct_php_type(&p.ty, p.optional, api)
                .unwrap_or_else(|| rust_type_to_php_type(&p.ty, p.is_ref, p.optional, type_paths));
            method_params_parts.push(format!("{} ${}", php_type, p.name));

            let doc = format!("     * @param {} ${}", php_type, p.name);
            param_docs.push(doc);
        }

        let method_params = method_params_parts.join(", ");

        // Type the return: known serde struct → its native PHP class; otherwise the scalar/mixed
        // mapping. The interface is host-implementable, so this is the type the host must return.
        let return_type = native_struct_php_type(&method.return_type, false, api)
            .unwrap_or_else(|| rust_type_to_php_type(&method.return_type, false, false, type_paths));

        let param_docs_str = if param_docs.is_empty() {
            String::new()
        } else {
            format!("\n{}", param_docs.join("\n"))
        };

        // Get docstring from method, sanitized for PHP target
        let doc_lines = if !method.doc.is_empty() {
            let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
            sanitized.lines().next().unwrap_or("").to_string()
        } else {
            format!("Trait method: {}", name)
        };

        out.push_str(&crate::backends::php::template_env::render(
            "php_interface_method.jinja",
            context! {
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
