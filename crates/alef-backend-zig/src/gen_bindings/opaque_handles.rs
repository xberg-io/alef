use alef_core::ir::{MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::errors::resolve_zig_error_type;
use super::functions::{optional_int_sentinel, zig_return_type};
use super::helpers::emit_cleaned_zig_doc;

/// Returns true if generating the param-conversion boilerplate for `p` will
/// emit a `try` expression (heap allocation for string duplication).
/// Builder setters that take string arguments call `dupeZ`, which is fallible,
/// so the enclosing method must declare an error-union return type.
fn method_param_needs_alloc(p: &ParamDef) -> bool {
    let inner = match &p.ty {
        TypeRef::Optional(t) => t.as_ref(),
        other => other,
    };
    matches!(
        inner,
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_)
    )
}

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
    streaming_item_types: &HashMap<String, String>,
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &ty.doc, "");
    let _ = writeln!(out, "pub const {type_name} = struct {{", type_name = ty.name);
    let _ = writeln!(out, "    _handle: *anyopaque,");
    let _ = writeln!(out);

    let type_snake = AsSnakeCase(&ty.name).to_string();

    for method in ty.methods.iter().filter(|m| !m.is_static) {
        emit_opaque_method(
            method,
            ty,
            prefix,
            &type_snake,
            declared_errors,
            struct_names,
            streaming_item_types,
            out,
        );
        let _ = writeln!(out);
    }

    // Synthetic destructor: every opaque-handle type owns a heap allocation in
    // the FFI and must be released via the matching `{prefix}_{snake}_free`
    // C symbol. Emit a `free()` method that performs that release.
    emit_opaque_free(ty, prefix, &type_snake, out);

    let _ = writeln!(out, "}};");
}

/// Emit a `free()` method that releases the underlying FFI handle by calling
/// `c.{prefix}_{snake_type}_free(self._handle)`. The C destructor is generated
/// by the FFI crate for every opaque handle type.
fn emit_opaque_free(ty: &TypeDef, prefix: &str, type_snake: &str, out: &mut String) {
    let upper_prefix = prefix.to_uppercase();
    let _ = writeln!(
        out,
        "    /// Release the underlying FFI handle. Safe to call once per instance."
    );
    let _ = writeln!(out, "    pub fn free(self: *{}) void {{", ty.name);
    let _ = writeln!(
        out,
        "        c.{prefix}_{type_snake}_free(@as(*c.{upper_prefix}{type_name}, @ptrCast(self._handle)));",
        type_name = ty.name,
    );
    let _ = writeln!(out, "    }}");
}

