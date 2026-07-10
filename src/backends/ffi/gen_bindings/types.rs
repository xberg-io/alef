use crate::backends::ffi::type_map::c_return_type_with_paths;
use crate::codegen::conversions::{core_enum_path, core_type_path};
use crate::codegen::naming::{pascal_to_snake, wire_variant_value};
use crate::core::ir::{CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

use super::helpers::{gen_ffi_unimplemented_body, gen_value_to_c, null_return_value};

fn is_primitive_c_type_override(c_type: &str) -> bool {
    matches!(
        c_type,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "f32"
            | "f64"
            | "int"
            | "bool"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "size_t"
            | "ssize_t"
            | "intptr_t"
            | "uintptr_t"
            | "ptrdiff_t"
            | "float"
            | "double"
            | "char"
    )
}

fn c_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

pub(super) fn gen_type_from_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let return_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_from_json.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => return_qualified,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

pub(super) fn gen_type_to_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let ptr_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_to_json.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => ptr_qualified,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

pub(super) fn gen_type_free(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let ptr_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_free.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => ptr_qualified,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_field_accessor(
    typ: &TypeDef,
    field: &FieldDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
    fields_c_types: &std::collections::HashMap<String, String>,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified_base = core_type_path(typ, core_import);
    let qualified = if typ.has_lifetime_params {
        format!("{qualified_base}<'static>")
    } else {
        qualified_base
    };
    let field_name = &field.name;

    let effective_ty = if field.optional {
        TypeRef::Optional(Box::new(field.ty.clone()))
    } else {
        field.ty.clone()
    };

    let field_core_import = if let Some(ref rust_path) = field.type_rust_path {
        let rust_path_norm = rust_path.replace('-', "_");
        if let Some(pos) = rust_path_norm.rfind("::") {
            let module_prefix = &rust_path_norm[..pos];
            if module_prefix == core_import
                || module_prefix.starts_with(&format!("{core_import}::"))
                || module_prefix.starts_with(&format!("{core_import}_"))
            {
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

    let lookup_key = format!("{}.{}", type_snake, field.name);
    let c_type_override = fields_c_types.get(&lookup_key).filter(|t| t.as_str() != "skip");
    let (mut ret_type, override_is_opaque_handle, override_type_name) = if let Some(override_type) = c_type_override {
        if !is_primitive_c_type_override(override_type) && override_type != "char*" {
            (
                format!("*mut {core_import}::{override_type}"),
                true,
                Some(override_type.clone()),
            )
        } else {
            (
                c_return_type_with_paths(&effective_ty, &field_core_import, path_map).into_owned(),
                false,
                None,
            )
        }
    } else {
        (
            c_return_type_with_paths(&effective_ty, &field_core_import, path_map).into_owned(),
            false,
            None,
        )
    };
    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }

    let null_ret = if override_is_opaque_handle {
        "std::ptr::null_mut()".to_string()
    } else {
        null_return_value(&effective_ty).to_string()
    };

    let needs_len_out = matches!(field.ty, TypeRef::Bytes);

    let body = gen_field_access_body(
        field,
        needs_len_out,
        enum_names,
        clone_names,
        override_is_opaque_handle,
        override_type_name.as_deref(),
    );

    crate::backends::ffi::template_env::render(
        "field_accessor_header.jinja",
        context! {
            field_name => field_name,
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
            ret_type => ret_type,
            needs_len_out => needs_len_out,
            null_return_value => null_ret,
            body => body,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Unwrap a field type to its underlying `Named` type name, peeling an outer
/// `Optional`. Returns `None` for primitives, strings, collections, etc.
fn underlying_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => underlying_named_type(inner),
        _ => None,
    }
}

/// Generate the body of a field accessor that reads from `obj.{field_name}`.
fn gen_field_access_body(
    field: &FieldDef,
    needs_len_out: bool,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
    override_is_opaque_handle: bool,
    override_type_name: Option<&str>,
) -> String {
    let field_name = &field.name;
    let mut out = String::with_capacity(2048);

    if override_is_opaque_handle && override_type_name.is_some() {
        let field_is_override_type = underlying_named_type(&field.ty)
            .zip(override_type_name)
            .is_some_and(|(field_named, ovr)| field_named == ovr);
        if !field_is_override_type {
            out.push_str("    std::ptr::null_mut()");
            return out;
        }
    }

    if field.optional {
        if needs_len_out {
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                context! { field_name => field_name },
            ));
            out.push_str("        Some(val) => {\n");
            out.push_str("            if !out_len.is_null() {\n");
            out.push_str("                // SAFETY: null check above guarantees out_len is a valid pointer.\n");
            out.push_str("                unsafe { *out_len = val.len(); }\n");
            out.push_str("            }\n");
            out.push_str("            val.as_ptr() as *mut u8\n");
            out.push_str("        }\n");
            out.push_str("        None => {\n");
            out.push_str("            if !out_len.is_null() {\n");
            out.push_str("                // SAFETY: null check above guarantees out_len is a valid pointer.\n");
            out.push_str("                unsafe { *out_len = 0; }\n");
            out.push_str("            }\n");
            out.push_str("            std::ptr::null_mut()\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
        } else if let TypeRef::Optional(inner) = &field.ty {
            let inner_null = null_return_value(&TypeRef::Optional(Box::new(*inner.clone())));
            let inner_val_expr = match inner.as_ref() {
                TypeRef::Primitive(_) => "*inner_val",
                _ => "inner_val",
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                context! { field_name => field_name },
            ));
            out.push_str("        Some(Some(inner_val)) => {\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => gen_value_to_c(inner_val_expr, inner, "            ", enum_names, clone_names),
                },
            ));
            out.push_str("        }\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                context! {
                    pattern => "Some(None)",
                    value => &inner_null.to_string(),
                },
            ));
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                context! {
                    pattern => "None",
                    value => &null_return_value(&TypeRef::Optional(Box::new(field.ty.clone()))).to_string(),
                },
            ));
            out.push_str("    }\n");
        } else {
            let val_expr = if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
                "val.0"
            } else if matches!(field.ty, TypeRef::Primitive(_)) {
                "*val"
            } else if field.is_boxed {
                "(**val)"
            } else {
                "val"
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                context! { field_name => field_name },
            ));
            out.push_str("        Some(val) => {\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => gen_value_to_c(val_expr, &field.ty, "            ", enum_names, clone_names),
                },
            ));
            out.push_str("        }\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                context! {
                    pattern => "None",
                    value => &null_return_value(&TypeRef::Optional(Box::new(field.ty.clone()))).to_string(),
                },
            ));
            out.push_str("    }\n");
        }
    } else if needs_len_out {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_field_access.jinja",
            context! { field_name => field_name },
        ));
        out.push_str("    if !out_len.is_null() {\n");
        out.push_str("// SAFETY: null check above guarantees out_len is a valid pointer.\n");
        out.push_str("        unsafe { *out_len = data.len(); }\n");
        out.push_str("    }\n");
        out.push_str("    data.as_ptr() as *mut u8\n");
    } else {
        let access_expr = if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
            format!("obj.{field_name}.0")
        } else if field.core_wrapper == CoreWrapper::Arc || field.is_boxed {
            format!("(*obj.{field_name})")
        } else {
            format!("obj.{field_name}")
        };
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_value_to_c(&access_expr, &field.ty, "    ", enum_names, clone_names),
            },
        ));
    }

    out
}

