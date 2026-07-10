use super::super::{emit_named_param_setup, emit_named_param_teardown};
use crate::backends::csharp::type_map::csharp_type;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

pub(super) fn ffi_ty_to_csharp_public(rust_ty: &str) -> &'static str {
    let normalized = rust_ty.trim();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return "string";
    }
    if matches!(normalized, "bool") {
        return "bool";
    }
    if matches!(normalized, "u8" | "uint8_t") {
        return "byte";
    }
    if matches!(normalized, "u16" | "uint16_t") {
        return "ushort";
    }
    if matches!(normalized, "u32" | "uint32_t") {
        return "uint";
    }
    if matches!(normalized, "u64" | "uint64_t" | "usize") {
        return "ulong";
    }
    if matches!(normalized, "i8" | "int8_t") {
        return "sbyte";
    }
    if matches!(normalized, "i16" | "int16_t") {
        return "short";
    }
    if matches!(normalized, "i32" | "int32_t" | "c_int") {
        return "int";
    }
    if matches!(normalized, "i64" | "int64_t" | "isize") {
        return "long";
    }
    if matches!(normalized, "f32" | "float") {
        return "float";
    }
    if matches!(normalized, "f64" | "double") {
        return "double";
    }
    "IntPtr"
}

/// Check if a method is a static constructor (named `new`, returns the owner type by name).
///
/// Returns true ONLY for static `new` methods that take at least one parameter. Zero-arg
/// `new()` constructors are already emitted by `gen_opaque_factory_method` via the
/// configured `client_constructor`, so emitting them here too would produce duplicate
/// definitions.
///
/// Methods like `with_cache_dir` that return the owner type are NOT constructors and are
/// handled as regular static factory methods. Only methods named `new` generate public
/// C# constructors.
pub(super) fn is_static_constructor(method: &MethodDef, type_name: &str) -> bool {
    if method.name != "new" {
        return false;
    }
    if !method.is_static || method.params.is_empty() {
        return false;
    }
    match &method.return_type {
        TypeRef::Named(n) => n == type_name,
        _ => false,
    }
}

/// Generate a public C# constructor for a static `new` FFI method on an opaque type.
///
/// For a method like `RouteBuilder::new(method: Method, path: &str) -> Self`,
/// the FFI backend generates `{prefix}_route_builder_new(method: i32, path: *const c_char) -> *mut RouteBuilderOpaque`.
/// This emits a public C# constructor that marshals parameters and calls the FFI function.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_static_constructor(
    method: &MethodDef,
    class_name: &str,
    exception_name: &str,
    types: &[TypeDef],
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    let param_parts: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_name = p.name.to_lower_camel_case();
            let param_type = csharp_type(&p.ty);
            format!("{param_type} {param_name}")
        })
        .collect();
    let param_list = param_parts.join(", ");

    out.push_str(&render(
        "opaque_static_constructor_summary.jinja",
        minijinja::context! { class_name },
    ));

    out.push_str(&render(
        "opaque_static_constructor_signature.jinja",
        minijinja::context! { class_name, param_list },
    ));

    emit_named_param_setup(
        &mut out,
        &method.params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    let native_args: Vec<String> = method
        .params
        .iter()
        .map(|p| super::super::native_call_arg(&p.ty, &p.name.to_lower_camel_case(), p.optional, true_opaque_types))
        .collect();
    let native_args_str = native_args.join(", ");

    let ffi_method_name = format!("{}New", class_name);

    out.push_str(&render(
        "opaque_static_constructor_handle.jinja",
        minijinja::context! { ffi_method_name, native_args_str },
    ));

    out.push_str(&render(
        "opaque_static_constructor_error_check.jinja",
        minijinja::context! { exception_name, fallback_message => "Constructor failed" },
    ));

    emit_named_param_teardown(&mut out, &method.params, true_opaque_types, enum_names);

    out.push_str(&render(
        "opaque_safehandle_init.jinja",
        minijinja::context! { class_name },
    ));

    out.push_str("    }\n");

    out
}

pub(super) fn gen_opaque_factory_method(
    class_name: &str,
    exception_name: &str,
    ctor: &ClientConstructorConfig,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    let param_list: String = ctor
        .params
        .iter()
        .map(|p| {
            let cs_type = ffi_ty_to_csharp_public(&p.ty);
            let cs_name = p.name.to_lower_camel_case();
            format!("{cs_type} {cs_name}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let call_args: String = ctor
        .params
        .iter()
        .map(|p| p.name.to_lower_camel_case())
        .collect::<Vec<_>>()
        .join(", ");

    let native_method = format!("{class_name}New");

    out.push_str(&render(
        "opaque_factory_method.jinja",
        minijinja::context! { class_name, exception_name, param_list, native_method, call_args },
    ));

    out
}
