//! Emits the **inbound** plugin trait bridge — Swift implements a Rust trait, Rust calls back.
//!
//! Whereas [`trait_bridge`](super::trait_bridge) generates **outbound** glue (Swift caller →
//! Rust trait object), this module generates the inverse: a Swift class conforms to a
//! protocol, Rust holds a handle, and Rust calls each method on the Swift instance via
//! `extern "Swift"` declarations.
//!
//! For each configured `TraitBridgeConfig` entry, this emits:
//!
//! 1. An `extern "Swift"` block declaring `Swift{Trait}Box` plus one FFI shim per method
//!    plus Plugin super-trait shims (`name`, `version`, `initialize`, `shutdown`, etc.).
//!    Complex types are JSON-serialised at the boundary; primitives, `String`, `Vec<u8>`,
//!    `Vec<leaf>` pass through directly.
//! 2. A `pub struct Swift{Trait}Wrapper` newtype holding the Swift handle plus a
//!    `OnceLock<String>` cache for `Plugin::name()` (the trait returns `&str`, so the
//!    owned name fetched from Swift must outlive the call). `unsafe impl Send + Sync`
//!    is justified by the kreuzberg `Plugin` super-trait's documented thread-safety
//!    requirement and ARC's safe shareability.
//! 3. `impl Plugin for Swift{Trait}Wrapper` forwarding `name`/`version`/`initialize`/
//!    `shutdown` to the Swift box; errors use the configured error constructor.
//! 4. `#[async_trait] impl {Trait} for Swift{Trait}Wrapper` forwarding each method,
//!    JSON-marshalling complex types, mapping `Result<_, String>` to the source-crate
//!    error type.
//! 5. A `register_*` free fn (when `register_fn` is configured) that constructs the
//!    wrapper, wraps it in `Arc`, and inserts it into the configured registry.
//!
//! ARC lifecycle: swift-bridge's `extern "Swift" type` declaration creates a Rust handle
//! that retains a reference to the Swift instance via `Unmanaged<T>.passRetained`. The
//! retained reference is released when the handle is dropped. Wrapping the handle in
//! `Arc` and storing it in a process-wide registry is therefore ARC-safe — the Swift
//! instance lives until the `Arc`'s last clone is dropped, at which point the swift-bridge
//! `Drop` impl releases the retained ARC reference.

use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator as _, TraitBridgeSpec};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, ParamDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

use crate::gen_rust_crate::trait_bridge::SwiftBridgeGenerator;
use crate::gen_rust_crate::type_bridge::{needs_json_bridge, swift_bridge_rust_type};