pub(super) fn gen_enum_from_i32(enum_def: &EnumDef, prefix: &str, _core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let variants: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

    crate::backends::ffi::template_env::render(
        "enum_from_i32.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            variants => variants,
        },
    )
}

pub(super) fn gen_enum_to_i32(enum_def: &EnumDef, prefix: &str, _core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let variants: Vec<String> = enum_def
        .variants
        .iter()
        .map(|v| wire_variant_value(&v.name, v.serde_rename.as_deref(), enum_def.serde_rename_all.as_deref()))
        .collect();

    crate::backends::ffi::template_env::render(
        "enum_to_i32.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            variants => variants,
        },
    )
}

/// Generate a private Rust helper `fn {enum_snake}_from_i32_rs(v: i32) -> Option<{qualified}>`.
///
/// This helper is used by generated FFI function bodies to reconstruct an enum value from its
/// `i32` discriminant. It is `pub(crate)` to avoid unused-item warnings and is not exported
/// to C. All FFI parameter-crossing enums need this helper regardless of their `Copy` status.
pub(super) fn gen_enum_from_i32_rs_helper(enum_def: &EnumDef, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let qualified = core_enum_path(enum_def, core_import);

    let mut arms = String::new();
    for (i, variant) in enum_def.variants.iter().enumerate() {
        arms.push_str(&crate::backends::ffi::template_env::render(
            "ffi_enum_from_i32_rs_arm.jinja",
            context! {
                index => i,
                qualified => qualified.clone(),
                variant_name => variant.name.clone(),
            },
        ));
    }

    crate::backends::ffi::template_env::render(
        "ffi_enum_from_i32_rs_helper.jinja",
        context! {
            enum_snake => enum_snake,
            qualified => qualified,
            arms => arms,
        },
    )
}

