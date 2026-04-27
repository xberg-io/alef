//! Emits the Rust-side trait bridge wrapper and trampolines.
//!
//! Each configured `TraitBridgeConfig` entry gets:
//!   - an `extern "Rust"` block with `{Trait}Box` + one free trampoline fn per method
//!   - a `pub struct {Trait}Box(pub Box<dyn Trait + Send + Sync>)` definition
//!   - one `pub fn {trait_snake}_call_{method}(this: &{Trait}Box, …)` trampoline per method

use crate::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

/// Emit the `extern "Rust"` block for a trait bridge.
///
/// Declares an opaque `{Trait}Box` type plus one free trampoline function per method:
/// `fn {trait_snake}_call_{method}(this: &{Trait}Box, args…) -> ret`.
/// All parameter/return types are flattened to swift-bridge-safe types (primitives,
/// String, Vec<leaf>). Complex types (Named, Optional, Map, Vec<non-leaf>) are JSON-bridged.
pub(crate) fn emit_extern_block_for_trait_bridge(trait_def: &TypeDef) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&format!("        type {}Box;\n", trait_def.name));

    let trait_snake = heck::AsSnakeCase(trait_def.name.as_str()).to_string();
    for method in &trait_def.methods {
        let method_name = method.name.to_snake_case();
        let fn_name = format!("{trait_snake}_call_{method_name}");

        let mut params = vec!["this: &".to_string() + &format!("{}Box", trait_def.name)];
        for p in &method.params {
            let bridge_ty = bridge_type_for_trait_method(&p.ty);
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {bridge_ty}"));
        }

        let return_ty = if method.error_type.is_some() {
            let ok_ty = bridge_type_for_trait_method(&method.return_type);
            if matches!(method.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type_for_trait_method(&method.return_type)
        };

        let params_str = params.join(", ");
        block.push_str(&format!("        fn {fn_name}({params_str}) -> {return_ty};\n"));
    }

    block.push_str("    }\n\n");
    block
}

/// Emit the Rust wrapper struct and trampoline functions for a trait bridge.
///
/// Emits:
/// - `pub struct {Trait}Box(pub Box<dyn source_crate::path::Trait + Send + Sync>);`
/// - For each method: a `pub fn {trait_snake}_call_{method}(this: &{Trait}Box, …) -> ret`
///   that delegates to `this.0.{method}(…)`.
/// - Async methods block on a current-thread Tokio runtime (same as async function shims).
pub(crate) fn emit_trait_bridge_wrapper<'a>(
    trait_def: &TypeDef,
    source_crate: &str,
    enum_names: &HashSet<&'a str>,
) -> String {
    let mut out = String::new();
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();

    // Derive the fully-qualified dyn trait path from rust_path (e.g. kreuzberg::plugins::OcrBackend).
    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    out.push_str(&format!(
        "pub struct {trait_name}Box(pub Box<dyn {trait_path} + Send + Sync>);\n\n"
    ));

    for method in &trait_def.methods {
        let method_name = method.name.to_snake_case();
        let fn_name = format!("{trait_snake}_call_{method_name}");

        // Build parameter list for the trampoline signature.
        // When a parameter needs to be passed as &mut to the trait, declare it `mut`
        // in the function signature so we can borrow mutably from the local binding.
        let mut sig_params = vec![format!("this: &{trait_name}Box")];
        for p in &method.params {
            let bridge_ty = bridge_type_for_trait_method(&p.ty);
            let name = p.name.to_snake_case();
            // Declare `mut` when the trait method takes `&mut` (is_mut=true on a Named type).
            let needs_mut = p.is_mut && matches!(p.ty, TypeRef::Named(_));
            if needs_mut {
                sig_params.push(format!("mut {name}: {bridge_ty}"));
            } else {
                sig_params.push(format!("{name}: {bridge_ty}"));
            }
        }
        let sig_params_str = sig_params.join(", ");

        let return_ty = if method.error_type.is_some() {
            let ok_ty = bridge_type_for_trait_method(&method.return_type);
            if matches!(method.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type_for_trait_method(&method.return_type)
        };

        // Build the call arguments — convert bridge types back to what the trait expects.
        let call_args: Vec<String> = method.params.iter().map(|p| trait_call_arg(p)).collect();
        let call_args_str = call_args.join(", ");
        let source_call = format!("this.0.{method_name}({call_args_str})");

        let body = emit_trait_method_body(method, &source_call, &return_ty, enum_names);

        out.push_str(&format!(
            "pub fn {fn_name}({sig_params_str}) -> {return_ty} {{\n{body}}}\n\n"
        ));
    }

    out
}

/// Bridge type for trait method parameters/return types.
/// All Named types, Optional types, Vec<non-leaf>, and Map types are JSON-bridged (String).
/// This matches `bridge_type` but applied to trait method contexts.
fn bridge_type_for_trait_method(ty: &TypeRef) -> String {
    bridge_type(ty)
}

/// Build the call-site argument expression for a trait method parameter.
/// JSON-bridged params are deserialized; Path params are converted to PathBuf/Path;
/// Named types are passed through directly (they are not bridge wrappers in trait context).
pub(crate) fn trait_call_arg(p: &alef_core::ir::ParamDef) -> String {
    let name = p.name.to_snake_case();

    // JSON-bridged types: deserialize from the bridged String.
    if needs_json_bridge(&p.ty) {
        let native_ty = crate::gen_rust_crate::type_bridge::swift_bridge_rust_type(&p.ty);
        let deser = format!("serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
        if p.is_ref {
            return format!("&{deser}");
        }
        return deser;
    }

    // Path: bridged as String; convert to PathBuf.
    if matches!(p.ty, TypeRef::Path) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(std::path::Path::new)");
            }
            return format!("{name}.map(std::path::PathBuf::from)");
        }
        if p.is_ref {
            return format!("std::path::Path::new(&{name})");
        }
        return format!("std::path::PathBuf::from({name})");
    }

    // Named types in trait bridges are swift-bridge wrapper newtypes (e.g. `OcrConfig` wrapper
    // which holds `pub kreuzberg::OcrConfig`). The trait method expects the inner type (possibly
    // behind a reference). Extract `.0` and apply the appropriate reference.
    if matches!(p.ty, TypeRef::Named(_)) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(|w| &w.0)");
            }
            return format!("{name}.map(|w| w.0)");
        }
        if p.is_mut {
            return format!("&mut {name}.0");
        }
        if p.is_ref {
            return format!("&{name}.0");
        }
        return format!("{name}.0");
    }

    // Primitives and String.
    if p.is_ref {
        match &p.ty {
            TypeRef::Bytes | TypeRef::String | TypeRef::Char => return format!("&{name}"),
            TypeRef::Vec(_) if p.optional => return format!("{name}.as_deref()"),
            _ => return format!("&{name}"),
        }
    }
    name
}