/// Inbound-specific type bridging.
///
/// All `Named` types are JSON-bridged at the inbound boundary because the Swift side of an
/// `extern "Swift"` shim cannot produce the opaque Rust newtype the way `extern "Rust"`
/// callers do; it has to send a JSON payload that Rust deserialises into the source type.
/// Primitive scalars, `String`, `Vec<u8>`, and `Vec<leaf>` pass through as-is.
fn inbound_bridge_type(ty: &TypeRef) -> String {
    if needs_inbound_json_bridge(ty) {
        return "String".to_string();
    }
    match ty {
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_bridge_type(inner)),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Like [`needs_json_bridge`] but additionally treats every `Named` type as JSON-bridged
/// for inbound transport. Vec<Named-leaf> stays a typed Vec (e.g. `Vec<String>`) when
/// the inner type is a primitive/leaf — only Named-leaf gets escalated.
fn needs_inbound_json_bridge(ty: &TypeRef) -> bool {
    if needs_json_bridge(ty) {
        return true;
    }
    matches!(ty, TypeRef::Named(_))
}

/// Emit the `extern "Rust"` block declaring the `register_*`/`unregister_*` Swift-callable
/// entry points. swift-bridge generates Swift glue that converts `Swift{Trait}Box` instances
/// to retained ARC pointers and forwards them into Rust.
pub(crate) fn emit_extern_block_for_inbound_registration(
    trait_def: &TypeDef,
    bridge_config: &TraitBridgeConfig,
) -> String {
    let trait_name = &trait_def.name;
    let box_name = format!("Swift{trait_name}Box");

    let mut block = String::new();
    let mut has_any = false;
    block.push_str("    extern \"Rust\" {\n");
    if let Some(register_fn) = bridge_config.register_fn.as_deref() {
        let camel = heck::AsLowerCamelCase(register_fn).to_string();
        block.push_str(&crate::template_env::render(
            "inbound_registration_fn.rs.jinja",
            minijinja::context! {
                camel => &camel,
                fn_name => register_fn,
                params => format!("swift_box: {box_name}"),
            },
        ));
        has_any = true;
    }
    if let Some(unregister_fn) = bridge_config.unregister_fn.as_deref() {
        let camel = heck::AsLowerCamelCase(unregister_fn).to_string();
        block.push_str(&crate::template_env::render(
            "inbound_registration_fn.rs.jinja",
            minijinja::context! {
                camel => &camel,
                fn_name => unregister_fn,
                params => "name: String",
            },
        ));
        has_any = true;
    }
    if let Some(clear_fn) = bridge_config.clear_fn.as_deref() {
        let camel = heck::AsLowerCamelCase(clear_fn).to_string();
        block.push_str(&crate::template_env::render(
            "inbound_registration_fn.rs.jinja",
            minijinja::context! {
                camel => &camel,
                fn_name => clear_fn,
                params => "",
            },
        ));
        has_any = true;
    }
    block.push_str("    }\n\n");
    if has_any { block } else { String::new() }
}

/// Returns true when the trait bridge config declares a Plugin super-trait.
fn has_plugin_super(bridge_config: &TraitBridgeConfig) -> bool {
    bridge_config
        .super_trait
        .as_deref()
        .map(|s| s == "Plugin" || s.ends_with("::Plugin"))
        .unwrap_or(false)
}

/// Emit the `extern "Swift"` block declaring `Swift{Trait}Box` and per-method FFI shims.
///
/// Each shim signature is the JSON-bridged form of the trait method: complex types become
/// `String` (JSON), primitives and `String`/`Vec<u8>` pass through directly. Methods that
/// can fail return `Result<RetBridge, String>` so the Swift side can surface errors.
pub(crate) fn emit_extern_block_for_inbound(trait_def: &TypeDef, bridge_config: &TraitBridgeConfig) -> String {
    let trait_name = &trait_def.name;
    let box_name = format!("Swift{trait_name}Box");
    let emit_plugin_shims = has_plugin_super(bridge_config);

    let mut block = String::new();
    block.push_str("    extern \"Swift\" {\n");
    block.push_str(&crate::template_env::render(
        "inbound_swift_type.rs.jinja",
        minijinja::context! {
            box_name => &box_name,
        },
    ));

    if emit_plugin_shims {
        // Plugin super-trait shims — only emitted when the trait has a Plugin super-trait.
        // We declare these as `&self` methods so swift-bridge treats them as instance methods
        // on `Swift{Trait}Box` and emits the proper `Unmanaged<T>.fromOpaque(this).takeUnretainedValue()`
        // dispatch on the Swift side. Free-fn declarations (with `this: &Box` as a regular param)
        // would force swift-bridge to FFI-encode the box as a value, which breaks for opaque
        // Swift handle types.
        block.push_str("        fn alef_name(&self) -> String;\n");
        block.push_str("        fn alef_version(&self) -> String;\n");
        // initialize/shutdown return a JSON envelope `{"ok":null}` / `{"err":"<msg>"}` —
        // swift-bridge 0.1.59 cannot bridge `Result<(), String>` from `extern "Swift"` (broken
        // codegen for `Result<RustString, RustString>` shape). The wrapper decodes the envelope.
        block.push_str("        fn alef_initialize(&self) -> String;\n");
        block.push_str("        fn alef_shutdown(&self) -> String;\n");
    }

    for method in &trait_def.methods {
        let method_snake = method.name.to_snake_case();

        let mut params = vec!["&self".to_string()];
        for p in &method.params {
            let bridge_ty = if p.optional {
                format!("Option<{}>", inbound_bridge_type(&p.ty))
            } else {
                inbound_bridge_type(&p.ty)
            };
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {bridge_ty}"));
        }

        let return_ty = inbound_return_type(method);
        let params_str = params.join(", ");
        block.push_str(&crate::template_env::render(
            "inbound_swift_method.rs.jinja",
            minijinja::context! {
                method_snake => &method_snake,
                params => &params_str,
                return_ty => &return_ty,
            },
        ));
        let _ = box_name; // silence unused if no methods iter has it
    }

    block.push_str("    }\n\n");
    block
}

/// Emit the wrapper struct and trait impls for an inbound plugin trait.
///
/// Generates the Send/Sync wrapper newtype, the `Plugin` super-trait impl, the
/// trait impl itself with JSON marshalling, and the `register_*`/`unregister_*` fns.
pub(crate) fn emit_inbound_wrapper(
    trait_def: &TypeDef,
    bridge_config: &TraitBridgeConfig,
    api: &ApiSurface,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    error_constructor: &str,
) -> String {
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();
    let box_name = format!("Swift{trait_name}Box");
    let wrapper_name = format!("Swift{trait_name}Wrapper");

    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    // Resolve Plugin super-trait path; fall back to `{source_crate}::plugins::Plugin`.
    let plugin_path = api
        .types
        .iter()
        .find(|t| t.is_trait && (t.name == "Plugin" || t.name.ends_with("::Plugin")))
        .map(|t| t.rust_path.replace('-', "_"))
        .unwrap_or_else(|| format!("{source_crate}::plugins::Plugin"));

    let emit_plugin = has_plugin_super(bridge_config);
    let mut out = String::new();

    // 1. Wrapper struct with name cache + Send/Sync.
    // The name_cache field is only needed for Plugin super-trait (which returns &str from name()).
    if emit_plugin {
        out.push_str(&crate::template_env::render(
            "inbound_wrapper_struct.rs.jinja",
            minijinja::context! {
                trait_name => trait_name,
                wrapper_name => &wrapper_name,
                box_name => &box_name,
            },
        ));
    } else {
        // Non-Plugin trait: emit a simpler wrapper struct without name_cache.
        out.push_str(&format!(
            "/// Rust-side wrapper around a Swift class implementing the `{trait_name}` protocol.\n\
             ///\n\
             /// The Swift instance is held via a `swift-bridge` opaque handle that retains\n\
             /// the underlying ARC reference for the lifetime of this struct. Send + Sync are\n\
             /// asserted unsafely: Swift classes used as trait bridges must be thread-safe.\n\
             pub struct {wrapper_name} {{\n\
             \x20   inner: ffi::{box_name},\n\
             }}\n\
             unsafe impl Send for {wrapper_name} {{}}\n\
             unsafe impl Sync for {wrapper_name} {{}}\n\
             \n\
             impl {wrapper_name} {{\n\
             \x20   /// Construct a new wrapper from a Swift `{box_name}` handle.\n\
             \x20   pub fn new(inner: ffi::{box_name}) -> Self {{\n\
             \x20       Self {{ inner }}\n\
             \x20   }}\n\
             }}\n"
        ));
        // Emit `Debug` when the trait's supertrait list includes it. The opaque swift-bridge
        // handle does not derive Debug, so we write a manual impl that identifies the wrapper
        // by name only — sufficient for trait satisfaction.
        if trait_def.super_traits.iter().any(|s| s == "Debug" || s.ends_with("::Debug")) {
            out.push_str(&format!(
                "impl ::std::fmt::Debug for {wrapper_name} {{\n\
                 \x20   fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {{\n\
                 \x20       f.debug_struct(\"{wrapper_name}\").finish_non_exhaustive()\n\
                 \x20   }}\n\
                 }}\n"
            ));
        }
    }

    // 2. Plugin super-trait impl — only when the trait declares Plugin as a super-trait.
    if emit_plugin {
        out.push_str(&crate::template_env::render(
            "inbound_plugin_impl.rs.jinja",
            minijinja::context! {
                plugin_path => &plugin_path,
                wrapper_name => &wrapper_name,
                result_type => result_type(source_crate, error_type, "()"),
            },
        ));
    }
    let _ = trait_snake;

    // 3. Trait impl.
    let has_async = trait_def.methods.iter().any(|m| m.is_async);
    out.push_str(&crate::template_env::render(
        "inbound_trait_impl_open.rs.jinja",
        minijinja::context! {
            has_async => has_async,
            trait_path => &trait_path,
            wrapper_name => &wrapper_name,
        },
    ));
    for method in &trait_def.methods {
        emit_inbound_method_impl(
            &mut out,
            method,
            &trait_snake,
            source_crate,
            type_paths,
            error_type,
            emit_plugin,
        );
    }
    out.push_str("}\n\n");

    // 4. Registration entry points.
    if let Some(register_fn) = bridge_config.register_fn.as_deref() {
        if let Some(registry_getter) = bridge_config.registry_getter.as_deref() {
            let extra_args = bridge_config
                .register_extra_args
                .as_deref()
                .map(|a| format!(", {a}"))
                .unwrap_or_default();
            out.push_str(&crate::template_env::render(
                "inbound_register_fn.rs.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    register_fn => register_fn,
                    box_name => &box_name,
                    trait_path => &trait_path,
                    wrapper_name => &wrapper_name,
                    registry_getter => registry_getter,
                    extra_args => &extra_args,
                },
            ));
        }
    }

    let spec = build_bridge_spec(
        bridge_config,
        trait_def,
        source_crate,
        type_paths,
        error_type,
        error_constructor,
    );
    let generator = SwiftBridgeGenerator;

    let unregister_code = generator.gen_unregistration_fn(&spec);
    if !unregister_code.is_empty() {
        out.push_str(&unregister_code);
        out.push('\n');
    }

    let clear_code = generator.gen_clear_fn(&spec);
    if !clear_code.is_empty() {
        out.push_str(&clear_code);
        out.push('\n');
    }

    out
}

