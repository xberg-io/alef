use super::super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::super::pinvoke::{is_bytes_result_func, is_bytes_result_method};
use super::super::{
    CAPSULE_PINVOKE_RETURN_TYPE, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    is_bridge_param, native_call_arg, native_call_args, needs_param_teardown, zero_sentinel_for_pinvoke_type,
};
use super::native_call::{FailureCheck, NativeCall};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::doc_emission;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Skip methods that take opaque handle FFI pointers as first arg but operate on non-opaque types.
/// These are validation/property functions that shouldn't be exposed as static methods.
/// Examples: header_metadata_is_valid, conversion_options_default (Rust naming, snake_case
pub(super) fn sanitize_doc_for_csharp(doc: &str) -> String {
    doc.lines()
        .filter_map(|line| {
            if line.trim().starts_with("use ") && line.contains("::") {
                return None;
            }
            Some(line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The cast a wrapper applies to a `byte[]` parameter's length before passing it.
const BYTES_LEN_CAST: &str = "(UIntPtr)";

/// The primary P/Invoke call of a free-function wrapper.
fn function_native_call<'a>(
    func: &'a FunctionDef,
    cs_native_name: &'a str,
    visible_params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    call_indent: &'a str,
    argument_indent: &'a str,
) -> NativeCall<'a> {
    NativeCall {
        cs_native_name,
        return_type: &func.return_type,
        receiver: None,
        has_error_type: func.error_type.is_some(),
        args: native_call_args(None, visible_params, true_opaque_types, BYTES_LEN_CAST),
        call_indent,
        argument_indent,
        failure_check: FailureCheck::LastError,
    }
}

/// The primary P/Invoke call of a method wrapper.
///
/// `receiver` is `method.receiver` untouched, including for a static method: that is exactly what
/// the FFI backend passes to the presence-companion eligibility authority, and the two must agree
/// or C# declares a `[DllImport]` for a symbol the crate never exported. ~keep
fn method_native_call<'a>(
    method: &'a MethodDef,
    cs_native_name: &'a str,
    visible_params: &[crate::core::ir::ParamDef],
    has_receiver: bool,
    true_opaque_types: &HashSet<String>,
    call_indent: &'a str,
    argument_indent: &'a str,
) -> NativeCall<'a> {
    NativeCall {
        cs_native_name,
        return_type: &method.return_type,
        receiver: method.receiver.as_ref(),
        has_error_type: method.error_type.is_some(),
        args: native_call_args(
            has_receiver.then_some("handle"),
            visible_params,
            true_opaque_types,
            BYTES_LEN_CAST,
        ),
        call_indent,
        argument_indent,
        failure_check: FailureCheck::NullSentinel,
    }
}

/// Generate a C# wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer
/// (`const TSLanguage *`) as an `IntPtr` for capsule types instead of an opaque alef handle.
/// The wrapper converts parameters, calls the C function, and constructs the host
/// `Language` (e.g. `new TreeSitter.Language(intPtr)`) from the raw pointer.
pub(super) fn gen_capsule_function_wrapper(
    func: &FunctionDef,
    exception_name: &str,
    _prefix: &str,
    cfg: &HostCapsuleTypeConfig,
) -> String {
    let mut out = String::with_capacity(1024);

    let func_cs_name = to_csharp_name(&func.name);
    doc_emission::emit_csharp_doc(&mut out, &func.doc, "        ", exception_name);

    let host_type = match cfg.required_host_type("Language", "csharp") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("        // ALEF ERROR: {e}\n"));
            return out;
        }
    };

    out.push_str(&format!("        public static {host_type} {func_cs_name}("));

    let param_strs: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let param_type = csharp_type(&p.ty);
            let param_name = p.name.to_lower_camel_case();
            format!("{param_type} {param_name}")
        })
        .collect();
    out.push_str(&param_strs.join(", "));
    out.push_str(")\n        {\n");

    let cs_native_name = to_csharp_name(&func.name);
    let c_params: Vec<String> = func.params.iter().map(|p| p.name.to_lower_camel_case()).collect();

    out.push_str("            var nativeResult = NativeMethods.");
    out.push_str(&cs_native_name);
    out.push('(');
    out.push_str(&c_params.join(", "));
    out.push_str(");\n");

    let zero = zero_sentinel_for_pinvoke_type(CAPSULE_PINVOKE_RETURN_TYPE);
    out.push_str(&format!("            if (nativeResult == {zero})\n"));
    out.push_str("            {\n");
    if matches!(func.return_type, TypeRef::Optional(_)) {
        out.push_str("                return null;\n");
    } else {
        out.push_str("                throw GetLastError();\n");
    }
    out.push_str("            }\n");

    if func.error_type.is_some() {
        out.push_str("            if (NativeMethods.LastErrorCode() != 0)\n");
        out.push_str("            {\n");
        out.push_str("                throw GetLastError();\n");
        out.push_str("            }\n");
    }

    let construct = match cfg.construct_required("nativeResult", "Language", "csharp") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("            // ALEF ERROR: {e}\n"));
            out.push_str("        }\n");
            return out;
        }
    };
    out.push_str(&format!("            return {construct};\n"));

    out.push_str("        }\n");

    out
}

