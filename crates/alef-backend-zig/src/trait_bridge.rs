//! Zig trait-bridge code generation.
//!
//! Emits one Zig extern struct (vtable) and one registration wrapper function
//! per configured `[[trait_bridges]]` entry.  The Zig consumer fills in the
//! struct with `callconv(.C)` function pointers and calls `register_*`.
//!
//! # C symbol convention
//!
//! The generated `register_{trait_snake}` shim calls
//! `c.{prefix}_register_{trait_snake}` — the symbol exposed by the
//! `kreuzberg-ffi` C layer (pattern: `{crate_prefix}_register_{trait_snake}`).
//! If the actual symbol differs, override the generated call site.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

/// Zig type string to use for a vtable slot parameter or return type.
///
/// All string/complex types collapse to `[*c]const u8` (C string pointer) since
/// the vtable slots use the raw C ABI — not the Zig-friendly wrapper layer.
fn vtable_param_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType::*;
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

/// Zig return type for a vtable slot.
///
/// Fallible methods always return `i32` (0 = success, non-zero = error).
/// Unit infallible methods return `void`.  Other infallible returns use the
/// primitive mapping.
fn vtable_return_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
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
    if method.error_type.is_some() {
        if !matches!(method.return_type, TypeRef::Unit) {
            params.push(("out_result".to_string(), "?*?[*c]u8".to_string()));
        }
        params.push(("out_error".to_string(), "?*?[*c]u8".to_string()));
    } else if !matches!(method.return_type, TypeRef::Unit) {
        params.push(("out_result".to_string(), "?*?[*c]u8".to_string()));
    }
    params
}