/// Build a [`TraitBridgeSpec`] from inbound-wrapper context so the
/// [`SwiftBridgeGenerator`] can be called without duplicating field extraction.
fn build_bridge_spec<'a>(
    bridge_config: &'a TraitBridgeConfig,
    trait_def: &'a TypeDef,
    source_crate: &'a str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    error_constructor: &str,
) -> TraitBridgeSpec<'a> {
    TraitBridgeSpec {
        trait_def,
        bridge_config,
        core_import: source_crate,
        wrapper_prefix: "Swift",
        type_paths: type_paths.clone(),
        error_type: error_type.to_string(),
        error_constructor: error_constructor.to_string(),
    }
}

/// Emit the shared helper functions used by every inbound wrapper:
///
/// - `plugin_error_from_string` — converts a stringified Swift error into the source crate's
///   configured error type.
/// - `decode_inbound_envelope` — deserialises a JSON envelope (`{"ok": <value>}` /
///   `{"err": "<message>"}`) returned from a fallible Swift trait method into a Rust `Result`.
///
/// We carry fallible results across the FFI as a JSON envelope rather than swift-bridge's
/// native `Result<T, E>` because swift-bridge 0.1.59's `Result<RustString, RustString>`
/// codegen has a bug (`error[E0609]: no field 'ok_or_err' on type '*mut RustString'`).
/// JSON envelopes also gives us a uniform way to ferry typed Ok values without per-method
/// FFI plumbing.
pub(crate) fn emit_plugin_error_helper(source_crate: &str, error_type: &str, error_constructor: &str) -> String {
    let error_type_path = error_type_path(source_crate, error_type);
    let plugin_error_constructor = error_constructor.replace("{msg}", "message");
    crate::template_env::render(
        "plugin_error_helper.rs.jinja",
        minijinja::context! {
            error_type_path => &error_type_path,
            plugin_error_constructor => &plugin_error_constructor,
        },
    )
}

