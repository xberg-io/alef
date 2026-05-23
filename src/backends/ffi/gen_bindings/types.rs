use crate::backends::ffi::type_map::c_return_type_with_paths;
use crate::codegen::conversions::{core_enum_path, core_type_path};
use crate::core::ir::{CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;
use minijinja::context;

use super::helpers::{gen_value_to_c, null_return_value};

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
            // C <stdint.h> / scalar spellings — a field override may name the C
            // type directly (e.g. `uint64_t`); these are still primitives and
            // must not be mistaken for opaque handle types.
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

// ---------------------------------------------------------------------------
// Type: from_json + free
// ---------------------------------------------------------------------------

pub(super) fn gen_type_from_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);

    crate::backends::ffi::template_env::render(
        "type_from_json.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

pub(super) fn gen_type_to_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);

    crate::backends::ffi::template_env::render(
        "type_to_json.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

pub(super) fn gen_type_free(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);

    crate::backends::ffi::template_env::render(
        "type_free.jinja",
        context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
        },
    )
}

// ---------------------------------------------------------------------------
// Field accessors
// ---------------------------------------------------------------------------

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
        // or "kreuzberg::types::OutputFormat" (already fully qualified with crate prefix)
        // or "mylib_http::openapi::OpenApiConfig" (sibling workspace crate, common in
        // multi-crate workspaces where the umbrella crate re-exports types).
        // We need the module path prefix without the type name itself.
        // Normalize dashes to underscores since IR paths use Cargo package names (dashes)
        // but Rust source code requires crate names (underscores).
        let rust_path_norm = rust_path.replace('-', "_");
        if let Some(pos) = rust_path_norm.rfind("::") {
            let module_prefix = &rust_path_norm[..pos];
            // Avoid double-prefixing: detect when module_prefix is already crate-qualified
            // — either with core_import directly, or with a sibling workspace crate whose
            // name starts with the same prefix (e.g. core_import "mylib" → "mylib_http",
            // "mylib_core", "mylib_extra"). The trailing `::` and `_` checks ensure
            // we only match crate-name segments, not unrelated identifiers.
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
    // The e2e `fields_c_types` map carries a `"skip"` sentinel meaning "omit this
    // field from generated C e2e assertions". It is not a C type spelling — the
    // binding accessor is still generated normally from the IR field type, so the
    // sentinel must be ignored here rather than treated as an opaque handle name.
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
        // Use path_map for Named types — it knows where the type actually lives
        // (e.g. mylib_http::ContactInfo) even when field.type_rust_path is None.
        // For non-Named types path_map is irrelevant and the call falls through to
        // the standard c_return_type behaviour.
        (
            c_return_type_with_paths(&effective_ty, &field_core_import, path_map).into_owned(),
            false,
            None,
        )
    };
    // Replace "Self" with the actual qualified type name in FFI signatures
    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }

    let null_ret = if override_is_opaque_handle {
        // Opaque-handle overrides return `*mut T`; the null sentinel is a null pointer.
        "std::ptr::null_mut()".to_string()
    } else {
        null_return_value(&effective_ty).to_string()
    };

    // Determine if we need an extra out-param for byte-length
    let needs_len_out = matches!(field.ty, TypeRef::Bytes) && !field.optional;

    // Generate the accessor body based on field type and any override
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

    // An opaque-handle override is a *fallback* for the genuinely impossible case:
    // a primitive field (e.g. `bool`) whose C type is overridden to a struct handle
    // (e.g. `CitationResult*`) — there is no value to box, so return null.
    //
    // But when the field's own type is *already* the Named override type (e.g. a
    // `status: BatchStatus` field overridden to `BatchStatus`), the override is
    // redundant for codegen: the accessor can box-and-return the real field value.
    // Fall through to normal codegen in that case rather than emitting a null stub.
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
                "val.0" // unwrap newtype inner value
            } else if matches!(field.ty, TypeRef::Primitive(_)) {
                "*val" // dereference for Copy types
            } else if field.is_boxed {
                "(**val)" // deref &Box<T> -> &T
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
        // Bytes with length out-param
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
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_value_to_c(&access_expr, &field.ty, "    ", enum_names, clone_names),
            },
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// Enum conversions
// ---------------------------------------------------------------------------

pub(super) fn gen_enum_from_i32(enum_def: &EnumDef, prefix: &str, _core_import: &str) -> String {
    let enum_snake = enum_def.name.to_snake_case();
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
    let enum_snake = enum_def.name.to_snake_case();
    let enum_name = &enum_def.name;
    let variants: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

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

/// Generate a `_free` function for an enum type returned as a heap-allocated pointer.
///
/// These are needed when a function returns `*mut EnumType` (via `Box::into_raw`), and the
/// caller must free the allocation. This applies to enums that derive `Copy`/`Clone` but are
/// returned through the pointer-based FFI API (e.g. field accessor methods on struct types).
pub(super) fn gen_enum_free(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = enum_def.name.to_snake_case();
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
    let enum_snake = enum_def.name.to_snake_case();
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
    let enum_snake = enum_def.name.to_snake_case();
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
    let enum_snake = enum_def.name.to_snake_case();
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
    let type_snake = typ.name.to_snake_case();
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
        },
    )
}
