//! Zig trait-bridge code generation.
//!
//! Emits one Zig extern struct (vtable) and one registration wrapper function
//! per configured `[[trait_bridges]]` entry.  The Zig consumer fills in the
//! struct with `callconv(.c)` function pointers and calls `register_*`.
//!
//! # C symbol convention
//!
//! The generated `register_{trait_snake}` shim calls
//! `c.{prefix}_register_{trait_snake}` — the symbol exposed by the
//! `sample_core-ffi` C layer (pattern: `{crate_prefix}_register_{trait_snake}`).
//! If the actual symbol differs, override the generated call site.
//!
//! # `TraitBridgeGenerator` implementation
//!
//! [`ZigTraitBridgeGenerator`] implements the shared [`TraitBridgeGenerator`]
//! trait so that the shared codegen driver can invoke the Zig-specific
//! `gen_unregistration_fn` and `gen_clear_fn` overrides.  The other required
//! methods are stubs — Zig code is produced through the standalone
//! [`emit_trait_bridge`] free function, not the shared driver.

use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::collections::HashSet;

/// Zig type string to use for a vtable slot parameter or return type.
///
/// All string/complex types collapse to `[*c]const u8` (C string pointer) since
/// the vtable slots use the raw C ABI — not the Zig-friendly wrapper layer.
///
/// CRITICAL: This function must NOT apply type substitution. The vtable ABI is C-compatible
/// and must remain stable. Excluded types appearing in vtable signatures should be kept as-is
/// so the C FFI layer can link correctly. Type substitution happens only at the Zig wrapper
/// level, not in the C ABI boundary.
fn vtable_param_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType::*;
            match p {
                Bool => "i32",
                U8 => "u8",
                U16 => "u16",
                U32 => "u32",
                U64 => "u64",
                I8 => "i8",
                I16 => "i16",
                I32 => "i32",
                I64 => "i64",
                F32 => "f32",
                F64 => "f64",
                Usize => "usize",
                Isize => "isize",
            }
        }
        TypeRef::Unit => "void",
        TypeRef::Duration => "i64",
        // All string/path/complex types become C string pointers at the C ABI boundary.
        _ => "[*c]const u8",
    }
}

/// Check if a method returns a type that requires out_result wrapping at the FFI boundary.
///
/// Methods that return strings, bytes, or complex types are wrapped with `out_result`
/// and return `i32` status code, even if they're infallible in Rust. This is because
/// the C FFI layer cannot return complex types directly.
fn method_needs_out_result(method: &MethodDef) -> bool {
    if method.error_type.is_some() && !matches!(method.return_type, TypeRef::Unit) {
        return true; // Fallible with non-unit return: needs out_result
    }
    if method.error_type.is_none() && !matches!(method.return_type, TypeRef::Unit | TypeRef::Primitive(_)) {
        return true; // Infallible but returns complex type: needs wrapping
    }
    false
}

/// Zig return type for a vtable slot.
///
/// Fallible methods always return `i32` (0 = success, non-zero = error).
/// Unit infallible methods return `void`.  Other infallible returns use the
/// primitive mapping. Infallible methods with complex returns are wrapped to return `i32`.
fn vtable_return_type(method: &MethodDef) -> String {
    if method.error_type.is_some() || method_needs_out_result(method) {
        "i32".to_string()
    } else {
        vtable_param_type(&method.return_type).to_string()
    }
}

/// Build a snake_case trait name from a PascalCase trait name.
///
/// Uses `heck::ToSnakeCase`, matching the pattern used by Go/C# backends.
fn trait_snake(trait_name: &str) -> String {
    trait_name.to_snake_case()
}

/// Emit a Zig param name for the C-ABI slot, expanding `Bytes` to ptr+len.
///
/// Returns a list of `(c_param_name, c_param_type)` pairs.
fn vtable_c_params(method: &MethodDef) -> Vec<(String, String)> {
    let mut params = vec![("ud".to_string(), "?*anyopaque".to_string())];
    for p in &method.params {
        if matches!(p.ty, TypeRef::Bytes) {
            params.push((format!("{}_ptr", p.name), "[*c]const u8".to_string()));
            params.push((format!("{}_len", p.name), "usize".to_string()));
        } else {
            params.push((p.name.clone(), vtable_param_type(&p.ty).to_string()));
        }
    }
    // Add out_result for methods that need output wrapping (fallible OR infallible-complex).
    if method_needs_out_result(method) {
        params.push(("out_result".to_string(), "?*?[*c]u8".to_string()));
    }
    // Add out_error for fallible methods.
    if method.error_type.is_some() {
        params.push(("out_error".to_string(), "?*?[*c]u8".to_string()));
    }
    params
}

