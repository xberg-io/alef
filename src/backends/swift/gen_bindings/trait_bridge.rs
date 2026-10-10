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
/// `first_class_types` is the set of types that are emitted as first-class structs (and thus
/// only available in the primary binding module, not accessible from RustBridge trait bridge code).
/// Both sets of types are marshalled as JSON strings at the trait boundary, as is every other
/// `Named` type the trait mentions -- see [`excluded_named_type_bridge_policy`].
///
/// Returns a list of (filename, content) tuples ready for emission.
pub fn gen_trait_bridge_files(
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
    exclude_types: &HashSet<String>,
    first_class_types: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut files = Vec::new();

    let function_param_bridges: Vec<_> = bridges
        .iter()
        .filter(|(_, bridge_cfg, _)| {
            !bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift")
                && matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam)
        })
        .collect();

    if !function_param_bridges.is_empty() {
        let content = emit_swift_plugin_bridge_protocol();
        files.push(("SwiftPluginBridge.swift".to_string(), content));
    }

    for (trait_name, bridge_cfg, trait_def) in bridges {
        if bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift") {
            continue;
        }

        if !matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam) {
            continue;
        }

        let mut combined_exclude = exclude_types.clone();
        for first_class in first_class_types {
            combined_exclude.insert(first_class.clone());
        }

        let content = gen_single_trait_bridge_file(trait_name, bridge_cfg, trait_def, &combined_exclude);
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
    crate::backends::swift::template_env::render("swift_plugin_bridge_protocol.swift.jinja", crate::alef_context! {})
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
///
/// **Every** `Named` type named anywhere in the trait's method signatures belongs in this set,
/// with no exemption for methods that carry a Rust-side default body. The bridge protocol and the
/// `Swift{Trait}Box` shims are emitted into `Sources/RustBridge/`, and the package's target graph
/// runs `<Module> -> RustBridge`, so nothing in `RustBridge` can name a public DTO without
/// inverting that dependency. `gen_rust_crate::plugin_inbound` states the same contract from the
/// Rust side: a `Named` value crosses the inbound boundary as a JSON `String`.
///
/// A `has_default_impl` exemption here was the root cause of alef #258. It let a defaulted
/// method's `Named` types keep their real Swift names in the emitted signature, which does not
/// resolve in `RustBridge`, and then invited two rounds of downstream patching -- an invented
/// enum case and a `JSONEncoder` call -- that each needed the type to be nameable to work. ~keep
pub fn excluded_named_type_bridge_policy(trait_def: &TypeDef, excluded_types: &HashSet<String>) -> HashSet<String> {
    let mut policy = excluded_types.clone();
    for method in &trait_def.methods {
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
///
/// No default-method bodies are emitted. A Rust trait's default body is not carried in the IR, so
/// any Swift stub alef could write here would be a guess -- and the inbound Rust wrapper calls the
/// Swift shim for defaulted methods too, so that guess would *replace* the Rust default at
/// runtime rather than shadow an unused stub. Declaring the method and letting conformance fail
/// is the loud failure; a stub that compiles and returns the wrong value is the silent one. ~keep
fn gen_single_trait_bridge_file(
    trait_name: &str,
    _bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    exclude_types: &HashSet<String>,
) -> String {
    let bridge_exclude_types = excluded_named_type_bridge_policy(trait_def, exclude_types);

    let protocol = bridge_protocol_name(trait_name);
    let mut protocol_methods = String::new();

    for method in &trait_def.methods {
        let method_camel = method.name.to_lower_camel_case();
        let params_sig = swift_method_params(&method.params, &bridge_exclude_types);
        let return_type = swift_return_type(&method.return_type, &bridge_exclude_types);
        let throws = if method.error_type.is_some() { " throws" } else { "" };
        // NOTE: async is removed — Swift{Trait}Bridge is now fully sync to match plugin protocol shape

        protocol_methods.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_protocol_method.swift.jinja",
            crate::alef_context! {
                method_name => method_camel,
                params => params_sig,
                throws_clause => throws,
                return_type => return_type,
                has_rust_default => method.has_default_impl,
            },
        ));
    }

    let mut adapter_methods = String::new();
    for method in &trait_def.methods {
        let method_camel = method.name.to_lower_camel_case();
        let params_sig = swift_method_params(&method.params, &bridge_exclude_types);
        let return_type = if method.error_type.is_some() {
            "String".to_string()
        } else {
            swift_return_type(&method.return_type, &bridge_exclude_types)
        };

        // NOTE: async is removed from adapter signatures since Swift{Trait}Bridge is now sync
        let throws_kw = if method.error_type.is_some() { " throws" } else { "" };

        let call_args = build_adapter_call_args(method);
        let call_args_str = call_args.join(", ");
        let method_body = if method.error_type.is_some() {
            // NOTE: async is removed, so only 'try' is used (not 'try await')
            let success_body = trait_adapter_success_body(&method.return_type, &bridge_exclude_types);
            crate::backends::swift::template_env::render(
                "swift_trait_adapter_error_body.swift.jinja",
                crate::alef_context! {
                    method_name => &method_camel,
                    call_args => &call_args_str,
                    success_body => success_body,
                    binds_result => adapter_binds_result(&method.return_type),
                },
            )
        } else {
            // NOTE: async is removed, all methods are now sync
            crate::backends::swift::template_env::render(
                "swift_trait_adapter_direct_body.swift.jinja",
                crate::alef_context! {
                    method_name => &method_camel,
                    call_args => &call_args_str,
                    binds_result => adapter_binds_result(&method.return_type),
                },
            )
        };

        adapter_methods.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_adapter_method.swift.jinja",
            crate::alef_context! {
                method_name => method_camel,
                params => params_sig,
                throws_clause => throws_kw,
                return_type => return_type,
                body => method_body,
            },
        ));
    }

    // NOTE: Registration functions are NOT emitted here. They live in <Binding>.swift

    crate::backends::swift::template_env::render(
        "swift_trait_bridge_file.swift.jinja",
        crate::alef_context! {
            trait_name => trait_name,
            adapter_class => format!("Swift{trait_name}Adapter"),
            protocol => protocol,
            protocol_methods => protocol_methods,
            adapter_methods => adapter_methods,
        },
    )
}

