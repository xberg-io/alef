//! Swift trait bridge codegen for outbound plugins.
//!
//! For each configured `TraitBridgeConfig` entry (when `bind_via = "function_param"`),
//! generates:
//!
//! 1. A Swift `protocol Swift<TraitName>Bridge` declaring the trait methods with
//!    async/throws matching the Rust trait method signatures. Excluded/internal types
//!    are marshalled as JSON strings at the boundary.
//! 2. A Swift `struct Swift<TraitName>Adapter` wrapping an instance of the protocol
//!    and exposing methods that handle marshalling (conversion from/to JSON for excluded types,
//!    conversion from/to proper Swift types for visible types).
//! 3. A `register<TraitName>(_ bridge: Swift<TraitName>Bridge)` function that
//!    constructs the adapter and calls into Rust to register it.

use crate::backends::swift::naming::bridge_protocol_name;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;

/// Generate Swift trait bridge protocol and adapter for outbound plugins.
///
/// `exclude_types` is the set of types that are not visible in the generated Swift binding.
/// These types are marshalled as JSON strings at the trait boundary.
///
/// Returns a list of (filename, content) tuples ready for emission.
pub fn gen_trait_bridge_files(
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
    exclude_types: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut files = Vec::new();

    // Collect function_param bridges (non-excluded Swift bridges)
    let function_param_bridges: Vec<_> = bridges
        .iter()
        .filter(|(_, bridge_cfg, _)| {
            !bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift")
                && matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam)
        })
        .collect();

    // Emit SwiftPluginBridge super-protocol as the first file if there are any function_param bridges
    if !function_param_bridges.is_empty() {
        let content = emit_swift_plugin_bridge_protocol();
        files.push(("SwiftPluginBridge.swift".to_string(), content));
    }

    for (trait_name, bridge_cfg, trait_def) in bridges {
        // Skip if swift is in exclude_languages
        if bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift") {
            continue;
        }

        // Skip if not function_param binding (only outbound plugins use this codegen)
        if !matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam) {
            continue;
        }

        let content = gen_single_trait_bridge_file(trait_name, bridge_cfg, trait_def, exclude_types);
        // Use the canonical protocol name as the filename base so the filename
        // stays in sync with the protocol declaration.
        let protocol = bridge_protocol_name(trait_name);
        let filename = format!("{protocol}.swift");
        files.push((filename, content));
    }

    files
}

/// Emit the SwiftPluginBridge super-protocol that all trait bridges inherit from.
///
/// This protocol declares the four Plugin trait super-methods that all plugins must implement:
/// name(), version(), initialize() throws, and shutdown() throws.
fn emit_swift_plugin_bridge_protocol() -> String {
    crate::backends::swift::template_env::render("swift_plugin_bridge_protocol.swift.jinja", minijinja::context! {})
}

/// Collect all Named type references recursively from a TypeRef.
pub fn collect_named_types(type_ref: &TypeRef, named_types: &mut HashSet<String>) {
    match type_ref {
        TypeRef::Named(name) => {
            named_types.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_named_types(inner, named_types);
        }
        TypeRef::Map(key, val) => {
            collect_named_types(key, named_types);
            collect_named_types(val, named_types);
        }
        _ => {}
    }
}

/// Compute the named types that must cross Swift trait bridge/plugin shim boundaries as strings.
pub fn excluded_named_type_bridge_policy(trait_def: &TypeDef, excluded_types: &HashSet<String>) -> HashSet<String> {
    let mut policy = excluded_types.clone();
    for method in &trait_def.methods {
        if method.has_default_impl {
            continue;
        }
        for param in &method.params {
            collect_named_types(&param.ty, &mut policy);
        }
        collect_named_types(&method.return_type, &mut policy);
    }
    policy
}

