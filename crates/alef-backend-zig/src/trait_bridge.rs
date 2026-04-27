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
}
