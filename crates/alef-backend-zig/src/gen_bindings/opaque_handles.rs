use alef_core::ir::{MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::AsSnakeCase;
use std::fmt::Write as FmtWrite;

use super::errors::resolve_zig_error_type;
use super::functions::{optional_int_sentinel, zig_return_type};
use super::helpers::emit_cleaned_zig_doc;

/// Emit a Zig struct wrapper for an opaque handle type (one with `is_opaque = true`
/// or `has_serde = false`) that has instance methods.
///
/// The emitted struct stores a `*anyopaque` handle obtained from the C FFI and
/// exposes each non-static, non-excluded method as a Zig function that dispatches
/// via `c.{prefix}_{snake_type}_{snake_method}(self._handle, ...)`.
///
/// Static methods are skipped — they are typically constructors like `new()` that
/// return the handle and should be accessed via the package-level `create_*`
/// functions instead.
pub(crate) fn emit_opaque_handle(
    ty: &TypeDef,
    prefix: &str,
    declared_errors: &[String],
    struct_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &ty.doc, "");
    let _ = writeln!(out, "pub const {type_name} = struct {{", type_name = ty.name);
    let _ = writeln!(out, "    _handle: *anyopaque,");
    let _ = writeln!(out);

    let type_snake = AsSnakeCase(&ty.name).to_string();

    for method in ty.methods.iter().filter(|m| !m.is_static) {
        emit_opaque_method(method, ty, prefix, &type_snake, declared_errors, struct_names, out);
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}};");
}

/// Emit a single method on an opaque handle wrapper struct.
fn emit_opaque_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    declared_errors: &[String],
    struct_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    // Note: async Rust methods are exposed as synchronous C functions via
    // `tokio::runtime::block_on` in the FFI layer. The zig backend calls
    // the synchronous C symbol directly — `is_async` is intentionally not
    // checked here to avoid skipping callable methods.

    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let method_snake = AsSnakeCase(&method.name).to_string();

    // Build parameter list: `self: *{TypeName}` followed by method params.
    // All struct-typed (non-opaque) params become `[]const u8` (JSON).
    // String/Path params become `[]const u8`.
    // Optional variants add `?` prefix.
    let mut param_parts: Vec<String> = Vec::new();
    param_parts.push(format!("self: *{}", ty.name));
    for p in &method.params {
        let ty_str = param_zig_type(&p.ty, p.optional, struct_names);
        param_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let params_str = param_parts.join(", ");

    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let ret_ty_inner = zig_return_type(&method.return_type, struct_names);
    let return_ty = if let Some(ref err_ty) = zig_error_type {
        format!("({}||error{{OutOfMemory}})!{}", err_ty, ret_ty_inner)
    } else {
        ret_ty_inner
    };

    let _ = writeln!(
        out,
        "    pub fn {method_name}({params}) !{return_ty} {{",
        method_name = method.name,
        params = params_str,
        return_ty = return_ty,
    );

    // Emit param conversions (string alloc, struct JSON handle creation).
    for p in &method.params {
        emit_method_param_conversion(p, prefix, struct_names, out);
    }

    // Build C argument list: handle pointer, then converted params.
    let upper_prefix = prefix.to_uppercase();
    let c_handle = format!(
        "@as(*c.{upper_prefix}{type_name}, @ptrCast(self._handle))",
        type_name = ty.name,
    );
    let mut c_args: Vec<String> = vec![c_handle];
    for p in &method.params {
        c_args.extend(method_c_arg_names(p, struct_names));
    }
    let c_call = format!(
        "c.{prefix}_{type_snake}_{method_snake}({args})",
        args = c_args.join(", ")
    );

    if zig_error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(out, "        _ = {c_call};");
        } else {
            let _ = writeln!(out, "        const _result = {c_call};");
        }
        let _ = writeln!(out, "        const _err_code = c.{prefix}_last_error_code();");
        let _ = writeln!(out, "        if (_err_code != 0) {{");
        let _ = writeln!(out, "            const _msg_ptr = c.{prefix}_last_error_context();");
        let _ = writeln!(
            out,
            "            const _msg_slice = if (_msg_ptr != null) std.mem.span(_msg_ptr.?) else \"unknown error\";"
        );
        let _ = writeln!(
            out,
            "            const _msg = try std.heap.c_allocator.dupe(u8, _msg_slice);"
        );
        let _ = writeln!(out, "            _ = _msg;");
        let _ = writeln!(out, "            return error.FfiError;");
        let _ = writeln!(out, "        }}");

        // Free params after error check.
        for p in &method.params {
            emit_method_param_free(p, prefix, struct_names, out);
        }

        if !matches!(method.return_type, TypeRef::Unit) {
            let ret_expr = method_unwrap_return_expr("_result", &method.return_type, prefix, struct_names);
            let _ = writeln!(out, "        return {ret_expr};");
        }
    } else {
        // Infallible method.
        for p in &method.params {
            emit_method_param_free(p, prefix, struct_names, out);
        }
        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(out, "        {c_call};");
        } else {
            let _ = writeln!(out, "        const _result = {c_call};");
            let ret_expr = method_unwrap_return_expr("_result", &method.return_type, prefix, struct_names);
            let _ = writeln!(out, "        return {ret_expr};");
        }
    }

    let _ = writeln!(out, "    }}");
}