/// Emit one `impl Trait for SwiftWrapper` method body.
fn emit_inbound_method_impl(
    out: &mut String,
    method: &MethodDef,
    trait_snake: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    emit_plugin: bool,
) {
    // For Plugin super-trait bridges: methods with a default impl are left to the
    // trait's own default (e.g. `as_sync_extractor` returning `Option<&dyn Sync…>`
    // cannot round-trip via the swift FFI). Skip them.
    //
    // For non-Plugin trait bridges: we must emit method bodies for ALL non-lifecycle
    // methods (including those with defaults) so Swift visitor callbacks actually fire.
    // If we skip them, the trait's no-op default runs and Swift callbacks are never
    // invoked — a silent bug.
    if emit_plugin && method.has_default_impl {
        return;
    }

    let method_snake = method.name.to_snake_case();

    // Build signature matching the original trait method.
    // Use the receiver kind from the IR so that `&mut self` methods are not silently
    // emitted as `&self`, which would cause E0053 ("incompatible type for trait").
    let receiver_token = match &method.receiver {
        Some(alef_core::ir::ReceiverKind::RefMut) => "&mut self",
        Some(alef_core::ir::ReceiverKind::Owned) => "self",
        // Default to `&self` for `Ref` and for the `None` case (static methods
        // should not reach here, but be defensive).
        _ => "&self",
    };
    let mut sig_params = vec![receiver_token.to_string()];
    for p in &method.params {
        let mut prefix = String::new();
        if p.is_ref {
            prefix.push('&');
        }
        if p.is_mut {
            prefix.push_str("mut ");
        }
        // When `is_ref: true` and the type is `Vec<T>`, the original Rust param was
        // `&[T]` — Rust idiomatically uses slices not `&Vec<T>` as params. Emit `[elem]`
        // so that prepending `&` gives `&[elem]`, matching the trait declaration.
        //
        // When `optional: true`, the original type was `Option<…>` — the wrapper is
        // stripped during IR extraction. Reconstruct it here so the signature matches
        // the trait (e.g. `Option<&str>` not `&str`).
        let inner_ty = if p.is_ref {
            match &p.ty {
                TypeRef::Vec(inner) => {
                    let elem = inbound_native_ty_owned(inner, source_crate, type_paths);
                    format!("[{elem}]")
                }
                other => inbound_native_ty(other, source_crate, type_paths),
            }
        } else {
            inbound_native_ty(&p.ty, source_crate, type_paths)
        };
        let full_ty = if p.optional {
            format!("Option<{prefix}{inner_ty}>")
        } else {
            format!("{prefix}{inner_ty}")
        };
        sig_params.push(format!("{}: {full_ty}", p.name.to_snake_case()));
    }

    let return_ty = inbound_impl_return_type(method, source_crate, type_paths, error_type);

    let async_kw = if method.is_async { "async " } else { "" };
    let params = sig_params.join(", ");
    out.push_str(&crate::template_env::render(
        "inbound_method_open.rs.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_snake => &method_snake,
            params => &params,
            return_ty => &return_ty,
        },
    ));

    // Emit per-param conversions (owned values for FFI).
    for p in &method.params {
        if let Some(line) = inbound_param_to_bridge(p) {
            out.push_str(&crate::template_env::render(
                "inbound_method_binding.rs.jinja",
                minijinja::context! {
                    line => &line,
                },
            ));
        }
    }

    let call_args: Vec<String> = method.params.iter().map(inbound_local_name).collect();
    let call_expr = format!("self.inner.alef_{method_snake}({})", call_args.join(", "));

    // returns_ref = true with Vec<String> return type: the Swift side returns Vec<String>
    // (the only type swift-bridge can ferry back), but the trait requires &[&str].
    // Box::leak the owned Vec into a 'static slice of &'static str.
    let is_mime_types_pattern = method.returns_ref
        && matches!(&method.return_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));

    if method.error_type.is_some() {
        // Fallible methods receive a JSON envelope String; decode_inbound_envelope deserialises
        // `{"ok": <value>}` or `{"err": "<message>"}` into a configured `Result<T, E>`.
        if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&crate::template_env::render(
                "inbound_method_result_unit.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            ));
        } else {
            let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
            out.push_str(&crate::template_env::render(
                "inbound_method_result_value.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                    native_ty => &native_ty,
                },
            ));
        }
    } else if is_mime_types_pattern {
        // &[&str] return: the Swift FFI shim returns Vec<String>; Box::leak it into
        // a 'static slice so the &[&str] borrow lifetime requirement is satisfied.
        // supported_mime_types() is called once per registration and the data is process-global.
        out.push_str(&crate::template_env::render(
            "inbound_method_mime_types.rs.jinja",
            minijinja::context! {
                call_expr => &call_expr,
            },
        ));
    } else if needs_inbound_json_bridge(&method.return_type) {
        let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
        out.push_str(&crate::template_env::render(
            "inbound_method_json_return.rs.jinja",
            minijinja::context! {
                call_expr => &call_expr,
                native_ty => &native_ty,
                trait_snake => trait_snake,
                method_snake => &method_snake,
            },
        ));
    } else {
        match &method.return_type {
            TypeRef::Unit => out.push_str(&crate::template_env::render(
                "inbound_method_unit_call.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            )),
            _ => out.push_str(&crate::template_env::render(
                "inbound_method_value_call.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            )),
        }
    }

    out.push_str("    }\n\n");
}

/// Convert a trait param into its bridged FFI form via a `let` binding when needed.
fn inbound_param_to_bridge(p: &ParamDef) -> Option<String> {
    let local = inbound_local_name(p);
    let name = p.name.to_snake_case();

    if needs_inbound_json_bridge(&p.ty) {
        // Named types may arrive as `&kreuzberg::OcrConfig` (is_ref) — serde::Serialize
        // is implemented for the type and `&T: Serialize when T: Serialize`, so a single
        // `to_string(&name)` call handles both owned and borrowed forms.
        if p.optional {
            return Some(format!(
                "let {local} = {name}.map(|v| ::serde_json::to_string(&v).expect(\"serializable param {name}\"));"
            ));
        }
        return Some(format!(
            "let {local} = ::serde_json::to_string(&{name}).expect(\"serializable param {name}\");"
        ));
    }

    // For optional (Option<T>) params the FFI conversion must be mapped over the Option.
    if p.optional {
        return match &p.ty {
            TypeRef::Path => Some(format!(
                "let {local} = {name}.map(|v| v.to_string_lossy().into_owned());"
            )),
            TypeRef::Bytes if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            TypeRef::String if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_string());")),
            TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            _ => None,
        };
    }

    match &p.ty {
        TypeRef::Path => {
            // FFI expects owned `String` (path-as-string).
            Some(format!("let {local} = {name}.to_string_lossy().into_owned();"))
        }
        TypeRef::Bytes => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_vec();"))
            } else {
                None
            }
        }
        TypeRef::String => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_string();"))
            } else {
                None
            }
        }
        TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.to_vec();")),
        _ => None,
    }
}

