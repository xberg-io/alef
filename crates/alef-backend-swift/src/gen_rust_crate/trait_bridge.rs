//! Emits the Rust-side trait bridge wrapper and trampolines.
//!
//! Each configured `TraitBridgeConfig` entry gets:
//!   - an `extern "Rust"` block with `{Trait}Box` + one free trampoline fn per method
//!   - a `pub struct {Trait}Box(pub Box<dyn Trait + Send + Sync>)` definition
//!   - one `pub fn {trait_snake}_call_{method}(this: &{Trait}Box, …)` trampoline per method
//!
//! [`SwiftBridgeGenerator`] implements [`alef_codegen::generators::trait_bridge::TraitBridgeGenerator`]
//! for the inbound plugin registration pattern (Swift implements a Rust trait). The
//! `gen_unregistration_fn` and `gen_clear_fn` overrides emit swift-bridge–visible `pub fn`
//! wrappers that delegate to the host crate's `unregister_*` / `clear_*` registry entry
//! points.

use crate::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// SwiftBridgeGenerator — TraitBridgeGenerator impl for the Swift backend
// ---------------------------------------------------------------------------

/// Swift-specific trait bridge generator.
///
/// The Swift inbound plugin pattern (Swift class implements a Rust trait) is
/// primarily handled by [`super::plugin_inbound`], which emits `extern "Swift"`
/// shims, wrapper structs, and the `Plugin` / trait impls. This generator
/// provides the [`TraitBridgeGenerator`] contract so that `gen_unregistration_fn`
/// and `gen_clear_fn` can be called uniformly from the plugin inbound emitter.
///
/// `gen_registration_fn` returns an empty string because registration requires
/// a `Swift{Trait}Box` argument whose type is only available in the inbound
/// `extern "Rust"` context; that function is emitted by `plugin_inbound` directly.
pub struct SwiftBridgeGenerator;

impl TraitBridgeGenerator for SwiftBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        // Swift handles are opaque swift-bridge types; no single Rust type name.
        "swift_bridge::opaque"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![]
    }

    fn gen_sync_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        // Not used: swift-bridge trampolines are emitted by emit_trait_bridge_wrapper, not
        // via the shared TraitBridgeGenerator infrastructure.
        String::new()
    }

    fn gen_async_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_constructor(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Returns an empty string. Registration requires a `Swift{Trait}Box` argument
    /// whose type is only available inside the `extern "Rust"` block produced by
    /// `plugin_inbound::emit_extern_block_for_inbound_registration`; that function
    /// emits the `register_*` entry point directly.
    fn gen_registration_fn(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Emit a `pub fn {name}(name: String) -> Result<(), String>` that unregisters
    /// a previously-registered plugin by name.
    ///
    /// The function calls into the configured registry directly — consistent with
    /// how `register_*` calls the registry in `plugin_inbound::emit_inbound_wrapper`.
    ///
    /// Returns an empty string when `spec.bridge_config.unregister_fn` is `None`
    /// or when `spec.bridge_config.registry_getter` is not set (no registry to
    /// call into).
    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let trait_name = &spec.trait_def.name;
        format!(
            "/// Unregister a previously-registered `{trait_name}` plugin by name.\n\
             pub fn {unregister_fn}(name: String) -> Result<(), String> {{\n\
             \x20\x20\x20\x20let registry = {registry_getter}();\n\
             \x20\x20\x20\x20let mut guard = registry.write();\n\
             \x20\x20\x20\x20guard.remove(&name).map_err(|e| e.to_string())\n\
             }}\n"
        )
    }

    /// Emit a `pub fn {name}() -> Result<(), String>` that clears all registered
    /// plugins of this type. Typically used in test teardown.
    ///
    /// The function calls into the configured registry directly — consistent with
    /// how `register_*` and `unregister_*` call the registry.
    ///
    /// Returns an empty string when `spec.bridge_config.clear_fn` is `None`
    /// or when `spec.bridge_config.registry_getter` is not set.
    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let trait_name = &spec.trait_def.name;
        format!(
            "/// Clear all registered `{trait_name}` plugins.\n\
             pub fn {clear_fn}() -> Result<(), String> {{\n\
             \x20\x20\x20\x20let registry = {registry_getter}();\n\
             \x20\x20\x20\x20let mut guard = registry.write();\n\
             \x20\x20\x20\x20guard.clear().map_err(|e| e.to_string())\n\
             }}\n"
        )
    }
}

