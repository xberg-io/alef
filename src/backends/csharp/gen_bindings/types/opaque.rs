use super::super::errors::{emit_return_marshalling_indented, emit_return_statement, emit_return_statement_indented};
use super::super::{
    StreamingMethodMeta, emit_named_param_setup, emit_named_param_teardown, emit_named_param_teardown_indented,
    returns_ptr,
};
use super::constructors::{gen_opaque_factory_method, gen_opaque_static_constructor, is_static_constructor};
use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::csharp::gen_bindings) fn gen_opaque_handle(
    typ: &TypeDef,
    types: &[TypeDef],
    namespace: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    all_opaque_type_names: &HashSet<String>,
    client_constructor: Option<&ClientConstructorConfig>,
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let has_streaming = typ
        .methods
        .iter()
        .any(|m| streaming_methods.contains(&m.name) && streaming_methods_meta.contains_key(&m.name));
    let has_methods = has_streaming || typ.methods.iter().any(|m| !streaming_methods.contains(&m.name));
    let uses_list = |tr: &TypeRef| -> bool {
        matches!(tr, TypeRef::Vec(_))
            || matches!(tr, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)))
    };
    let needs_list = has_streaming
        || (has_methods
            && typ
                .methods
                .iter()
                .any(|m| uses_list(&m.return_type) || m.params.iter().any(|p| uses_list(&p.ty))));
    let needs_async = has_streaming
        || (has_methods
            && typ
                .methods
                .iter()
                .any(|m| m.is_async && !streaming_methods.contains(&m.name)));

    let class_name = csharp_type_name(&typ.name);
    let free_method = format!("{}Free", class_name);

    let doc_lines = super::super::sanitize_doc_lines_for_csharp(&typ.doc);
    let has_doc = !doc_lines.is_empty();

    let mut out = render(
        "opaque_handle_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "free_method": free_method,
            "has_methods": has_methods,
            "needs_list": needs_list,
            "needs_async": needs_async,
            "needs_streaming": has_streaming,
            "doc": has_doc,
            "doc_lines": doc_lines,
        })),
    );
    out.push('\n');

    let true_opaque_types = all_opaque_type_names;
    for method in &typ.methods {
        if streaming_methods.contains(&method.name) {
            if let Some(meta) = streaming_methods_meta.get(&method.name) {
                out.push('\n');
                out.push_str(&gen_opaque_streaming_method(method, &class_name, exception_name, meta));
            }
            continue;
        }
        if method.returns_ref_to_owner(&typ.name) {
            continue;
        }
        if is_static_constructor(method, &typ.name) {
            out.push('\n');
            out.push_str(&gen_opaque_static_constructor(
                method,
                &class_name,
                exception_name,
                types,
                enum_names,
                true_opaque_types,
            ));
            continue;
        }
        out.push('\n');
        out.push_str(&gen_opaque_method(
            method,
            types,
            &class_name,
            exception_name,
            enum_names,
            true_opaque_types,
        ));
    }

    if let Some(ctor) = client_constructor {
        out.push('\n');
        out.push_str(&gen_opaque_factory_method(&class_name, exception_name, ctor));
    }

    out.push_str("}\n");

    out
}

/// Map a Rust FFI type string to the C# type used in the public factory method signature.
pub(super) fn gen_opaque_streaming_method(
    method: &MethodDef,
    class_name: &str,
    exception_name: &str,
    meta: &StreamingMethodMeta,
) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let cs_method_name = to_csharp_name(&method.name);
    let cs_type_name = class_name.to_string();
    let item_pascal = csharp_type_name(&meta.item_type);

    let req_param = method.params.iter().find(|p| matches!(&p.ty, TypeRef::Named(_)));
    let (req_pascal, req_param_name) = match req_param {
        Some(p) => match &p.ty {
            TypeRef::Named(n) => (csharp_type_name(n), p.name.to_lower_camel_case()),
            _ => (item_pascal.clone(), "req".to_string()),
        },
        None => (item_pascal.clone(), "req".to_string()),
    };
    let req_param_type = req_pascal.clone();

    let start_native = format!("{cs_type_name}{cs_method_name}Start");
    let next_native = format!("{cs_type_name}{cs_method_name}Next");
    let free_native = format!("{cs_type_name}{cs_method_name}Free");
    let req_from_json = format!("{req_pascal}FromJson");
    let req_free = format!("{req_pascal}Free");
    let item_to_json = format!("{item_pascal}ToJson");
    let item_free = format!("{item_pascal}Free");

    let doc_lines = super::super::sanitize_doc_lines_for_csharp(&method.doc);
    let has_doc = !doc_lines.is_empty();
    let public_method_name = if cs_method_name.ends_with("Async") {
        cs_method_name.clone()
    } else {
        format!("{cs_method_name}Async")
    };
    render(
        "opaque_streaming_method.jinja",
        Value::from_serialize(serde_json::json!({
            "has_doc": has_doc,
            "doc_lines": doc_lines,
            "method_name": public_method_name,
            "item_type": item_pascal,
            "request_type": req_param_type,
            "request_param": req_param_name,
            "request_from_json": req_from_json,
            "request_free": req_free,
            "start_native": start_native,
            "next_native": next_native,
            "free_native": free_native,
            "item_to_json": item_to_json,
            "item_free": item_free,
            "exception_name": exception_name,
        })),
    )
}

