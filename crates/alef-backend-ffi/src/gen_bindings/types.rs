use crate::type_map::c_return_type;
use alef_codegen::conversions::core_type_path;
use alef_core::ir::{CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

use super::helpers::{gen_value_to_c, null_return_value};

// ---------------------------------------------------------------------------
// Type: from_json + free
// ---------------------------------------------------------------------------

pub(super) fn gen_type_from_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let mut out = String::with_capacity(2048);

    writeln!(
        out,
        "/// Create a `{type_name}` from a JSON string. Returns null on failure."
    )
    .ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// JSON string must be valid UTF-8 and null-terminated.").ok();
    writeln!(
        out,
        "/// Returned handle must be freed with `{prefix}_{type_snake}_free`."
    )
    .ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {prefix}_{type_snake}_from_json(json: *const c_char) -> *mut {qualified} {{"
    )
    .ok();
    writeln!(out, "    clear_last_error();").ok();
    writeln!(out, "    if json.is_null() {{").ok();
    writeln!(
        out,
        "        set_last_error(1, \"Null pointer passed for JSON string\");"
    )
    .ok();
    writeln!(out, "        return std::ptr::null_mut();").ok();
    writeln!(out, "    }}").ok();
    writeln!(
        out,
        "    let c_str = match unsafe {{ CStr::from_ptr(json) }}.to_str() {{"
    )
    .ok();
    writeln!(out, "        Ok(s) => s,").ok();
    writeln!(out, "        Err(_) => {{").ok();
    writeln!(out, "            set_last_error(1, \"Invalid UTF-8 in JSON string\");").ok();
    writeln!(out, "            return std::ptr::null_mut();").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }};").ok();
    writeln!(out, "    match serde_json::from_str::<{qualified}>(c_str) {{").ok();
    writeln!(out, "        Ok(val) => Box::into_raw(Box::new(val)),").ok();
    writeln!(out, "        Err(e) => {{").ok();
    writeln!(out, "            set_last_error(2, &e.to_string());").ok();
    writeln!(out, "            std::ptr::null_mut()").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();

    out
}

pub(super) fn gen_type_to_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let mut out = String::with_capacity(2048);

    writeln!(
        out,
        "/// Serialize a `{type_name}` to a JSON string. Returns null on failure."
    )
    .ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(
        out,
        "/// `ptr` must be a valid, non-null pointer returned by a `{prefix}` function."
    )
    .ok();
    writeln!(
        out,
        "/// The returned string must be freed with `{prefix}_free_string`."
    )
    .ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {prefix}_{type_snake}_to_json(ptr: *const {qualified}) -> *mut c_char {{"
    )
    .ok();
    writeln!(out, "    clear_last_error();").ok();
    writeln!(out, "    if ptr.is_null() {{").ok();
    writeln!(out, "        set_last_error(1, \"Null pointer passed to to_json\");").ok();
    writeln!(out, "        return std::ptr::null_mut();").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    let val = unsafe {{ &*ptr }};").ok();
    writeln!(out, "    match serde_json::to_string(val) {{").ok();
    writeln!(out, "        Ok(s) => match CString::new(s) {{").ok();
    writeln!(out, "            Ok(cs) => cs.into_raw(),").ok();
    writeln!(out, "            Err(e) => {{").ok();
    writeln!(out, "                set_last_error(2, &e.to_string());").ok();
    writeln!(out, "                std::ptr::null_mut()").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }},").ok();
    writeln!(out, "        Err(e) => {{").ok();
    writeln!(out, "            set_last_error(2, &e.to_string());").ok();
    writeln!(out, "            std::ptr::null_mut()").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();

    out
}

pub(super) fn gen_type_free(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let mut out = String::with_capacity(2048);

    writeln!(out, "/// Free a `{type_name}` handle.").ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// Pointer must have been returned by this library, or be null.").ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {prefix}_{type_snake}_free(ptr: *mut {qualified}) {{"
    )
    .ok();
    writeln!(out, "    if !ptr.is_null() {{").ok();
    writeln!(out, "        unsafe {{ drop(Box::from_raw(ptr)); }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();

    out
}

// ---------------------------------------------------------------------------
// Field accessors
// ---------------------------------------------------------------------------