/// Emit the `extern "Rust"` block for a trait bridge.
///
/// Declares an opaque `{Trait}Box` type plus one free trampoline function per method:
/// `fn {trait_snake}_call_{method}(this: &{Trait}Box, args…) -> ret`.
/// All parameter/return types are flattened to swift-bridge-safe types (primitives,
/// String, Vec<leaf>). Complex types (Named, Optional, Map, Vec<non-leaf>) are JSON-bridged.
pub(crate) fn emit_extern_block_for_trait_bridge(trait_def: &TypeDef) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::template_env::render(
        "trait_extern_type.jinja",
        minijinja::context! {
            trait_name => &trait_def.name,
        },
    ));

    let trait_snake = heck::AsSnakeCase(trait_def.name.as_str()).to_string();

    // Phantom `Vec<{Trait}Box>` reference: swift-bridge auto-generates Swift Vec
    // accessor methods for every opaque `type Foo;` declaration. Those Swift methods
    // reference C symbols `__swift_bridge__$Vec_FooBox$len` etc. which swift-bridge-build
    // only emits on the Rust side when the type appears in a `Vec<Foo>` somewhere
    // in an extern block. Without this phantom, the generated Swift fails to link.
    // Name does NOT use a leading underscore — Swift treats `_`-prefixed C names as
    // private and excludes them from the imported module scope.
    block.push_str(&crate::template_env::render(
        "trait_phantom_fn.jinja",
        minijinja::context! {
            trait_name => &trait_def.name,
            trait_snake => &trait_snake,
        },
    ));

    for method in &trait_def.methods {
        let method_name = method.name.to_snake_case();
        let fn_name = format!("{trait_snake}_call_{method_name}");

        let mut params = vec!["this: &".to_string() + &format!("{}Box", trait_def.name)];
        for p in &method.params {
            let bridge_ty = bridge_type_for_trait_method(&p.ty);
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {bridge_ty}"));
        }

        // swift-bridge 0.1.59 cannot parse `Result<T, E>` in `extern "Rust"` blocks.
        // Error-returning methods use a plain `String` return carrying a JSON envelope:
        // `{"ok": <value>}` on success or `{"err": "<message>"}` on failure.
        let return_ty = if method.error_type.is_some() {
            "String".to_string()
        } else {
            bridge_type_for_trait_method(&method.return_type)
        };

        let params_str = params.join(", ");
        block.push_str(&crate::template_env::render(
            "trait_method_fn.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &params_str,
                return_type => &return_ty,
            },
        ));
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
pub(crate) fn emit_trait_bridge_wrapper(trait_def: &TypeDef, source_crate: &str, enum_names: &HashSet<&str>) -> String {
    let mut out = String::new();
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();

    // Derive the fully-qualified dyn trait path from rust_path (e.g. kreuzberg::plugins::OcrBackend).
    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    out.push_str(&crate::template_env::render(
        "trait_struct.jinja",
        minijinja::context! {
            trait_name => trait_name,
            trait_path => &trait_path,
        },
    ));

    // Phantom Vec<{Trait}Box> implementation paired with the extern declaration —
    // never actually called, but its existence forces swift-bridge-build to emit
    // the `__swift_bridge__$Vec_{Trait}Box$*` C symbols that the auto-generated
    // Swift Vec extension references.
    out.push_str(&crate::template_env::render(
        "trait_phantom_impl.jinja",
        minijinja::context! {
            trait_name => trait_name,
            trait_snake => &trait_snake,
        },
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

        // swift-bridge 0.1.59 cannot parse `Result<T, E>` in `extern "Rust"` blocks.
        // Error-returning methods return plain `String` (JSON envelope `{"ok":...}` / `{"err":...}`).
        let return_ty = if method.error_type.is_some() {
            "String".to_string()
        } else {
            bridge_type_for_trait_method(&method.return_type)
        };

        // Build the call arguments — convert bridge types back to what the trait expects.
        let call_args: Vec<String> = method.params.iter().map(trait_call_arg).collect();
        let call_args_str = call_args.join(", ");
        let source_call = format!("this.0.{method_name}({call_args_str})");

        let body = emit_trait_method_body(method, &source_call, &return_ty, enum_names);

        out.push_str(&crate::template_env::render(
            "trait_method_impl.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &sig_params_str,
                return_type => &return_ty,
                body => &body,
            },
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

    // Build a JSON-envelope String for a Result-returning method.
    // swift-bridge 0.1.59 cannot parse `Result<T, E>` in `extern "Rust"` blocks, so we use
    // a plain `String` carrying `{"ok": <serialised-value>}` on success or
    // `{"err": "<message>"}` on failure. The Swift caller deserialises this envelope.
    let envelope_result_expr = |base: String| -> String {
        // Serialise the ok value to a JSON fragment.
        let ok_fragment = if matches!(method.return_type, TypeRef::Unit) {
            // () → "null"
            "\"null\"".to_string()
        } else if needs_json_bridge(&method.return_type) {
            // Complex type: already serialisable — produce the JSON string of the value.
            "serde_json::to_string(&v).expect(\"serializable return\")".to_string()
        } else {
            match &method.return_type {
                TypeRef::String | TypeRef::Path => "v.to_string()".to_string(),
                // Named leaf: convert to wrapper then serialise.
                TypeRef::Named(name) => {
                    let expr = if enum_names.contains(name.as_str()) {
                        format!("{name}::from(v)")
                    } else {
                        format!("{name}(v)")
                    };
                    format!("serde_json::to_string(&{expr}).expect(\"serializable return\")")
                }
                // Primitive: format! round-trip.
                _ => "v.to_string()".to_string(),
            }
        };

        format!(
            "match {base} {{\n\
             \x20\x20\x20\x20Ok(v) => format!(\"{{\\\"ok\\\": {{}}}}\", {ok_fragment}),\n\
             \x20\x20\x20\x20Err(e) => format!(\"{{\\\"err\\\": \\\"{{}}\\\"}}\" , e),\n\
             }}"
        )
    };

    if method.is_async {
        let await_expr = format!("{source_call}.await");
        if method.error_type.is_some() {
            let enveloped = envelope_result_expr(await_expr);
            format!(
                "    ::tokio::runtime::Builder::new_current_thread()\n        .enable_all()\n        .build()\n        .expect(\"build tokio runtime\")\n        .block_on(async {{ {enveloped} }})\n"
            )
        } else {
            let inner = wrap_return(await_expr);
            format!(
                "    ::tokio::runtime::Builder::new_current_thread()\n        .enable_all()\n        .build()\n        .expect(\"build tokio runtime\")\n        .block_on(async {{ {inner} }})\n"
            )
        }
    } else if method.error_type.is_some() {
        let enveloped = envelope_result_expr(source_call.to_string());
        format!("    {enveloped}\n")
    } else {
        let wrapped = wrap_return(source_call.to_string());
        format!("    {wrapped}\n")
    }
}
