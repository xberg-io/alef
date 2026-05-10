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

use alef_codegen::generators::trait_bridge::{TraitBridgeSpec, gen_bridge_plugin_impl, gen_bridge_trait_impl};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, TypeDef, TypeRef};
use heck::ToPascalCase;
use std::collections::HashMap;

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
    /// FFI function/type prefix (e.g., `"kreuzberg"`).
    pub prefix: String,
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for qualifying `Named` types.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
    /// Optional Rust expression that constructs an `error_type` value from a
    /// `String` named `msg`, used by the Plugin super-trait `initialize` and
    /// `shutdown` shims. Sourced from `[ffi] plugin_error_constructor` in the
    /// crate config. When `None`, the plugin shims fall back to a generic
    /// `format!`-style constructor that doesn't depend on a specific error
    /// variant shape.
    pub plugin_error_constructor: Option<String>,
}

impl FfiBridgeGenerator {
    /// VTable struct name: `{PascalPrefix}{TraitName}VTable`.
    pub(super) fn vtable_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = self.prefix.to_pascal_case();
        format!("{}{}VTable", pascal, spec.trait_def.name)
    }

    /// Bridge struct name: `{PascalPrefix}{TraitName}Bridge`.
    pub(super) fn bridge_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = self.prefix.to_pascal_case();
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
                if has_error {
                    v.push("out_error: *mut *mut std::ffi::c_char".to_string());
                }
                v
            }
            TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                // Complex return: JSON-encode into an out_result string
                let mut v = vec!["out_result: *mut *mut std::ffi::c_char".to_string()];
                if has_error {
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

        let ret = if has_error {
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
    crate::template_env::render("ffi_set_out_error_helper.jinja", minijinja::context! {})
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
        // Include excluded types so trait methods that reference them (e.g. `&InternalDocument`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    let generator = FfiBridgeGenerator {
        prefix: prefix.to_string(),
        core_import: core_import.to_string(),
        type_paths: type_paths.clone(),
        error_type: error_type.to_string(),
        plugin_error_constructor: plugin_error_constructor.map(str::to_string),
    };

    let spec = TraitBridgeSpec {
        trait_def: trait_type,
        bridge_config: bridge_cfg,
        core_import,
        wrapper_prefix: &prefix.to_pascal_case(),
        type_paths,
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

    // Trait impl — uses shared gen_bridge_trait_impl which calls gen_sync/async_method_body
    out.push_str(&gen_bridge_trait_impl(&spec, &generator));
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
/// - `trait_name`: Rust trait name, e.g. `"HtmlVisitor"`.
pub fn gen_bridge_new_free(prefix: &str, pascal_prefix: &str, trait_name: &str) -> String {
    let bridge_name = format!("{pascal_prefix}{trait_name}Bridge");
    let vtable_name = format!("{pascal_prefix}{trait_name}VTable");

    // snake_case: e.g. "HtmHtmlVisitorBridge" → "htm_html_visitor_bridge"
    let bridge_snake = to_snake_case(&bridge_name);
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
/// behaviour (e.g. `HtmHtmlVisitorBridge` → `htm_html_visitor_bridge`).
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::*;

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("my_lib::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
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
        }
    }

    fn make_method(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
            is_async: false,
            is_static: false,
            error_type: if has_error {
                Some("Box<dyn std::error::Error + Send + Sync>".to_string())
            } else {
                None
            },
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: has_default,
        }
    }

    fn sample_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        }
    }

    fn sample_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        }
    }

    #[test]
    fn test_vtable_struct_is_repr_c() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let bridge_cfg = sample_bridge_cfg("OcrBackend");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(code.contains("#[repr(C)]"), "vtable must be #[repr(C)]");
        assert!(
            code.contains("MlOcrBackendVTable"),
            "vtable name must include prefix + trait name"
        );
    }

    #[test]
    fn test_vtable_has_method_fn_ptrs() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![
                make_method("process", TypeRef::String, true, false),
                make_method("status", TypeRef::Primitive(PrimitiveType::I32), false, true),
            ],
        );
        let bridge_cfg = sample_bridge_cfg("OcrBackend");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(code.contains("pub process:"), "vtable must have fn ptr for 'process'");
        assert!(code.contains("pub status:"), "vtable must have fn ptr for 'status'");
        assert!(
            code.contains("pub free_user_data:"),
            "vtable must have free_user_data destructor"
        );
    }

    #[test]
    fn test_vtable_fn_ptrs_take_user_data() {
        let trait_def = make_trait_def(
            "Checker",
            vec![make_method(
                "ping",
                TypeRef::Primitive(PrimitiveType::Bool),
                false,
                false,
            )],
        );
        let bridge_cfg = sample_bridge_cfg("Checker");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "lib",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("user_data: *const std::ffi::c_void"),
            "every vtable fn pointer must accept user_data as first param"
        );
    }

    #[test]
    fn test_bridge_struct_fields() {
        let trait_def = make_trait_def("Runner", vec![make_method("run", TypeRef::Unit, true, false)]);
        let bridge_cfg = sample_bridge_cfg("Runner");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "my_lib",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(code.contains("vtable: MyLibRunnerVTable"), "bridge must hold vtable");
        assert!(
            code.contains("user_data: *const std::ffi::c_void"),
            "bridge must hold user_data"
        );
        assert!(code.contains("cached_name: String"), "bridge must hold cached_name");
    }

    #[test]
    fn test_bridge_is_send_sync() {
        let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
        let bridge_cfg = sample_bridge_cfg("Worker");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "w",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("unsafe impl Send for WWorkerBridge"),
            "bridge must be Send"
        );
        assert!(
            code.contains("unsafe impl Sync for WWorkerBridge"),
            "bridge must be Sync"
        );
    }

    #[test]
    fn test_bridge_has_drop_impl_for_free_user_data() {
        let trait_def = make_trait_def("Plugin", vec![make_method("tick", TypeRef::Unit, false, false)]);
        let bridge_cfg = sample_bridge_cfg("Plugin");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "p",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("impl Drop for PPluginBridge"),
            "bridge must implement Drop"
        );
        assert!(code.contains("free_user_data"), "Drop impl must call free_user_data");
    }

    #[test]
    fn test_super_trait_generates_plugin_impl() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "kr",
            "kreuzberg",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("impl kreuzberg::Plugin for KrOcrBackendBridge"),
            "must generate Plugin impl"
        );
        assert!(code.contains("fn name(&self)"), "Plugin impl must have name()");
        assert!(code.contains("fn version(&self)"), "Plugin impl must have version()");
        assert!(
            code.contains("fn initialize(&self)"),
            "Plugin impl must have initialize()"
        );
        assert!(code.contains("fn shutdown(&self)"), "Plugin impl must have shutdown()");
    }

    #[test]
    fn test_register_fn_generates_extern_c() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: None,
            registry_getter: Some("kreuzberg::registry::get_ocr".to_string()),
            register_fn: Some("register_ocr_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "kr",
            "kreuzberg",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("extern \"C\" fn kr_register_ocr_backend"),
            "register fn must be extern C with correct name"
        );
        assert!(
            code.contains("extern \"C\" fn kr_unregister_ocr_backend"),
            "unregister fn must be extern C with correct name"
        );
        assert!(code.contains("#[unsafe(no_mangle)]"), "register fn must be no_mangle");
    }

    #[test]
    fn test_register_fn_validates_name_null() {
        let trait_def = make_trait_def("MyTrait", vec![make_method("do_thing", TypeRef::Unit, true, false)]);
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "MyTrait".to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some("register_my_trait".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // Null name check must be present in register fn
        assert!(
            code.contains("if name.is_null()"),
            "register fn must check for null name"
        );
    }

    #[test]
    fn test_register_fn_validates_required_fn_ptrs() {
        let trait_def = make_trait_def(
            "Transform",
            vec![
                make_method("transform", TypeRef::String, true, false), // required
                make_method("describe", TypeRef::String, false, true),  // optional (has default)
            ],
        );
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "Transform".to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some("register_transform".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // Required method fn pointer must be validated; optional one need not be
        assert!(
            code.contains("vtable.transform.is_none()"),
            "required fn ptr must be validated non-null"
        );
    }

    #[test]
    fn test_safety_comments_present() {
        let trait_def = make_trait_def("Processor", vec![make_method("run", TypeRef::String, true, false)]);
        let bridge_cfg = TraitBridgeConfig {
            trait_name: "Processor".to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some("register_processor".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("// SAFETY:"),
            "generated code must contain SAFETY comments"
        );
        assert!(
            code.contains("unsafe"),
            "generated code must use unsafe for raw pointer ops"
        );
    }

    #[test]
    fn test_trait_impl_generated() {
        let trait_def = make_trait_def("Scanner", vec![make_method("scan", TypeRef::String, true, false)]);
        let bridge_cfg = sample_bridge_cfg("Scanner");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "sc",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("impl my_lib::Scanner for ScScannerBridge"),
            "must generate trait impl"
        );
        assert!(code.contains("fn scan("), "trait impl must contain the method");
    }

    #[test]
    fn test_string_param_marshalled_to_c_char() {
        let trait_def = make_trait_def(
            "Greeter",
            vec![MethodDef {
                name: "greet".to_string(),
                params: vec![ParamDef {
                    name: "message".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
            }],
        );
        let bridge_cfg = sample_bridge_cfg("Greeter");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "g",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // The vtable fn pointer for 'greet' must accept *const c_char for the message param
        assert!(
            code.contains("*const std::ffi::c_char"),
            "string param must map to *const c_char in vtable"
        );
    }

    #[test]
    fn test_c_param_type_mappings() {
        assert_eq!(
            FfiBridgeGenerator::c_param_type(&TypeRef::String),
            "*const std::ffi::c_char"
        );
        assert_eq!(FfiBridgeGenerator::c_param_type(&TypeRef::Bytes), "*const u8");
        assert_eq!(
            FfiBridgeGenerator::c_param_type(&TypeRef::Primitive(PrimitiveType::Bool)),
            "i32"
        );
        assert_eq!(FfiBridgeGenerator::c_param_type(&TypeRef::Duration), "u64");
    }

    #[test]
    fn test_c_return_convention_unit_fallible() {
        let (out_params, ret) = FfiBridgeGenerator::c_return_convention(&TypeRef::Unit, true);
        assert_eq!(ret, "i32");
        assert_eq!(out_params.len(), 1);
        assert!(out_params[0].contains("out_error"));
    }

    #[test]
    fn test_c_return_convention_string_infallible() {
        let (out_params, ret) = FfiBridgeGenerator::c_return_convention(&TypeRef::String, false);
        // Infallible string: return type is i32 (pointer to out_result pattern)
        assert_eq!(out_params.len(), 1);
        assert!(out_params[0].contains("out_result"));
        // No error out-param
        assert!(!out_params.iter().any(|p| p.contains("out_error")));
        let _ = ret;
    }

    // ---------------------------------------------------------------------------
    // Bug-regression tests: one per fixed bug so regressions are caught immediately.
    // ---------------------------------------------------------------------------

    /// Bug 1: Bare excluded-type references.
    ///
    /// When a trait method references a type that was excluded from the binding surface
    /// (present in `api.excluded_type_paths`), the generated trait impl must use the
    /// fully-qualified Rust path, not the bare type name.
    ///
    /// Example: `fn render(&self, doc: &InternalDocument)` must emit
    /// `&my_lib::internal::InternalDocument`, not `&InternalDocument`.
    #[test]
    fn bug1_excluded_type_is_fully_qualified_in_trait_impl() {
        let internal_doc_method = MethodDef {
            name: "render".to_string(),
            params: vec![alef_core::ir::ParamDef {
                name: "doc".to_string(),
                ty: TypeRef::Named("InternalDocument".to_string()),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("Renderer", vec![internal_doc_method]);
        let bridge_cfg = sample_bridge_cfg("Renderer");

        // Include InternalDocument as an excluded type path
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: {
                let mut m = ::std::collections::HashMap::new();
                m.insert(
                    "InternalDocument".to_string(),
                    "my_lib::internal::InternalDocument".to_string(),
                );
                m
            },
        };

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        assert!(
            code.contains("&my_lib::internal::InternalDocument"),
            "excluded type must be fully-qualified, not bare;\n\
             actual code:\n{code}"
        );
        assert!(
            !code.contains("&InternalDocument"),
            "bare type reference must not appear in generated trait impl;\n\
             actual code:\n{code}"
        );
    }

    /// Bug 2: Sync method bodies must use the trait's error type, not `Box::from`.
    ///
    /// `gen_vtable_call_body(inside_closure=false)` is used for synchronous trait method
    /// bodies.  Those methods return `Result<T, KreuzbergError>`, so error construction
    /// must call `spec.make_error(...)` (e.g. `MyError::from(...)`), not `Box::from(...)`.
    /// `Box::from` is correct only inside the async `_SendFn` closure where the return type
    /// is `Box<dyn Error + Send + Sync>`.
    #[test]
    fn bug2_sync_method_body_uses_trait_error_type_not_box_from() {
        use alef_codegen::generators::trait_bridge::TraitBridgeSpec;

        let method = MethodDef {
            name: "run".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("MyError".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("Backend", vec![method.clone()]);
        let bridge_cfg = sample_bridge_cfg("Backend");

        let spec = TraitBridgeSpec {
            trait_def: &trait_def,
            bridge_config: &bridge_cfg,
            core_import: "my_lib",
            wrapper_prefix: "Ml",
            type_paths: ::std::collections::HashMap::new(),
            error_type: "MyError".to_string(),
            error_constructor: "MyError::from({msg})".to_string(),
        };

        let generator = FfiBridgeGenerator {
            prefix: "ml".to_string(),
            core_import: "my_lib".to_string(),
            type_paths: ::std::collections::HashMap::new(),
            error_type: "MyError".to_string(),
            plugin_error_constructor: None,
        };

        // Sync body (inside_closure = false): must use MyError::from, not Box::from
        let sync_body = generator.gen_vtable_call_body(&method, &spec, false);
        assert!(
            sync_body.contains("MyError::from("),
            "sync method body must use the trait's error constructor;\n\
             actual body:\n{sync_body}"
        );
        assert!(
            !sync_body.contains("Err(Box::from("),
            "sync method body must NOT use Box::from (that's for the async closure);\n\
             actual body:\n{sync_body}"
        );

        // Closure body (inside_closure = true): must use Box::from, not MyError::from
        let closure_body = generator.gen_vtable_call_body(&method, &spec, true);
        assert!(
            closure_body.contains("Err(Box::from("),
            "async closure body must use Box::from;\n\
             actual body:\n{closure_body}"
        );
    }

    /// Bug 3: `Vec<String> + returns_ref` methods must emit `&[&str]` in the trait impl,
    /// and the bridge struct must gain a `{method_name}_strs: &'static [&'static str]` field
    /// populated at construction time.
    #[test]
    fn bug3_returns_ref_vec_string_emits_slice_ref_and_cache_field() {
        let method = MethodDef {
            name: "supported_mime_types".to_string(),
            params: vec![],
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: true, // `fn supported_mime_types(&self) -> &[&str]`
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("DocumentExtractor", vec![method]);
        let bridge_cfg = sample_bridge_cfg("DocumentExtractor");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "kr",
            "kreuzberg",
            "KreuzbergError",
            "KreuzbergError::from({msg})",
            None,
            &api,
        );

        // The trait impl return type must be `&[&str]`, not `Vec<String>`
        assert!(
            code.contains("fn supported_mime_types(&self) -> &[&str]"),
            "returns_ref Vec<String> must produce &[&str] in trait impl;\n\
             actual code:\n{code}"
        );

        // The bridge struct must have the cache field
        assert!(
            code.contains("supported_mime_types_strs: &'static [&'static str]"),
            "bridge struct must have supported_mime_types_strs cache field;\n\
             actual code:\n{code}"
        );

        // The trait impl body must return from the cache field
        assert!(
            code.contains("self.supported_mime_types_strs"),
            "trait impl body must return from the cached field;\n\
             actual code:\n{code}"
        );

        // The constructor must populate the cache field by calling the vtable
        assert!(
            code.contains("Box::leak"),
            "constructor must use Box::leak to build &'static [&'static str];\n\
             actual code:\n{code}"
        );
    }

    /// Bug 4: Methods with `has_default_impl = true` must NOT get a generated body.
    ///
    /// When a trait provides a default implementation, the bridge should let the
    /// trait's own default take effect rather than generating a vtable-forwarding body.
    #[test]
    fn bug4_has_default_impl_method_not_generated_in_trait_impl() {
        let required = make_method("run", TypeRef::String, true, false); // required
        let optional = make_method("shutdown", TypeRef::Unit, false, true); // has default
        let trait_def = make_trait_def("Backend", vec![required, optional]);
        let bridge_cfg = sample_bridge_cfg("Backend");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // Required method must appear in the trait impl
        assert!(
            code.contains("fn run("),
            "required method must appear in trait impl;\n\
             actual code:\n{code}"
        );

        // Default method must NOT appear — let the trait's own default take effect
        assert!(
            !code.contains("fn shutdown("),
            "method with has_default_impl=true must NOT get a generated body;\n\
             actual code:\n{code}"
        );
    }

    /// Bug 5: Async method with a `&str` param must clone the param with `.to_string()`
    /// before moving it into the `spawn_blocking` closure, not with `.clone()`.
    ///
    /// `.clone()` on `&str` returns `&str` — the original borrow escapes into the closure,
    /// triggering E0521 ("borrowed data escapes outside of method").  `.to_string()`
    /// produces an owned `String` that is `'static` and safe to move into the closure.
    #[test]
    fn bug5_async_str_param_uses_to_string_not_clone() {
        let method = MethodDef {
            name: "process".to_string(),
            params: vec![ParamDef {
                name: "mime_type".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,  // &str — the borrow that escapes without .to_string()
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Unit,
            is_async: true,  // async method — closure must own all captured data
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("Backend", vec![method]);
        let bridge_cfg = sample_bridge_cfg("Backend");
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // The closure capture must convert &str to String, not clone the borrow.
        assert!(
            code.contains("let mime_type = mime_type.to_string()"),
            "async &str param must be captured via .to_string() to avoid E0521;\n\
             actual code:\n{code}"
        );
        assert!(
            !code.contains("let mime_type = mime_type.clone()"),
            "async &str param must NOT use .clone() (returns &str, still borrows);\n\
             actual code:\n{code}"
        );
    }

    /// Bug 6: Async method whose trait return type is an excluded Named type must:
    ///   (a) emit the fully-qualified path in the method SIGNATURE, and
    ///   (b) deserialize JSON from the C ABI back to that type in the closure BODY.
    ///
    /// Before the fix the generator emitted `Result<String, _>` in the signature and
    /// `Ok(cs.to_string_lossy().into_owned())` in the body — both wrong for Named returns.
    #[test]
    fn bug6_async_excluded_type_return_signature_and_deserialization() {
        let method = MethodDef {
            name: "extract_bytes".to_string(),
            params: vec![ParamDef {
                name: "content".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Named("InternalDocument".to_string()),
            is_async: true,
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("Extractor", vec![method]);
        let bridge_cfg = sample_bridge_cfg("Extractor");

        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: {
                let mut m = ::std::collections::HashMap::new();
                m.insert(
                    "InternalDocument".to_string(),
                    "my_lib::internal::InternalDocument".to_string(),
                );
                m
            },
        };

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
            None,
            &api,
        );

        // Signature must use the fully-qualified path, not String.
        assert!(
            code.contains("-> std::result::Result<my_lib::internal::InternalDocument,"),
            "async method return type must be qualified excluded type in signature;\n\
             actual code:\n{code}"
        );
        assert!(
            !code.contains("-> std::result::Result<String,"),
            "async method return type must NOT be String for Named return types;\n\
             actual code:\n{code}"
        );

        // Closure body must deserialize JSON back to InternalDocument, not pass String through.
        assert!(
            code.contains("serde_json::from_str::<my_lib::internal::InternalDocument>"),
            "async closure body must deserialize JSON to InternalDocument;\n\
             actual code:\n{code}"
        );
        assert!(
            !code.contains("Ok(cs.to_string_lossy().into_owned())"),
            "async closure body must NOT return raw String for Named return types;\n\
             actual code:\n{code}"
        );
    }
}