pub(super) fn gen_field_accessor(typ: &TypeDef, field: &FieldDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let field_name = &field.name;

    let effective_ty = if field.optional {
        TypeRef::Optional(Box::new(field.ty.clone()))
    } else {
        field.ty.clone()
    };

    // When the field has a specific type_rust_path, use it for Named types to avoid
    // ambiguity when multiple types share the same short name.
    let field_core_import = if let Some(ref rust_path) = field.type_rust_path {
        // type_rust_path may be e.g. "types::extraction::OutputFormat" (relative)
        // or "kreuzberg::types::OutputFormat" (already fully qualified with crate prefix).
        // We need the module path prefix without the type name itself.
        // Normalize dashes to underscores since IR paths use Cargo package names (dashes)
        // but Rust source code requires crate names (underscores).
        let rust_path_norm = rust_path.replace('-', "_");
        if let Some(pos) = rust_path_norm.rfind("::") {
            let module_prefix = &rust_path_norm[..pos];
            // Avoid double-prefixing: if rust_path already starts with core_import,
            // use it as-is. Otherwise prepend core_import.
            if module_prefix == core_import || module_prefix.starts_with(&format!("{core_import}::")) {
                module_prefix.to_string()
            } else {
                format!("{core_import}::{module_prefix}")
            }
        } else {
            core_import.to_string()
        }
    } else {
        core_import.to_string()
    };

    let mut ret_type = c_return_type(&effective_ty, &field_core_import).into_owned();
    // Replace "Self" with the actual qualified type name in FFI signatures
    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }
    let mut out = String::with_capacity(2048);

    writeln!(out, "/// Get the `{field_name}` field from a `{type_name}`.").ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// Pointer must be a valid handle returned by this library.").ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();

    // Determine if we need an extra out-param for byte-length
    let needs_len_out = matches!(field.ty, TypeRef::Bytes) && !field.optional;

    if needs_len_out {
        writeln!(
            out,
            "pub unsafe extern \"C\" fn {prefix}_{type_snake}_{field_name}(ptr: *const {qualified}, out_len: *mut usize) -> {ret_type} {{"
        )
        .ok();
    } else {
        writeln!(
            out,
            "pub unsafe extern \"C\" fn {prefix}_{type_snake}_{field_name}(ptr: *const {qualified}) -> {ret_type} {{"
        )
        .ok();
    }

    // Null-check on ptr
    writeln!(out, "    if ptr.is_null() {{").ok();
    writeln!(out, "        return {};", null_return_value(&effective_ty)).ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    let obj = unsafe {{ &*ptr }};").ok();

    // Generate the accessor body based on field type
    write!(out, "{}", gen_field_access_body(field, needs_len_out)).ok();

    write!(out, "}}").ok();
    out
}

/// Generate the body of a field accessor that reads from `obj.{field_name}`.
fn gen_field_access_body(field: &FieldDef, needs_len_out: bool) -> String {
    let field_name = &field.name;
    let mut out = String::with_capacity(2048);

    if field.optional {
        // Wrap in match on Option — val is a reference from &Option<T> destructure
        // When is_boxed: val is &Box<T>, so deref twice (**val) to get &T
        // When newtype_wrapper: the core field is Option<NewtypeT> but IR ty is Primitive;
        //   val is &NewtypeT so we must access val.0 to get the inner primitive.
        //
        // Special case: field.ty = Optional(Primitive) means the Rust field is
        // Option<Option<Primitive>> (outer=field.optional, inner=field.ty). Both the
        // outer None and the inner None collapse to the primitive's zero/false sentinel.
        if let TypeRef::Optional(inner) = &field.ty {
            // Option<Option<T>>: outer Some gives val: &Option<inner>, inner Some gives the value.
            let inner_null = null_return_value(&TypeRef::Optional(Box::new(*inner.clone())));
            let inner_val_expr = match inner.as_ref() {
                TypeRef::Primitive(_) => "*inner_val",
                _ => "inner_val",
            };
            writeln!(out, "    match &obj.{field_name} {{").ok();
            writeln!(out, "        Some(val) => match val {{").ok();
            writeln!(out, "            Some(inner_val) => {{").ok();
            write!(out, "{}", gen_value_to_c(inner_val_expr, inner, "                ")).ok();
            writeln!(out, "            }}").ok();
            writeln!(out, "            None => {inner_null},").ok();
            writeln!(out, "        }}").ok();
            writeln!(
                out,
                "        None => {},",
                null_return_value(&TypeRef::Optional(Box::new(field.ty.clone())))
            )
            .ok();
            writeln!(out, "    }}").ok();
        } else {
            let val_expr = if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
                "val.0" // unwrap newtype inner value
            } else if matches!(field.ty, TypeRef::Primitive(_)) {
                "*val" // dereference for Copy types
            } else if field.is_boxed {
                "(**val)" // deref &Box<T> -> &T
            } else {
                "val"
            };
            writeln!(out, "    match &obj.{field_name} {{").ok();
            writeln!(out, "        Some(val) => {{").ok();
            write!(out, "{}", gen_value_to_c(val_expr, &field.ty, "            ")).ok();
            writeln!(out, "        }}").ok();
            writeln!(
                out,
                "        None => {},",
                null_return_value(&TypeRef::Optional(Box::new(field.ty.clone())))
            )
            .ok();
            writeln!(out, "    }}").ok();
        }
    } else if needs_len_out {
        // Bytes with length out-param
        writeln!(out, "    let data = &obj.{field_name};").ok();
        writeln!(out, "    if !out_len.is_null() {{").ok();
        writeln!(out, "        unsafe {{ *out_len = data.len(); }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    data.as_ptr() as *mut u8").ok();
    } else {
        // When is_boxed: obj.field_name is Box<T>, deref to get T before cloning.
        // When core_wrapper=Arc: obj.field_name is Arc<T>, deref to get &T before cloning.
        // When newtype_wrapper: obj.field_name is NewtypeT; access .0 to get the inner primitive.
        let access_expr = if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
            format!("obj.{field_name}.0") // unwrap newtype inner value
        } else if field.core_wrapper == CoreWrapper::Arc || field.is_boxed {
            format!("(*obj.{field_name})") // deref Arc<T>/Box<T> to get &T
        } else {
            format!("obj.{field_name}")
        };
        write!(out, "{}", gen_value_to_c(&access_expr, &field.ty, "    ")).ok();
    }

    out
}