/// Generate a `_free` function for an enum type returned as a heap-allocated pointer.
///
/// These are needed when a function returns `*mut EnumType` (via `Box::into_raw`), and the
/// caller must free the allocation. This applies to enums that derive `Copy`/`Clone` but are
/// returned through the pointer-based FFI API (e.g. field accessor methods on struct types).
pub(super) fn gen_enum_free(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_free.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

/// Generate a `_to_json` function for an enum type returned as a heap-allocated pointer.
///
/// Serializes the enum to a JSON string using serde. Only generated for enums that
/// derive `Serialize` (i.e. `has_serde` is true).
pub(super) fn gen_enum_to_json(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_to_json.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

/// Generate a `_to_string` function for an enum type returned as a heap-allocated pointer.
///
/// Renders the unit-variant name as serde would serialize it (e.g.
/// `BatchStatus::Completed` → `"completed"`), but stripped of the surrounding
/// JSON quotes so plain C string-comparison works. Only generated for enums
/// whose runtime serialization yields a string (`has_serde`); compound enums
/// would JSON-encode as objects and `as_str()` returns `None`.
pub(super) fn gen_enum_to_string(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_to_string.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

/// Generate a `_from_json` function for an enum type (for parameter passing from Java).
///
/// Deserializes the enum from a JSON string. Only generated for enums that
/// derive `Deserialize` (i.e. `has_serde` is true).
pub(super) fn gen_enum_from_json(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_from_json.jinja",
        context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

pub(super) fn gen_type_new(
    typ: &TypeDef,
    prefix: &str,
    core_import: &str,
    params_str: &str,
    body: &str,
    err_ty: &str,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);

    crate::backends::ffi::template_env::render(
        "type_new.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
            params => params_str,
            body => body,
            err_ty => err_ty,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate an opaque static constructor from a method definition.
///
/// For an opaque type with a static method that returns `Self` or `Result<Self, E>`,
/// emits an `#[no_mangle] pub unsafe extern "C" fn {prefix}_{type_snake}_{method_name}(...) -> *mut {TypeOpaque}`
/// that wraps the core call and returns a heap-allocated opaque handle.
///
/// The FFI symbol name is derived from the method name (e.g. `compile` → `_compile`,
/// `new` → `_new`), NOT hardcoded to `_new`. This allows named constructors like
/// `MetaSchema::compile` to be exported alongside or instead of `new`.
///
/// Parameters are marshalled from FFI types (enum params as i32, strings as *const c_char, etc.)
/// to core types via param conversion helpers. If the method signature is sanitized,
/// an unimplemented stub is generated instead.
pub(super) fn gen_opaque_static_constructor(
    typ: &TypeDef,
    method: &crate::core::ir::MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &ahash::AHashMap<String, String>,
    enum_names: &ahash::AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;

    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let return_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };
    let method_snake = c_symbol_component(&method.name);
    let ffi_fn_name = format!("{prefix}_{type_snake}_{method_snake}");
    let will_be_unimplemented = method.sanitized;

    let mut out = String::with_capacity(4096);

    let mut ffi_params = Vec::new();
    for p in &method.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        let c_type = crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
            &p.ty,
            core_import,
            path_map,
            enum_names,
            p.is_mut,
        );
        ffi_params.push(format!("    {}: {}", param_name, c_type));

        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            ffi_params.push(format!("    {}: usize", len_param_name));
        }
    }

    let allow_clippy = if ffi_params.len() > 7 {
        "#[allow(clippy::too_many_arguments)]\n"
    } else {
        ""
    };

    out.push_str(&crate::backends::ffi::template_env::render(
        "ffi_opaque_constructor_header.jinja",
        context! {
            allow_clippy => allow_clippy,
            ffi_fn_name => ffi_fn_name.clone(),
            ffi_params => ffi_params.join(",\n"),
            return_qualified => return_qualified.clone(),
        },
    ));

    if method.error_type.is_some() {
        out.push_str("    clear_last_error();\n");
    }

    if will_be_unimplemented {
        let unsupported_return = TypeRef::Named(type_name.to_string());
        out.push_str(&gen_ffi_unimplemented_body(
            &unsupported_return,
            &format!("{type_name}::new"),
            method.error_type.is_some(),
        ));
        out.push('\n');
        out.push_str("}\n");
        return out;
    }

    for p in &method.params {
        match &p.ty {
            TypeRef::String => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_string_param.jinja",
                    context! { name => p.name.clone() },
                ));
            }
            TypeRef::Named(n) if enum_names.contains(n.as_str()) => {
                let enum_snake = c_symbol_component(n);
                let param_name = &p.name;
                let rs_name = format!("{param_name}_rs");
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_enum_discriminant_match.jinja",
                    context! {
                        rs_name => rs_name,
                        enum_snake => enum_snake,
                        name => param_name,
                        error_message => format!("invalid discriminant for {n}"),
                        fail_ret => "return std::ptr::null_mut();",
                    },
                ));
            }
            TypeRef::Named(_) => {
                // SAFETY: the pointer is generated by the caller from a valid T allocation;
                let param_name = &p.name;
                let rs_name = format!("{param_name}_rs");
                let clone_suffix = if p.is_ref { "" } else { ".clone()" };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_named_param.jinja",
                    context! {
                        rs_name => rs_name,
                        param_name => param_name,
                        clone_suffix => clone_suffix,
                    },
                ));
            }
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_bool_param.jinja",
                    context! { name => p.name.clone() },
                ));
            }
            _ => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_passthrough_param.jinja",
                    context! { name => p.name.clone() },
                ));
            }
        }
    }

    let call_args = method
        .params
        .iter()
        .map(|p| {
            if p.is_ref {
                format!("&{}_rs", p.name)
            } else {
                format!("{}_rs", p.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let _ = type_name;
    out.push_str(&crate::backends::ffi::template_env::render(
        "ffi_opaque_constructor_call.jinja",
        context! {
            qualified => qualified,
            method_name => method.name.clone(),
            call_args => call_args,
        },
    ));

    if method.error_type.is_some() {
        out.push_str("    match result {\n");
        out.push_str("        Ok(value) => Box::into_raw(Box::new(value)),\n");
        out.push_str("        Err(e) => {\n");
        out.push_str("            set_last_error(1, &e.to_string());\n");
        out.push_str("            std::ptr::null_mut()\n");
        out.push_str("        }\n");
        out.push_str("    }\n");
    } else {
        out.push_str("    Box::into_raw(Box::new(result))\n");
    }
    out.push_str("}\n");

    out
}

/// Check if a method is an opaque static constructor eligible for C export.
///
/// A static constructor must:
/// - be marked `is_static` (no `self` receiver)
/// - return the owner type by value — either `TypeRef::Named(type_name)` (infallible)
///   or unwrapped through a `Result<Self, _>` return (the error type is tracked in
///   `method.error_type`, so the IR `return_type` is still `Named(type_name)` even for
///   fallible constructors)
///
/// There is NO name restriction: `new`, `compile`, `from_config`, etc. are all eligible.
/// This enables named constructors like `Schema::compile` to be exported as
/// `{prefix}_schema_compile` rather than being silently dropped.
///
/// Excluded:
/// - `default` — the `_default` C symbol collides with the auto-emitted `_new` when
///   both `new` and `default` exist on a type. `default` is almost always an
///   infallible no-arg factory; callers use the `_new` path.
/// - `to_json` / `from_json` — lifecycle helpers emitted by a separate path.
/// - `clone` — not a constructor; clones an existing instance.
pub(super) fn is_static_constructor(method: &crate::core::ir::MethodDef, type_name: &str) -> bool {
    if !method.is_static {
        return false;
    }
    if matches!(method.name.as_str(), "default" | "to_json" | "from_json" | "clone") {
        return false;
    }
    if method.returns_ref {
        return false;
    }
    match &method.return_type {
        crate::core::ir::TypeRef::Named(n) => n == type_name,
        _ => false,
    }
}