fn inbound_local_name(p: &ParamDef) -> String {
    p.name.to_snake_case()
}

/// FFI shim return type for `extern "Swift"` declarations.
///
/// Returns `String` for fallible methods (carrying a JSON envelope `{"ok": ...}` /
/// `{"err": "..."}`) instead of `Result<T, String>`. swift-bridge 0.1.59's
/// `Result<RustString, RustString>` codegen has a bug — `convert_ffi_result_ok_value_to_rust_value`
/// emits `result.ok_or_err` on a bare `*mut RustString` instead of the `ResultPtrAndPtr`
/// wrapper, producing `error[E0609]: no field 'ok_or_err' on type '*mut RustString'`.
/// Encoding the result as a JSON envelope sidesteps the limitation while preserving the
/// error-channel semantics; the Rust-side wrapper deserialises and reconstitutes the
/// `Result` after the FFI call.
fn inbound_return_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
        // Always return JSON envelope for fallible methods.
        return "String".to_string();
    }
    inbound_bridge_type(&method.return_type)
}

fn inbound_impl_return_type(
    method: &MethodDef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
) -> String {
    // When the trait method returns &[&str] (returns_ref = true, Vec<String>),
    // emit the slice reference form so the generated impl signature matches the trait.
    if method.returns_ref {
        if let TypeRef::Vec(inner) = &method.return_type {
            let elem = match inner.as_ref() {
                TypeRef::String => "&'static str".to_string(),
                other => inbound_native_ty(other, source_crate, type_paths),
            };
            return format!("&'static [{elem}]");
        }
    }

    // Return types are owned (not borrowed) — use the owned form so e.g. `String` is emitted
    // for `TypeRef::String` rather than the unsized `str` that `inbound_native_ty` uses for
    // parameter positions.
    let inner = inbound_native_ty_owned(&method.return_type, source_crate, type_paths);
    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            result_type(source_crate, error_type, "()")
        } else {
            result_type(source_crate, error_type, &inner)
        }
    } else {
        inner
    }
}

