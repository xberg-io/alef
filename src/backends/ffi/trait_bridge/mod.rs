//! C FFI trait bridge code generation using the vtable + opaque `user_data` pattern.
//!
//! For each `[[trait_bridges]]` entry, this module generates:
//!
//! 1. A `#[repr(C)]` vtable struct with one `Option<extern "C" fn(...)>` field per method,
//!    plus `free_user_data`.
//! 2. A bridge struct holding `vtable`, `user_data: *const c_void`, and `cached_name: String`.
//! 3. `impl Plugin for FfiBridge` (when a `super_trait` is configured).
//! 4. `impl Trait for FfiBridge` forwarding each method through the vtable.
//! 5. A `{prefix}_register_{trait_snake}` `extern "C"` function.
//! 6. A `{prefix}_unregister_{trait_snake}` `extern "C"` function.
//!
//! C has no closures or objects, so thread-safety is the caller's responsibility.
//! Every generated `unsafe impl Send + Sync` is annotated with a SAFETY comment
//! explaining this contract.

mod call_body;
mod helpers;
mod registration;
mod vtable;

use crate::codegen::generators::trait_bridge::{TraitBridgeSpec, gen_bridge_plugin_impl};
use crate::codegen::naming::{pascal_to_snake, to_class_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

use helpers::prim_to_c;

// ---------------------------------------------------------------------------
// FfiBridgeGenerator — implements TraitBridgeGenerator for the vtable ABI
// ---------------------------------------------------------------------------

/// FFI-specific trait bridge generator.
///
/// Produces vtable structs and bridge structs that implement Rust traits by
/// forwarding calls through C function pointers.  The caller owns `user_data`
/// and guarantees thread-safety.
pub struct FfiBridgeGenerator {
    /// FFI function/type prefix (e.g., `"sample_core"`).
    pub prefix: String,
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for qualifying `Named` types.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
    pub error_type: String,
    /// Optional Rust expression that constructs an `error_type` value from a
    /// `String` named `msg`, used by the Plugin super-trait `initialize` and
    /// `shutdown` shims. Sourced from `[ffi] plugin_error_constructor` in the
    /// crate config. When `None`, the plugin shims fall back to a generic
    /// `format!`-style constructor that doesn't depend on a specific error
    /// variant shape.
    pub plugin_error_constructor: Option<String>,
    /// Set of type names (from `TypeDef.has_lifetime_params`) that carry a
    /// lifetime parameter in their Rust definition (e.g. `SyntaxContext<'a>`).
    /// When a trait method parameter references one of these types with `is_ref=true`,
    /// the generated trait impl signature emits `&Type<'_>` so it matches the
    /// trait definition exactly.
    pub lifetime_type_names: HashSet<String>,
}

impl FfiBridgeGenerator {
    /// VTable struct name: `{PascalPrefix}{TraitName}VTable`.
    pub(super) fn vtable_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = to_class_name(&self.prefix);
        format!("{}{}VTable", pascal, spec.trait_def.name)
    }

    /// Bridge struct name: `{PascalPrefix}{TraitName}Bridge`.
    pub(super) fn bridge_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = to_class_name(&self.prefix);
        format!("{}{}Bridge", pascal, spec.trait_def.name)
    }

    /// Map a `TypeRef` to the C-ABI parameter type string.
    ///
    /// String params become `*const std::ffi::c_char`.
    /// Named/complex params become JSON-encoded `*const std::ffi::c_char`.
    /// Primitives map directly.
    pub(super) fn c_param_type(ty: &TypeRef) -> String {
        match ty {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "*const std::ffi::c_char".to_string(),
            TypeRef::Bytes => "*const u8".to_string(),
            TypeRef::Primitive(p) => prim_to_c(p).to_string(),
            TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Complex types go over the wire as JSON strings
                "*const std::ffi::c_char".to_string()
            }
            TypeRef::Optional(inner) => {
                // Optional string/named → nullable pointer; optional primitive → primitive (0 = None)
                match inner.as_ref() {
                    TypeRef::Primitive(p) => prim_to_c(p).to_string(),
                    _ => "*const std::ffi::c_char".to_string(),
                }
            }
            TypeRef::Unit => "()".to_string(),
            TypeRef::Duration => "u64".to_string(),
        }
    }

    /// Map a `TypeRef` return to the C-ABI out-param + return-type convention.
    ///
    /// Returns:
    /// - A list of additional out-parameters to append to the function signature.
    /// - The C return type (`i32` for fallible, or the direct primitive for infallible simple types).
    pub(super) fn c_return_convention(ty: &TypeRef, has_error: bool) -> (Vec<String>, String) {
        // For complex return types (Named, Vec, Map, String), always include out_error
        // even for infallible methods, to maintain stack alignment and C# FFI compatibility
        let needs_out_error = matches!(
            ty,
            TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::String | TypeRef::Json
        ) || has_error;

        let out_params = match ty {
            TypeRef::Unit => {
                if has_error {
                    vec!["out_error: *mut *mut std::ffi::c_char".to_string()]
                } else {
                    vec![]
                }
            }
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => {
                let mut v = vec!["out_result: *mut *mut std::ffi::c_char".to_string()];
                if needs_out_error {
                    v.push("out_error: *mut *mut std::ffi::c_char".to_string());
                }
                v
            }
            TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Complex return: JSON-encode into an out_result string
                let mut v = vec!["out_result: *mut *mut std::ffi::c_char".to_string()];
                if needs_out_error {
                    v.push("out_error: *mut *mut std::ffi::c_char".to_string());
                }
                v
            }
            _ => {
                if has_error {
                    vec!["out_error: *mut *mut std::ffi::c_char".to_string()]
                } else {
                    vec![]
                }
            }
        };

        let ret = if has_error || needs_out_error {
            "i32".to_string()
        } else {
            match ty {
                TypeRef::Primitive(p) => prim_to_c(p).to_string(),
                TypeRef::Unit => "()".to_string(),
                TypeRef::Duration => "u64".to_string(),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Primitive(p) => prim_to_c(p).to_string(),
                    _ => "i32".to_string(), // nullable pointer returns 0/1 via out_result
                },
                _ => "i32".to_string(),
            }
        };

        (out_params, ret)
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Generate the shared FFI error-setting helper function (once per module).
pub fn gen_ffi_set_out_error_helper() -> String {
    crate::backends::ffi::template_env::render("ffi_set_out_error_helper.jinja", minijinja::context! {})
}