/// Generate Swift trait bridge code for a single trait.
///
/// `exclude_types` contains type names that are not visible in the Swift binding surface.
/// These types are marshalled as JSON strings at trait boundaries.
fn gen_single_trait_bridge_file(
    trait_name: &str,
    _bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    exclude_types: &HashSet<String>,
) -> String {
    let bridge_exclude_types = excluded_named_type_bridge_policy(trait_def, exclude_types);

    let protocol = bridge_protocol_name(trait_name);
    let mut protocol_methods = String::new();

    // B2 fix: Ensure protocol declares all trait methods. Both protocol and adapter
    // iterate the same trait_def.methods, so if a method appears in the protocol,
    // it MUST appear in the adapter. This guarantees callers can invoke any
    // protocol-declared method on the adapter without compile errors.
    for method in &trait_def.methods {
        let method_camel = method.name.to_lower_camel_case();
        // Protocol method parameters marshal excluded types as JSON strings (like Java does)
        let params_sig = swift_method_params(&method.params, &bridge_exclude_types);
        // Protocol method return types marshal excluded types as JSON strings (like Java does)
        let return_type = swift_return_type(&method.return_type, &bridge_exclude_types);
        let throws = if method.error_type.is_some() { " throws" } else { "" };
        // NOTE: async is removed — Swift{Trait}Bridge is now fully sync to match plugin protocol shape

        // Emit all methods as protocol requirements
        protocol_methods.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_protocol_method.swift.jinja",
            minijinja::context! {
                method_name => method_camel,
                params => params_sig,
                throws_clause => throws,
                return_type => return_type,
            },
        ));
    }

    // Emit an extension providing Swift defaults for methods that have default impls in Rust.
    // This allows conformers to opt out of implementing them.
    let mut default_methods = String::new();

    for method in &trait_def.methods {
        if !method.has_default_impl {
            continue;
        }

        let method_camel = method.name.to_lower_camel_case();
        let params_sig = swift_method_params(&method.params, &bridge_exclude_types);
        let return_type = swift_return_type(&method.return_type, &bridge_exclude_types);
        let throws = if method.error_type.is_some() { " throws" } else { "" };

        // Generate default body based on return type
        let default_body = match &method.return_type {
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "return true".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I32) => "return 50".to_string(), // priority default
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64) => "return 0".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize) => "return 0".to_string(),
            TypeRef::String => "return \"\"".to_string(),
            TypeRef::Unit => "".to_string(), // no return needed
            TypeRef::Vec(_) => "return []".to_string(),
            TypeRef::Named(_) => "return \"{}\"".to_string(), // excluded types are strings
            _ => "return nil".to_string(),
        };

        default_methods.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_default_method.swift.jinja",
            minijinja::context! {
                method_name => method_camel,
                params => params_sig,
                throws_clause => throws,
                return_type => return_type,
                default_body => default_body,
            },
        ));
    }

    // B2 fix: Method entry points — these marshal types across the boundary.
    // The adapter must register ALL methods declared in the protocol above.
    // Both loops iterate trait_def.methods in the same order, guaranteeing parity.
    let mut adapter_methods = String::new();
    for method in &trait_def.methods {
        let method_camel = method.name.to_lower_camel_case();
        // Build parameter signature for the adapter method (input from Rust across the boundary)
        let params_sig = swift_method_params(&method.params, &bridge_exclude_types);
        // Build return type for the adapter method (output back to Rust)
        // If the method has an error type, the adapter returns String (JSON envelope);
        // otherwise, it returns the original type
        let return_type = if method.error_type.is_some() {
            "String".to_string()
        } else {
            swift_return_type(&method.return_type, &bridge_exclude_types)
        };

        // Build async/throws keywords for the adapter method signature
        // NOTE: async is removed from adapter signatures since Swift{Trait}Bridge is now sync
        let throws_kw = if method.error_type.is_some() { " throws" } else { "" };

        // Generate method body: construct call arguments and handle return value.
        let call_args = build_adapter_call_args(method);
        let call_args_str = call_args.join(", ");
        let method_body = if method.error_type.is_some() {
            // Error-returning method: wrap result in try-catch and return JSON envelope
            // NOTE: async is removed, so only 'try' is used (not 'try await')
            let success_body = trait_adapter_success_body(&method.return_type, &bridge_exclude_types);
            crate::backends::swift::template_env::render(
                "swift_trait_adapter_error_body.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    call_args => &call_args_str,
                    success_body => success_body,
                },
            )
        } else {
            // Sync method without error: return the result directly, no encoding
            // NOTE: async is removed, all methods are now sync
            crate::backends::swift::template_env::render(
                "swift_trait_adapter_direct_body.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    call_args => &call_args_str,
                },
            )
        };

        adapter_methods.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_adapter_method.swift.jinja",
            minijinja::context! {
                method_name => method_camel,
                params => params_sig,
                throws_clause => throws_kw,
                return_type => return_type,
                body => method_body,
            },
        ));
    }

    // NOTE: Registration functions are NOT emitted here. They live in <Binding>.swift
    // (generated by `emit_trait_bridge_forwarders`) and accept the Box-based type
    // (`Swift{Trait}Box`) from `Sources/RustBridge/Plugins.swift` that the RustBridge
    // `register_*` entry point actually expects. Emitting a duplicate here that passes
    // `Swift{Trait}Adapter` would cause a type mismatch at the `try RustBridge.{camel}(adapter)`
    // call site because Adapter and Box are distinct, incompatible types.

    crate::backends::swift::template_env::render(
        "swift_trait_bridge_file.swift.jinja",
        minijinja::context! {
            trait_name => trait_name,
            protocol => protocol,
            protocol_methods => protocol_methods,
            default_methods => default_methods,
            adapter_methods => adapter_methods,
        },
    )
}