/// Emit the body of a trait method trampoline, handling sync vs async and error types.
pub(crate) fn emit_trait_method_body(
    method: &MethodDef,
    source_call: &str,
    _return_ty: &str,
    enum_names: &HashSet<&str>,
) -> String {
    // Wrap the return value for methods that return Named types (bridged as JSON or swift-bridge
    // newtype wrappers). JSON-bridged types use serde_json::to_string. Named types not
    // JSON-bridged (i.e. plain Named leaf types) need to be wrapped in the bridge newtype.
    // Enum wrappers use `::from(val)`; struct newtypes use `T(val)`.
    let wrap_return = |expr: String| -> String {
        if needs_json_bridge(&method.return_type) {
            format!("serde_json::to_string(&({expr})).expect(\"serializable return\")")
        } else {
            match &method.return_type {
                TypeRef::String | TypeRef::Path => format!("{expr}.to_string()"),
                // Named leaf types are represented as bridge wrapper newtypes — wrap.
                TypeRef::Named(name) => {
                    if enum_names.contains(name.as_str()) {
                        format!("{name}::from({expr})")
                    } else {
                        format!("{name}({expr})")
                    }
                }
                _ => expr,
            }
        }
    };

    // Build the error+value mapping expression for a Result-returning method.
    // Handles JSON-bridged, String/Path, Named newtypes/enums, and plain cases.
    let map_result_expr = |base: String| -> String {
        if needs_json_bridge(&method.return_type) && !matches!(method.return_type, TypeRef::Unit) {
            format!("{base}.map(|v| serde_json::to_string(&v).expect(\"serializable return\")).map_err(|e| e.to_string())")
        } else if matches!(method.return_type, TypeRef::String | TypeRef::Path) {
            format!("{base}.map(|s| s.to_string()).map_err(|e| e.to_string())")
        } else if let TypeRef::Named(wrapper) = &method.return_type {
            // Result<kreuzberg::T, E> → Result<BridgeWrapper, String>
            let w = wrapper.clone();
            if enum_names.contains(w.as_str()) {
                format!("{base}.map(|v| {w}::from(v)).map_err(|e| e.to_string())")
            } else {
                format!("{base}.map(|v| {w}(v)).map_err(|e| e.to_string())")
            }
        } else {
            format!("{base}.map_err(|e| e.to_string())")
        }
    };

    if method.is_async {
        let await_expr = format!("{source_call}.await");
        if method.error_type.is_some() {
            let mapped = map_result_expr(await_expr);
            format!("    ::tokio::runtime::Builder::new_current_thread()\n        .enable_all()\n        .build()\n        .expect(\"build tokio runtime\")\n        .block_on(async {{ {mapped} }})\n")
        } else {
            let inner = wrap_return(await_expr);
            format!("    ::tokio::runtime::Builder::new_current_thread()\n        .enable_all()\n        .build()\n        .expect(\"build tokio runtime\")\n        .block_on(async {{ {inner} }})\n")
        }
    } else if method.error_type.is_some() {
        let mapped = map_result_expr(source_call.to_string());
        format!("    {mapped}\n")
    } else {
        let wrapped = wrap_return(source_call.to_string());
        format!("    {wrapped}\n")
    }
}