/// Emit a `make_{trait_snake}_vtable(comptime T: type, instance: *T) I{Trait}` helper.
///
/// The helper builds `callconv(.C)` thunks for every vtable slot so the consumer
/// only needs to write plain Zig methods on their type.
///
/// # Limitations
///
/// - Methods returning non-unit values through `out_result` use `unreachable` for
///   the conversion path when the type cannot be expressed as a direct C primitive
///   (complex types are documented as requiring manual implementation).
/// - Lifecycle slots (`name_fn`, `version_fn`, `initialize_fn`, `shutdown_fn`) are
///   emitted with `unreachable` bodies as stubs — the consumer overrides the
///   relevant field in the returned vtable if needed.
pub fn emit_make_vtable(trait_name: &str, has_super_trait: bool, trait_def: &TypeDef, out: &mut String) {
    let snake = trait_snake(trait_name);

    out.push_str(&format!(
        "/// Build an `I{trait_name}` vtable for a concrete Zig type `T`.\n"
    ));
    out.push_str("///\n");
    out.push_str(&format!(
        "/// `T` must implement every method of `{trait_name}` as a plain Zig function.\n"
    ));
    out.push_str("/// Each slot is wrapped in a `callconv(.C)` thunk that casts `user_data`\n");
    out.push_str("/// back to `*T` and forwards the call.\n");
    out.push_str("///\n");
    out.push_str("/// # Usage\n");
    out.push_str("/// ```zig\n");
    out.push_str("/// const vtable = make_{snake}_vtable(MyType, &my_instance);\n");
    out.push_str(&format!(
        "/// _ = register_{snake}(\"my-impl\", vtable, &my_instance, &out_error);\n"
    ));
    out.push_str("/// ```\n");
    out.push_str(&format!(
        "pub fn make_{snake}_vtable(comptime T: type, instance: *T) I{trait_name} {{\n"
    ));
    out.push_str("    _ = instance; // instance is passed as user_data by the caller\n");
    out.push_str(&format!("    return I{trait_name}{{\n"));

    // Lifecycle stubs when super_trait is present
    if has_super_trait {
        out.push_str("        .name_fn = struct {\n");
        out.push_str(
            "            fn thunk(user_data: ?*anyopaque, out_name: ?*?[*c]u8) callconv(.C) void {\n",
        );
        out.push_str("                _ = user_data;\n");
        out.push_str("                _ = out_name;\n");
        out.push_str("                unreachable; // override .name_fn in the returned vtable\n");
        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push_str("\n");

        out.push_str("        .version_fn = struct {\n");
        out.push_str(
            "            fn thunk(user_data: ?*anyopaque, out_version: ?*?[*c]u8) callconv(.C) void {\n",
        );
        out.push_str("                _ = user_data;\n");
        out.push_str("                _ = out_version;\n");
        out.push_str("                unreachable; // override .version_fn in the returned vtable\n");
        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push_str("\n");

        out.push_str("        .initialize_fn = struct {\n");
        out.push_str(
            "            fn thunk(user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.C) i32 {\n",
        );
        out.push_str("                _ = user_data;\n");
        out.push_str("                _ = out_error;\n");
        out.push_str("                return 0;\n");
        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push_str("\n");

        out.push_str("        .shutdown_fn = struct {\n");
        out.push_str(
            "            fn thunk(user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.C) i32 {\n",
        );
        out.push_str("                _ = user_data;\n");
        out.push_str("                _ = out_error;\n");
        out.push_str("                return 0;\n");
        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push_str("\n");
    }

    // Per-method thunks
    for method in &trait_def.methods {
        let method_snake = method.name.to_snake_case();
        let c_params = vtable_c_params(method);
        let ret = vtable_return_type(method);

        // Build the thunk parameter list string
        let params_str = c_params
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");

        out.push_str(&format!("        .{method_snake} = struct {{\n"));
        out.push_str(&format!(
            "            fn thunk({params_str}) callconv(.C) {ret} {{\n"
        ));

        // Cast user_data to *T
        out.push_str("                const self: *T = @ptrCast(@alignCast(ud));\n");

        // Reconstruct Bytes slices and build forwarding arg list
        let mut call_args: Vec<String> = Vec::new();
        for p in &method.params {
            if matches!(p.ty, TypeRef::Bytes) {
                out.push_str(&format!(
                    "                const {}_slice = {}_ptr[0..{}_len];\n",
                    p.name, p.name, p.name
                ));
                call_args.push(format!("{}_slice", p.name));
            } else {
                call_args.push(p.name.clone());
            }
        }

        let args_str = call_args.join(", ");

        if method.error_type.is_some() {
            // Fallible method: call returns error union, write out_result/out_error
            let has_result_out = !matches!(method.return_type, TypeRef::Unit);
            out.push_str(&format!(
                "                if (self.{method_snake}({args_str})) |result| {{\n"
            ));
            if has_result_out {
                // Write result via out_result pointer — for complex types this is unreachable
                match &method.return_type {
                    TypeRef::Primitive(_) | TypeRef::Unit => {
                        out.push_str("                    if (out_result) |ptr| ptr.* = result;\n");
                    }
                    _ => {
                        // String/Bytes/complex: cannot safely convert without allocator context
                        out.push_str(
                            "                    _ = result; _ = out_result; unreachable; // complex return: implement manually\n",
                        );
                    }
                }
            }
            out.push_str("                    return 0;\n");
            out.push_str("                } else |err| {\n");
            out.push_str("                    _ = err;\n");
            out.push_str(
                "                    if (out_error) |ptr| ptr.* = null; // caller checks error code\n",
            );
            out.push_str("                    return 1;\n");
            out.push_str("                }\n");
        } else {
            match &method.return_type {
                TypeRef::Unit => {
                    out.push_str(&format!("                self.{method_snake}({args_str});\n"));
                }
                TypeRef::Primitive(_) => {
                    out.push_str(&format!(
                        "                return self.{method_snake}({args_str});\n"
                    ));
                }
                _ => {
                    // Non-unit infallible non-primitive: pass through (e.g., [*c]const u8)
                    out.push_str(&format!(
                        "                return self.{method_snake}({args_str});\n"
                    ));
                }
            }
        }

        out.push_str("            }\n");
        out.push_str("        }.thunk,\n");
        out.push_str("\n");
    }

    // free_user_data stub — does nothing by default; caller overrides if needed
    out.push_str("        .free_user_data = struct {\n");
    out.push_str(
        "            fn thunk(user_data: ?*anyopaque) callconv(.C) void {\n",
    );
    out.push_str("                _ = user_data;\n");
    out.push_str("            }\n");
    out.push_str("        }.thunk,\n");

    out.push_str("    };\n");
    out.push_str("}\n");
}