fn trait_adapter_success_body(return_type: &TypeRef, bridge_exclude_types: &HashSet<String>) -> String {
    let expression = match return_type {
        TypeRef::Unit => Some("marshal_ok_result(Empty())"),
        TypeRef::String => Some("marshal_ok_result(String(result))"),
        TypeRef::Primitive(_) | TypeRef::Bytes | TypeRef::Char => Some("marshal_ok_result(result)"),
        TypeRef::Vec(inner) => match **inner {
            TypeRef::String => Some("marshal_ok_result(result.map { String($0) })"),
            _ => Some("marshal_ok_result(try JSONEncoder().encode(result))"),
        },
        TypeRef::Named(name) if bridge_exclude_types.contains(name) => None,
        _ => Some("marshal_ok_result(try JSONEncoder().encode(result))"),
    };

    if let Some(expression) = expression {
        crate::backends::swift::template_env::render(
            "swift_trait_adapter_success.swift.jinja",
            minijinja::context! {
                expression => expression,
            },
        )
    } else {
        // Excluded type: result is the native Swift struct (not Encodable).
        // Encode directly and wrap in JSON string manually.
        crate::backends::swift::template_env::render(
            "swift_trait_adapter_excluded_success.swift.jinja",
            minijinja::context! {},
        )
    }
}

