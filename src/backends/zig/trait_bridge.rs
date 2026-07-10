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
        return true;
    }
    if method.error_type.is_none() && !matches!(method.return_type, TypeRef::Unit | TypeRef::Primitive(_)) {
        return true;
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
    if method_needs_out_result(method) {
        params.push(("out_result".to_string(), "?*?[*c]u8".to_string()));
    }
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

    for method in &trait_def.methods {
        if ffi_skip_methods.iter().any(|skip| skip == &method.name) {
            continue;
        }

        let method_snake = method.name.to_snake_case();
        let c_params = vtable_c_params(method);
        let ret = vtable_return_type(method);

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

        out.push_str("                const self: *T = @ptrCast(@alignCast(ud));\n");

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

        let ok_binding = if method.params.iter().any(|p| p.name == "value") {
            "ok_value"
        } else {
            "value"
        };

        let needs_out_result = method_needs_out_result(method);
        let has_error_type = method.error_type.is_some();
        let is_infallible_complex = !has_error_type && needs_out_result;

        if has_error_type {
            let has_result_out = !matches!(method.return_type, TypeRef::Unit);
            out.push_str(&crate::backends::zig::template_env::render(
                "thunk_fn_signature.jinja",
                minijinja::context! {
                    method_snake => &method_snake,
                    args_str => &args_str,
                    ok_binding => &ok_binding,
                },
            ));
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
                        out.push_str(&crate::backends::zig::template_env::render(
                            "thunk_if_fallible.jinja",
                            minijinja::context! {
                                ok_binding => &ok_binding,
                                is_string_like => true,
                            },
                        ));
                        success_path_diverges = true;
                    }
                }
            } else {
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
            out.push_str("                const ");
            out.push_str(ok_binding);
            out.push_str(" = self.");
            out.push_str(&method_snake);
            out.push('(');
            out.push_str(&args_str);
            out.push_str(");\n");
            out.push_str("                if (out_result) |ptr| {\n");
            out.push_str("                    ptr.* = @constCast(");
            out.push_str(ok_binding);
            out.push_str(");\n");
            out.push_str("                }\n");
            out.push_str("                return 0;\n");
        } else {
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

    let _excluded_strs: HashSet<&str> = excluded_types.iter().map(|s| s.as_str()).collect();

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

    for method in &trait_def.methods {
        if bridge_cfg.ffi_skip_methods.iter().any(|skip| skip == &method.name) {
            continue;
        }

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

        let mut params = vec!["user_data: ?*anyopaque".to_string()];
        for p in &method.params {
            let ty = vtable_param_type(&p.ty);
            if matches!(p.ty, TypeRef::Bytes) {
                params.push(format!("{}_ptr: [*c]const u8", p.name));
                params.push(format!("{}_len: usize", p.name));
            } else {
                params.push(format!("{}: {ty}", p.name));
            }
        }

        if method_needs_out_result(method) {
            params.push("out_result: ?*?[*c]u8".to_string());
        }
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

    out.push_str("    /// Called by the Rust runtime to release strings returned by callbacks.\n");
    out.push_str("    free_string: ?*const fn (ptr: [*c]u8) callconv(.c) void = null,\n");
    out.push('\n');

    out.push_str("    /// Called by the Rust runtime when the bridge is dropped.\n");
    out.push_str("    /// Use this to release any Zig-side state held via `user_data`.\n");
    out.push_str("    free_user_data: ?*const fn (user_data: ?*anyopaque) callconv(.c) void = null,\n");

    out.push_str("};\n");
    out.push('\n');

    if matches!(bridge_cfg.bind_via, BridgeBinding::FunctionParam) {
        let c_register = format!("c.{prefix}_register_{snake}");
        let c_unregister = format!("c.{prefix}_unregister_{snake}");
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

    emit_make_vtable(
        trait_name,
        has_super_trait,
        trait_def,
        excluded_types,
        out,
        &bridge_cfg.ffi_skip_methods,
    );
}

/// Zig-specific [`TraitBridgeGenerator`] implementation.
///
/// Carries the FFI symbol prefix (e.g., `"sample_core"`) used when deriving the
/// C symbol for `unregister_*` and `clear_*` wrappers.
///
/// The required trait methods that produce *Rust* source (`gen_sync_method_body`,
/// `gen_async_method_body`, `gen_constructor`, `gen_registration_fn`) return
/// empty strings because Zig bridge code is produced by the standalone
/// `emit_trait_bridge` free function, not the shared driver.
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
mod tests;