/// Generate all trait bridge code for a single `[[trait_bridges]]` entry.
///
/// This function deliberately does NOT use `gen_bridge_all()` from the shared
/// infrastructure because the FFI bridge struct has a different layout
/// (`vtable + user_data + cached_name`) vs. the standard `inner + cached_name`
/// produced by `gen_bridge_wrapper_struct`.  Instead it calls the shared helpers
/// individually and generates the struct/constructor/drop manually.
#[allow(clippy::too_many_arguments)]
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    prefix: &str,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    plugin_error_constructor: Option<&str>,
    api: &ApiSurface,
) -> String {
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        // Include excluded types so trait methods that reference them (for example, `&HiddenDoc`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    let lifetime_type_names: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_lifetime_params)
        .map(|t| t.name.clone())
        .collect();

    let generator = FfiBridgeGenerator {
        prefix: prefix.to_string(),
        core_import: core_import.to_string(),
        type_paths: type_paths.clone(),
        error_type: error_type.to_string(),
        plugin_error_constructor: plugin_error_constructor.map(str::to_string),
        lifetime_type_names,
    };

    let wrapper_prefix = to_class_name(prefix);
    // Re-derive lifetime_type_names for the shared spec (the generator holds its own copy,
    // but TraitBridgeSpec also needs it so gen_bridge_trait_impl can emit `<'_>` in sigs).
    let spec_lifetime_type_names: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_lifetime_params)
        .map(|t| t.name.clone())
        .collect();
    let spec = TraitBridgeSpec {
        trait_def: trait_type,
        bridge_config: bridge_cfg,
        core_import,
        wrapper_prefix: &wrapper_prefix,
        type_paths,
        lifetime_type_names: spec_lifetime_type_names,
        error_type: error_type.to_string(),
        error_constructor: error_constructor.to_string(),
    };

    let mut out = String::with_capacity(4096);

    // Note: imports (c_void, c_char, CStr, CString, Arc) are emitted by the caller
    // via builder.add_import() to avoid duplicates with the main gen_lib_rs imports.
    // ffi_set_out_error is also emitted once by the caller (gen_lib_rs) for all trait bridges

    // VTable struct
    out.push_str(&generator.gen_vtable_struct(&spec));
    out.push('\n');

    // Bridge struct (custom layout: vtable + user_data + cached_name)
    out.push_str(&generator.gen_bridge_struct(&spec));
    out.push('\n');

    // Drop impl
    out.push_str(&generator.gen_bridge_drop(&spec));
    out.push('\n');

    // Constructor
    out.push_str(&generator.gen_constructor_impl(&spec));
    out.push('\n');

    // Plugin / super-trait impl (custom FFI version; do NOT use gen_bridge_plugin_impl
    // because that generates PyO3-style delegation through generator.gen_sync_method_body
    // which references `self.inner`, but our bridge uses `self.vtable` directly)
    if let Some(plugin_impl) = generator.gen_ffi_plugin_impl(&spec) {
        out.push_str(&plugin_impl);
        out.push('\n');
    } else {
        // Try the shared gen_bridge_plugin_impl as a fallback (no super_trait configured)
        if let Some(plugin_impl) = gen_bridge_plugin_impl(&spec, &generator) {
            out.push_str(&plugin_impl);
            out.push('\n');
        }
    }

    // Trait impl — generate for FFI, including methods with default impls (which the vtable
    // must forward through). Unlike most bindings, FFI bridges must implement ALL methods
    // because the vtable pattern requires forwarding even methods with defaults.
    out.push_str(&generator.gen_ffi_trait_impl(&spec));
    out.push('\n');

    // Registration + unregistration functions
    if spec.bridge_config.register_fn.is_some() {
        out.push('\n');
        out.push_str(&generator.gen_registration_fn_impl(&spec));
    }

    out
}