/// Emit a `make_{trait_snake}_vtable(comptime T: type, instance: *T) I{Trait}` helper.
///
/// The helper builds `callconv(.c)` thunks for every vtable slot so the consumer
/// only needs to write plain Zig methods on their type.
///
/// # Limitations
///
/// - Methods returning non-unit values through `out_result` return an error code
///   when the type cannot be expressed as a direct C primitive (complex types are
///   documented as requiring manual implementation).
/// - Lifecycle slots (`name_fn`, `version_fn`, `initialize_fn`, `shutdown_fn`) are
///   emitted as no-op/null-result stubs; consumers override the relevant field
///   in the returned vtable when needed.
pub fn emit_make_vtable(
    trait_name: &str,
    has_super_trait: bool,
    trait_def: &TypeDef,
    excluded_types: &HashSet<String>,
    out: &mut String,
    ffi_skip_methods: &[String],
) {
    let snake = trait_snake(trait_name);
    let _excluded_strs: HashSet<&str> = excluded_types.iter().map(|s| s.as_str()).collect();

    out.push_str(&crate::backends::zig::template_env::render(
        "vtable_header_doc.jinja",
        minijinja::context! {
            trait_name => trait_name,
            snake => &snake,
        },
    ));
    out.push_str(&crate::backends::zig::template_env::render(
        "vtable_impl_method.jinja",
        minijinja::context! {
            snake => &snake,
            trait_name => trait_name,
        },
    ));
    out.push_str(&crate::backends::zig::template_env::render(
        "vtable_make_fn_header.jinja",
        minijinja::context! {
            trait_name => trait_name,
        },
    ));

    // Lifecycle stubs when super_trait is present
    if has_super_trait {
        out.push_str(&crate::backends::zig::template_env::render(
            "vtable_field_name_fn.jinja",
            minijinja::context! {},
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "vtable_field_version_fn.jinja",
            minijinja::context! {},
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "vtable_field_initialize_fn.jinja",
            minijinja::context! {},
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "vtable_field_shutdown_fn.jinja",
            minijinja::context! {},
        ));
    }

    // Per-method thunks
    for method in &trait_def.methods {
        // Skip methods listed in ffi_skip_methods — they cannot be represented in the C ABI.
        if ffi_skip_methods.iter().any(|skip| skip == &method.name) {
            continue;
        }
        // CRITICAL: Do NOT substitute excluded types in thunk C ABI signatures!
        // The thunk must match the C ABI exactly or it won't call correctly.
        // Substitution should never happen at the C boundary.

        let method_snake = method.name.to_snake_case();
        let c_params = vtable_c_params(method);
        let ret = vtable_return_type(method);

        // Build the thunk parameter list string
        let params_str = c_params
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");

        out.push_str(&crate::backends::zig::template_env::render(
            "vtable_instance_field.jinja",
            minijinja::context! {
                method_snake => &method_snake,
                params_str => &params_str,
                ret => &ret,
            },
        ));

        // Cast user_data to *T
        out.push_str("                const self: *T = @ptrCast(@alignCast(ud));\n");

        // Pass Bytes parameters directly as C pointers.
        // The Zig vtable ABI uses C pointers ([*c]const u8), not slices.
        // Discard the len parameter since it's not used.
        let mut call_args: Vec<String> = Vec::new();
        for p in &method.params {
            if matches!(p.ty, TypeRef::Bytes) {
                out.push_str(&crate::backends::zig::template_env::render(
                    "thunk_discard_bytes_len.jinja",
                    minijinja::context! {
                        param_name => &p.name,
                    },
                ));
                call_args.push(format!("{}_ptr", p.name));
            } else {
                call_args.push(p.name.clone());
            }
        }

        let args_str = call_args.join(", ");

        // Pick a capture name for the success branch that won't collide with method
        // params. Methods can have a param literally called `result`; using that as
        // the unwrap binding shadows the outer scope (zig 0.16+ flags this).
        let ok_binding = if method.params.iter().any(|p| p.name == "value") {
            "ok_value"
        } else {
            "value"
        };

        // Check if this method needs out_result wrapping (either fallible or infallible-complex).
        let needs_out_result = method_needs_out_result(method);
        let has_error_type = method.error_type.is_some();
        let is_infallible_complex = !has_error_type && needs_out_result;

        if has_error_type {
            // Fallible method: call returns error union, write out_result/out_error
            let has_result_out = !matches!(method.return_type, TypeRef::Unit);
            out.push_str(&crate::backends::zig::template_env::render(
                "thunk_fn_signature.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                    args_str => &args_str,
                    ok_binding => &ok_binding,
                },
            ));
            // Write result via out_result pointer. Complex result types cannot be
            // converted without caller-owned allocation context, so that branch
            // returns an error code and suppresses the trailing success return.
            let mut success_path_diverges = false;
            if has_result_out {
                match &method.return_type {
                    TypeRef::Primitive(_) | TypeRef::Unit => {
                        out.push_str(&crate::backends::zig::template_env::render(
                            "thunk_result_assign.jinja",
                            minijinja::context! {
                                ok_binding => &ok_binding,
                            },
                        ));
                    }
                    _ => {
                        // String/Bytes/complex: cannot safely convert without allocator context
                        out.push_str(&crate::backends::zig::template_env::render(
                            "thunk_if_fallible.jinja",
                            minijinja::context! {
                                ok_binding => &ok_binding,
                            },
                        ));
                        success_path_diverges = true;
                    }
                }
            } else {
                // Unit return on success — discard the captured Void to silence unused-variable.
                out.push_str(&crate::backends::zig::template_env::render(
                    "thunk_if_ok_result.jinja",
                    minijinja::context! {
                        ok_binding => &ok_binding,
                    },
                ));
            }
            if !success_path_diverges {
                out.push_str("                    return 0;\n");
            }
            out.push_str("                } else |err| {\n");
            out.push_str("                    _ = err;\n");
            out.push_str("                    if (out_error) |ptr| ptr.* = null; // caller checks error code\n");
            out.push_str("                    return 1;\n");
            out.push_str("                }\n");
        } else if is_infallible_complex {
            // Infallible method returning complex type: call directly, write to out_result, return 0.
            // The method is expected to return a pointer to a NUL-terminated C string ([*c]const u8).
            out.push_str("                const ");
            out.push_str(ok_binding);
            out.push_str(" = self.");
            out.push_str(&method_snake);
            out.push('(');
            out.push_str(&args_str);
            out.push_str(");\n");
            // Write the returned string pointer to out_result.
            // Cast away const if necessary to match the mutable out_result pointer.
            out.push_str("                if (out_result) |ptr| {\n");
            out.push_str("                    ptr.* = @constCast(");
            out.push_str(ok_binding);
            out.push_str(");\n");
            out.push_str("                }\n");
            out.push_str("                return 0;\n");
        } else {
            // Infallible methods return directly via the function return type.
            match &method.return_type {
                TypeRef::Unit => {
                    out.push_str(&crate::backends::zig::template_env::render(
                        "thunk_if_error.jinja",
                        minijinja::context! {
                            method_snake => &method_snake,
                            args_str => &args_str,
                        },
                    ));
                }
                TypeRef::Primitive(_) => {
                    out.push_str(&crate::backends::zig::template_env::render(
                        "thunk_infallible_return.jinja",
                        minijinja::context! {
                            method_snake => &method_snake,
                            args_str => &args_str,
                        },
                    ));
                }
                _ => {
                    // Non-unit infallible non-primitive: pass through (e.g., [*c]const u8)
                    out.push_str(&crate::backends::zig::template_env::render(
                        "thunk_infallible_return.jinja",
                        minijinja::context! {
                            method_snake => &method_snake,
                            args_str => &args_str,
                        },
                    ));
                }
            }
        }

        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push('\n');
    }

    // free_user_data stub — does nothing by default; caller overrides if needed
    out.push_str(&crate::backends::zig::template_env::render(
        "vtable_free_user_data.jinja",
        minijinja::context! {},
    ));

    out.push_str("    };\n");
    out.push_str("}\n");
}