fn result_type(source_crate: &str, error_type: &str, ok_type: &str) -> String {
    format!(
        "std::result::Result<{ok_type}, {}>",
        error_type_path(source_crate, error_type)
    )
}

fn error_type_path(source_crate: &str, error_type: &str) -> String {
    if error_type.contains("::") || error_type.contains('<') {
        error_type.to_string()
    } else {
        format!("{source_crate}::{error_type}")
    }
}

/// Resolve a Named type to its fully-qualified Rust path. Falls back to `{source_crate}::{name}`
/// when the lookup misses (covers shared types declared at the crate root).
fn resolve_named_path(
    name: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(path) = type_paths.get(name) {
        return path.replace('-', "_");
    }
    format!("{source_crate}::{name}")
}

/// Render the owned native return type (used in JSON-deserialise calls). Named types are
/// resolved via `type_paths`. Inner types in containers use the owned form.
fn inbound_native_return_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_return_ty(k, source_crate, type_paths),
            inbound_native_return_ty(v, source_crate, type_paths)
        ),
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Render a TypeRef in its native (non-bridged) Rust form, qualifying Named types via
/// `type_paths`. Used for the `impl Trait` signature.
fn inbound_native_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Unit => "()".to_string(),
        TypeRef::String => "str".to_string(),
        TypeRef::Bytes => "[u8]".to_string(),
        TypeRef::Path => "::std::path::Path".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Json => "::serde_json::Value".to_string(),
        TypeRef::Duration => "::std::time::Duration".to_string(),
        TypeRef::Primitive(p) => primitive_str(p).to_string(),
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_ty_owned(k, source_crate, type_paths),
            inbound_native_ty_owned(v, source_crate, type_paths)
        ),
    }
}

/// Owned form (for use inside `Vec`/`Option`/`HashMap`): swap unsized types (`str`,
/// `[u8]`, `Path`) with their owned equivalents.
fn inbound_native_ty_owned(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => inbound_native_ty(ty, source_crate, type_paths),
    }
}

fn primitive_str(p: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType::*;
    match p {
        Bool => "bool",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        Isize => "isize",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        Usize => "usize",
        F32 => "f32",
        F64 => "f64",
    }
}