/// Emit the vtable extern struct and registration shim for a single trait bridge.
///
/// `prefix` is the C FFI prefix (e.g., `"kreuzberg"`).
/// `bridge_cfg` is the trait bridge configuration entry.
/// `trait_def` is the IR type definition for the trait (must have `is_trait = true`).
/// `out` is the output buffer to append to.
pub fn emit_trait_bridge(prefix: &str, bridge_cfg: &TraitBridgeConfig, trait_def: &TypeDef, out: &mut String) {
    let trait_name = &trait_def.name;
    let snake = trait_snake(trait_name);
    let has_super_trait = bridge_cfg.super_trait.is_some();

    // -------------------------------------------------------------------------
    // Vtable struct: I{Trait}
    // -------------------------------------------------------------------------
    out.push_str(&format!("/// Vtable for a Zig implementation of the `{trait_name}` trait.\n"));
    out.push_str("/// Fill each function pointer, then pass this struct to the corresponding\n");
    out.push_str(&format!("/// `register_{snake}` function to register your implementation.\n"));
    out.push_str(&format!("pub const I{trait_name} = extern struct {{\n"));

    // Plugin lifecycle slots — always present when a super_trait is configured.
    if has_super_trait {
        out.push_str("    /// Return the plugin name into `out_name` (heap-allocated, caller frees).\n");
        out.push_str("    name_fn: ?*const fn (user_data: ?*anyopaque, out_name: ?*?[*c]u8) callconv(.C) void = null,\n");
        out.push_str("\n");

        out.push_str("    /// Return the plugin version into `out_version` (heap-allocated, caller frees).\n");
        out.push_str("    version_fn: ?*const fn (user_data: ?*anyopaque, out_version: ?*?[*c]u8) callconv(.C) void = null,\n");
        out.push_str("\n");

        out.push_str("    /// Initialise the plugin; return 0 on success, non-zero on error.\n");
        out.push_str(
            "    initialize_fn: ?*const fn (user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.C) i32 = null,\n",
        );
        out.push_str("\n");

        out.push_str("    /// Shut down the plugin; return 0 on success, non-zero on error.\n");
        out.push_str(
            "    shutdown_fn: ?*const fn (user_data: ?*anyopaque, out_error: ?*?[*c]u8) callconv(.C) i32 = null,\n",
        );
        out.push_str("\n");
    }

    // Trait method slots
    for method in &trait_def.methods {
        if !method.doc.is_empty() {
            for line in method.doc.lines() {
                out.push_str(&format!("    /// {line}\n"));
            }
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

        // Fallible methods get out-result and out-error pointers.
        if method.error_type.is_some() {
            if !matches!(method.return_type, TypeRef::Unit) {
                params.push("out_result: ?*?[*c]u8".to_string());
            }
            params.push("out_error: ?*?[*c]u8".to_string());
        } else if !matches!(method.return_type, TypeRef::Unit) {
            // Infallible non-void: return via out_result too for uniformity
            params.push("out_result: ?*?[*c]u8".to_string());
        }

        let params_str = params.join(", ");
        out.push_str(&format!(
            "    {method_snake}: ?*const fn ({params_str}) callconv(.C) {ret} = null,\n"
        ));
        out.push('\n');
    }

    // free_user_data — always last; called by Rust Drop to release the Zig-side handle.
    out.push_str("    /// Called by the Rust runtime when the bridge is dropped.\n");
    out.push_str("    /// Use this to release any Zig-side state held via `user_data`.\n");
    out.push_str(
        "    free_user_data: ?*const fn (user_data: ?*anyopaque) callconv(.C) void = null,\n",
    );

    out.push_str("};\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // Registration shim: register_{trait_snake}
    // -------------------------------------------------------------------------
    let c_register = format!("c.{prefix}_register_{snake}");
    let c_unregister = format!("c.{prefix}_unregister_{snake}");

    out.push_str(&format!(
        "/// Register a `{trait_name}` implementation with the Rust runtime.\n"
    ));
    out.push_str("///\n");
    out.push_str("/// `name`     — null-terminated plugin name.\n");
    out.push_str("/// `vtable`   — filled `I{trait_name}` struct with all required function pointers.\n");
    out.push_str("/// `user_data`— opaque pointer passed back as the first argument of every vtable call.\n");
    out.push_str("///\n");
    out.push_str("/// Returns 0 on success; non-zero on failure (error text written to `out_error`).\n");
    out.push_str(&format!(
        "pub fn register_{snake}(name: [*c]const u8, vtable: I{trait_name}, user_data: ?*anyopaque, out_error: ?*?[*c]u8) i32 {{\n"
    ));
    out.push_str(&format!("    return {c_register}(name, vtable, user_data, out_error);\n"));
    out.push_str("}\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // Unregistration shim: unregister_{trait_snake}
    // -------------------------------------------------------------------------
    out.push_str(&format!(
        "/// Unregister a previously registered `{trait_name}` implementation by name.\n"
    ));
    out.push_str("///\n");
    out.push_str("/// Returns 0 on success; non-zero on failure.\n");
    out.push_str(&format!(
        "pub fn unregister_{snake}(name: [*c]const u8, out_error: ?*?[*c]u8) i32 {{\n"
    ));
    out.push_str(&format!("    return {c_unregister}(name, out_error);\n"));
    out.push_str("}\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // Comptime vtable builder: make_{trait_snake}_vtable
    // -------------------------------------------------------------------------
    emit_make_vtable(trait_name, has_super_trait, trait_def, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};

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
        }
    }

    fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: super_trait.map(|s| s.to_string()),
            registry_getter: None,
            register_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
        }
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
        emit_trait_bridge("demo", &bridge_cfg, &trait_def, &mut out);

        // Vtable struct
        assert!(out.contains("pub const IValidator = extern struct {"), "missing vtable struct: {out}");
        // Method slot present
        assert!(out.contains("validate:"), "missing validate slot: {out}");
        // user_data first arg
        assert!(out.contains("user_data: ?*anyopaque"), "missing user_data: {out}");
        // callconv(.C) present
        assert!(out.contains("callconv(.C)"), "missing callconv: {out}");
        // free_user_data slot
        assert!(out.contains("free_user_data:"), "missing free_user_data: {out}");
        // Registration shim
        assert!(out.contains("pub fn register_validator("), "missing register fn: {out}");
        assert!(out.contains("c.demo_register_validator("), "wrong C symbol: {out}");
        // Unregistration shim
        assert!(out.contains("pub fn unregister_validator("), "missing unregister fn: {out}");
        assert!(out.contains("c.demo_unregister_validator("), "wrong unregister C symbol: {out}");
        // No plugin lifecycle when no super_trait
        assert!(!out.contains("name_fn:"), "should not emit name_fn without super_trait: {out}");
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
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("kreuzberg::plugins::Plugin"));

        let mut out = String::new();
        emit_trait_bridge("kreuzberg", &bridge_cfg, &trait_def, &mut out);

        // Struct name
        assert!(out.contains("pub const IOcrBackend = extern struct {"), "missing vtable: {out}");
        // Plugin lifecycle slots emitted
        assert!(out.contains("name_fn:"), "missing name_fn: {out}");
        assert!(out.contains("version_fn:"), "missing version_fn: {out}");
        assert!(out.contains("initialize_fn:"), "missing initialize_fn: {out}");
        assert!(out.contains("shutdown_fn:"), "missing shutdown_fn: {out}");
        // Trait method slots
        assert!(out.contains("process_image:"), "missing process_image slot: {out}");
        assert!(out.contains("supports_language:"), "missing supports_language slot: {out}");
        // Bytes param expands to ptr + len
        assert!(out.contains("image_bytes_ptr:"), "missing bytes ptr expansion: {out}");
        assert!(out.contains("image_bytes_len:"), "missing bytes len expansion: {out}");
        // Fallible method gets out_error
        assert!(out.contains("out_error:"), "missing out_error for fallible method: {out}");
        // C symbols use kreuzberg prefix
        assert!(
            out.contains("c.kreuzberg_register_ocr_backend("),
            "wrong register symbol: {out}"
        );
        assert!(
            out.contains("c.kreuzberg_unregister_ocr_backend("),
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
        emit_trait_bridge("demo", &bridge_cfg, &trait_def, &mut out);

        // Helper function declaration
        assert!(
            out.contains("pub fn make_validator_vtable(comptime T: type, instance: *T)"),
            "missing make_validator_vtable: {out}"
        );
        // Returns the vtable type
        assert!(out.contains("IValidator{"), "missing vtable literal: {out}");
        // Thunk casts user_data
        assert!(
            out.contains("@ptrCast(@alignCast(ud))"),
            "missing @ptrCast cast: {out}"
        );
        // callconv(.C) in thunk
        assert!(out.contains("callconv(.C)"), "missing callconv(.C) in thunk: {out}");
        // validate thunk field
        assert!(out.contains(".validate ="), "missing .validate thunk field: {out}");
        // free_user_data thunk
        assert!(out.contains(".free_user_data ="), "missing .free_user_data thunk: {out}");
        // No lifecycle stubs without super_trait
        assert!(!out.contains(".name_fn ="), "must not emit .name_fn without super_trait: {out}");
    }

    #[test]
    fn make_vtable_with_super_trait_emits_lifecycle_stubs() {
        let trait_def = make_trait_def("OcrBackend", vec![]);
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("kreuzberg::Plugin"));

        let mut out = String::new();
        emit_trait_bridge("kreuzberg", &bridge_cfg, &trait_def, &mut out);

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
    fn make_vtable_bytes_param_reconstructs_slice_in_thunk() {
        let trait_def = make_trait_def(
            "Processor",
            vec![make_method(
                "process",
                vec![make_param("data", TypeRef::Bytes)],
                TypeRef::Unit,
                None,
            )],
        );
        let bridge_cfg = make_bridge_cfg("Processor", None);

        let mut out = String::new();
        emit_trait_bridge("demo", &bridge_cfg, &trait_def, &mut out);

        // Thunk receives ptr+len params
        assert!(out.contains("data_ptr: [*c]const u8"), "missing data_ptr param: {out}");
        assert!(out.contains("data_len: usize"), "missing data_len param: {out}");
        // Thunk reconstructs slice
        assert!(
            out.contains("data_ptr[0..data_len]"),
            "thunk must reconstruct slice from ptr+len: {out}"
        );
        // Thunk calls self.process with the slice
        assert!(out.contains("self.process(data_slice)"), "thunk must call self.process: {out}");
    }

    #[test]
    fn make_vtable_fallible_method_returns_i32_error_code() {
        let trait_def = make_trait_def(
            "Parser",
            vec![make_method(
                "parse",
                vec![],
                TypeRef::Unit,
                Some("ParseError"),
            )],
        );
        let bridge_cfg = make_bridge_cfg("Parser", None);

        let mut out = String::new();
        emit_trait_bridge("demo", &bridge_cfg, &trait_def, &mut out);

        // Thunk returns i32 (fallible → i32 return)
        assert!(
            out.contains("callconv(.C) i32"),
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
        emit_trait_bridge("demo", &bridge_cfg, &trait_def, &mut out);

        // Infallible primitive method: thunk returns the value directly
        assert!(
            out.contains("return self.count()"),
            "primitive return must be forwarded directly: {out}"
        );
    }
}