/// Emit the vtable extern struct and registration shim for a single trait bridge.
///
/// `prefix` is the C FFI prefix (e.g., `"sample_core"`, `"sample-crawler"`).
/// `error_type` is the Zig error set type name (e.g., `"SampleCrateError"`, `"CrawlError"`).
/// `bridge_cfg` is the trait bridge configuration entry.
/// `trait_def` is the IR type definition for the trait (must have `is_trait = true`).
/// `excluded_types` is the set of type names that are excluded from the public binding surface.
/// `out` is the output buffer to append to.
pub fn emit_trait_bridge(
    prefix: &str,
    error_type: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    excluded_types: &HashSet<String>,
    out: &mut String,
) {
    let trait_name = &trait_def.name;
    let snake = trait_snake(trait_name);
    let has_super_trait = bridge_cfg.super_trait.is_some();

    // Excluded types are NOT used in vtable signatures (they're C ABI, must stay stable).
    // This collection is kept for potential future use in wrapper-only contexts.
    let _excluded_strs: HashSet<&str> = excluded_types.iter().map(|s| s.as_str()).collect();

    // -------------------------------------------------------------------------
    // Vtable struct: I{Trait}
    // -------------------------------------------------------------------------
    out.push_str(&crate::backends::zig::template_env::render(
        "trait_vtable_header.jinja",
        minijinja::context! {
            trait_name => trait_name,
            snake => &snake,
        },
    ));
    out.push_str(&crate::backends::zig::template_env::render(
        "trait_struct_header.jinja",
        minijinja::context! {
            trait_name => trait_name,
        },
    ));

    // Plugin lifecycle slots — always present when a super_trait is configured.
    if has_super_trait {
        out.push_str("    /// Return the plugin name into `out_name` (heap-allocated, caller frees).\n");
        out.push_str(
            "    name_fn: ?*const fn (user_data: ?*anyopaque, out_name: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null,\n",
        );
        out.push('\n');

        out.push_str("    /// Return the plugin version into `out_version` (heap-allocated, caller frees).\n");
        out.push_str(
            "    version_fn: ?*const fn (user_data: ?*anyopaque, out_version: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null,\n",
        );
        out.push('\n');

        out.push_str("    /// Initialise the plugin; return 0 on success, non-zero on error.\n");
        out.push_str(
            "    initialize_fn: ?*const fn (user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.c) i32 = null,\n",
        );
        out.push('\n');

        out.push_str("    /// Shut down the plugin; return 0 on success, non-zero on error.\n");
        out.push_str(
            "    shutdown_fn: ?*const fn (user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.c) i32 = null,\n",
        );
        out.push('\n');
    }

    // Trait method slots
    for method in &trait_def.methods {
        // Skip methods listed in ffi_skip_methods — they cannot be represented in the C ABI.
        if bridge_cfg.ffi_skip_methods.iter().any(|skip| skip == &method.name) {
            continue;
        }

        // CRITICAL: Do NOT substitute excluded types in vtable struct signatures!
        // The vtable is a C ABI struct, and changing parameter/return types breaks
        // linking with the FFI layer. Excluded types remain as-is here.
        // Substitution only applies to Zig-level wrapper code, not the C boundary.

        if !method.doc.is_empty() {
            out.push_str(&crate::backends::zig::template_env::render(
                "trait_method_doc_lines.jinja",
                minijinja::context! {
                    method_doc_lines => method.doc.lines().collect::<Vec<_>>(),
                },
            ));
        }

        let ret = vtable_return_type(method);
        let method_snake = method.name.to_snake_case();

        // Build the parameter list: user_data first, then method params.
        let mut params = vec!["user_data: ?*anyopaque".to_string()];
        for p in &method.params {
            let ty = vtable_param_type(&p.ty);
            // Bytes expand to two args (ptr + len)
            if matches!(p.ty, TypeRef::Bytes) {
                params.push(format!("{}_ptr: [*c]const u8", p.name));
                params.push(format!("{}_len: usize", p.name));
            } else {
                params.push(format!("{}: {ty}", p.name));
            }
        }

        // Methods with out_result wrapping: fallible OR infallible-complex.
        if method_needs_out_result(method) {
            params.push("out_result: ?*?[*c]u8".to_string());
        }
        // Fallible methods also get out_error.
        if method.error_type.is_some() {
            params.push("out_error: ?*?[*c]u8".to_string());
        }

        let params_str = params.join(", ");
        out.push_str(&crate::backends::zig::template_env::render(
            "trait_method_signature.jinja",
            minijinja::context! {
                method_snake => &method_snake,
                params_str => &params_str,
                ret => &ret,
            },
        ));
    }

    // free_string/free_user_data — always last; Rust calls free_string for callback-owned strings.
    out.push_str("    /// Called by the Rust runtime to release strings returned by callbacks.\n");
    out.push_str("    free_string: ?*const fn (ptr: [*c]u8) callconv(.c) void = null,\n");
    out.push('\n');

    // free_user_data — always last; called by Rust Drop to release the Zig-side handle.
    out.push_str("    /// Called by the Rust runtime when the bridge is dropped.\n");
    out.push_str("    /// Use this to release any Zig-side state held via `user_data`.\n");
    out.push_str("    free_user_data: ?*const fn (user_data: ?*anyopaque) callconv(.c) void = null,\n");

    out.push_str("};\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // Registration / unregistration shims (function-param binding only).
    //
    // When `bind_via = "options_field"` the bridge is wired to a field on a
    // configured options struct (e.g. `ConversionOptions.visitor`); there is
    // no `{prefix}_register_{trait}` / `{prefix}_unregister_{trait}` C
    // symbol to call. Emitting the shims unconditionally would produce code
    // that fails to link. Options-field bridges instead consume the C
    // vtable directly via a small `..._handle_from_vtable` helper (see
    // below).
    // -------------------------------------------------------------------------
    if matches!(bridge_cfg.bind_via, BridgeBinding::FunctionParam) {
        let c_register = format!("c.{prefix}_register_{snake}");
        let c_unregister = format!("c.{prefix}_unregister_{snake}");
        // C-side vtable type as cimported by Zig. cbindgen emits a struct of
        // the form `{UPPERCASE_PREFIX}{PascalPrefix}{TraitName}VTable` — the
        // Rust source carries the `{PascalPrefix}` prefix in its own struct
        // name (mirrors `FfiBridgeGenerator::vtable_name`) and cbindgen
        // prepends its configured uppercase `prefix`. Zig cimport surfaces
        // `typedef struct X` as `c.struct_X`.
        //
        // Concrete example for prefix `sample` + trait `Backend`:
        //   Rust source:   `pub struct SampleBackendVTable { … }`
        //   cbindgen out:  `typedef struct SAMPLESampleBackendVTable { … }`
        //   Zig cimport:   `c.struct_SAMPLESampleBackendVTable`
        let c_vtable_type = format!(
            "c.struct_{prefix_upper}{prefix_pascal}{trait_name}VTable",
            prefix_upper = prefix.to_uppercase(),
            prefix_pascal = prefix.to_upper_camel_case(),
        );

        out.push_str(&crate::backends::zig::template_env::render(
            "register_fn_doc1.jinja",
            minijinja::context! {
                trait_name => trait_name,
                snake => &snake,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "register_fn_signature.jinja",
            minijinja::context! {
                snake => &snake,
                trait_name => trait_name,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "register_fn_body.jinja",
            minijinja::context! {
                c_register => &c_register,
                c_vtable_type => &c_vtable_type,
            },
        ));
        out.push_str("}\n");
        out.push('\n');

        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_doc.jinja",
            minijinja::context! {
                trait_name => trait_name,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_signature.jinja",
            minijinja::context! {
                snake => &snake,
                error_type => error_type,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_body.jinja",
            minijinja::context! {
                c_unregister => &c_unregister,
                ffi_prefix => prefix,
                error_type => error_type,
            },
        ));
        out.push_str("}\n");
        out.push('\n');

        // ---------------------------------------------------------------
        // Clear wrapper (registry-wide reset).
        //
        // The Zig wrapper is named after `bridge_cfg.clear_fn` verbatim
        // (e.g. `clear_ocr_backends` — pluralised by convention to signal
        // multi-removal). The underlying C FFI symbol follows the singular
        // trait-snake naming used elsewhere in `sample_core-ffi`
        // (`sample_core_clear_ocr_backend`), so derive `c_clear` from
        // `trait_snake` rather than from `clear_fn`.
        // ---------------------------------------------------------------
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let c_clear = format!("c.{prefix}_clear_{snake}");

            out.push_str(&crate::backends::zig::template_env::render(
                "clear_fn_doc.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "clear_fn_signature.jinja",
                minijinja::context! {
                    clear_fn => clear_fn,
                    error_type => error_type,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "clear_fn_body.jinja",
                minijinja::context! {
                    c_clear => &c_clear,
                    ffi_prefix => prefix,
                    error_type => error_type,
                },
            ));
            out.push_str("}\n");
            out.push('\n');
        }
    } else {
        // Options-field binding: emit a vtable -> handle helper that wraps the
        // C callbacks struct into the trait-object handle expected by the
        // generated options-field setter. The upstream
        // FFI must export `{prefix}_{trait_snake}_handle_from_callbacks` with
        // the standard `extern "C" fn(*const T) -> *mut Handle` shape.
        let ctor_fn = format!("c.{prefix}_{snake}_handle_from_callbacks");
        if let Some(handle_type) = bridge_cfg.type_alias.as_deref() {
            let callbacks_type = format!(
                "c.{}{}VisitorCallbacks",
                prefix.to_uppercase(),
                prefix.to_upper_camel_case()
            );
            out.push_str(&crate::backends::zig::template_env::render(
                "trait_options_handle_from_vtable.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    handle_type => handle_type,
                    prefix => prefix,
                    snake => snake,
                    callbacks_type => callbacks_type,
                    ctor_fn => ctor_fn,
                },
            ));
        }
    }

    // -------------------------------------------------------------------------
    // Comptime vtable builder: make_{trait_snake}_vtable
    // -------------------------------------------------------------------------
    // INVARIANT: For every trait bridge, emit_make_vtable MUST be called
    // unconditionally. This ensures that e2e test fixtures referencing
    // `make_{trait_snake}_vtable(...)` will compile. Failure to emit this
    // builder causes "undeclared identifier" errors in zig e2e tests.
    // See: tests/backends_zig_snapshot_test.rs::trait_bridge_vtable_builder_coverage
    emit_make_vtable(
        trait_name,
        has_super_trait,
        trait_def,
        excluded_types,
        out,
        &bridge_cfg.ffi_skip_methods,
    );
}

// ---------------------------------------------------------------------------
// TraitBridgeGenerator implementation for the Zig backend
// ---------------------------------------------------------------------------

/// Zig-specific [`TraitBridgeGenerator`] implementation.
///
/// Carries the FFI symbol prefix (e.g., `"sample_core"`) used when deriving the
/// C symbol for `unregister_*` and `clear_*` wrappers.
///
/// The required trait methods that produce *Rust* source (`gen_sync_method_body`,
/// `gen_async_method_body`, `gen_constructor`, `gen_registration_fn`) return
/// empty strings because Zig bridge code is produced by the standalone
/// [`emit_trait_bridge`] free function, not the shared driver.
pub struct ZigTraitBridgeGenerator {
    /// FFI symbol prefix (e.g., `"sample_core"`).
    pub prefix: String,
}

impl ZigTraitBridgeGenerator {
    /// Construct a new generator for the given FFI symbol prefix.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self { prefix: prefix.into() }
    }
}

impl TraitBridgeGenerator for ZigTraitBridgeGenerator {
    // ------------------------------------------------------------------
    // Stub methods — Zig bridge code is emitted by `emit_trait_bridge`.
    // ------------------------------------------------------------------

    fn foreign_object_type(&self) -> &str {
        ""
    }

    fn bridge_imports(&self) -> Vec<String> {
        Vec::new()
    }

    fn gen_sync_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_async_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_constructor(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_registration_fn(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    // ------------------------------------------------------------------
    // Zig-specific overrides
    // ------------------------------------------------------------------

    /// Emit a Zig wrapper that calls `c.{prefix}_{unregister_fn}(name, out_error)`.
    ///
    /// Returns an empty string when `spec.bridge_config.unregister_fn` is `None`.
    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let c_unregister = format!("c.{}_{}", self.prefix, unregister_fn);

        let mut out = String::new();
        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_doc.jinja",
            minijinja::context! {
                trait_name => spec.trait_def.name.as_str(),
            },
        ));
        // Emit the signature directly: the configured `unregister_fn` is the
        // complete Zig function name, not just the trait-snake suffix.
        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_configured_signature.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "unregister_fn_body.jinja",
            minijinja::context! {
                c_unregister => &c_unregister,
            },
        ));
        out.push_str("}\n");
        out
    }

    /// Emit a Zig wrapper that calls `c.{prefix}_{clear_fn}(out_error)`.
    ///
    /// Returns an empty string when `spec.bridge_config.clear_fn` is `None`.
    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let c_clear = format!("c.{}_{}", self.prefix, clear_fn);

        let mut out = String::new();
        out.push_str(&crate::backends::zig::template_env::render(
            "clear_fn_doc.jinja",
            minijinja::context! {
                trait_name => spec.trait_def.name.as_str(),
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "clear_fn_signature.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
            },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "clear_fn_body.jinja",
            minijinja::context! {
                c_clear => &c_clear,
            },
        ));
        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            fields: Vec::<FieldDef>::new(),
            methods,
            is_opaque: true,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async: false,
            is_static: false,
            error_type: error_type.map(|s| s.to_string()),
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
        }
    }

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
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
        }
    }

    fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: super_trait.map(|s| s.to_string()),
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    #[test]
    fn trait_vtable_includes_free_string_and_status_lifecycle_callbacks() {
        let trait_def = make_trait_def("Backend", vec![make_method("run", vec![], TypeRef::Unit, None)]);
        let bridge_cfg = make_bridge_cfg("Backend", Some("Plugin"));
        let mut out = String::new();

        emit_trait_bridge(
            "sample",
            "SampleError",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        assert!(out.contains("free_string: ?*const fn (ptr: [*c]u8) callconv(.c) void = null"));
        assert!(out.contains("name_fn: ?*const fn (user_data: ?*anyopaque, out_name: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null"));
        assert!(out.contains("version_fn: ?*const fn (user_data: ?*anyopaque, out_version: ?*?[*c]u8, out_error: ?*?[*c]u8) callconv(.c) i32 = null"));
    }

    #[test]
    fn single_method_trait_emits_vtable_and_register() {
        let trait_def = make_trait_def(
            "Validator",
            vec![make_method(
                "validate",
                vec![make_param("input", TypeRef::String)],
                TypeRef::Primitive(PrimitiveType::Bool),
                None,
            )],
        );
        let bridge_cfg = make_bridge_cfg("Validator", None);

        let mut out = String::new();
        emit_trait_bridge(
            "demo",
            "error",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        // Vtable struct
        assert!(
            out.contains("pub const IValidator = extern struct {"),
            "missing vtable struct: {out}"
        );
        // Method slot present
        assert!(out.contains("validate:"), "missing validate slot: {out}");
        // user_data first arg
        assert!(out.contains("user_data: ?*anyopaque"), "missing user_data: {out}");
        // callconv(.c) present
        assert!(out.contains("callconv(.c)"), "missing callconv: {out}");
        // free_user_data slot
        assert!(out.contains("free_user_data:"), "missing free_user_data: {out}");
        // Registration shim
        assert!(out.contains("pub fn register_validator("), "missing register fn: {out}");
        assert!(out.contains("c.demo_register_validator("), "wrong C symbol: {out}");
        // Unregistration shim
        assert!(
            out.contains("pub fn unregister_validator("),
            "missing unregister fn: {out}"
        );
        assert!(
            out.contains("c.demo_unregister_validator("),
            "wrong unregister C symbol: {out}"
        );
        // No plugin lifecycle when no super_trait
        assert!(
            !out.contains("name_fn:"),
            "should not emit name_fn without super_trait: {out}"
        );
    }

    #[test]
    fn emit_trait_bridge_emits_clear_fn_when_configured() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![make_method(
                "process",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                Some("OcrError"),
            )],
        );
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", Some("sample_crate::plugins::Plugin"));
        bridge_cfg.clear_fn = Some("clear_ocr_backends".to_string());

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        assert!(
            out.contains("pub fn clear_ocr_backends() SampleCrateError!void"),
            "missing clear_ocr_backends signature: {out}"
        );
        // C symbol uses the singular trait-snake suffix to match sample_core-ffi naming.
        assert!(
            out.contains("c.sample_crate_clear_ocr_backend(&_out_error)"),
            "wrong C symbol target for clear wrapper: {out}"
        );
        // Doc comment present.
        assert!(
            out.contains("/// Remove ALL registered `OcrBackend` plugins"),
            "missing clear doc comment: {out}"
        );
    }

    #[test]
    fn emit_trait_bridge_omits_clear_fn_when_not_configured() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![make_method(
                "process",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                Some("OcrError"),
            )],
        );
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("sample_crate::plugins::Plugin"));
        // clear_fn left as None.

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        assert!(
            !out.contains("pub fn clear_"),
            "should not emit any clear_* fn when clear_fn is None: {out}"
        );
    }

    #[test]
    fn multi_method_trait_with_super_trait_emits_lifecycle_slots() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![
                make_method(
                    "process_image",
                    vec![
                        make_param("image_bytes", TypeRef::Bytes),
                        make_param("config", TypeRef::String),
                    ],
                    TypeRef::String,
                    Some("OcrError"),
                ),
                make_method(
                    "supports_language",
                    vec![make_param("lang", TypeRef::String)],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    None,
                ),
            ],
        );
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("sample_crate::plugins::Plugin"));

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        // Struct name
        assert!(
            out.contains("pub const IOcrBackend = extern struct {"),
            "missing vtable: {out}"
        );
        // Plugin lifecycle slots emitted
        assert!(out.contains("name_fn:"), "missing name_fn: {out}");
        assert!(out.contains("version_fn:"), "missing version_fn: {out}");
        assert!(out.contains("initialize_fn:"), "missing initialize_fn: {out}");
        assert!(out.contains("shutdown_fn:"), "missing shutdown_fn: {out}");
        // Trait method slots
        assert!(out.contains("process_image:"), "missing process_image slot: {out}");
        assert!(
            out.contains("supports_language:"),
            "missing supports_language slot: {out}"
        );
        // Bytes param expands to ptr + len
        assert!(out.contains("image_bytes_ptr:"), "missing bytes ptr expansion: {out}");
        assert!(out.contains("image_bytes_len:"), "missing bytes len expansion: {out}");
        // Fallible method gets out_error
        assert!(
            out.contains("out_error:"),
            "missing out_error for fallible method: {out}"
        );
        // C symbols use sample_core prefix
        assert!(
            out.contains("c.sample_crate_register_ocr_backend("),
            "wrong register symbol: {out}"
        );
        assert!(
            out.contains("c.sample_crate_unregister_ocr_backend("),
            "wrong unregister symbol: {out}"
        );
        // Registration shim signature
        assert!(
            out.contains("pub fn register_ocr_backend("),
            "missing register_ocr_backend fn: {out}"
        );
    }

    // -----------------------------------------------------------------
    // make_*_vtable tests
    // -----------------------------------------------------------------

    #[test]
    fn make_vtable_emits_comptime_function_and_thunk() {
        let trait_def = make_trait_def(
            "Validator",
            vec![make_method(
                "validate",
                vec![make_param("input", TypeRef::String)],
                TypeRef::Primitive(PrimitiveType::Bool),
                None,
            )],
        );
        let bridge_cfg = make_bridge_cfg("Validator", None);

        let mut out = String::new();
        emit_trait_bridge(
            "demo",
            "error",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        // Helper function declaration
        assert!(
            out.contains("pub fn make_validator_vtable(comptime T: type, instance: *T)"),
            "missing make_validator_vtable: {out}"
        );
        // Returns the vtable type
        assert!(out.contains("IValidator{"), "missing vtable literal: {out}");
        // Thunk casts user_data
        assert!(out.contains("@ptrCast(@alignCast(ud))"), "missing @ptrCast cast: {out}");
        // callconv(.c) in thunk
        assert!(out.contains("callconv(.c)"), "missing callconv(.c) in thunk: {out}");
        // validate thunk field
        assert!(out.contains(".validate ="), "missing .validate thunk field: {out}");
        // free_user_data thunk
        assert!(
            out.contains(".free_user_data ="),
            "missing .free_user_data thunk: {out}"
        );
        // No lifecycle stubs without super_trait
        assert!(
            !out.contains(".name_fn ="),
            "must not emit .name_fn without super_trait: {out}"
        );
    }

    #[test]
    fn make_vtable_with_super_trait_emits_lifecycle_stubs() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("sample_crate::Plugin"));

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        assert!(
            out.contains("pub fn make_ocr_backend_vtable(comptime T: type, instance: *T)"),
            "missing make_ocr_backend_vtable: {out}"
        );
        assert!(out.contains(".name_fn ="), "missing .name_fn stub: {out}");
        assert!(out.contains(".version_fn ="), "missing .version_fn stub: {out}");
        assert!(out.contains(".initialize_fn ="), "missing .initialize_fn stub: {out}");
        assert!(out.contains(".shutdown_fn ="), "missing .shutdown_fn stub: {out}");
    }

    #[test]
    fn make_vtable_bytes_param_passes_c_pointer_in_thunk() {
        let trait_def = make_trait_def(
            "Processor",
            vec![make_method(
                "process",
                vec![make_param("data", TypeRef::Bytes)],
                TypeRef::Unit,
                None,
            )],
        );

        let mut out = String::new();
        emit_make_vtable(
            "Processor",
            false,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
            &[],
        );

        // Thunk receives ptr+len params
        assert!(out.contains("data_ptr: [*c]const u8"), "missing data_ptr param: {out}");
        assert!(out.contains("data_len: usize"), "missing data_len param: {out}");
        // The Zig vtable ABI passes the raw C pointer through; the len is discarded.
        assert!(out.contains("_ = data_len;"), "thunk must discard the len param: {out}");
        // Thunk calls self.process with the C pointer (not a reconstructed slice).
        assert!(
            out.contains("self.process(data_ptr);"),
            "thunk must call self.process with the C pointer: {out}"
        );
    }

    #[test]
    fn make_vtable_fallible_method_returns_i32_error_code() {
        let trait_def = make_trait_def(
            "Parser",
            vec![make_method("parse", vec![], TypeRef::Unit, Some("ParseError"))],
        );
        let bridge_cfg = make_bridge_cfg("Parser", None);

        let mut out = String::new();
        emit_trait_bridge(
            "demo",
            "error",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        // Thunk returns i32 (fallible → i32 return)
        assert!(
            out.contains("callconv(.c) i32"),
            "fallible thunk must return i32: {out}"
        );
        // Returns 0 on success
        assert!(out.contains("return 0;"), "must return 0 on success: {out}");
        // Returns 1 on error
        assert!(out.contains("return 1;"), "must return 1 on error: {out}");
        // Error branch writes to out_error
        assert!(out.contains("out_error"), "must write to out_error: {out}");
    }

    #[test]
    fn make_vtable_primitive_return_passes_through() {
        let trait_def = make_trait_def(
            "Counter",
            vec![make_method(
                "count",
                vec![],
                TypeRef::Primitive(PrimitiveType::I32),
                None,
            )],
        );
        let bridge_cfg = make_bridge_cfg("demo", None);

        let mut out = String::new();
        emit_trait_bridge(
            "demo",
            "error",
            &bridge_cfg,
            &trait_def,
            &std::collections::HashSet::new(),
            &mut out,
        );

        // Infallible primitive method: thunk returns the value directly
        assert!(
            out.contains("return self.count()"),
            "primitive return must be forwarded directly: {out}"
        );
    }

    // -----------------------------------------------------------------
    // ZigTraitBridgeGenerator tests
    // -----------------------------------------------------------------

    fn make_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
        use crate::codegen::generators::trait_bridge::TraitBridgeSpec;
        use std::collections::HashMap;
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "sample_crate",
            wrapper_prefix: "Zig",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
            error_type: "SampleCrateError".to_string(),
            error_constructor: "SampleCrateError::msg({msg})".to_string(),
        }
    }

    #[test]
    fn gen_unregistration_fn_emits_wrapper_when_configured() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.unregister_fn = Some("unregister_ocr_backend".to_string());

        let generator = ZigTraitBridgeGenerator::new("sample_crate");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_unregistration_fn(&spec);

        assert!(!out.is_empty(), "expected non-empty output when unregister_fn is set");
        assert!(
            out.contains("pub fn unregister_ocr_backend("),
            "wrong function name: {out}"
        );
        assert!(
            out.contains("c.sample_crate_unregister_ocr_backend("),
            "wrong C symbol: {out}"
        );
        assert!(
            out.contains("out_error: ?*?[*c]u8") || out.contains("out_error"),
            "missing out_error param: {out}"
        );
        assert!(out.contains("return "), "missing return statement: {out}");
        assert!(out.ends_with("}\n"), "missing closing brace: {out}");
    }

    #[test]
    fn gen_unregistration_fn_returns_empty_when_not_configured() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let bridge_cfg = make_bridge_cfg("OcrBackend", None); // unregister_fn is None

        let generator = ZigTraitBridgeGenerator::new("sample_crate");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_unregistration_fn(&spec);

        assert!(
            out.is_empty(),
            "expected empty output when unregister_fn is None, got: {out}"
        );
    }

    #[test]
    fn gen_clear_fn_emits_wrapper_when_configured() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.clear_fn = Some("clear_ocr_backends".to_string());

        let generator = ZigTraitBridgeGenerator::new("sample_crate");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_clear_fn(&spec);

        assert!(!out.is_empty(), "expected non-empty output when clear_fn is set");
        assert!(out.contains("pub fn clear_ocr_backends("), "wrong function name: {out}");
        assert!(
            out.contains("c.sample_crate_clear_ocr_backends("),
            "wrong C symbol: {out}"
        );
        assert!(
            out.contains("out_error: ?*?[*c]u8") || out.contains("out_error"),
            "missing out_error param: {out}"
        );
        assert!(out.contains("return "), "missing return statement: {out}");
        assert!(out.ends_with("}\n"), "missing closing brace: {out}");
    }

    #[test]
    fn gen_clear_fn_returns_empty_when_not_configured() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let bridge_cfg = make_bridge_cfg("OcrBackend", None); // clear_fn is None

        let generator = ZigTraitBridgeGenerator::new("sample_crate");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_clear_fn(&spec);

        assert!(
            out.is_empty(),
            "expected empty output when clear_fn is None, got: {out}"
        );
    }

    #[test]
    fn gen_unregistration_fn_uses_snake_case_function_name_verbatim() {
        // The configured `unregister_fn` name is used as-is (not re-derived from the trait).
        let trait_def = make_trait_def("DocumentExtractor", vec![]);
        let mut bridge_cfg = make_bridge_cfg("DocumentExtractor", None);
        bridge_cfg.unregister_fn = Some("unregister_extractor".to_string());

        let generator = ZigTraitBridgeGenerator::new("demo");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_unregistration_fn(&spec);

        assert!(
            out.contains("pub fn unregister_extractor("),
            "must use configured fn name verbatim: {out}"
        );
        assert!(
            out.contains("c.demo_unregister_extractor("),
            "must use configured fn name in C symbol: {out}"
        );
    }

    #[test]
    fn gen_clear_fn_uses_configured_fn_name_verbatim() {
        let trait_def = make_trait_def("DocumentExtractor", vec![]);
        let mut bridge_cfg = make_bridge_cfg("DocumentExtractor", None);
        bridge_cfg.clear_fn = Some("clear_all_extractors".to_string());

        let generator = ZigTraitBridgeGenerator::new("demo");
        let spec = make_spec(&trait_def, &bridge_cfg);
        let out = generator.gen_clear_fn(&spec);

        assert!(
            out.contains("pub fn clear_all_extractors("),
            "must use configured fn name verbatim: {out}"
        );
        assert!(
            out.contains("c.demo_clear_all_extractors("),
            "must use configured fn name in C symbol: {out}"
        );
    }

    #[test]
    fn vtable_preserves_named_types_for_c_abi_compatibility() {
        // Test that VTable signatures do NOT substitute excluded types.
        // The vtable is a C ABI struct and must preserve the exact C types.
        let mut excluded = std::collections::HashSet::new();
        excluded.insert("InternalDocument".to_string());
        excluded.insert("ExtractionResult".to_string());

        let trait_def = make_trait_def(
            "DocumentExtractor",
            vec![
                make_method(
                    "extract_bytes",
                    vec![
                        make_param("content", TypeRef::Bytes),
                        make_param("mime_type", TypeRef::String),
                    ],
                    TypeRef::Named("InternalDocument".to_string()),
                    Some("SampleCrateError"),
                ),
                make_method(
                    "process_result",
                    vec![make_param("result", TypeRef::Named("ExtractionResult".to_string()))],
                    TypeRef::Unit,
                    None,
                ),
            ],
        );
        let bridge_cfg = make_bridge_cfg("DocumentExtractor", None);

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &excluded,
            &mut out,
        );

        // VTable struct must be present with the trait name
        assert!(
            out.contains("pub const IDocumentExtractor = extern struct {"),
            "missing vtable struct"
        );

        // Method slots must NOT have type substitution — they should use C ABI types
        // ([*c]const u8, i32, etc.) not Zig types. The excluded types should appear
        // as C pointers, not as Json or other substitutions.
        assert!(
            out.contains("extract_bytes:") && out.contains("callconv(.c)"),
            "extract_bytes method slot missing"
        );
        assert!(out.contains("process_result:"), "process_result method slot missing");

        // Bytes param expands to ptr + len in vtable signature
        assert!(
            out.contains("content_ptr: [*c]const u8") && out.contains("content_len: usize"),
            "Bytes param should expand to ptr+len in C ABI"
        );

        // The result param should be [*c]const u8 (C string), not the Zig type
        // ExtractionResult or Json or any substitution
        assert!(
            out.contains("result: [*c]const u8"),
            "Named types in vtable should map to [*c]const u8, not be substituted"
        );

        // Return type should be i32 (error code) for fallible methods, not substituted
        let has_fallible_return = out.contains("callconv(.c) i32");
        assert!(has_fallible_return, "fallible method should return i32 for error code");
    }

    #[test]
    fn make_vtable_thunks_preserve_c_abi_types() {
        // Test that thunk function signatures preserve C ABI types.
        let mut excluded = std::collections::HashSet::new();
        excluded.insert("InternalDocument".to_string());

        let trait_def = make_trait_def(
            "Renderer",
            vec![make_method(
                "render",
                vec![make_param("doc", TypeRef::Named("InternalDocument".to_string()))],
                TypeRef::Bytes,
                Some("SampleCrateError"),
            )],
        );
        let bridge_cfg = make_bridge_cfg("Renderer", None);

        let mut out = String::new();
        emit_trait_bridge(
            "sample_crate",
            "SampleCrateError",
            &bridge_cfg,
            &trait_def,
            &excluded,
            &mut out,
        );

        // make_renderer_vtable should exist
        assert!(
            out.contains("pub fn make_renderer_vtable(comptime T: type, instance: *T)"),
            "make_renderer_vtable helper missing"
        );

        // Thunk for render method should use C ABI types in its signature
        assert!(out.contains(".render ="), "render thunk field missing");

        // Thunk should have callconv(.c) and i32 return for the fallible method
        assert!(
            out.contains("callconv(.c) i32"),
            "thunk should return i32 for error code"
        );

        // The parameter should be [*c]const u8 (C string from doc param)
        assert!(
            out.contains("doc: [*c]const u8"),
            "thunk param should be C ABI type, not substituted"
        );
        assert!(
            !out.contains("unreachable"),
            "generated vtable helpers must not use unreachable stubs: {out}"
        );
        // Complex fallible returns serialize to JSON ([]u8). When JSON serialization
        // is not yet implemented, the thunk returns null as a placeholder.
        // The vtable still compiles, allowing e2e tests to run (they'll exercise
        // the null path and validate error handling).
        assert!(
            out.contains("ptr.* = null") || out.contains("ptr.* = ."),
            "complex fallible vtable returns must return a safe placeholder: {out}"
        );
    }
}