/// Whether the adapter body binds the bridged call to `result`.
///
/// A `Void`-returning bridge method must be called as a bare statement: binding it declares
/// `result` as `()`, which Swift reports twice -- an unexpected `Void` inference and an unused
/// variable -- and a warnings-as-errors build rejects. This is the same question
/// [`trait_adapter_success_body`] answers when it marshals `Empty()` for `TypeRef::Unit` without
/// ever reading `result`, so both sites must keep deriving it from this one predicate. ~keep
fn adapter_binds_result(return_type: &TypeRef) -> bool {
    !matches!(return_type, TypeRef::Unit)
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
            crate::alef_context! {
                expression => expression,
            },
        )
    } else {
        crate::backends::swift::template_env::render(
            "swift_trait_adapter_excluded_success.swift.jinja",
            crate::alef_context! {},
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
            let ty = if p.optional { format!("{ty}?") } else { ty };
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
            let ty = if p.optional { format!("{ty}?") } else { ty };
            format!("{}: {}", name, ty)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Get the Swift type name for a TypeRef.
///
/// Protocol methods use native types (excluded types as native structs).
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
        TypeRef::Named(name) => name.clone(),
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
///
/// `Map` always declares plain `String` -- one JSON blob -- regardless of its key/value types.
/// This is the same "container has no native swift-bridge representation, so the whole value is
/// one blob" rule `gen_rust_crate::plugin_inbound::inbound_bridge_type` applies on the Rust side
/// (see the `~keep` rule there for the full contract and the `Vec<Named>` per-element
/// counterpart). Recursing through `Map` here previously declared `[String: String]` for a
/// `Map<_, Named>`, whose values are themselves JSON-encoded -- double-encoding the payload and
/// disagreeing with `plugin_marshal::swift_shim_param_ffi_type`/`swift_shim_param_decode`, which
/// already always treat `Map` as one `RustString` blob at the FFI-shim layer (alef-tasks #309).
///
/// `Optional<Vec<Named>>` (Named excluded, i.e. JSON-bridged) declares plain `String?` -- one
/// JSON blob, `nil` for `None` -- rather than recursing to `[String]?` (alef-tasks #333). This is
/// the OUTBOUND counterpart of `gen_rust_crate::plugin_inbound::inbound_bridge_type`'s `~keep`
/// rule, and it reaches the opposite conclusion from the inbound side on purpose: the inbound
/// (`extern "Swift"`) boundary can extend `Vec<Named>`'s per-element exception through `Option`
/// because that boundary already accepts a plain `Option<Vec<String>>`, but this outbound
/// direction backs the `{Trait}Box` FFI trampolines declared via `extern "Rust"`
/// (`gen_rust_crate::trait_bridge::emit_extern_block_for_trait_bridge`), and that boundary does
/// not support `Option<Vec<T>>` for any element type -- proven by the DTO-getter fix "bridge
/// optional vectors through JSON", which collapsed the identical `Optional<Vec<T>>` shape to one
/// JSON blob for the same `extern "Rust"` limitation. Recursing here would declare `[String]?`
/// on the protocol while `plugin_marshal::swift_shim_return_ffi_type` (which cannot produce an
/// `Option`-wrapped `RustVec` FFI type either) still declares `RustString`, and the shim's
/// `return RustString(...)` would not compile against a `[String]?` bridge-call expression.
/// ~keep
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
            if exclude_types.contains(name) {
                "String".to_string()
            } else {
                name.clone()
            }
        }
        TypeRef::Vec(inner) => format!("[{}]", swift_type_name(inner, exclude_types)),
        TypeRef::Map(_, _) => "String".to_string(),
        TypeRef::Optional(_)
            if crate::backends::swift::gen_rust_crate::trait_bridge::trait_type_is_single_json_blob(ty, |name| {
                exclude_types.contains(name)
            }) =>
        {
            "String?".to_string()
        }
        TypeRef::Optional(inner) => format!("{}?", swift_type_name(inner, exclude_types)),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "TimeInterval".to_string(),
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
/// - `public typealias` re-exports of every bridge protocol declared in `Sources/RustBridge/`
/// - Unregister `name:` label overloads for all bridge-bound traits
/// - Register overloads that accept bridge protocols and wrap them in adapters
/// - Stub adapter classes that implement full trait protocols with sensible defaults
///
/// The protocol produced by [`gen_trait_bridge_files`] lives in the `RustBridge` target, and the
/// package's target graph runs `<Module> -> RustBridge`, so a type that conforms to it (`class
/// MyBackend: Swift{Trait}Bridge`) cannot see the protocol name from a file that only
/// `import <Module>`s -- unlike a *call* to a forwarder function, which resolves through the
/// module boundary fine. The register-overload functions emitted below already reference the
/// protocol by name (`any Swift{Trait}Bridge`) and already sit in this file's `import RustBridge`
/// scope, so this file is where the protocol needs re-exporting too. Reusing the same
/// `public typealias {Name} = RustBridge.{Name}` shape the main module file already uses for
/// every opaque handle type (`gen_bindings::mod::opaque_handle_aliases`) keeps one re-export
/// idiom in this backend instead of introducing a blanket `@_exported import`. ~keep
///
/// Returns `(filename, content)` ready for emission.
pub fn gen_bridge_registration_overloads_file(
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
) -> Option<(String, String)> {
    let trait_bridges: Vec<_> = bridges
        .iter()
        .filter(|(_, bridge_cfg, _)| {
            !bridge_cfg.exclude_languages.iter().any(|lang| lang == "swift")
                && matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::FunctionParam)
        })
        .collect();

    if trait_bridges.is_empty() {
        return None;
    }

    let mut protocol_aliases = String::new();
    for (trait_name, _, _) in &trait_bridges {
        let protocol = bridge_protocol_name(trait_name);
        protocol_aliases.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            crate::alef_context! { name => &protocol },
        ));
    }

    let mut unregister_overloads = String::new();
    for (trait_name, _, _) in &trait_bridges {
        let pascal_name = trait_bridge_pascal_name(trait_name);
        unregister_overloads.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_unregister_overload.swift.jinja",
            crate::alef_context! {
                pascal_name => &pascal_name,
            },
        ));
    }

    let mut register_overloads = String::new();
    for (trait_name, _, _) in &trait_bridges {
        let pascal_name = trait_bridge_pascal_name(trait_name);
        register_overloads.push_str(&crate::backends::swift::template_env::render(
            "swift_trait_register_overload.swift.jinja",
            crate::alef_context! {
                pascal_name => &pascal_name,
            },
        ));
    }

    let content = crate::backends::swift::template_env::render(
        "swift_trait_bridge_overloads.swift.jinja",
        crate::alef_context! {
            protocol_aliases => protocol_aliases,
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
mod tests;

#[cfg(test)]
mod void_binding_tests;