/// Emit Swift method parameter signature from MethodDef params.
///
/// Protocol methods use native types (excluded types as native structs, not marshalled).
#[allow(dead_code)]
fn swift_method_params_native(params: &[crate::core::ir::ParamDef], exclude_types: &HashSet<String>) -> String {
    if params.is_empty() {
        return String::new();
    }

    params
        .iter()
        .map(|p| {
            let name = p.name.to_snake_case();
            let ty = swift_type_name_native(&p.ty, exclude_types);
            format!("{}: {}", name, ty)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Emit Swift method parameter signature from MethodDef params.
///
/// Adapter methods marshal excluded types as JSON strings for the C boundary.
/// Uses camelCase labels to match Swift naming conventions.
fn swift_method_params(params: &[crate::core::ir::ParamDef], exclude_types: &HashSet<String>) -> String {
    if params.is_empty() {
        return String::new();
    }

    params
        .iter()
        .map(|p| {
            let name = p.name.to_lower_camel_case();
            let ty = swift_type_name(&p.ty, exclude_types);
            format!("{}: {}", name, ty)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Get the Swift type name for a TypeRef.
///
/// Protocol methods use native types (excluded types as native structs).
// Protocol methods use native types; `_exclude_types` is accepted for API
// symmetry with `swift_type_name` but is not consulted — excluded types are
// always emitted by their native name in protocol contexts.
#[allow(dead_code)]
fn swift_type_name_native(ty: &TypeRef, _exclude_types: &HashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "Bool".to_string(),
            crate::core::ir::PrimitiveType::I8 => "Int8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "Int16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "Int32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "Int64".to_string(),
            crate::core::ir::PrimitiveType::U8 => "UInt8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "UInt16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "UInt32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "UInt64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "UInt".to_string(),
            crate::core::ir::PrimitiveType::Isize => "Int".to_string(),
            crate::core::ir::PrimitiveType::F32 => "Float".to_string(),
            crate::core::ir::PrimitiveType::F64 => "Double".to_string(),
        },
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Data".to_string(),
        TypeRef::Path => "URL".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Named(name) => {
            // Protocol uses native type name (excluded or not)
            name.clone()
        }
        TypeRef::Vec(inner) => format!("[{}]", swift_type_name_native(inner, _exclude_types)),
        TypeRef::Map(k, v) => format!(
            "[{}: {}]",
            swift_type_name_native(k, _exclude_types),
            swift_type_name_native(v, _exclude_types)
        ),
        TypeRef::Optional(inner) => format!("{}?", swift_type_name_native(inner, _exclude_types)),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "TimeInterval".to_string(),
    }
}

/// Get the Swift type name for a TypeRef.
///
/// Adapter methods marshal excluded/internal types (not in the visible binding surface) as JSON strings.
fn swift_type_name(ty: &TypeRef, exclude_types: &HashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "Bool".to_string(),
            crate::core::ir::PrimitiveType::I8 => "Int8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "Int16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "Int32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "Int64".to_string(),
            crate::core::ir::PrimitiveType::U8 => "UInt8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "UInt16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "UInt32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "UInt64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "UInt".to_string(), // Maps to platform-dependent size
            crate::core::ir::PrimitiveType::Isize => "Int".to_string(), // Maps to platform-dependent size
            crate::core::ir::PrimitiveType::F32 => "Float".to_string(),
            crate::core::ir::PrimitiveType::F64 => "Double".to_string(),
        },
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Data".to_string(),
        TypeRef::Path => "URL".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Named(name) => {
            // If the named type is excluded (internal/not visible), marshal as JSON string
            if exclude_types.contains(name) {
                "String".to_string() // JSON-marshalled as String
            } else {
                name.clone()
            }
        }
        TypeRef::Vec(inner) => format!("[{}]", swift_type_name(inner, exclude_types)),
        TypeRef::Map(k, v) => format!(
            "[{}: {}]",
            swift_type_name(k, exclude_types),
            swift_type_name(v, exclude_types)
        ),
        TypeRef::Optional(inner) => format!("{}?", swift_type_name(inner, exclude_types)),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Json => "String".to_string(), // JSON is marshalled as String
        TypeRef::Duration => "TimeInterval".to_string(), // Duration -> TimeInterval in Swift
    }
}

/// Emit Swift return type from TypeRef for adapter methods (marshalled types).
fn swift_return_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> String {
    swift_type_name(ty, exclude_types)
}

/// Build the call arguments for the adapter method with Swift argument labels.
///
/// Swift requires explicit labels for all method arguments.
/// The protocol methods use camelCase labels, so we match that here.
fn build_adapter_call_args(method: &crate::core::ir::MethodDef) -> Vec<String> {
    method
        .params
        .iter()
        .map(|p| {
            let camel = p.name.to_lower_camel_case();
            format!("{}: {}", camel, camel)
        })
        .collect()
}

/// Generate the registration overloads file (`BridgeRegistrationOverloads.swift`)
/// that provides convenience register/unregister functions for bridge stubs.
///
/// This file contains:
/// - Unregister `name:` label overloads for all bridge-bound traits
/// - Register overloads that accept bridge protocols and wrap them in adapters
/// - Stub adapter classes that implement full trait protocols with sensible defaults
///
/// Returns `(filename, content)` ready for emission.
pub fn gen_bridge_registration_overloads_file(
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
) -> Option<(String, String)> {
    let trait_bridges: Vec<_> = bridges
        .iter()
        .filter(|(_, bridge_cfg, _)| {
            // Only include bridges not excluded from swift
            !bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift")
                && matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam)
        })
        .collect();

    if trait_bridges.is_empty() {
        return None;
    }

    // Note: _loadBytesFromPathOrUtf8 is emitted by the swift_bridge_registration_overloads template,
    // not here, to avoid duplication.

    let mut unregister_overloads = String::new();
    for (trait_name, _, _) in &trait_bridges {
        let pascal_name = trait_bridge_pascal_name(trait_name);
        unregister_overloads.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_unregister_overload.swift.jinja",
            minijinja::context! {
                pascal_name => &pascal_name,
            },
        ));
    }

    let mut register_overloads = String::new();
    for (trait_name, _, _) in &trait_bridges {
        let pascal_name = trait_bridge_pascal_name(trait_name);
        register_overloads.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_register_overload.swift.jinja",
            minijinja::context! {
                pascal_name => &pascal_name,
            },
        ));
    }

    let content = crate::backends::swift::template_env::render(
        "swift_trait_bridge_overloads.swift.jinja",
        minijinja::context! {
            unregister_overloads => unregister_overloads,
            register_overloads => register_overloads,
        },
    );

    Some(("BridgeRegistrationOverloads.swift".to_string(), content))
}