/// Generate exported `{prefix}_{bridge_snake}_new` and `{prefix}_{bridge_snake}_free`
/// C functions for options-field bridge mode.
///
/// These allow non-Rust callers (Go, Java, C#) to create and destroy a bridge handle
/// entirely through the C ABI without linking against the Rust crate.  `bridge_new`
/// takes a fully-populated vtable (function pointers filled in by the caller) and an
/// opaque `user_data` pointer, boxes a bridge value, and returns a raw pointer.
/// `bridge_free` destroys it.
///
/// Crucially, referencing `vtable: *const {VtableName}` in the exported function
/// signature forces cbindgen to emit the full struct definition for the vtable type,
/// which callers (Go) must fill in before calling `bridge_new`.
///
/// # Parameters
///
/// - `prefix`: C symbol prefix, e.g. `"htm"`.
/// - `pascal_prefix`: PascalCase prefix, e.g. `"Htm"`.
/// - `trait_name`: Rust trait name.
pub fn gen_bridge_new_free(prefix: &str, pascal_prefix: &str, trait_name: &str) -> String {
    let bridge_name = format!("{pascal_prefix}{trait_name}Bridge");
    let vtable_name = format!("{pascal_prefix}{trait_name}VTable");

    // snake_case: e.g. "DemoXmlWalkerBridge" -> "demo_xml_walker_bridge"
    let bridge_snake = ffi_symbol_component(&bridge_name);
    let fn_new = format!("{prefix}_{bridge_snake}_new");
    let fn_free = format!("{prefix}_{bridge_snake}_free");

    format!(
        r#"/// Create a new `{bridge_name}` from a vtable and opaque user_data pointer.
///
/// Returns a heap-allocated `{bridge_name}` on success, or null if `vtable` is null.
/// The caller is responsible for calling `{fn_free}` exactly once when the bridge is
/// no longer needed.
///
/// # Safety
///
/// `vtable` must be a non-null pointer to a fully initialised `{vtable_name}` that
/// remains valid for the lifetime of the returned bridge.  `user_data` must be valid
/// for any thread that calls methods on this bridge.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_new}(
    vtable: *const {vtable_name},
    user_data: *const std::ffi::c_void,
) -> *mut {bridge_name} {{
    if vtable.is_null() {{
        return std::ptr::null_mut();
    }}
    // SAFETY: vtable is non-null (checked above); caller guarantees it is valid for this call.
    let bridge = unsafe {{ {bridge_name}::new(String::new(), *vtable, user_data) }};
    Box::into_raw(Box::new(bridge))
}}

/// Free a `{bridge_name}` created by `{fn_new}`.
///
/// After this call `ptr` is invalid. Passing null is a no-op.
///
/// # Safety
///
/// `ptr` must be either null or a non-null pointer returned by `{fn_new}` that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_free}(ptr: *mut {bridge_name}) {{
    if !ptr.is_null() {{
        // SAFETY: ptr is non-null and was created via Box::into_raw in {fn_new}.
        drop(unsafe {{ Box::from_raw(ptr) }});
    }}
}}"#,
    )
}

/// Convert a PascalCase identifier to snake_case for C symbol generation.
///
/// Consecutive uppercase letters are treated as a single word to match cbindgen's
/// behaviour (for example, `DemoXmlWalkerBridge` -> `demo_xml_walker_bridge`).
fn ffi_symbol_component(s: &str) -> String {
    pascal_to_snake(s)
}

#[cfg(test)]
mod tests;