/// Generate a single public method on an opaque handle class.
///
/// The method delegates to `NativeMethods.{TypeName}{MethodName}(this.Handle, ...)`.
pub(super) fn gen_opaque_method(
    method: &MethodDef,
    types: &[TypeDef],
    class_name: &str,
    exception_name: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    let visible_params: Vec<crate::core::ir::ParamDef> = method.params.clone();

    let method_doc_lines = super::super::sanitize_doc_lines_for_csharp(&method.doc);
    if !method_doc_lines.is_empty() {
        out.push_str(&render(
            "doc_comment_block.jinja",
            minijinja::context! {
                has_doc => true,
                indent => "    ",
                doc_lines => method_doc_lines,
            },
        ));
    }

    let return_type_str = if method.is_async {
        if method.return_type == TypeRef::Unit {
            "async Task".to_string()
        } else {
            let return_type = csharp_type(&method.return_type);
            render("async_task_return_type.jinja", minijinja::context! { return_type })
                .trim_end_matches('\n')
                .to_string()
        }
    } else if method.return_type == TypeRef::Unit {
        "void".to_string()
    } else {
        csharp_type(&method.return_type).to_string()
    };

    let method_cs_name = to_csharp_name(&method.name);
    let public_method_name = if method.is_async && !method_cs_name.ends_with("Async") {
        format!("{method_cs_name}Async")
    } else {
        method_cs_name.clone()
    };
    let is_static = method.is_static || method.receiver.is_none();
    let static_kw = if is_static { "static " } else { "" };
    out.push_str(
        render(
            "opaque_method_header.jinja",
            minijinja::context! { static_kw, return_type_str, method_cs_name => public_method_name },
        )
        .trim_end_matches('\n'),
    );

    for (i, param) in visible_params.iter().enumerate() {
        let param_name = param.name.to_lower_camel_case();
        let param_type = csharp_type(&param.ty);
        if param.optional && !param_type.ends_with('?') {
            out.push_str(
                render(
                    "param_decl_optional.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_required.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        }
        if i < visible_params.len() - 1 {
            out.push_str(", ");
        }
    }
    out.push_str(")\n    {\n");

    emit_named_param_setup(
        &mut out,
        &visible_params,
        "        ",
        true_opaque_types,
        exception_name,
        types,
        enum_names,
    );

    let cs_native_name = format!("{class_name}{method_cs_name}");

    if super::super::functions::is_bytes_result_method(method) {
        let mut args_block = String::new();
        let arg_indent = if method.is_async {
            "                "
        } else {
            "            "
        };
        if !is_static {
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => arg_indent, arg => "Handle" },
            ));
        }
        for param in visible_params.iter() {
            let param_name = param.name.to_lower_camel_case();
            let arg = super::super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
            args_block.push_str(&render(
                "native_arg_line.jinja",
                minijinja::context! { indent => arg_indent, arg },
            ));
            if matches!(param.ty, TypeRef::Bytes) {
                args_block.push_str(&render(
                    "native_bytes_len_arg_line.jinja",
                    minijinja::context! { indent => arg_indent, param_name, optional => param.optional },
                ));
            }
        }
        out.push_str(&render(
            "opaque_bytes_result_call.jinja",
            minijinja::context! {
                is_async => method.is_async,
                native_method_name => &cs_native_name,
                args_block => &args_block,
                exception_name,
            },
        ));
        out.push_str("    }\n\n");
        return out;
    }

    if method.is_async {
        if method.return_type == TypeRef::Unit {
            out.push_str("        await Task.Run(() =>\n        {\n");
        } else {
            out.push_str("        return await Task.Run(() =>\n        {\n");
        }

        if method.return_type != TypeRef::Unit {
            out.push_str("            var nativeResult = ");
        } else {
            out.push_str("            ");
        }

        out.push_str(&render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        ));
        if !is_static {
            out.push_str("                Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(",\n");
                out.push_str(render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_async.jinja",
                            minijinja::context! { arg => super::super::bytes_len_arg("(nuint)", &param_name, param.optional) },
                        )
                        .trim_end_matches('\n'),
                    );
                }
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(
                        render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'),
                    );
                } else {
                    out.push_str(",\n");
                    out.push_str(
                        render("indented_arg_async.jinja", minijinja::context! { arg }).trim_end_matches('\n'),
                    );
                }
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_async.jinja",
                            minijinja::context! { arg => super::super::bytes_len_arg("(nuint)", &param_name, param.optional) },
                        )
                        .trim_end_matches('\n'),
                    );
                }
            }
        }
        out.push_str("\n            );\n");

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(&render(
                    "null_result_return.jinja",
                    minijinja::context! { indent => "            " },
                ));
            } else {
                out.push_str(&render(
                    "null_result_throw.jinja",
                    minijinja::context! { indent => "            ", exception_name, cs_native_name },
                ));
            }
        } else if method.error_type.is_some() {
            out.push_str(&render(
                "last_error_context_throw.jinja",
                minijinja::context! { indent => "            ", operation => &cs_native_name, exception_name },
            ));
        }

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "            ",
            enum_names,
            true_opaque_types,
            &HashSet::new(),
        );
        emit_named_param_teardown_indented(&mut out, &visible_params, "            ", true_opaque_types, enum_names);
        emit_return_statement_indented(&mut out, &method.return_type, "            ");
        out.push_str("        });\n");
    } else {
        if method.return_type != TypeRef::Unit {
            out.push_str("        var nativeResult = ");
        } else {
            out.push_str("        ");
        }

        out.push_str(&render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        ));
        if !is_static {
            out.push_str("            Handle");
            for param in &visible_params {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                out.push_str(",\n");
                out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_sync.jinja",
                            minijinja::context! { arg => super::super::bytes_len_arg("(nuint)", &param_name, param.optional) },
                        )
                        .trim_end_matches('\n'),
                    );
                }
            }
        } else {
            for (i, param) in visible_params.iter().enumerate() {
                let param_name = param.name.to_lower_camel_case();
                let arg = super::super::native_call_arg(&param.ty, &param_name, param.optional, true_opaque_types);
                if i == 0 {
                    out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                } else {
                    out.push_str(",\n");
                    out.push_str(render("indented_arg_sync.jinja", minijinja::context! { arg }).trim_end_matches('\n'));
                }
                if matches!(param.ty, TypeRef::Bytes) {
                    out.push_str(",\n");
                    out.push_str(
                        render(
                            "indented_arg_sync.jinja",
                            minijinja::context! { arg => super::super::bytes_len_arg("(nuint)", &param_name, param.optional) },
                        )
                        .trim_end_matches('\n'),
                    );
                }
            }
        }
        out.push_str("\n        );\n");

        if method.return_type != TypeRef::Unit && returns_ptr(&method.return_type) {
            if matches!(method.return_type, TypeRef::Optional(_)) {
                out.push_str(&render(
                    "null_result_return.jinja",
                    minijinja::context! { indent => "        " },
                ));
            } else {
                out.push_str(&render(
                    "null_result_throw.jinja",
                    minijinja::context! { indent => "        ", exception_name, cs_native_name },
                ));
            }
        } else if method.error_type.is_some() {
            out.push_str(&render(
                "last_error_context_throw.jinja",
                minijinja::context! { indent => "        ", operation => &cs_native_name, exception_name },
            ));
        }

        emit_return_marshalling_indented(
            &mut out,
            &method.return_type,
            "        ",
            enum_names,
            true_opaque_types,
            &HashSet::new(),
        );
        emit_named_param_teardown(&mut out, &visible_params, true_opaque_types, enum_names);
        emit_return_statement(&mut out, &method.return_type);
    }

    out.push_str("    }\n");
    out
}