/// Emit a streaming method on an opaque handle wrapper struct.
///
/// Streaming methods use the iterator-handle pattern (`_start` / `_next` / `_free`)
/// rather than the callback-based C symbol. The Zig wrapper:
///   1. Creates the request handle from JSON.
///   2. Calls `{prefix}_{type_snake}_{method_name}_start(client, req_handle)`.
///   3. Loops `{prefix}_{type_snake}_{method_name}_next(handle)` until null.
///   4. Serialises each chunk to JSON, keeping the last one.
///   5. Frees the stream handle.
///   6. Returns the last chunk JSON (or `"{}"` if the stream was empty).
fn emit_opaque_streaming_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    item_type: &str,
    declared_errors: &[String],
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let method_snake = AsSnakeCase(&method.name).to_string();
    let item_snake = AsSnakeCase(item_type).to_string();
    let upper_prefix = prefix.to_uppercase();

    // Streaming methods take a single JSON request parameter.
    // The error type comes from the method's error_type annotation.
    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors))
        .unwrap_or_else(|| "anyerror".to_string());

    // The Zig wrapper signature: self + one JSON request slice → JSON string.
    let req_param = method.params.first().map(|p| p.name.as_str()).unwrap_or("req");

    let _ = writeln!(
        out,
        "    pub fn {method_name}(self: *{type_name}, {req_param}: []const u8) ({zig_error_type}||error{{OutOfMemory}})![]u8 {{",
        method_name = method.name,
        type_name = ty.name,
    );

    // Build the request handle.
    let req_param_lower = req_param.to_lowercase();
    let _ = writeln!(
        out,
        "        const {req_param_lower}_z = try std.heap.c_allocator.dupeZ(u8, {req_param_lower});",
    );
    // Derive the request type from the first param's type.
    let req_type_snake = if let Some(p) = method.params.first() {
        if let TypeRef::Named(n) = &p.ty {
            AsSnakeCase(n).to_string()
        } else {
            "chat_completion_request".to_string()
        }
    } else {
        "chat_completion_request".to_string()
    };
    let _ = writeln!(
        out,
        "        const {req_param_lower}_handle = c.{prefix}_{req_type_snake}_from_json({req_param_lower}_z.ptr);",
    );
    let _ = writeln!(out, "        std.heap.c_allocator.free({req_param_lower}_z);");
    let _ = writeln!(
        out,
        "        if ({req_param_lower}_handle == null) {{ return _first_error({zig_error_type}); }}",
    );
    let _ = writeln!(
        out,
        "        defer c.{prefix}_{req_type_snake}_free({req_param_lower}_handle);",
    );

    // Start the stream.
    let c_handle_cast = format!(
        "@as(*c.{upper_prefix}{type_name}, @ptrCast(self._handle))",
        type_name = ty.name
    );
    let _ = writeln!(
        out,
        "        const _stream_handle = c.{prefix}_{type_snake}_{method_snake}_start({c_handle_cast}, {req_param_lower}_handle);",
    );
    let _ = writeln!(
        out,
        "        if (_stream_handle == null) {{ return _first_error({zig_error_type}); }}",
    );
    let _ = writeln!(
        out,
        "        defer c.{prefix}_{type_snake}_{method_snake}_free(_stream_handle);",
    );

    // Collect chunks; keep the last one.
    let _ = writeln!(out, "        var _last_json: ?[]u8 = null;");
    let _ = writeln!(out, "        while (true) {{");
    let _ = writeln!(
        out,
        "            const _chunk = c.{prefix}_{type_snake}_{method_snake}_next(_stream_handle);",
    );
    let _ = writeln!(out, "            if (_chunk == null) break;");
    let _ = writeln!(out, "            if (_last_json) |j| std.heap.c_allocator.free(j);");
    let _ = writeln!(
        out,
        "            const _chunk_json_ptr = c.{prefix}_{item_snake}_to_json(_chunk);",
    );
    let _ = writeln!(out, "            c.{prefix}_{item_snake}_free(_chunk);",);
    let _ = writeln!(out, "            if (_chunk_json_ptr == null) continue;");
    let _ = writeln!(out, "            const _chunk_slice = std.mem.span(_chunk_json_ptr);",);
    let _ = writeln!(
        out,
        "            _last_json = try std.heap.c_allocator.dupe(u8, _chunk_slice);",
    );
    let _ = writeln!(out, "            c.{prefix}_free_string(_chunk_json_ptr);",);
    let _ = writeln!(out, "        }}");

    // Return last chunk JSON or empty object if stream was empty.
    let _ = writeln!(
        out,
        "        return _last_json orelse try std.heap.c_allocator.dupe(u8, \"{{}}\");",
    );
    let _ = writeln!(out, "    }}");
}

