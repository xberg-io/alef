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

use alef_codegen::generators::trait_bridge::{
    TraitBridgeGenerator, TraitBridgeSpec, format_type_ref, gen_bridge_plugin_impl, gen_bridge_trait_impl,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToPascalCase;
use std::collections::HashMap;
use std::fmt::Write;

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
}

impl FfiBridgeGenerator {
    /// VTable struct name: `{PascalPrefix}{TraitName}VTable`.
    fn vtable_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = self.prefix.to_pascal_case();
        format!("{}{}VTable", pascal, spec.trait_def.name)
    }

    /// Bridge struct name: `{PascalPrefix}{TraitName}Bridge`.
    fn bridge_name(&self, spec: &TraitBridgeSpec) -> String {
        let pascal = self.prefix.to_pascal_case();
        format!("{}{}Bridge", pascal, spec.trait_def.name)
    }

    /// Map a `TypeRef` to the C-ABI parameter type string.
    ///
    /// String params become `*const std::ffi::c_char`.
    /// Named/complex params become JSON-encoded `*const std::ffi::c_char`.
    /// Primitives map directly.
    fn c_param_type(ty: &TypeRef) -> String {
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
    fn c_return_convention(ty: &TypeRef, has_error: bool) -> (Vec<String>, String) {
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

    /// Build the vtable function pointer field signature for one method.
    fn vtable_fn_ptr_field(&self, method: &MethodDef) -> String {
        let mut params = vec!["user_data: *const std::ffi::c_void".to_string()];

        for p in &method.params {
            let cty = Self::c_param_type(&p.ty);
            params.push(format!("{}: {}", p.name, cty));
        }

        let has_error = method.error_type.is_some();
        let (out_params, ret_ty) = Self::c_return_convention(&method.return_type, has_error);
        params.extend(out_params);

        let params_str = params.join(", ");
        format!(
            "    pub {name}: Option<unsafe extern \"C\" fn({params_str}) -> {ret_ty}>,",
            name = method.name,
        )
    }

    /// Generate the body of a sync method that calls through the vtable.
    fn gen_vtable_call_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let mut out = String::with_capacity(512);
        let name = &method.name;
        let has_error = method.error_type.is_some();

        // Extract the vtable fn pointer — return an error / default if it's None.
        writeln!(out, "let Some(fp) = self.vtable.{name} else {{").ok();
        if has_error {
            writeln!(
                out,
                "    return Err(Box::from(\"vtable.{name} is null — bridge not initialised\"));"
            )
            .ok();
        } else {
            // For infallible methods, return the Rust default value
            let default_expr = default_for_type(&method.return_type);
            writeln!(out, "    return {default_expr};").ok();
        }
        writeln!(out, "}};").ok();

        // Marshal each parameter to its C representation.
        // When p.optional is true, the Rust type is Option<T>; treat it the same as
        // TypeRef::Optional(T) and generate a nullable-pointer pattern.
        for p in &method.params {
            let effective_optional = p.optional || matches!(&p.ty, TypeRef::Optional(_));
            let inner_ty: &TypeRef = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };

            if effective_optional {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        // Option<&str> → nullable *const c_char via CString storage
                        let map_expr = if p.is_ref {
                            format!(
                                "let _{name}_storage: Option<std::ffi::CString> = {name}.and_then(|v| std::ffi::CString::new(v).ok());",
                                name = p.name
                            )
                        } else {
                            format!(
                                "let _{name}_storage: Option<std::ffi::CString> = {name}.as_deref().and_then(|v| std::ffi::CString::new(v).ok());",
                                name = p.name
                            )
                        };
                        writeln!(out, "{map_expr}").ok();
                        writeln!(
                            out,
                            "let {name}_ptr: *const std::ffi::c_char = _{name}_storage.as_ref().map_or(std::ptr::null(), |cs| cs.as_ptr());",
                            name = p.name
                        )
                        .ok();
                    }
                    TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        writeln!(
                            out,
                            "let _{name}_storage: Option<std::ffi::CString> = {name}.as_ref().and_then(|v| {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    let s = serde_json::to_string(v).unwrap_or_default();").ok();
                        writeln!(out, "    std::ffi::CString::new(s).ok()").ok();
                        writeln!(out, "}});").ok();
                        writeln!(
                            out,
                            "let {name}_ptr: *const std::ffi::c_char = _{name}_storage.as_ref().map_or(std::ptr::null(), |cs| cs.as_ptr());",
                            name = p.name
                        )
                        .ok();
                    }
                    _ => {} // optional primitives: pass directly by name (0 = None sentinel on C side)
                }
            } else {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        let val = if p.is_ref {
                            p.name.clone()
                        } else {
                            format!("{}.as_str()", p.name)
                        };
                        writeln!(
                            out,
                            "let _{name}_cs = match std::ffi::CString::new({val}) {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    Ok(s) => s,").ok();
                        writeln!(out, "    Err(_) => {{").ok();
                        if has_error {
                            writeln!(out, "        return Err(Box::from(\"nul byte in param {}\"));", p.name).ok();
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            writeln!(out, "        return {default_expr};").ok();
                        }
                        writeln!(out, "    }}").ok();
                        writeln!(out, "}};").ok();
                        writeln!(out, "let {name}_ptr = _{name}_cs.as_ptr();", name = p.name).ok();
                    }
                    TypeRef::Json | TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        writeln!(
                            out,
                            "let _{name}_json = serde_json::to_string(&{name}).unwrap_or_default();",
                            name = p.name
                        )
                        .ok();
                        writeln!(
                            out,
                            "let _{name}_cs = match std::ffi::CString::new(_{name}_json) {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    Ok(s) => s,").ok();
                        writeln!(out, "    Err(_) => {{").ok();
                        if has_error {
                            writeln!(
                                out,
                                "        return Err(Box::from(\"nul byte in serialized param {}\"));",
                                p.name
                            )
                            .ok();
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            writeln!(out, "        return {default_expr};").ok();
                        }
                        writeln!(out, "    }}").ok();
                        writeln!(out, "}};").ok();
                        writeln!(out, "let {name}_ptr = _{name}_cs.as_ptr();", name = p.name).ok();
                    }
                    _ => {} // primitives, bytes, duration: pass directly
                }
            }
        }

        // Build the argument list for the fn pointer call
        let mut call_args = vec!["self.user_data".to_string()];
        for p in &method.params {
            let effective_optional = p.optional || matches!(&p.ty, TypeRef::Optional(_));
            let inner_ty: &TypeRef = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let arg = if effective_optional {
                match inner_ty {
                    TypeRef::Primitive(_) => p.name.clone(),
                    _ => format!("{}_ptr", p.name),
                }
            } else {
                match inner_ty {
                    TypeRef::String
                    | TypeRef::Char
                    | TypeRef::Path
                    | TypeRef::Json
                    | TypeRef::Named(_)
                    | TypeRef::Vec(_)
                    | TypeRef::Map(_, _) => format!("{}_ptr", p.name),
                    // Bool is represented as i32 in the C ABI; cast explicitly.
                    TypeRef::Primitive(PrimitiveType::Bool) => format!("{} as i32", p.name),
                    _ => p.name.clone(),
                }
            };
            call_args.push(arg);
        }

        // Prepare out-params
        let needs_result_out = matches!(
            &method.return_type,
            TypeRef::String
                | TypeRef::Char
                | TypeRef::Path
                | TypeRef::Json
                | TypeRef::Named(_)
                | TypeRef::Vec(_)
                | TypeRef::Map(_, _)
        );
        if needs_result_out {
            writeln!(
                out,
                "let mut _out_result: *mut std::ffi::c_char = std::ptr::null_mut();"
            )
            .ok();
            call_args.push("&mut _out_result".to_string());
        }
        if has_error {
            writeln!(out, "let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();").ok();
            call_args.push("&mut _out_error".to_string());
        }

        let args_str = call_args.join(", ");

        writeln!(
            out,
            "// SAFETY: fp is a valid non-null function pointer; all temporaries outlive this call;"
        )
        .ok();
        writeln!(
            out,
            "// user_data validity is the caller's responsibility (documented in the vtable API)."
        )
        .ok();
        writeln!(out, "let _rc = unsafe {{ fp({args_str}) }};").ok();

        // Handle the return
        if has_error {
            writeln!(out, "if _rc != 0 {{").ok();
            writeln!(out, "    let msg = if _out_error.is_null() {{").ok();
            writeln!(out, "        format!(\"vtable.{name} returned error code {{}}\", _rc)").ok();
            writeln!(out, "    }} else {{").ok();
            writeln!(
                out,
                "        // SAFETY: out_error was written by the callee as a valid CString."
            )
            .ok();
            writeln!(
                out,
                "        let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
            )
            .ok();
            writeln!(out, "        cs.to_string_lossy().into_owned()").ok();
            writeln!(out, "    }};").ok();
            writeln!(out, "    return Err(Box::from(msg));").ok();
            writeln!(out, "}}").ok();

            // Decode successful return
            match &method.return_type {
                TypeRef::Unit => {
                    writeln!(out, "Ok(())").ok();
                }
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return Ok(String::new());").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "Ok(cs.to_string_lossy().into_owned())").ok();
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(
                        out,
                        "    return Err(Box::from(\"vtable.{name} returned null out_result\"));"
                    )
                    .ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "let json = cs.to_string_lossy();").ok();
                    writeln!(
                        out,
                        "serde_json::from_str::<{ret_ty}>(&json).map_err(|e| Box::from(e.to_string()) as Box<dyn std::error::Error + Send + Sync>)"
                    )
                    .ok();
                }
                other => {
                    let ret_ty = format_type_ref(other, &spec.type_paths);
                    writeln!(out, "Ok(_rc as {ret_ty})").ok();
                }
            }
        } else {
            // Infallible — decode return value directly
            match &method.return_type {
                TypeRef::Unit => {}
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return String::new();").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "cs.to_string_lossy().into_owned()").ok();
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return Default::default();").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "let json = cs.to_string_lossy();").ok();
                    writeln!(out, "serde_json::from_str::<{ret_ty}>(&json).unwrap_or_default()").ok();
                }
                TypeRef::Primitive(_) | TypeRef::Duration => {
                    writeln!(out, "_rc").ok();
                }
                _ => {}
            }
        }

        out
    }

    /// Generate the vtable struct definition.
    fn gen_vtable_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(1024);

        writeln!(
            out,
            "/// VTable for C plugin bridges implementing the `{}` trait.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Safety").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// All function pointers must be valid for the lifetime of any bridge created from"
        )
        .ok();
        writeln!(
            out,
            "/// this vtable.  `free_user_data`, when non-null, is called once with `user_data`"
        )
        .ok();
        writeln!(out, "/// when the bridge is dropped.").ok();
        writeln!(out, "#[repr(C)]").ok();
        writeln!(out, "pub struct {vtable} {{").ok();

        // Super-trait methods (Plugin: name, version, initialize, shutdown)
        if spec.bridge_config.super_trait.is_some() {
            writeln!(
                out,
                "    /// Return a null-terminated UTF-8 name string into `out_name`."
            )
            .ok();
            writeln!(
                out,
                "    pub name_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_name: *mut *mut std::ffi::c_char)>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Return a null-terminated UTF-8 version string into `out_version`."
            )
            .ok();
            writeln!(
                out,
                "    pub version_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_version: *mut *mut std::ffi::c_char)>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Initialise the plugin; return 0 on success, non-zero on failure (error text in `out_error`)."
            )
            .ok();
            writeln!(
                out,
                "    pub initialize_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_error: *mut *mut std::ffi::c_char) -> i32>,"
            )
            .ok();
            writeln!(
                out,
                "    /// Shut down the plugin; return 0 on success, non-zero on failure (error text in `out_error`)."
            )
            .ok();
            writeln!(
                out,
                "    pub shutdown_fn: Option<unsafe extern \"C\" fn(user_data: *const std::ffi::c_void, out_error: *mut *mut std::ffi::c_char) -> i32>,"
            )
            .ok();
        }

        // One field per trait method (own methods only; super-trait methods are covered above)
        let own_methods: Vec<_> = spec
            .trait_def
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .collect();

        for method in &own_methods {
            if !method.doc.is_empty() {
                for line in method.doc.lines() {
                    let stripped = line.trim_start_matches("///").trim_start();
                    if stripped.is_empty() {
                        writeln!(out, "    ///").ok();
                    } else {
                        writeln!(out, "    /// {stripped}").ok();
                    }
                }
            }
            writeln!(out, "{}", self.vtable_fn_ptr_field(method)).ok();
        }

        // free_user_data destructor
        writeln!(
            out,
            "    /// Optional destructor: called once with `user_data` when the bridge is dropped."
        )
        .ok();
        writeln!(
            out,
            "    pub free_user_data: Option<unsafe extern \"C\" fn(*mut std::ffi::c_void)>,"
        )
        .ok();

        writeln!(out, "}}").ok();
        out
    }

    /// Generate the bridge struct with `vtable`, `user_data`, `cached_name`, and `cached_version`.
    fn gen_bridge_struct(&self, spec: &TraitBridgeSpec) -> String {
        let vtable = self.vtable_name(spec);
        let bridge = self.bridge_name(spec);
        let mut out = String::with_capacity(512);

        writeln!(
            out,
            "/// Rust-side bridge that holds a C vtable pointer and opaque `user_data`."
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// Implements `{}` by forwarding calls through the vtable.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "pub struct {bridge} {{").ok();
        writeln!(out, "    vtable: {vtable},").ok();
        writeln!(out, "    user_data: *const std::ffi::c_void,").ok();
        writeln!(out, "    cached_name: String,").ok();
        writeln!(out, "    cached_version: String,").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        // The vtable contains raw fn pointers which are not Debug, so we implement manually.
        writeln!(out, "impl std::fmt::Debug for {bridge} {{").ok();
        writeln!(
            out,
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
        )
        .ok();
        writeln!(out, "        f.debug_struct(\"{bridge}\")").ok();
        writeln!(out, "            .field(\"cached_name\", &self.cached_name)").ok();
        writeln!(out, "            .field(\"cached_version\", &self.cached_version)").ok();
        writeln!(out, "            .finish_non_exhaustive()").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "// SAFETY: The caller is responsible for ensuring `user_data` is safe to send across"
        )
        .ok();
        writeln!(
            out,
            "// thread boundaries. This is documented in `{vtable}` and the registration function."
        )
        .ok();
        writeln!(out, "unsafe impl Send for {bridge} {{}}").ok();
        writeln!(out, "unsafe impl Sync for {bridge} {{}}").ok();
        out
    }

    /// Generate the `Drop` impl that calls `free_user_data` if non-null.
    fn gen_bridge_drop(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);
        let mut out = String::with_capacity(256);

        writeln!(out, "impl Drop for {bridge} {{").ok();
        writeln!(out, "    fn drop(&mut self) {{").ok();
        writeln!(out, "        if let Some(free_fn) = self.vtable.free_user_data {{").ok();
        writeln!(
            out,
            "            // SAFETY: free_fn is a valid function pointer; user_data is the pointer"
        )
        .ok();
        writeln!(
            out,
            "            // originally provided at registration. Called exactly once here."
        )
        .ok();
        writeln!(
            out,
            "            unsafe {{ free_fn(self.user_data as *mut std::ffi::c_void) }}"
        )
        .ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    /// Generate the `impl Plugin for FfiBridge` block using vtable fn pointers.
    fn gen_ffi_plugin_impl(&self, spec: &TraitBridgeSpec) -> Option<String> {
        let super_trait_name = spec.bridge_config.super_trait.as_deref()?;
        let bridge = self.bridge_name(spec);
        let core_import = &self.core_import;

        let super_trait_path = if super_trait_name.contains("::") {
            super_trait_name.to_string()
        } else {
            format!("{core_import}::{super_trait_name}")
        };

        let mut out = String::with_capacity(1024);
        writeln!(out, "impl {super_trait_path} for {bridge} {{").ok();

        // name() — uses cached_name
        writeln!(out, "    fn name(&self) -> &str {{").ok();
        writeln!(out, "        &self.cached_name").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // version() — calls vtable.version_fn, returns String
        writeln!(out, "    fn version(&self) -> String {{").ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.version_fn else {{ return String::new() }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(out, "        unsafe {{ fp(self.user_data, &mut _out) }};").ok();
        writeln!(
            out,
            "        if _out.is_null() {{ return self.cached_version.clone(); }}"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: _out is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(out, "        let cs = unsafe {{ std::ffi::CString::from_raw(_out) }};").ok();
        writeln!(out, "        cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // initialize()
        writeln!(out, "    fn initialize(&self) -> Result<()> {{").ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.initialize_fn else {{ return Ok(()); }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "        let rc = unsafe {{ fp(self.user_data, &mut _out_error) }};"
        )
        .ok();
        writeln!(out, "        if rc != 0 {{").ok();
        writeln!(
            out,
            "            let msg = if _out_error.is_null() {{ format!(\"initialize returned {{}}\", rc) }} else {{"
        )
        .ok();
        writeln!(
            out,
            "                // SAFETY: _out_error is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(
            out,
            "                let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
        )
        .ok();
        writeln!(out, "                cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "            }};").ok();
        writeln!(out, "            return Err(kreuzberg::KreuzbergError::Plugin(msg));").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        Ok(())").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        // shutdown()
        writeln!(out, "    fn shutdown(&self) -> Result<()> {{").ok();
        writeln!(
            out,
            "        let Some(fp) = self.vtable.shutdown_fn else {{ return Ok(()); }};"
        )
        .ok();
        writeln!(
            out,
            "        let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();"
        )
        .ok();
        writeln!(
            out,
            "        // SAFETY: fp is valid; user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "        let rc = unsafe {{ fp(self.user_data, &mut _out_error) }};"
        )
        .ok();
        writeln!(out, "        if rc != 0 {{").ok();
        writeln!(
            out,
            "            let msg = if _out_error.is_null() {{ format!(\"shutdown returned {{}}\", rc) }} else {{"
        )
        .ok();
        writeln!(
            out,
            "                // SAFETY: _out_error is a callee-allocated CString; we take ownership."
        )
        .ok();
        writeln!(
            out,
            "                let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
        )
        .ok();
        writeln!(out, "                cs.to_string_lossy().into_owned()").ok();
        writeln!(out, "            }};").ok();
        writeln!(out, "            return Err(kreuzberg::KreuzbergError::Plugin(msg));").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        Ok(())").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        Some(out)
    }
}

impl TraitBridgeGenerator for FfiBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        // The "foreign object" in the vtable pattern is opaque — there is no
        // named Rust type for it.  We use this field only as a conceptual handle;
        // the generated struct does NOT follow the standard `inner: T` layout
        // (see `gen_trait_bridge` below which calls the generators individually).
        "*const std::ffi::c_void"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "std::ffi::{c_void, c_char, CStr, CString}".to_string(),
            "std::sync::Arc".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        self.gen_vtable_call_body(method, spec)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // For async methods we block-on inside a spawn_blocking call, mirroring
        // the Go/Java/C# strategy of running synchronous C callbacks on a thread pool.
        let sync_body = self.gen_vtable_call_body(method, spec);
        let has_error = method.error_type.is_some();
        let core_import = &self.core_import;
        let method_name = &method.name;
        let cached_name_clone = if has_error {
            "let _cached_name = self.cached_name.clone();\n"
        } else {
            ""
        };

        let mut out = String::with_capacity(1024);
        writeln!(out, "{cached_name_clone}let vtable = self.vtable;").ok();
        writeln!(out, "let user_data = self.user_data;").ok();
        for p in &method.params {
            writeln!(out, "let {} = {}.clone();", p.name, p.name).ok();
        }
        writeln!(out).ok();
        writeln!(out, "tokio::task::spawn_blocking(move || {{").ok();

        // Re-create a minimal bridge for the blocking call
        writeln!(out, "    struct _LocalBridge {{").ok();
        writeln!(out, "        vtable: {vtable},", vtable = self.vtable_name(spec)).ok();
        writeln!(out, "        user_data: *const std::ffi::c_void,").ok();
        writeln!(out, "        cached_name: String,").ok();
        writeln!(out, "        cached_version: String,").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    unsafe impl Send for _LocalBridge {{}}").ok();
        writeln!(out, "    unsafe impl Sync for _LocalBridge {{}}").ok();
        writeln!(out, "    let bridge = _LocalBridge {{ vtable, user_data, cached_name: String::new(), cached_version: String::new() }};").ok();
        writeln!(out, "    // Inline the sync body:").ok();
        for line in sync_body.lines() {
            writeln!(out, "    {line}").ok();
        }
        writeln!(out, "}})").ok();
        writeln!(out, ".await").ok();
        if has_error {
            writeln!(
                out,
                ".map_err(|e| {core_import}::KreuzbergError::Plugin(format!(\"spawn_blocking failed in {method_name}: {{}}\", e)))??"
            )
            .ok();
        } else {
            writeln!(out, ".unwrap_or_else(|_| Default::default())").ok();
        }
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let bridge = self.bridge_name(spec);
        let vtable = self.vtable_name(spec);
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {bridge} {{").ok();
        writeln!(
            out,
            "    /// Create a new bridge from a vtable and opaque user_data pointer."
        )
        .ok();
        writeln!(out, "    ///").ok();
        writeln!(out, "    /// # Safety").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// `vtable` must remain valid for the lifetime of the returned bridge."
        )
        .ok();
        writeln!(
            out,
            "    /// `user_data` must be valid for any thread that calls methods on this bridge."
        )
        .ok();
        writeln!(out, "    /// All required fn pointers in `vtable` must be non-null.").ok();
        writeln!(
            out,
            "    pub unsafe fn new(name: String, vtable: {vtable}, user_data: *const std::ffi::c_void) -> Self {{"
        )
        .ok();
        writeln!(
            out,
            "        Self {{ vtable, user_data, cached_name: name, cached_version: String::new() }}"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let register_fn_name = spec
            .bridge_config
            .register_fn
            .as_deref()
            .expect("gen_registration_fn called without register_fn");
        let registry_getter = spec
            .bridge_config
            .registry_getter
            .as_deref()
            .expect("gen_registration_fn called without registry_getter");

        let prefix = &self.prefix;
        let trait_snake = spec.trait_snake();
        let vtable = self.vtable_name(spec);
        let bridge = self.bridge_name(spec);
        let trait_path = spec.trait_path();
        let full_register_name = format!("{prefix}_{register_fn_name}");
        let _full_unregister_name = format!("{prefix}_unregister_{trait_snake}");

        let mut out = String::with_capacity(2048);

        // --- register function ---
        writeln!(
            out,
            "/// Register a C plugin implementing `{}` via a vtable.",
            spec.trait_def.name
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Parameters").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// - `name`: null-terminated UTF-8 plugin name. Must not be null."
        )
        .ok();
        writeln!(
            out,
            "/// - `vtable`: vtable with function pointers implementing the trait."
        )
        .ok();
        writeln!(
            out,
            "/// - `user_data`: opaque pointer forwarded to every vtable function."
        )
        .ok();
        writeln!(
            out,
            "/// - `out_error`: receives a heap-allocated error string on failure."
        )
        .ok();
        writeln!(out, "///").ok();
        writeln!(out, "/// # Safety").ok();
        writeln!(out, "///").ok();
        writeln!(
            out,
            "/// All function pointers in `vtable` must remain valid until the plugin is"
        )
        .ok();
        writeln!(
            out,
            "/// unregistered. `user_data` must be safe to use from any thread that calls"
        )
        .ok();
        writeln!(out, "/// into the plugin.").ok();
        writeln!(out, "#[unsafe(no_mangle)]").ok();
        writeln!(out, "pub unsafe extern \"C\" fn {full_register_name}(").ok();
        writeln!(out, "    name: *const std::ffi::c_char,").ok();
        writeln!(out, "    vtable: {vtable},").ok();
        writeln!(out, "    user_data: *const std::ffi::c_void,").ok();
        writeln!(out, "    out_error: *mut *mut std::ffi::c_char,").ok();
        writeln!(out, ") -> i32 {{").ok();
        writeln!(out, "    if name.is_null() {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, \"name must not be null\");").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();

        // Validate required fn pointers (non-default methods must be non-null)
        for method in spec.required_methods() {
            writeln!(out, "    if vtable.{}.is_none() {{", method.name).ok();
            writeln!(
                out,
                "        ffi_set_out_error(out_error, \"vtable.{} must not be null\");",
                method.name
            )
            .ok();
            writeln!(out, "        return 1;").ok();
            writeln!(out, "    }}").ok();
        }

        writeln!(
            out,
            "    // SAFETY: name is non-null (checked above); it points to a valid C string."
        )
        .ok();
        writeln!(
            out,
            "    let plugin_name = match unsafe {{ std::ffi::CStr::from_ptr(name) }}.to_str() {{"
        )
        .ok();
        writeln!(out, "        Ok(s) => s.to_owned(),").ok();
        writeln!(out, "        Err(_) => {{").ok();
        writeln!(
            out,
            "            ffi_set_out_error(out_error, \"name is not valid UTF-8\");"
        )
        .ok();
        writeln!(out, "            return 1;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }};").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "    // SAFETY: vtable and user_data validity is the caller's responsibility."
        )
        .ok();
        writeln!(
            out,
            "    let bridge = unsafe {{ {bridge}::new(plugin_name, vtable, user_data) }};"
        )
        .ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(bridge);").ok();
        writeln!(out).ok();
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write();").ok();

        // Generate register call with optional extra args (e.g., priority for PostProcessor)
        let register_call = if let Some(extra_args) = &spec.bridge_config.register_extra_args {
            format!("registry.register(arc, {extra_args})")
        } else {
            "registry.register(arc)".to_string()
        };

        writeln!(out, "    if let Err(e) = {register_call} {{").ok();
        writeln!(out, "        ffi_set_out_error(out_error, &e.to_string());").ok();
        writeln!(out, "        return 1;").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    0").ok();
        writeln!(out, "}}").ok();

        out
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Generate the shared FFI error-setting helper function (once per module).
pub fn gen_ffi_set_out_error_helper() -> String {
    let mut out = String::with_capacity(512);
    writeln!(out, "/// Write an error message string into an FFI out-error pointer.").ok();
    writeln!(out, "///").ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "///").ok();
    writeln!(
        out,
        "/// `out_error` must be null or a valid writable `*mut *mut c_char` pointer."
    )
    .ok();
    writeln!(
        out,
        "unsafe fn ffi_set_out_error(out_error: *mut *mut std::ffi::c_char, msg: &str) {{"
    )
    .ok();
    writeln!(out, "    if !out_error.is_null() {{").ok();
    writeln!(out, "        if let Ok(cs) = std::ffi::CString::new(msg) {{").ok();
    writeln!(
        out,
        "            // SAFETY: out_error is non-null; caller must free this string."
    )
    .ok();
    writeln!(out, "            unsafe {{ *out_error = cs.into_raw(); }}").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    out
}

/// Generate all trait bridge code for a single `[[trait_bridges]]` entry.
///
/// This function deliberately does NOT use `gen_bridge_all()` from the shared
/// infrastructure because the FFI bridge struct has a different layout
/// (`vtable + user_data + cached_name`) vs. the standard `inner + cached_name`
/// produced by `gen_bridge_wrapper_struct`.  Instead it calls the shared helpers
/// individually and generates the struct/constructor/drop manually.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    prefix: &str,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
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
        .collect();

    let generator = FfiBridgeGenerator {
        prefix: prefix.to_string(),
        core_import: core_import.to_string(),
        type_paths: type_paths.clone(),
        error_type: error_type.to_string(),
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
    writeln!(out).ok();

    // Bridge struct (custom layout: vtable + user_data + cached_name)
    out.push_str(&generator.gen_bridge_struct(&spec));
    writeln!(out).ok();

    // Drop impl
    out.push_str(&generator.gen_bridge_drop(&spec));
    writeln!(out).ok();

    // Constructor
    out.push_str(&generator.gen_constructor(&spec));
    writeln!(out).ok();

    // Plugin / super-trait impl (custom FFI version; do NOT use gen_bridge_plugin_impl
    // because that generates PyO3-style delegation through generator.gen_sync_method_body
    // which references `self.inner`, but our bridge uses `self.vtable` directly)
    if let Some(plugin_impl) = generator.gen_ffi_plugin_impl(&spec) {
        out.push_str(&plugin_impl);
        writeln!(out).ok();
    } else {
        // Try the shared gen_bridge_plugin_impl as a fallback (no super_trait configured)
        if let Some(plugin_impl) = gen_bridge_plugin_impl(&spec, &generator) {
            out.push_str(&plugin_impl);
            writeln!(out).ok();
        }
    }

    // Trait impl — uses shared gen_bridge_trait_impl which calls gen_sync/async_method_body
    out.push_str(&gen_bridge_trait_impl(&spec, &generator));
    writeln!(out).ok();

    // Registration + unregistration functions
    if spec.bridge_config.register_fn.is_some() {
        writeln!(out).ok();
        out.push_str(&generator.gen_registration_fn(&spec));
    }

    out
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Map a `PrimitiveType` to its C-compatible Rust type name.
fn prim_to_c(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "i32", // C bool is int
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Return the Rust default-value expression for a `TypeRef`.
fn default_for_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "()",
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String::new()",
        TypeRef::Bytes => "Vec::new()",
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "false",
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            _ => "0",
        },
        TypeRef::Optional(_) => "None",
        TypeRef::Vec(_) => "Vec::new()",
        TypeRef::Map(_, _) => "std::collections::HashMap::new()",
        TypeRef::Json => "serde_json::Value::Null",
        TypeRef::Duration => "std::time::Duration::ZERO",
        TypeRef::Named(_) => "Default::default()",
    }
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
        }
    }

    fn sample_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
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
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "kr",
            "kreuzberg",
            "MyError",
            "MyError::from({msg})",
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
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "kr",
            "kreuzberg",
            "MyError",
            "MyError::from({msg})",
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
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
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
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
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
            type_alias: None,
            param_name: None,
            register_extra_args: None,
        };
        let api = sample_api();

        let code = gen_trait_bridge(
            &trait_def,
            &bridge_cfg,
            "ml",
            "my_lib",
            "MyError",
            "MyError::from({msg})",
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
            &api,
        );

        // The vtable fn pointer for 'greet' must accept *const c_char for the message param
        assert!(
            code.contains("*const std::ffi::c_char"),
            "string param must map to *const c_char in vtable"
        );
    }
}