/// Generate a static wrapper method for a streaming method on an opaque type.
/// Delegates to the instance method on the opaque handle class.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_wrapper_function(
    func: &FunctionDef,
    exception_name: &str,
    _prefix: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    enum_data_variant_names: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    _has_visitor_callbacks: bool,
    types: &[crate::core::ir::TypeDef],
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::with_capacity(1024);

    let visible_params: Vec<crate::core::ir::ParamDef> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    doc_emission::emit_csharp_doc(&mut out, &func.doc, "    ", exception_name);
    for param in &visible_params {
        if !func.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&render(
                "param_doc.jinja",
                crate::alef_context! { param_name, optional_text },
            ));
        }
    }

    out.push_str("    public static ");

    if func.is_async {
        if func.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            let return_type = csharp_type(&func.return_type);
            out.push_str(
                render("async_task_return_type.jinja", crate::alef_context! { return_type }).trim_end_matches('\n'),
            );
        }
    } else if func.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&func.return_type));
    }

    out.push(' ');
    let func_name = to_csharp_name(&func.name);
    if func.is_async && !func_name.ends_with("Async") {
        out.push_str(&func_name);
        out.push_str("Async");
    } else {
        out.push_str(&func_name);
    }
    out.push('(');

    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(
                render(
                    "param_decl_optional.jinja",
                    crate::alef_context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_required.jinja",
                    crate::alef_context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", crate::alef_context! { param_name }));
        }
    }

    if is_bytes_result_func(func) {
        let cs_native_name = to_csharp_name(&func.name);
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        let mut args_block = String::new();
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                crate::alef_context! { indent => "            ", arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    crate::alef_context! { indent => "            ", param_name, optional => param.optional },
                ));
            }
        }
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(
            &mut cleanup_block,
            &visible_params,
            "            ",
            true_opaque_types,
            enum_names,
        );
        out.push_str(&render(
            "bytes_result_call.jinja",
            crate::alef_context! {
                native_method_name => &cs_native_name,
                args_block => &args_block,
                cleanup_block => &cleanup_block,
            },
        ));
        out.push_str("    }\n\n");
        return out;
    }

    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    let cs_native_name = to_csharp_name(&func.name);

    let needs_outer_try = needs_param_teardown(&visible_params, true_opaque_types, enum_names);

    if func.is_async {
        if needs_outer_try {
            out.push_str("        try\n        {\n");
        }

        let lambda_indent = if needs_outer_try { "            " } else { "        " };
        let body_indent = if needs_outer_try {
            "                "
        } else {
            "            "
        };
        let argument_indent = if needs_outer_try {
            "                    "
        } else {
            "                "
        };

        out.push_str(lambda_indent);
        if func.return_type == TypeRef::Unit {
            out.push_str("await Task.Run(() =>\n");
        } else {
            out.push_str("return await Task.Run(() =>\n");
        }
        out.push_str(lambda_indent);
        out.push_str("{\n");

        function_native_call(
            func,
            &cs_native_name,
            &visible_params,
            true_opaque_types,
            body_indent,
            argument_indent,
        )
        .emit(&mut out);

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            body_indent,
            enum_names,
            true_opaque_types,
            handle_returned_types,
            enum_data_variant_names,
        );
        emit_return_statement_indented(&mut out, &func.return_type, body_indent);
        out.push_str(lambda_indent);
        out.push_str("});\n");

        if needs_outer_try {
            out.push_str("        }\n        finally\n        {\n");
            emit_named_param_teardown_indented(
                &mut out,
                &visible_params,
                "            ",
                true_opaque_types,
                enum_names,
            );
            out.push_str("        }\n");
        }
    } else {
        if needs_outer_try {
            out.push_str("        try\n        {\n");
        }

        let call_indent = if needs_outer_try { "            " } else { "        " };
        let argument_indent = if needs_outer_try {
            "                "
        } else {
            "            "
        };

        function_native_call(
            func,
            &cs_native_name,
            &visible_params,
            true_opaque_types,
            call_indent,
            argument_indent,
        )
        .emit(&mut out);

        let body_indent = call_indent;

        emit_return_marshalling_indented(
            &mut out,
            &func.return_type,
            body_indent,
            enum_names,
            true_opaque_types,
            handle_returned_types,
            enum_data_variant_names,
        );

        if needs_outer_try {
            emit_return_statement_indented(&mut out, &func.return_type, body_indent);
            out.push_str("        }\n        finally\n        {\n");
            emit_named_param_teardown_indented(
                &mut out,
                &visible_params,
                "            ",
                true_opaque_types,
                enum_names,
            );
            out.push_str("        }\n");
        } else {
            emit_named_param_teardown(&mut out, &visible_params, true_opaque_types, enum_names);
            emit_return_statement(&mut out, &func.return_type);
        }
    }

    out.push_str("    }\n\n");

    out
}