// ---------------------------------------------------------------------------
// Enum conversions
// ---------------------------------------------------------------------------

pub(super) fn gen_enum_from_i32(enum_def: &EnumDef, prefix: &str, _core_import: &str) -> String {
    let enum_snake = enum_def.name.to_snake_case();
    let enum_name = &enum_def.name;
    let mut out = String::with_capacity(2048);

    writeln!(
        out,
        "/// Convert an integer to a `{enum_name}` variant. Returns -1 on invalid input."
    )
    .ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(out, "/// Caller must ensure all pointer arguments are valid or null.").ok();
    writeln!(
        out,
        "/// Returned pointers must be freed with the appropriate free function."
    )
    .ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {prefix}_{enum_snake}_from_i32(value: i32) -> i32 {{"
    )
    .ok();
    writeln!(out, "    match value {{").ok();
    for (idx, variant) in enum_def.variants.iter().enumerate() {
        writeln!(out, "        {idx} => {idx}, // {}", variant.name).ok();
    }
    writeln!(out, "        _ => {{").ok();
    writeln!(out, "            set_last_error(1, \"Invalid {enum_name} variant\");").ok();
    writeln!(out, "            -1").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

pub(super) fn gen_enum_to_i32(enum_def: &EnumDef, prefix: &str, _core_import: &str) -> String {
    let enum_snake = enum_def.name.to_snake_case();
    let enum_name = &enum_def.name;
    let mut out = String::with_capacity(2048);

    writeln!(
        out,
        "/// Convert a `{enum_name}` variant name (C string) to its integer value. Returns -1 on invalid input."
    )
    .ok();
    writeln!(out, "/// # Safety").ok();
    writeln!(
        out,
        "/// Caller must ensure `ptr` is a valid pointer to a `c_char` or null."
    )
    .ok();
    writeln!(out, "#[unsafe(no_mangle)]").ok();
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {prefix}_{enum_snake}_from_str(name: *const c_char) -> i32 {{"
    )
    .ok();
    writeln!(out, "    if name.is_null() {{").ok();
    writeln!(out, "        set_last_error(1, \"Null pointer passed for enum name\");").ok();
    writeln!(out, "        return -1;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    let s = match unsafe {{ CStr::from_ptr(name) }}.to_str() {{").ok();
    writeln!(out, "        Ok(s) => s,").ok();
    writeln!(out, "        Err(_) => {{").ok();
    writeln!(out, "            set_last_error(1, \"Invalid UTF-8 in enum name\");").ok();
    writeln!(out, "            return -1;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }};").ok();
    writeln!(out, "    match s {{").ok();
    for (idx, variant) in enum_def.variants.iter().enumerate() {
        writeln!(out, "        \"{}\" => {idx},", variant.name).ok();
    }
    writeln!(out, "        _ => {{").ok();
    writeln!(out, "            set_last_error(1, \"Unknown {enum_name} variant\");").ok();
    writeln!(out, "            -1").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}