/// Convert a snake_case or kebab-case identifier to PascalCase.
fn trait_bridge_pascal_name(s: &str) -> String {
    crate::codegen::naming::to_class_name(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::BridgeBinding;

    fn make_trait_def(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("testcrate::{}", name),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            param_name: None,
            type_alias: None,
            exclude_languages: vec![],
            super_trait: None,
            registry_getter: None,
            register_fn: Some(format!("register{}", trait_name)),
            unregister_fn: None,
            clear_fn: None,
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    #[test]
    fn test_trait_bridge_protocol_generated() {
        let trait_def = make_trait_def("TextBackend");
        let bridge_cfg = make_bridge_cfg("TextBackend");
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let exclude_types = HashSet::new();
        let files = gen_trait_bridge_files(&bridges, &exclude_types);

        // Should emit SwiftPluginBridge.swift first, then SwiftTextBackendBridge.swift
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "SwiftPluginBridge.swift");
        assert!(files[0].1.contains("protocol SwiftPluginBridge: AnyObject"));
        assert_eq!(files[1].0, "SwiftTextBackendBridge.swift");
        assert!(
            files[1]
                .1
                .contains("protocol SwiftTextBackendBridge: SwiftPluginBridge")
        );
    }

    #[test]
    fn test_trait_bridge_excludes_swift_language() {
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend");
        bridge_cfg.exclude_languages = vec!["swift".to_string()];
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let exclude_types = HashSet::new();
        let files = gen_trait_bridge_files(&bridges, &exclude_types);

        assert!(files.is_empty());
    }

    #[test]
    fn test_trait_bridge_skips_non_function_param() {
        let trait_def = make_trait_def("TextBackend");
        let mut bridge_cfg = make_bridge_cfg("TextBackend");
        bridge_cfg.bind_via = BridgeBinding::OptionsField;
        let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
        let exclude_types = HashSet::new();
        let files = gen_trait_bridge_files(&bridges, &exclude_types);

        assert!(files.is_empty());
    }

    #[test]
    fn test_swift_type_mapping() {
        use crate::core::ir::PrimitiveType;
        let exclude_types = HashSet::new();
        assert_eq!(swift_type_name(&TypeRef::String, &exclude_types), "String");
        assert_eq!(swift_type_name(&TypeRef::Bytes, &exclude_types), "Data");
        assert_eq!(swift_type_name(&TypeRef::Unit, &exclude_types), "Void");
        assert_eq!(
            swift_type_name(&TypeRef::Primitive(PrimitiveType::I32), &exclude_types),
            "Int32"
        );
        assert_eq!(swift_type_name(&TypeRef::Duration, &exclude_types), "TimeInterval");
    }

    #[test]
    fn test_swift_marshals_excluded_types_as_json() {
        let mut exclude_types = HashSet::new();
        exclude_types.insert("PrivatePayload".to_string());
        assert_eq!(
            swift_type_name(&TypeRef::Named("PrivatePayload".to_string()), &exclude_types),
            "String",
            "Excluded types should be marshalled as JSON strings"
        );
        assert_eq!(
            swift_type_name(&TypeRef::Named("VisibleResult".to_string()), &exclude_types),
            "VisibleResult",
            "Non-excluded types should keep their original names"
        );
    }

    #[test]
    fn test_bridge_policy_derives_named_types_from_trait_methods() {
        use crate::core::ir::{MethodDef, ParamDef};

        let mut trait_def = make_trait_def("Processor");
        trait_def.methods.push(MethodDef {
            name: "process".to_string(),
            params: vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("PrivatePayload".to_string()),
                optional: false,
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
            }],
            return_type: TypeRef::Named("VisibleResult".to_string()),
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        let policy = excluded_named_type_bridge_policy(&trait_def, &HashSet::new());

        assert!(policy.contains("PrivatePayload"));
        assert!(policy.contains("VisibleResult"));
        assert!(!policy.contains("UnmentionedResult"));
        assert!(!policy.contains("UnmentionedPayload"));
    }

    /// Verify that gen_single_trait_bridge_file does NOT emit a `public func register*`
    /// function. Registration is handled by `emit_trait_bridge_forwarders` in `<Binding>.swift`
    /// which uses the Box-based type (`SwiftDocumentExtractorBox`) that the RustBridge
    /// `register_*` entry point actually expects. A duplicate here would pass the
    /// incompatible `Adapter` type, causing a compile error.
    #[test]
    fn test_no_register_fn_in_trait_bridge_file() {
        let trait_def = make_trait_def("DocumentExtractor");
        let bridge_cfg = make_bridge_cfg("DocumentExtractor");
        let bridges = vec![("DocumentExtractor".to_string(), &bridge_cfg, &trait_def)];
        let exclude_types = HashSet::new();
        let files = gen_trait_bridge_files(&bridges, &exclude_types);

        // Should emit SwiftPluginBridge.swift first, then SwiftDocumentExtractorBridge.swift
        assert_eq!(files.len(), 2);
        let content = &files[1].1;

        // The protocol and adapter must still be emitted.
        assert!(
            content.contains("protocol SwiftDocumentExtractorBridge: SwiftPluginBridge"),
            "protocol must be emitted with SwiftPluginBridge inheritance, got:\n{content}"
        );
        assert!(
            content.contains("final class SwiftDocumentExtractorAdapter"),
            "adapter class must be emitted, got:\n{content}"
        );

        // A register function must NOT be emitted — it is emitted by emit_trait_bridge_forwarders
        // in <Binding>.swift and uses SwiftDocumentExtractorBox (not Adapter).
        assert!(
            !content.contains("public func registerDocumentExtractor("),
            "register function must NOT be emitted in the bridge file (would use wrong Adapter type), got:\n{content}"
        );
    }

    /// Verify that when an excluded type appears in a trait method
    /// signature, the protocol accepts the native type but the adapter marshals it as JSON String.
    #[test]
    fn test_excluded_type_in_method_becomes_string() {
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind};

        let mut trait_def = make_trait_def("DocumentExtractor");
        // Add a method with an excluded named type as return type.
        trait_def.methods.push(MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![ParamDef {
                name: "content".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
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
            }],
            return_type: TypeRef::Named("PrivatePayload".to_string()),
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        let bridge_cfg = make_bridge_cfg("DocumentExtractor");
        let bridges = vec![("DocumentExtractor".to_string(), &bridge_cfg, &trait_def)];

        // Pass PrivatePayload in exclude_types (mimicking what mod.rs does when
        // it augments from IR binding_excluded types).
        let mut exclude_types = HashSet::new();
        exclude_types.insert("PrivatePayload".to_string());

        let files = gen_trait_bridge_files(&bridges, &exclude_types);
        // Should emit SwiftPluginBridge.swift first, then SwiftDocumentExtractorBridge.swift
        assert_eq!(files.len(), 2);
        let content = &files[1].1;

        // Protocol method must return String (JSON) for excluded type PrivatePayload
        assert!(
            content.contains("func extractBytes(content: Data) throws -> String"),
            "protocol method must marshal excluded type to String, got:\n{content}"
        );

        // Adapter method must return String (JSON marshalling)
        assert!(
            content.contains("func extractBytesCall(content: Data) throws -> String"),
            "adapter method must marshal to String, got:\n{content}"
        );

        // Marshalling helper must be present
        assert!(
            content.contains("marshal_encode_excluded"),
            "marshal_encode_excluded helper must be present, got:\n{content}"
        );
    }

    #[test]
    fn test_bridge_registration_overloads_file() {
        let trait_def = make_trait_def("MyLib");
        let bridge_cfg = make_bridge_cfg("MyLib");
        let bridges = vec![("MyLib".to_string(), &bridge_cfg, &trait_def)];

        let result = gen_bridge_registration_overloads_file(&bridges);
        assert!(result.is_some(), "should generate file");

        let (filename, content) = result.unwrap();
        assert_eq!(filename, "BridgeRegistrationOverloads.swift");

        // Check for expected sections
        assert!(
            content.contains("// MARK: - Unregister name: label overloads"),
            "missing unregister overload section"
        );
        assert!(
            content.contains("public func unregisterMyLib(name: String) throws"),
            "missing unregister overload"
        );
        assert!(
            content.contains("try RustBridge.unregisterMyLib(name)"),
            "unregister label overload must delegate to the RustBridge function, not itself"
        );
        assert!(
            !content.contains("try unregisterMyLib(name)\n"),
            "unregister label overload must not recursively call itself"
        );

        // Check for register overload
        assert!(
            content.contains("// MARK: - Bridge → Box register overloads"),
            "missing register overload section"
        );
        assert!(
            content.contains("public func registerMyLib(_ bridge: any SwiftMyLibBridge) throws"),
            "missing register overload"
        );

        // NOTE: adapter class, lifecycle stub methods (`name()`, `version()`,
        // `initialize()`, `shutdown()`), and `_BridgeStubError` emission were
        // removed in commit `23a58ff9e` ("drop async from trait bridge"). Plugins
        // are now hand-authored in `Plugins.swift` rather than emitted into
        // `BridgeRegistrationOverloads.swift`, so the corresponding assertions
        // were retired alongside the codegen.
    }

    #[test]
    fn test_bridge_registration_overloads_empty_when_no_bridges() {
        let bridges: Vec<(String, &TraitBridgeConfig, &TypeDef)> = vec![];
        let result = gen_bridge_registration_overloads_file(&bridges);
        assert!(result.is_none(), "should not generate file when no bridges");
    }

    #[test]
    fn test_bridge_registration_overloads_skips_excluded_language() {
        let trait_def = make_trait_def("MyLib");
        let mut bridge_cfg = make_bridge_cfg("MyLib");
        bridge_cfg.exclude_languages = vec!["swift".to_string()];
        let bridges = vec![("MyLib".to_string(), &bridge_cfg, &trait_def)];

        let result = gen_bridge_registration_overloads_file(&bridges);
        assert!(result.is_none(), "should skip bridges excluded from swift");
    }

    // NOTE: previously asserted that async trait methods produced async stubs in
    // `BridgeRegistrationOverloads.swift`. That stub generation was intentionally
    // removed in commit `23a58ff9e` ("drop async from trait bridge"), so the
    // assertion is no longer applicable. The test was retired alongside the feature.

    #[test]
    fn test_pascal_case_conversion() {
        assert_eq!(trait_bridge_pascal_name("my_lib"), "MyLib");
        assert_eq!(trait_bridge_pascal_name("text_backend"), "TextBackend");
        assert_eq!(trait_bridge_pascal_name("test"), "Test");
        assert_eq!(trait_bridge_pascal_name("a"), "A");
    }

    /// Regression test for B3: String return conversion in trait adapter success body.
    /// When a method returns String, the RustString FFI result must be wrapped in String(...).
    /// When a method returns Vec<String>, each element must be converted via .map { String($0) }.
    #[test]
    fn test_b3_string_return_conversion_trait_adapter() {
        use crate::core::ir::{MethodDef, ReceiverKind};

        let mut trait_def = make_trait_def("TextExtractor");

        // Method returning String
        trait_def.methods.push(MethodDef {
            name: "extract_text".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        // Method returning Vec<String>
        trait_def.methods.push(MethodDef {
            name: "split_text".to_string(),
            params: vec![],
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        });

        let bridge_cfg = make_bridge_cfg("TextExtractor");
        let bridges = vec![("TextExtractor".to_string(), &bridge_cfg, &trait_def)];
        let exclude_types = HashSet::new();

        let files = gen_trait_bridge_files(&bridges, &exclude_types);
        assert_eq!(files.len(), 2);
        let content = &files[1].1;

        // String return must be wrapped: marshal_ok_result(String(result))
        assert!(
            content.contains("marshal_ok_result(String(result))"),
            "String return must be wrapped in String(...) converter, got:\n{content}"
        );

        // Vec<String> return must map: marshal_ok_result(result.map { String($0) })
        assert!(
            content.contains("marshal_ok_result(result.map { String($0) })"),
            "Vec<String> return must map each element, got:\n{content}"
        );
    }
}