/// Zig type for a method parameter (same rules as function params).
fn param_zig_type(ty: &TypeRef, optional: bool, struct_names: &std::collections::HashSet<String>) -> String {
    let inner = match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]const u8".to_string()
        }
        TypeRef::Named(name) if struct_names.contains(name) => "[]const u8".to_string(),
        TypeRef::Optional(inner) => {
            let inner_str = param_zig_type(inner, false, struct_names);
            return format!("?{inner_str}");
        }
        other => super::types::zig_field_type(other, false),
    };
    if optional { format!("?{inner}") } else { inner }
}

/// Emit allocation/conversion lines for a method parameter before the C call.
fn emit_method_param_conversion(
    p: &alef_core::ir::ParamDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    let name = &p.name;
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );

    // Optional string: conditional allocPrintSentinel.
    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        let _ = writeln!(
            out,
            "        const {name}_z: ?[:0]u8 = if ({name}) |s| try std.heap.c_allocator.dupeZ(u8, s) else null;"
        );
        return;
    }

    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            let _ = writeln!(
                out,
                "        const {name}_z = try std.heap.c_allocator.dupeZ(u8, {name});"
            );
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            let _ = writeln!(
                out,
                "        const {name}_z = try std.heap.c_allocator.dupeZ(u8, {name});"
            );
        }
        TypeRef::Named(n) if struct_names.contains(n) => {
            let snake = AsSnakeCase(n).to_string();
            let _ = writeln!(
                out,
                "        const {name}_z = try std.heap.c_allocator.dupeZ(u8, {name});"
            );
            let _ = writeln!(
                out,
                "        const {name}_handle = c.{prefix}_{snake}_from_json({name}_z.ptr);"
            );
        }
        _ => {}
    }
}

/// Free allocations made in `emit_method_param_conversion`.
fn emit_method_param_free(
    p: &alef_core::ir::ParamDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    let name = &p.name;
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );

    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        let _ = writeln!(out, "        if ({name}_z) |z| std.heap.c_allocator.free(z);");
        return;
    }

    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            let _ = writeln!(out, "        std.heap.c_allocator.free({name}_z);");
        }
        TypeRef::Named(n) if struct_names.contains(n) => {
            let snake = AsSnakeCase(n).to_string();
            let _ = writeln!(out, "        std.heap.c_allocator.free({name}_z);");
            let _ = writeln!(out, "        c.{prefix}_{snake}_free({name}_handle);");
        }
        _ => {}
    }
}

/// Build the C argument name(s) for a method parameter.
fn method_c_arg_names(p: &alef_core::ir::ParamDef, struct_names: &std::collections::HashSet<String>) -> Vec<String> {
    if let TypeRef::Named(n) = &p.ty {
        if struct_names.contains(n.as_str()) {
            return vec![format!("{}_handle", p.name)];
        }
    }
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        return vec![format!("if ({0}_z) |z| z.ptr else null", p.name)];
    }
    // Optional integer primitive: substitute the max-value sentinel for None.
    // The Rust FFI layer uses `<Type>::MAX` as the sentinel for Option<T>.
    // Handle both TypeRef::Optional(Primitive) and p.optional + TypeRef::Primitive.
    {
        let prim_opt: Option<&PrimitiveType> = match &p.ty {
            TypeRef::Optional(inner) => {
                if let TypeRef::Primitive(prim) = inner.as_ref() { Some(prim) } else { None }
            }
            TypeRef::Primitive(prim) if p.optional => Some(prim),
            _ => None,
        };
        if let Some(prim) = prim_opt {
            if let Some(sentinel) = optional_int_sentinel(prim) {
                return vec![format!("if ({name}) |v| v else {sentinel}", name = p.name)];
            }
        }
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            vec![format!("{}_z", p.name)]
        }
        TypeRef::Bytes => {
            vec![format!("{}.ptr", p.name), format!("{}.len", p.name)]
        }
        _ => vec![p.name.clone()],
    }
}

/// Produce the Zig return expression for an opaque method result.
fn method_unwrap_return_expr(
    raw: &str,
    ty: &TypeRef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!(
                "blk: {{\n            const slice = std.mem.span({raw});\n            const owned = try std.heap.c_allocator.dupe(u8, slice);\n            c.{prefix}_free_string({raw});\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = AsSnakeCase(name).to_string();
            format!(
                "blk: {{\n            const _json_ptr = c.{prefix}_{snake}_to_json({raw});\n            const _json_slice = std.mem.span(_json_ptr);\n            const owned = try std.heap.c_allocator.dupe(u8, _json_slice);\n            c.{prefix}_free_string(_json_ptr);\n            c.{prefix}_{snake}_free({raw});\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) => {
            // Opaque handle type (no serde): unwrap the nullable C pointer (guaranteed
            // non-null after the error-code check above) and wrap in the Zig struct.
            format!("{name}{{ ._handle = {raw}.? }}")
        }
        _ => raw.to_string(),
    }
}