/// Generate a wrapper function for a function with a bridge field binding (e.g., visitor on options).
///
/// This handles functions where a trait bridge is injected into a struct field rather than
/// as a function parameter. The pattern:
/// 1. Extract the bridge value from the wrapped type (e.g., visitor from IHtmlVisitor)
/// 2. Serialize the options struct to JSON (skipping the bridge field)
/// 3. Deserialize into a native options handle
/// 4. If bridge present, create a bridge, inject into options, call convert, free bridge
/// 5. Otherwise, just call convert directly
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_wrapper_method(
    method: &MethodDef,
    exception_name: &str,
    _prefix: &str,
    type_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    enum_data_variant_names: &HashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    types: &[crate::core::ir::TypeDef],
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::with_capacity(1024);

    let visible_params: Vec<crate::core::ir::ParamDef> = method
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();

    let sanitized_doc = sanitize_doc_for_csharp(&method.doc);
    doc_emission::emit_csharp_doc(&mut out, &sanitized_doc, "    ", exception_name);
    for param in &visible_params {
        if !method.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&render(
                "param_doc.jinja",
                crate::alef_context! { param_name, optional_text },
            ));
        }
    }

    out.push_str("    public static ");

    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("async Task");
        } else {
            let return_type = csharp_type(&method.return_type);
            out.push_str(
                render("async_task_return_type.jinja", crate::alef_context! { return_type }).trim_end_matches('\n'),
            );
        }
    } else if method.return_type == TypeRef::Unit {
        out.push_str("void");
    } else {
        out.push_str(&csharp_type(&method.return_type));
    }

    let method_name = to_csharp_name(&method.name);
    let method_cs_name = if method.is_async && !method_name.ends_with("Async") {
        format!("{}{}Async", type_name, method_name)
    } else {
        format!("{}{}", type_name, method_name)
    };
    out.push(' ');
    out.push_str(&method_cs_name);
    out.push('(');

    let has_receiver = !method.is_static && method.receiver.is_some();
    if has_receiver {
        // The receiver crosses the C ABI as `AlefHandle` (`uint64_t`), not a pointer. ~keep
        out.push_str("ulong handle");
        if !visible_params.is_empty() {
            out.push_str(", ");
        }
    }

    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(
                render(
                    "param_decl_optional.jinja",
                    crate::alef_context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_required.jinja",
                    crate::alef_context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        }

        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }

    out.push_str(")\n    {\n");

    for param in &visible_params {
        let is_enum = matches!(&param.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        if !param.optional && !is_enum && matches!(param.ty, TypeRef::String | TypeRef::Named(_) | TypeRef::Bytes) {
            let param_name = param.name.to_lower_camel_case();
            out.push_str(&render("null_check.jinja", crate::alef_context! { param_name }));
        }
    }

    let cs_native_name = format!("{}{}", csharp_type_name(type_name), to_csharp_name(&method.name));

    if is_bytes_result_method(method) {
        emit_named_param_setup(
            &mut out,
            &visible_params,
            "        ",
            true_opaque_types,
            exception_name,
            types,
            enum_names,
        );
        let mut args_block = String::new();
        if has_receiver {
            args_block.push_str(&render(
                "native_arg_line.jinja",
                crate::alef_context! { indent => "            ", arg => "handle" },
            ));
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                crate::alef_context! { indent => "            ", arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    crate::alef_context! { indent => "            ", param_name, optional => param.optional },
                ));
            }
        }
        let mut cleanup_block = String::new();
        emit_named_param_teardown_indented(
            &mut cleanup_block,
            &visible_params,
            "            ",
            true_opaque_types,
            enum_names,
        );
        out.push_str(&render(
            "bytes_result_call.jinja",
            crate::alef_context! {
                native_method_name => &cs_native_name,
                args_block => &args_block,
                cleanup_block => &cleanup_block,
            },
        ));
        out.push_str("    }\n\n");
        return out;
    }

    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("        await Task.Run(() =>\n        {\n");
        } else {
            out.push_str("        return await Task.Run(() =>\n        {\n");
        }

        method_native_call(
            method,
            &cs_native_name,
            &visible_params,
            has_receiver,
            true_opaque_types,
            "            ",
            "                ",
        )
        .emit(&mut out);

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "            ",
            enum_names,
            true_opaque_types,
            handle_returned_types,
            enum_data_variant_names,
        );
        emit_named_param_teardown_indented(&mut out, &visible_params, "            ", true_opaque_types, enum_names);
        emit_return_statement_indented(&mut out, &method.return_type, "            ");
        out.push_str("        });\n");
    } else {
        method_native_call(
            method,
            &cs_native_name,
            &visible_params,
            has_receiver,
            true_opaque_types,
            "        ",
            "            ",
        )
        .emit(&mut out);

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "        ",
            enum_names,
            true_opaque_types,
            handle_returned_types,
            enum_data_variant_names,
        );
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types, enum_names);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::HostCapsuleTypeConfig;
    use crate::core::ir::{CoreWrapper, FunctionDef, ParamDef, TypeRef, VersionAnnotation};
    use std::collections::HashMap;

    #[test]
    fn capsule_function_wrapper_uses_correct_pinvoke_name() {
        let func = FunctionDef {
            name: "get_language".to_string(),
            rust_path: "test::get_language".to_string(),
            original_rust_path: "test::get_language".to_string(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: CoreWrapper::default(),
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation::default(),
        };

        let cfg = HostCapsuleTypeConfig {
            host_type: "TreeSitter.Language".to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: String::new(),
            ..Default::default()
        };

        let code = gen_capsule_function_wrapper(&func, "TestException", "sample_ffi", &cfg);

        assert!(
            code.contains("NativeMethods.GetLanguage(name)"),
            "Generated code should call NativeMethods.GetLanguage, got:\n{}",
            code
        );
        assert!(
            !code.contains("NativeMethods.sample_ffi_get_language"),
            "Generated code should NOT call NativeMethods.sample_ffi_get_language (snake_case), got:\n{}",
            code
        );
    }

    /// Regression: `bytes_result_call.jinja` used to call `NativeMethods.FreeBytes` on the
    /// success path inside `try`, before `return result;`, while `finally` held only the
    /// unrelated argument-marshalling `cleanup_block`. An exception from `Marshal.Copy` (between
    /// obtaining the native buffer and freeing it) skipped the free and leaked it. The free must
    /// now be the first statement in `finally`, guaranteed on every exit path. ~keep
    #[test]
    fn free_function_bytes_result_frees_inside_finally_not_inline() {
        let func = FunctionDef {
            name: "get_bytes".to_string(),
            rust_path: "test::get_bytes".to_string(),
            original_rust_path: "test::get_bytes".to_string(),
            params: vec![],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation::default(),
        };

        let code = gen_wrapper_function(
            &func,
            "TestException",
            "sample_ffi",
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
        );

        let finally_pos = code
            .find("        finally\n        {\n")
            .expect("missing finally block");
        let free_pos = code.find("NativeMethods.FreeBytes").expect("missing FreeBytes call");
        let return_pos = code.find("return result;").expect("missing return result;");
        assert!(
            return_pos < finally_pos && finally_pos < free_pos,
            "FreeBytes must run after the normal-path return, via `finally`, not inline before \
             it:\n{code}"
        );
    }

    fn func_returning(name: &str, return_type: TypeRef) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test::{name}"),
            original_rust_path: format!("test::{name}"),
            params: vec![],
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation::default(),
        }
    }

    fn wrap(func: &FunctionDef) -> String {
        gen_wrapper_function(
            func,
            "TestException",
            "sample_ffi",
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
        )
    }

    fn pinvoke(func: &FunctionDef, capsule_types: &HashMap<String, HostCapsuleTypeConfig>) -> String {
        super::super::super::pinvoke::gen_pinvoke_for_func(
            &format!("sample_ffi_{}", func.name),
            func,
            &HashSet::new(),
            &HashSet::new(),
            capsule_types,
            &ahash::AHashSet::new(),
        )
    }

    /// Regression for CS0034 (`Operator '==' is ambiguous on operands of type 'ulong' and 'nint'`):
    /// the null check must be derived from the same fact as the `extern` signature emitted for the
    /// same function, never from an independently-recomputed condition.
    ///
    /// The string half of this test is load-bearing and must not be deleted: the handle half alone
    /// would also be satisfied by blanket-replacing every `IntPtr.Zero` in the emitters with `0`,
    /// which would silently corrupt the far more common `char*`-returning wrappers whose P/Invoke
    /// genuinely returns `IntPtr`. Both directions have to hold at once. ~keep
    #[test]
    fn null_check_sentinel_matches_pinvoke_return_type_for_handle_and_string_returns() {
        let handle_func = func_returning(
            "find_thing",
            TypeRef::Optional(Box::new(TypeRef::Named("Thing".into()))),
        );
        let handle_decl = pinvoke(&handle_func, &HashMap::new());
        let handle_body = wrap(&handle_func);
        assert!(
            handle_decl.contains("internal static extern ulong FindThing("),
            "a handle return must be declared `ulong`:\n{handle_decl}"
        );
        assert!(
            handle_body.contains("if (nativeResult == 0)"),
            "a `ulong`-returning P/Invoke must be null-checked against the scalar `0`:\n{handle_body}"
        );
        assert!(
            !handle_body.contains("IntPtr.Zero"),
            "comparing a `ulong` against `IntPtr.Zero` is the CS0034 defect:\n{handle_body}"
        );

        let string_func = func_returning("find_name", TypeRef::Optional(Box::new(TypeRef::String)));
        let string_decl = pinvoke(&string_func, &HashMap::new());
        let string_body = wrap(&string_func);
        assert!(
            string_decl.contains("internal static extern IntPtr FindName("),
            "a `char*` return must be declared `IntPtr`:\n{string_decl}"
        );
        assert!(
            string_body.contains("if (nativeResult == IntPtr.Zero)"),
            "an `IntPtr`-returning P/Invoke must keep its `IntPtr.Zero` check:\n{string_body}"
        );
    }

    /// A host-native capsule return is exported by the FFI crate as a raw `*const T`
    /// (`backends::ffi::gen_bindings::capsule::capsule_c_return_type`), not as an `AlefHandle`, so
    /// its P/Invoke must say `IntPtr` even though the IR spells the return `TypeRef::Named`. Before
    /// the fix the declaration said `ulong` while the capsule wrapper compared against
    /// `IntPtr.Zero` — the exact CS0034 break, and the reason the sentinel is now derived from the
    /// declaration rather than recomputed from `TypeRef`. ~keep
    #[test]
    fn capsule_return_declares_intptr_and_checks_intptr_zero() {
        let func = func_returning("get_language", TypeRef::Named("Language".into()));
        let cfg = HostCapsuleTypeConfig {
            host_type: "TreeSitter.Language".to_string(),
            construct_expr: "new TreeSitter.Language({ptr})".to_string(),
            ..Default::default()
        };
        let capsule_types: HashMap<String, HostCapsuleTypeConfig> =
            [("Language".to_string(), cfg.clone())].into_iter().collect();

        let decl = pinvoke(&func, &capsule_types);
        assert!(
            decl.contains("internal static extern IntPtr GetLanguage("),
            "a capsule return crosses as a raw pointer and must be declared `IntPtr`:\n{decl}"
        );
        assert!(
            !pinvoke(&func, &HashMap::new()).contains("extern IntPtr GetLanguage("),
            "without capsule config the same `Named` return must still be the `ulong` AlefHandle"
        );

        let body = gen_capsule_function_wrapper(&func, "TestException", "sample_ffi", &cfg);
        assert!(
            body.contains("if (nativeResult == IntPtr.Zero)"),
            "the capsule wrapper must null-check against the sentinel its `IntPtr` declaration \
             pairs with:\n{body}"
        );
    }

    fn method_returning(name: &str, return_type: TypeRef) -> crate::core::ir::MethodDef {
        crate::core::ir::MethodDef {
            name: name.to_string(),
            return_type,
            is_static: true,
            ..Default::default()
        }
    }

    fn pinvoke_method(
        method: &crate::core::ir::MethodDef,
        capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
    ) -> String {
        super::super::super::pinvoke::gen_pinvoke_for_method(
            &format!("sample_ffi_registry_{}", method.name),
            "RegistryGetLanguage",
            method,
            capsule_types,
            &ahash::AHashSet::new(),
        )
    }

    /// The declared return type of a `[DllImport]`, or `None` when no declaration was emitted.
    /// Returning `Option` is what lets the cross-shape tests below prove they compared two real
    /// declarations instead of two empty strings. ~keep
    fn declared_return_type(declaration: &str) -> Option<&str> {
        const MARKER: &str = "internal static extern ";
        let rest = declaration.split_once(MARKER)?.1;
        rest.split_whitespace().next()
    }

    /// The invariant the extraction shape must never decide on its own: with no capsule config,
    /// the same `Named` return declares the same P/Invoke type whether the API surface presents
    /// it as a `FunctionDef` or as a `MethodDef`. Both emitters now derive that through
    /// `pinvoke_return_type_with_capsules`, so they cannot drift apart. ~keep
    #[test]
    fn named_return_declares_same_pinvoke_type_for_function_and_method_shapes() {
        let func = func_returning("get_language", TypeRef::Named("Language".into()));
        let method = method_returning("get_language", TypeRef::Named("Language".into()));

        let func_decl = pinvoke(&func, &HashMap::new());
        let method_decl = pinvoke_method(&method, &HashMap::new());
        let func_ty =
            declared_return_type(&func_decl).expect("the function shape must actually emit a `[DllImport]` to compare");
        let method_ty =
            declared_return_type(&method_decl).expect("the method shape must actually emit a `[DllImport]` to compare");

        assert_eq!(
            func_ty, method_ty,
            "the same `Named` return must declare the same P/Invoke type in both shapes; \
             function said `{func_ty}`, method said `{method_ty}`"
        );
        assert_eq!(
            func_ty, "ulong",
            "an alef-owned handle crosses as the `AlefHandle` scalar"
        );
    }

    /// The one place the two shapes legitimately disagree, pinned so it can only change
    /// deliberately.
    ///
    /// The FFI backend emits a capsule return differently per shape and the C# declaration must
    /// mirror whichever C signature exists: `gen_free_function` returns the host-owned
    /// `*const T` from `capsule::capsule_c_return_type` (`ffi/gen_bindings/functions/
    /// orchestration.rs:723`), while `gen_method_wrapper` (`orchestration.rs:69`) takes no capsule
    /// config and falls through to `AlefHandle` (`ffi/type_map.rs:279`) — made intentional by
    /// commit `6855c6d57`, which keeps a capsule type's opaque `_new`/`_free` exports alive
    /// precisely when it is reachable as a method return. Declaring `IntPtr` on the method path
    /// would therefore be a live ABI mismatch against a `uint64_t` symbol, not a fix. If the FFI
    /// method wrapper ever becomes capsule-aware, flip `FfiEmitter::Method` and this test
    /// together. ~keep
    #[test]
    fn capsule_return_shape_split_mirrors_the_ffi_emitter_that_produced_the_symbol() {
        let capsule_types: HashMap<String, HostCapsuleTypeConfig> = [(
            "Language".to_string(),
            HostCapsuleTypeConfig {
                host_type: "TreeSitter.Language".to_string(),
                construct_expr: "new TreeSitter.Language({ptr})".to_string(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect();

        let func = func_returning("get_language", TypeRef::Named("Language".into()));
        let method = method_returning("get_language", TypeRef::Named("Language".into()));

        let func_decl = pinvoke(&func, &capsule_types);
        let method_decl = pinvoke_method(&method, &capsule_types);
        let func_ty =
            declared_return_type(&func_decl).expect("the function shape must actually emit a `[DllImport]` to compare");
        let method_ty =
            declared_return_type(&method_decl).expect("the method shape must actually emit a `[DllImport]` to compare");

        assert_eq!(
            func_ty, "IntPtr",
            "a capsule return from a free function crosses as the host-owned `*const T`"
        );
        assert_eq!(
            method_ty, "ulong",
            "a capsule return from a method is boxed by alef and crosses as `AlefHandle`; \
             declaring `IntPtr` here would not match the emitted C symbol"
        );
    }

    /// `gen_wrapper_function` with caller-supplied `enum_names`/`enum_data_variant_names`,
    /// standing in for `wrap()` (which always passes empty sets and so can never exercise the
    /// enum-return branch under test here).
    fn wrap_enum(
        func: &FunctionDef,
        enum_names: &HashSet<String>,
        enum_data_variant_names: &HashSet<String>,
    ) -> String {
        gen_wrapper_function(
            func,
            "TestException",
            "sample_ffi",
            enum_names,
            &HashSet::new(),
            &HashSet::new(),
            enum_data_variant_names,
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
        )
    }

    /// Regression for the CS1503 root cause: `RefreshOutcome` is a Rust enum with a
    /// data-carrying variant, so it boxes as `AlefHandle` exactly like a struct return (the FFI
    /// crate's `gen_owned_value_to_c` has no enum-ness branch for owned conversion — see
    /// `enum_names_with_data_variants` in `marshalling.rs`). Before the fix, `enum_names`
    /// membership alone routed *any* enum return to the direct-`nativeResult`
    /// `Marshal.PtrToStringUTF8` branch, which is only valid for a value that already crosses as
    /// a raw pointer — but `nativeResult` here is `ulong` (see the paired `pinvoke` assertion),
    /// so that branch produced the exact `ulong`-to-`nint` CS1503 the bug report measured.
    ///
    /// Async and sync are asserted from the *same* enum_names/enum_data_variant_names input,
    /// proving the fix is keyed on enum-ness of the return type, not on `is_async` (the original,
    /// corrected hypothesis) — a sync free function with an identical data-carrying-enum return
    /// was equally broken before this fix, it simply had no example in the corpus that surfaced
    /// it. ~keep
    #[test]
    fn async_and_sync_data_carrying_enum_returns_both_use_to_json_round_trip() {
        let enum_names: HashSet<String> = ["RefreshOutcome".to_string()].into_iter().collect();
        let enum_data_variant_names = enum_names.clone();

        let mut async_func = func_returning("refresh_catalog", TypeRef::Named("RefreshOutcome".into()));
        async_func.is_async = true;
        let async_body = wrap_enum(&async_func, &enum_names, &enum_data_variant_names);

        let sync_func = func_returning("refresh_catalog", TypeRef::Named("RefreshOutcome".into()));
        let sync_body = wrap_enum(&sync_func, &enum_names, &enum_data_variant_names);

        for (label, body) in [("async", &async_body), ("sync", &sync_body)] {
            assert!(
                body.contains("NativeMethods.RefreshOutcomeToJson(nativeResult)"),
                "{label} data-carrying enum return must exchange the handle for JSON via the \
                 ToJson companion, matching a plain data struct return:\n{body}"
            );
            assert!(
                body.contains("NativeMethods.RefreshOutcomeFree(nativeResult)"),
                "{label} data-carrying enum return must free the handle after extracting JSON:\n{body}"
            );
            assert!(
                !body.contains("Marshal.PtrToStringUTF8(nativeResult)"),
                "{label} must never pass the `ulong` handle `nativeResult` straight to \
                 `Marshal.PtrToStringUTF8` (expects `nint`) — that is the CS1503 defect:\n{body}"
            );
        }
    }

    /// Structural companion to the test above: derives the P/Invoke declared return type from
    /// `gen_pinvoke_for_func`'s emitted text and the argument passed to
    /// `Marshal.PtrToStringUTF8(...)` from the wrapper body's emitted text — rather than
    /// hardcoding either spelling — and proves they can never agree for `nativeResult` itself
    /// (declared `ulong`, so passing it to a `Marshal.PtrToStringUTF8` call that wants `nint`
    /// cannot type-check), while the argument the body actually uses (`jsonPtr`, sourced from the
    /// separate `RefreshOutcomeToJson` call) is exactly the pointer-typed value that call needs.
    /// Survives a future template rename of `jsonPtr` because it greps the emitted text rather
    /// than asserting the literal identifier is `"jsonPtr"` up front. ~keep
    #[test]
    fn ptr_to_string_utf8_argument_never_names_the_ulong_declared_native_result() {
        let enum_names: HashSet<String> = ["RefreshOutcome".to_string()].into_iter().collect();
        let enum_data_variant_names = enum_names.clone();
        let func = func_returning("refresh_catalog", TypeRef::Named("RefreshOutcome".into()));

        let decl = pinvoke(&func, &HashMap::new());
        let declared_ty =
            declared_return_type(&decl).expect("the function shape must actually emit a `[DllImport]` to compare");
        assert_eq!(
            declared_ty, "ulong",
            "a data-carrying enum return declares the scalar AlefHandle, same as a struct return"
        );

        let body = wrap_enum(&func, &enum_names, &enum_data_variant_names);
        const CALL: &str = "Marshal.PtrToStringUTF8(";
        let call_start = body.find(CALL).expect("body must call PtrToStringUTF8 to extract JSON");
        let after_call = &body[call_start + CALL.len()..];
        let arg = after_call
            .split(')')
            .next()
            .expect("PtrToStringUTF8 call must be closed")
            .trim();

        assert_ne!(
            arg, "nativeResult",
            "PtrToStringUTF8's argument must never be the {declared_ty}-declared `nativeResult` \
             — that is the CS1503 ulong-to-nint defect:\n{body}"
        );
        assert_eq!(
            arg, "jsonPtr",
            "PtrToStringUTF8 must be called with the IntPtr-typed JSON pointer obtained from the \
             ToJson companion, not any other variable:\n{body}"
        );
    }

    /// Regression for alef task #155: `liter-llm` v1.17.3's real `RefreshOutcome` is
    /// fieldless-only (`Disabled`, `FromCache`, `Fetched`), not the data-carrying fixture the
    /// tests above use. `gen_owned_value_to_c` in the FFI crate has no fieldless-vs-data-carrying
    /// branch for owned return conversion — *every* enum return boxes as `AlefHandle` — so a
    /// fieldless-only enum needs the exact same `{Pascal}ToJson`/`{Pascal}Free` round trip. Before
    /// this fix, `enum_names_with_data_variants` (`marshalling.rs`) excluded fieldless enums from
    /// `enum_data_variant_names`, so this case fell through to the direct-`nativeResult`
    /// `Marshal.PtrToStringUTF8` branch and produced the exact `ulong`-to-`nint` CS1503 the bug
    /// report measured. ~keep
    #[test]
    fn fieldless_only_enum_return_now_uses_the_to_json_round_trip() {
        let enum_names: HashSet<String> = ["Status".to_string()].into_iter().collect();
        let func = func_returning("get_status", TypeRef::Named("Status".into()));
        // Mirrors the post-fix `enum_names_with_data_variants`, which no longer filters by
        // variant shape: it now equals `enum_names` for every real enum.
        let enum_data_variant_names = enum_names.clone();

        let body = wrap_enum(&func, &enum_names, &enum_data_variant_names);

        assert!(
            body.contains("NativeMethods.StatusToJson(nativeResult)"),
            "a fieldless-only enum return must exchange the handle for JSON via the ToJson \
             companion, matching a data-carrying enum return:\n{body}"
        );
        assert!(
            body.contains("NativeMethods.StatusFree(nativeResult)"),
            "a fieldless-only enum return must free the handle after extracting JSON:\n{body}"
        );
        assert!(
            !body.contains("Marshal.PtrToStringUTF8(nativeResult)"),
            "a fieldless-only enum return must never pass the `ulong` handle `nativeResult` \
             straight to `Marshal.PtrToStringUTF8` (expects `nint`) — that is the CS1503 \
             defect:\n{body}"
        );
    }
}
