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
//!    `shutdown` to the Swift box; errors translate to `KreuzbergError::Plugin`.
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

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, ParamDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

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
        block.push_str(&format!(
            "        #[swift_bridge(swift_name = \"{camel}\")]\n        fn {register_fn}(swift_box: {box_name}) -> Result<(), String>;\n"
        ));
        has_any = true;
    }
    if let Some(unregister_fn) = bridge_config.unregister_fn.as_deref() {
        let camel = heck::AsLowerCamelCase(unregister_fn).to_string();
        block.push_str(&format!(
            "        #[swift_bridge(swift_name = \"{camel}\")]\n        fn {unregister_fn}(name: String) -> Result<(), String>;\n"
        ));
        has_any = true;
    }
    block.push_str("    }\n\n");
    if has_any { block } else { String::new() }
}

/// Emit the `extern "Swift"` block declaring `Swift{Trait}Box` and per-method FFI shims.
///
/// Each shim signature is the JSON-bridged form of the trait method: complex types become
/// `String` (JSON), primitives and `String`/`Vec<u8>` pass through directly. Methods that
/// can fail return `Result<RetBridge, String>` so the Swift side can surface errors.
pub(crate) fn emit_extern_block_for_inbound(trait_def: &TypeDef) -> String {
    let trait_name = &trait_def.name;
    let box_name = format!("Swift{trait_name}Box");

    let mut block = String::new();
    block.push_str("    extern \"Swift\" {\n");
    block.push_str(&format!("        type {box_name};\n"));

    // Plugin super-trait shims (always emitted — every plugin trait extends Plugin).
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

    for method in &trait_def.methods {
        let method_snake = method.name.to_snake_case();

        let mut params = vec!["&self".to_string()];
        for p in &method.params {
            let bridge_ty = inbound_bridge_type(&p.ty);
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {bridge_ty}"));
        }

        let return_ty = inbound_return_type(method);
        let params_str = params.join(", ");
        block.push_str(&format!(
            "        fn alef_{method_snake}({params_str}) -> {return_ty};\n"
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

    let mut out = String::new();

    // 1. Wrapper struct with name cache + Send/Sync.
    out.push_str(&format!(
        "/// Rust-side wrapper around a Swift class implementing the `{trait_name}` plugin protocol.\n"
    ));
    out.push_str("///\n");
    out.push_str("/// The Swift instance is held via a `swift-bridge` opaque handle that retains\n");
    out.push_str("/// the underlying ARC reference for the lifetime of this struct. Send + Sync are\n");
    out.push_str("/// asserted unsafely: Swift classes used as kreuzberg plugins must be thread-safe\n");
    out.push_str("/// (the `Plugin` super-trait requires it), and ARC handles themselves are safe to share.\n");
    out.push_str(&format!("pub struct {wrapper_name} {{\n"));
    out.push_str(&format!("    inner: ffi::{box_name},\n"));
    out.push_str("    /// Cached `Plugin::name()` — required because the trait returns `&str` but\n");
    out.push_str("    /// the Swift FFI shim returns an owned `String`. Populated lazily on first access.\n");
    out.push_str("    name_cache: ::std::sync::OnceLock<String>,\n");
    out.push_str("}\n");
    out.push_str(&format!("unsafe impl Send for {wrapper_name} {{}}\n"));
    out.push_str(&format!("unsafe impl Sync for {wrapper_name} {{}}\n\n"));

    out.push_str(&format!("impl {wrapper_name} {{\n"));
    out.push_str(&format!(
        "    /// Construct a new wrapper from a Swift `{box_name}` handle.\n"
    ));
    out.push_str(&format!("    pub fn new(inner: ffi::{box_name}) -> Self {{\n"));
    out.push_str("        Self { inner, name_cache: ::std::sync::OnceLock::new() }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // 2. Plugin super-trait impl.
    out.push_str(&format!("impl {plugin_path} for {wrapper_name} {{\n"));
    out.push_str("    fn name(&self) -> &str {\n");
    out.push_str("        self.name_cache.get_or_init(|| self.inner.alef_name()).as_str()\n");
    out.push_str("    }\n\n");
    out.push_str("    fn version(&self) -> String {\n");
    out.push_str("        self.inner.alef_version()\n");
    out.push_str("    }\n\n");
    out.push_str(&format!("    fn initialize(&self) -> {source_crate}::Result<()> {{\n"));
    out.push_str("        decode_inbound_envelope::<()>(&self.inner.alef_initialize()).map(|_| ())\n");
    out.push_str("    }\n\n");
    out.push_str(&format!("    fn shutdown(&self) -> {source_crate}::Result<()> {{\n"));
    out.push_str("        decode_inbound_envelope::<()>(&self.inner.alef_shutdown()).map(|_| ())\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
    let _ = trait_snake;

    // 3. Trait impl.
    let has_async = trait_def.methods.iter().any(|m| m.is_async);
    if has_async {
        out.push_str("#[async_trait::async_trait]\n");
    }
    out.push_str(&format!("impl {trait_path} for {wrapper_name} {{\n"));
    for method in &trait_def.methods {
        emit_inbound_method_impl(&mut out, method, &trait_snake, source_crate, type_paths);
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
            out.push_str(&format!(
                "/// Register a Swift class implementation as a `{trait_name}` plugin.\n"
            ));
            out.push_str("///\n");
            out.push_str(
                "/// Wraps the Swift handle in `Arc<SwiftXxxWrapper>` and inserts it into the host registry.\n",
            );
            out.push_str("/// Errors from the registry are stringified for swift-bridge transport.\n");
            out.push_str(&format!(
                "pub fn {register_fn}(swift_box: ffi::{box_name}) -> Result<(), String> {{\n"
            ));
            out.push_str(&format!(
                "    let arc: ::std::sync::Arc<dyn {trait_path}> = ::std::sync::Arc::new({wrapper_name}::new(swift_box));\n"
            ));
            out.push_str(&format!("    let registry = {registry_getter}();\n"));
            out.push_str("    let mut guard = registry.write();\n");
            out.push_str(&format!(
                "    guard.register(arc{extra_args}).map_err(|e| e.to_string())\n"
            ));
            out.push_str("}\n\n");
        }
    }

    if let Some(unregister_fn) = bridge_config.unregister_fn.as_deref() {
        if let Some(registry_getter) = bridge_config.registry_getter.as_deref() {
            out.push_str(&format!(
                "/// Unregister a previously-registered `{trait_name}` plugin by name.\n"
            ));
            out.push_str(&format!(
                "pub fn {unregister_fn}(name: String) -> Result<(), String> {{\n"
            ));
            out.push_str(&format!("    let registry = {registry_getter}();\n"));
            out.push_str("    let mut guard = registry.write();\n");
            out.push_str("    guard.remove(&name).map_err(|e| e.to_string())\n");
            out.push_str("}\n\n");
        }
    }

    out
}

/// Emit the shared helper functions used by every inbound wrapper:
///
/// - `plugin_error_from_string` — converts a stringified Swift error into the source crate's
///   `KreuzbergError::Plugin`.
/// - `decode_inbound_envelope` — deserialises a JSON envelope (`{"ok": <value>}` /
///   `{"err": "<message>"}`) returned from a fallible Swift trait method into a Rust `Result`.
///
/// We carry fallible results across the FFI as a JSON envelope rather than swift-bridge's
/// native `Result<T, E>` because swift-bridge 0.1.59's `Result<RustString, RustString>`
/// codegen has a bug (`error[E0609]: no field 'ok_or_err' on type '*mut RustString'`).
/// JSON envelopes also gives us a uniform way to ferry typed Ok values without per-method
/// FFI plumbing.
pub(crate) fn emit_plugin_error_helper(source_crate: &str) -> String {
    format!(
        "/// Convert a stringified Swift error into the source crate's `KreuzbergError::Plugin`.\n\
         #[allow(dead_code)]\n\
         fn plugin_error_from_string(message: String) -> {source_crate}::KreuzbergError {{\n\
             {source_crate}::KreuzbergError::Plugin {{ message, plugin_name: \"swift\".to_string() }}\n\
         }}\n\n\
         /// JSON envelope returned by every fallible Swift trait method. Carries `Ok(T)`\n\
         /// as `{{\"ok\": <serialised T>}}` and `Err(String)` as `{{\"err\": \"<message>\"}}`.\n\
         /// Avoids swift-bridge 0.1.59's broken `Result<RustString, RustString>` codegen.\n\
         #[allow(dead_code)]\n\
         #[derive(::serde::Deserialize)]\n\
         #[serde(rename_all = \"snake_case\")]\n\
         enum InboundEnvelope<T> {{ Ok(T), Err(String) }}\n\n\
         /// Deserialise a JSON envelope returned from a Swift FFI shim into a typed Result.\n\
         #[allow(dead_code)]\n\
         fn decode_inbound_envelope<T>(json: &str) -> {source_crate}::Result<T>\n\
         where\n\
             T: ::serde::de::DeserializeOwned,\n\
         {{\n\
             match ::serde_json::from_str::<InboundEnvelope<T>>(json) {{\n\
                 Ok(InboundEnvelope::Ok(value)) => Ok(value),\n\
                 Ok(InboundEnvelope::Err(message)) => Err(plugin_error_from_string(message)),\n\
                 Err(e) => Err(plugin_error_from_string(format!(\"swift returned malformed envelope: {{e}}\"))),\n\
             }}\n\
         }}\n\n"
    )
}

/// Emit one `impl Trait for SwiftWrapper` method body.
fn emit_inbound_method_impl(
    out: &mut String,
    method: &MethodDef,
    trait_snake: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) {
    let method_snake = method.name.to_snake_case();

    // Build signature matching the original trait method.
    let mut sig_params = vec!["&self".to_string()];
    for p in &method.params {
        let ty = inbound_native_ty(&p.ty, source_crate, type_paths);
        let mut prefix = String::new();
        if p.is_ref {
            prefix.push('&');
        }
        if p.is_mut {
            prefix.push_str("mut ");
        }
        sig_params.push(format!("{}: {prefix}{ty}", p.name.to_snake_case()));
    }

    let return_ty = inbound_impl_return_type(method, source_crate, type_paths);

    let async_kw = if method.is_async { "async " } else { "" };
    out.push_str(&format!(
        "    {async_kw}fn {method_snake}({}) -> {return_ty} {{\n",
        sig_params.join(", ")
    ));

    // Emit per-param conversions (owned values for FFI).
    for p in &method.params {
        if let Some(line) = inbound_param_to_bridge(p) {
            out.push_str(&format!("        {line}\n"));
        }
    }

    let call_args: Vec<String> = method.params.iter().map(inbound_local_name).collect();
    let call_expr = format!("self.inner.alef_{method_snake}({})", call_args.join(", "));

    if method.error_type.is_some() {
        // Fallible methods receive a JSON envelope String; decode_inbound_envelope deserialises
        // `{"ok": <value>}` or `{"err": "<message>"}` into a `Result<T, KreuzbergError::Plugin>`.
        if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&format!("        let envelope = {call_expr};\n"));
            out.push_str("        decode_inbound_envelope::<()>(&envelope).map(|_| ())\n");
        } else {
            let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
            out.push_str(&format!("        let envelope = {call_expr};\n"));
            out.push_str(&format!("        decode_inbound_envelope::<{native_ty}>(&envelope)\n"));
        }
    } else if needs_inbound_json_bridge(&method.return_type) {
        let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
        out.push_str(&format!("        let json = {call_expr};\n"));
        out.push_str(&format!(
            "        ::serde_json::from_str::<{native_ty}>(&json).expect(\"swift {trait_snake}.{method_snake} returned invalid JSON\")\n"
        ));
    } else {
        match &method.return_type {
            TypeRef::Unit => out.push_str(&format!("        {call_expr};\n")),
            _ => out.push_str(&format!("        {call_expr}\n")),
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
        return Some(format!(
            "let {local} = ::serde_json::to_string(&{name}).expect(\"serializable param {name}\");"
        ));
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
) -> String {
    let inner = inbound_native_ty(&method.return_type, source_crate, type_paths);
    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            format!("{source_crate}::Result<()>")
        } else {
            format!("{source_crate}::Result<{inner}>")
        }
    } else {
        inner
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
