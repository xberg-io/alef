use crate::core::ir::MethodDef;

use super::{
    TraitBridgeSpec, gen_bridge_clear_fn, gen_bridge_debug_impl, gen_bridge_plugin_impl, gen_bridge_registration_fn,
    gen_bridge_trait_impl, gen_bridge_unregistration_fn, gen_bridge_wrapper_struct,
};

pub trait TraitBridgeGenerator {
    /// The type of the wrapped foreign object (e.g., `"Py<PyAny>"`, `"ThreadsafeFunction"`).
    fn foreign_object_type(&self) -> &str;

    /// Additional `use` imports needed for the bridge code.
    fn bridge_imports(&self) -> Vec<String>;

    /// Generate the body of a synchronous method bridge.
    ///
    /// The returned string is inserted inside the trait impl method. It should
    /// call through to the foreign object and convert the result.
    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String;

    /// Generate the body of an async method bridge.
    ///
    /// The returned string is the body of a `Box::pin(async move { ... })` block.
    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String;

    /// Generate the constructor body that validates and wraps the foreign object.
    ///
    /// Should check that the foreign object provides all required methods and
    /// return `Self { ... }` on success.
    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String;

    /// Generate the complete registration function including attributes, signature, and body.
    ///
    /// Each backend needs different function signatures (PyO3 takes `py: Python`,
    /// NAPI takes `#[napi]` with JS params, FFI takes `extern "C"` with raw pointers),
    /// so the generator owns the full function.
    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String;

    /// Generate an unregistration function for the bridge.
    ///
    /// Default implementation returns an empty string — backends opt in by
    /// emitting a function whose name is `spec.bridge_config.unregister_fn`
    /// (when set) and whose body calls into the host crate's
    /// `unregister_*(name)` plugin entry point.
    fn gen_unregistration_fn(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Generate a clear-all-plugins function for the bridge.
    ///
    /// Default implementation returns an empty string — backends opt in by
    /// emitting a function whose name is `spec.bridge_config.clear_fn`
    /// (when set) and whose body calls into the host crate's `clear_*()`
    /// plugin entry point. Typically used in test teardown.
    fn gen_clear_fn(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Whether the `#[async_trait]` macro should require `Send` on its futures.
    ///
    /// Returns `true` (default) for most targets. WASM is single-threaded so its
    /// trait bounds don't include `Send`; implementors should return `false` there.
    fn async_trait_is_send(&self) -> bool {
        true
    }

    /// Presence-check expression for a Rust-defaulted trait method.
    ///
    /// Return `Some(expr)` — a Rust expression, valid inside the bridge's trait-impl
    /// methods, evaluating to `true` when the wrapped foreign object provides
    /// `method` — to opt the method in to defaulted-method forwarding: the bridge
    /// then calls the host implementation when the host defines the method and
    /// falls back to the trait's genuine Rust default body (via a per-method
    /// delegate) otherwise.
    ///
    /// The default `None` keeps the prior behavior: the method is omitted from the
    /// bridge impl and the Rust default always runs, ignoring host implementations.
    fn gen_method_presence_check(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        None
    }

    /// Presence-check expression for a `Plugin` lifecycle method (`initialize`,
    /// `shutdown`) synthesized by the super-trait impl.
    ///
    /// Return `Some(expr)` to make the bridge treat a host object that doesn't
    /// define the method as a no-op (`Ok(())`) instead of failing at
    /// registration/unregistration. The default `None` keeps the prior behavior
    /// (the generated body decides, typically erroring on a missing method).
    fn gen_lifecycle_presence_check(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        None
    }

    /// Extra fields on the wrapper struct, as `(name, rust_type)` pairs.
    ///
    /// Backends whose foreign objects cannot be probed safely at call time
    /// (thread-affine values like Ruby `Value`s or JS objects behind an env)
    /// cache method-presence flags here at construction; their
    /// `gen_constructor` must initialize every declared field.
    fn extra_bridge_fields(&self, _spec: &TraitBridgeSpec) -> Vec<(String, String)> {
        Vec::new()
    }
}

pub struct BridgeOutput {
    /// Import paths (e.g., `"std::sync::Arc"`) — callers should add via `builder.add_import()`.
    pub imports: Vec<String>,
    /// The generated code (struct, impls, registration fn).
    pub code: String,
}

/// Generate the complete trait bridge code block: struct, impls, and
/// optionally a registration function.
///
/// Returns [`BridgeOutput`] with imports separated from code so callers can
/// route imports through `builder.add_import()` (which deduplicates).
pub fn gen_bridge_all(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> BridgeOutput {
    let imports = generator.bridge_imports();
    let mut out = String::with_capacity(4096);

    // Wrapper struct
    out.push_str(&gen_bridge_wrapper_struct(spec, generator));
    out.push_str("\n\n");

    // Debug impl (required by Plugin super-trait Debug bound)
    out.push_str(&gen_bridge_debug_impl(spec));
    out.push_str("\n\n");

    // Constructor (impl block with new())
    out.push_str(&generator.gen_constructor(spec));
    out.push_str("\n\n");

    // Plugin super-trait impl (if applicable)
    if let Some(plugin_impl) = gen_bridge_plugin_impl(spec, generator) {
        out.push_str(&plugin_impl);
        out.push_str("\n\n");
    }

    // Trait impl
    out.push_str(&gen_bridge_trait_impl(spec, generator));

    // Default delegates — only when the generator forwards defaulted methods
    let delegates = super::gen_bridge_default_delegates(spec, generator);
    if !delegates.is_empty() {
        out.push_str("\n\n");
        out.push_str(&delegates);
    }

    // Registration function — only when register_fn is configured
    if let Some(reg_fn_code) = gen_bridge_registration_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&reg_fn_code);
    }

    // Unregistration function — only when unregister_fn is configured AND
    // the backend has opted in (non-empty body).
    if let Some(unreg_fn_code) = gen_bridge_unregistration_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&unreg_fn_code);
    }

    // Clear-all function — only when clear_fn is configured AND the backend
    // has opted in (non-empty body).
    if let Some(clear_fn_code) = gen_bridge_clear_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&clear_fn_code);
    }

    BridgeOutput { imports, code: out }
}
