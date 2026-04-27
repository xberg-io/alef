use alef_core::ir::{FunctionDef, ParamDef, TypeRef};

use super::errors::resolve_zig_error_type;
use super::helpers::emit_cleaned_zig_doc;
use super::types::zig_field_type;

pub(crate) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    declared_errors: &[String],
    top_level_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &f.doc, "");

    // Rename param names that would shadow a top-level decl (Zig 0.16+ rejects
    // shadowing of file-scope identifiers by function parameters).
    let renamed_params: Vec<ParamDef> = f
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if top_level_names.contains(&p2.name) {
                p2.name = format!("{}_arg", p2.name);
            }
            p2
        })
        .collect();
    let f_local = FunctionDef {
        params: renamed_params,
        ..f.clone()
    };
    let f = &f_local;

    // Build the wrapper-level parameter list (Zig-idiomatic types, not raw C types).
    let params: Vec<String> = f.params.iter().map(format_param_wrapper).collect();

    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let return_ty = if let Some(error_type) = &zig_error_type {
        format!("({}||error{{OutOfMemory}})!{}", error_type, zig_return_type(&f.return_type))
    } else {
        zig_return_type(&f.return_type)
    };

    out.push_str(&format!("pub fn {}({}) {} {{\n", f.name, params.join(", "), return_ty));

    // Emit allocation/conversion boilerplate for each parameter.
    for p in &f.params {
        emit_param_conversion(p, out);
    }

    // Build the C argument list.
    let c_args: Vec<String> = f.params.iter().flat_map(c_arg_names).collect();
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));

    if let Some(error_type) = &zig_error_type {
        // Fallible function: call C, then check last_error_code(). Zig requires `_`
        // (single underscore) to discard a value; named locals must be used.
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&format!("    _ = {c_call};\n"));
        } else {
            out.push_str(&format!("    const _result = {c_call};\n"));
        }
        out.push_str(&format!("    if (c.{prefix}_last_error_code() != 0) {{\n"));
        out.push_str(&format!("        return _first_error({error_type});\n"));
        out.push_str("    }\n");

        // Free owned C strings after the error check.
        for p in &f.params {
            emit_param_free(p, out);
        }

        // Produce the Zig return value from `_result`.
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str("    return;\n");
        } else {
            let ret_expr = unwrap_return_expr("_result", &f.return_type);
            out.push_str(&format!("    return {ret_expr};\n"));
        }
    } else {
        // Infallible function: free params, return directly.
        for p in &f.params {
            emit_param_free(p, out);
        }
        if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&format!("    {c_call};\n"));
        } else {
            out.push_str(&format!("    const _result = {c_call};\n"));
            let ret_expr = unwrap_return_expr("_result", &f.return_type);
            out.push_str(&format!("    return {ret_expr};\n"));
        }
    }

    out.push_str("}\n");
}

/// Return the Zig-wrapper parameter type string for a function parameter.
fn format_param_wrapper(p: &ParamDef) -> String {
    let ty_str = zig_param_type(&p.ty, p.optional);
    format!("{}: {}", p.name, ty_str)
}

/// Zig type used at the wrapper boundary for a function parameter.
///
/// - `String`, `Path` → `[]const u8`  (body allocates null-terminated copy)
/// - `Bytes`          → `[]const u8`  (body passes `.ptr` + `.len`)
/// - `Vec`, `Map`     → `[]const u8`  (caller supplies JSON; body passes as C string)
/// - Everything else  → same as struct-field type
fn zig_param_type(ty: &TypeRef, optional: bool) -> String {
    let inner = match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]const u8".to_string()
        }
        TypeRef::Optional(inner) => {
            let inner_str = zig_param_type(inner, false);
            return format!("?{inner_str}");
        }
        other => zig_field_type(other, false),
    };
    if optional { format!("?{inner}") } else { inner }
}

/// Emit the allocation / conversion lines needed before the C call for `p`.
///
/// String/Path: allocate a null-terminated copy via `std.heap.c_allocator`.
/// Vec/Map:     same — caller supplies a JSON `[]const u8`; we need a sentinel-
///              terminated copy to pass to `*const c_char` parameters.
/// Bytes:       nothing needed; `.ptr` and `.len` are used directly in `c_arg_names`.
fn emit_param_conversion(p: &ParamDef, out: &mut String) {
    let name = &p.name;
    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            out.push_str(&format!("    const {name}_z: [*:0]u8 = try std.fmt.allocPrintZ(\n"));
            out.push_str(&format!("        std.heap.c_allocator, \"{{s}}\", .{{{name}}},\n"));
            out.push_str("    );\n");
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Caller supplies JSON bytes; we need a null-terminated C string copy.
            out.push_str("    // Vec/Map parameters are passed as JSON strings across the FFI boundary.\n");
            out.push_str(&format!("    const {name}_z: [*:0]u8 = try std.fmt.allocPrintZ(\n"));
            out.push_str(&format!("        std.heap.c_allocator, \"{{s}}\", .{{{name}}},\n"));
            out.push_str("    );\n");
        }
        _ => {
            // No conversion needed — Bytes uses .ptr/.len directly, primitives pass through.
        }
    }
}

/// Emit the deallocation lines for allocations made in `emit_param_conversion`.
///
/// These are emitted after the C call (and after the error check) so the
/// allocations are always freed even when an error is returned.
fn emit_param_free(p: &ParamDef, out: &mut String) {
    let name = &p.name;
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str(&format!(
                "    std.heap.c_allocator.free({name}_z[0..std.mem.len({name}_z)]);\n"
            ));
        }
        _ => {}
    }
}

/// The C argument name(s) to use for a given wrapper parameter.
///
/// Bytes expand to two arguments: `.ptr` and `.len`.
/// String/Path/Vec/Map expand to the `_z` null-terminated copy.
/// Everything else passes the parameter directly.
fn c_arg_names(p: &ParamDef) -> Vec<String> {
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

/// Produce the Zig expression that converts a raw C return value (`raw`) to the
/// wrapper return type.
///
/// String/Path/Json/Vec/Map: copy the C string to an owned Zig slice, then free
/// the FFI allocation via `_free_string`.
/// Everything else: pass through unchanged.
fn unwrap_return_expr(raw: &str, ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Copy the null-terminated C string to an owned Zig allocation, then free the C copy.
            let mut s = String::new();
            s.push_str("blk: {\n");
            s.push_str(&format!("        const slice = std.mem.sliceTo({raw}, 0);\n"));
            s.push_str("        const owned = try std.heap.c_allocator.dupe(u8, slice);\n");
            s.push_str(&format!("        _free_string({raw});\n"));
            s.push_str("        break :blk owned;\n");
            s.push_str("    }");
            s
        }
        _ => raw.to_string(),
    }
}

/// Build the Zig return type for a function (not for struct fields).
///
/// Owned string/JSON/collection returns are `[]u8` (allocated slice); everything
/// else matches the struct-field mapping.
pub(crate) fn zig_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => "[]u8".to_string(),
        other => zig_field_type(other, false),
    }
}