/// Emit a single method on an opaque handle wrapper struct.
#[allow(clippy::too_many_arguments)]
fn emit_opaque_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    declared_errors: &[String],
    struct_names: &std::collections::HashSet<String>,
    streaming_item_types: &HashMap<String, String>,
    out: &mut String,
) {
    // Note: async Rust methods are exposed as synchronous C functions via
    // `tokio::runtime::block_on` in the FFI layer. The zig backend calls
    // the synchronous C symbol directly — `is_async` is intentionally not
    // checked here to avoid skipping callable methods.

    // Streaming methods use the iterator-handle pattern (_start/_next/_free)
    // rather than the callback-based C symbol. Detect them early and delegate.
    if let Some(item_type) = streaming_item_types.get(&method.name) {
        emit_opaque_streaming_method(method, ty, prefix, type_snake, item_type, declared_errors, out);
        return;
    }

    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let method_snake = AsSnakeCase(&method.name).to_string();

    // Z2 fix: Zig 0.16+ forbids a function parameter from having the same name
    // as the enclosing function. Builder setters like `pub fn visitor(..., visitor: ...)` hit
    // this. Rename the offending parameter to `value` so the declaration is unambiguous.
    // The rename is applied to a local clone so the rest of the emit logic is unaffected.
    let renamed_params: Vec<ParamDef> = method
        .params
        .iter()
        .map(|p| {
            if p.name == method.name {
                let mut p2 = p.clone();
                p2.name = "value".to_string();
                p2
            } else {
                p.clone()
            }
        })
        .collect();
    let effective_params: &[ParamDef] = &renamed_params;

    // Build parameter list: `self: *{TypeName}` followed by method params.
    // All struct-typed (non-opaque) params become `[]const u8` (JSON).
    // String/Path params become `[]const u8`.
    // Optional variants add `?` prefix.
    let mut param_parts: Vec<String> = Vec::new();
    param_parts.push(format!("self: *{}", ty.name));
    for p in effective_params {
        let ty_str = param_zig_type(&p.ty, p.optional, struct_names);
        param_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let params_str = param_parts.join(", ");

    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    // Z4 fix: when any parameter requires a heap allocation (String/Path/Vec/Map/Named),
    // the body will emit `try dupeZ(...)`. Zig requires the function to return an error
    // union for `try` to be legal. Wrap the return type in `error{OutOfMemory}!T` when
    // no explicit error type is set but the body uses allocation.
    let body_needs_try = effective_params.iter().any(method_param_needs_alloc)
        || matches!(
            &method.return_type,
            TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _)
        )
        || matches!(&method.return_type, TypeRef::Named(name) if struct_names.contains(name));

    let ret_ty_inner = zig_return_type(&method.return_type, struct_names);
    let return_ty = if let Some(ref err_ty) = zig_error_type {
        format!("({}||error{{OutOfMemory}})!{}", err_ty, ret_ty_inner)
    } else if body_needs_try {
        format!("error{{OutOfMemory}}!{}", ret_ty_inner)
    } else {
        ret_ty_inner
    };

    let _ = writeln!(
        out,
        "    pub fn {method_name}({params}) {return_ty} {{",
        method_name = method.name,
        params = params_str,
        return_ty = return_ty,
    );

    // Emit param conversions (string alloc, struct JSON handle creation).
    for p in effective_params {
        emit_method_param_conversion(p, prefix, struct_names, out);
    }

    // Detect Bytes return: the C FFI uses a multi-out-parameter convention
    // (`uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap`) and
    // returns `int32_t` status. Caller passes pointers to local storage and
    // reads the buffer back after the call.
    let returns_bytes = matches!(method.return_type, TypeRef::Bytes);
    if returns_bytes {
        let _ = writeln!(out, "        var _out_ptr: [*c]u8 = undefined;");
        let _ = writeln!(out, "        var _out_len: usize = 0;");
        let _ = writeln!(out, "        var _out_cap: usize = 0;");
    }

    // Build C argument list: handle pointer, then converted params, then
    // (for Bytes returns) the three out-param pointers.
    let upper_prefix = prefix.to_uppercase();
    let c_handle = format!(
        "@as(*c.{upper_prefix}{type_name}, @ptrCast(self._handle))",
        type_name = ty.name,
    );
    let mut c_args: Vec<String> = vec![c_handle];
    for p in effective_params {
        c_args.extend(method_c_arg_names(p, struct_names));
    }
    if returns_bytes {
        c_args.push("&_out_ptr".to_string());
        c_args.push("&_out_len".to_string());
        c_args.push("&_out_cap".to_string());
    }
    let c_call = format!(
        "c.{prefix}_{type_snake}_{method_snake}({args})",
        args = c_args.join(", ")
    );

    if let Some(ref err_ty) = zig_error_type {
        if matches!(method.return_type, TypeRef::Unit) || returns_bytes {
            // Discard status / unit return — error state is queried via
            // `{prefix}_last_error_code()`.
            let _ = writeln!(out, "        _ = {c_call};");
        } else {
            let _ = writeln!(out, "        const _result = {c_call};");
        }
        let _ = writeln!(out, "        if (c.{prefix}_last_error_code() != 0) {{");
        let _ = writeln!(out, "            return _first_error({err_ty});");
        let _ = writeln!(out, "        }}");

        // Free params after error check.
        for p in effective_params {
            emit_method_param_free(p, prefix, struct_names, out);
        }

        if returns_bytes {
            // Copy the FFI-owned buffer into a Zig-owned heap allocation, then
            // release the FFI buffer via `{prefix}_free_bytes`.
            let _ = writeln!(
                out,
                "        const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);"
            );
            let _ = writeln!(out, "        c.{prefix}_free_bytes(_out_ptr, _out_len, _out_cap);");
            let _ = writeln!(out, "        return _owned;");
        } else if !matches!(method.return_type, TypeRef::Unit) {
            let ret_expr = method_unwrap_return_expr("_result", &method.return_type, prefix, struct_names);
            let _ = writeln!(out, "        return {ret_expr};");
        }
    } else {
        // Infallible method (or method using only error{OutOfMemory} from alloc).
        for p in effective_params {
            emit_method_param_free(p, prefix, struct_names, out);
        }
        if returns_bytes {
            let _ = writeln!(out, "        _ = {c_call};");
            let _ = writeln!(
                out,
                "        const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);"
            );
            let _ = writeln!(out, "        c.{prefix}_free_bytes(_out_ptr, _out_len, _out_cap);");
            let _ = writeln!(out, "        return _owned;");
        } else if matches!(method.return_type, TypeRef::Unit) {
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

    // `Option<NamedStruct>` may surface as `TypeRef::Optional(Named(_))` OR
    // as `TypeRef::Named(_)` with `p.optional = true` depending on how the
    // IR builder normalised the Rust source. Handle both shapes uniformly
    // before falling through to the non-optional path.
    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if let Some(n) = optional_named {
        let snake = AsSnakeCase(n).to_string();
        let _ = writeln!(
            out,
            "        const {name}_z: ?[:0]u8 = if ({name}) |v| try std.heap.c_allocator.dupeZ(u8, v) else null;"
        );
        let _ = writeln!(
            out,
            "        const {name}_handle = if ({name}_z) |z| c.{prefix}_{snake}_from_json(z.ptr) else null;"
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
        TypeRef::Optional(inner) => {
            if let TypeRef::Vec(_) | TypeRef::Map(_, _) = inner.as_ref() {
                let _ = writeln!(
                    out,
                    "        const {name}_z: ?[:0]u8 = if ({name}) |v| try std.heap.c_allocator.dupeZ(u8, v) else null;"
                );
            }
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

    // Mirror the conversion logic: an Optional<NamedStruct> may be encoded
    // as `TypeRef::Optional(Named(_))` or `Named(_)` + `p.optional`.
    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if let Some(n) = optional_named {
        let snake = AsSnakeCase(n).to_string();
        let _ = writeln!(out, "        if ({name}_z) |z| std.heap.c_allocator.free(z);");
        let _ = writeln!(
            out,
            "        if ({name}_handle != null) c.{prefix}_{snake}_free({name}_handle);"
        );
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
        TypeRef::Optional(inner) => {
            if let TypeRef::Vec(_) | TypeRef::Map(_, _) = inner.as_ref() {
                let _ = writeln!(out, "        if ({name}_z) |z| std.heap.c_allocator.free(z);");
            }
        }
        _ => {}
    }
}

/// Build the C argument name(s) for a method parameter.
fn method_c_arg_names(p: &alef_core::ir::ParamDef, struct_names: &std::collections::HashSet<String>) -> Vec<String> {
    // `Option<NamedStruct>` parameters use a conditional handle: `null` when
    // the caller passed `null`, otherwise the FFI handle produced via
    // `_from_json`. Check the optional form first so non-optional Named
    // params don't shadow it. The optional may be encoded as `Optional(Named)`
    // or as `Named` + `p.optional = true`.
    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if optional_named.is_some() {
        return vec![format!("{}_handle", p.name)];
    }
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
                if let TypeRef::Primitive(prim) = inner.as_ref() {
                    Some(prim)
                } else {
                    None
                }
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
