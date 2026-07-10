use crate::backends::csharp::type_map::csharp_type;
use crate::codegen::doc_emission;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::ir::{FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Skip methods that take opaque handle FFI pointers as first arg but operate on non-opaque types.
/// These are validation/property functions that shouldn't be exposed as static methods.
/// Examples: header_metadata_is_valid, conversion_options_default (Rust naming, snake_case
pub(super) fn gen_bridge_field_wrapper_function(
    func: &FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    exception_name: &str,
    _enum_names: &HashSet<String>,
    _true_opaque_types: &HashSet<String>,
    _handle_returned_types: &HashSet<String>,
) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::with_capacity(2048);

    let visible_params: Vec<crate::core::ir::ParamDef> = func.params.to_vec();

    doc_emission::emit_csharp_doc(&mut out, &func.doc, "    ", exception_name);
    for param in &visible_params {
        if !func.doc.is_empty() {
            let param_name = param.name.to_lower_camel_case();
            let optional_text = if param.optional { "Optional." } else { "" };
            out.push_str(&render(
                "bridge_field_param_doc.jinja",
                minijinja::context! { param_name, optional_text },
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
                render("async_task_generic.jinja", minijinja::context! { return_type }).trim_end_matches('\n'),
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
                    "param_decl_inline_optional.jinja",
                    minijinja::context! { param_type, param_name },
                )
                .trim_end_matches('\n'),
            );
        } else {
            out.push_str(
                render(
                    "param_decl_inline_required.jinja",
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

    let options_param = &bridge_match.param_name;
    let options_param_camel = options_param.to_lower_camel_case();
    let field_name = &bridge_match.field_name;
    let field_name_pascal = to_csharp_name(field_name);
    let trait_pascal = csharp_type_name(&bridge_match.bridge.trait_name);
    let options_pascal = csharp_type_name(&bridge_match.options_type);
    let result_pascal = match &func.return_type {
        TypeRef::Named(name) => csharp_type_name(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => csharp_type_name(name),
            _ => csharp_type(&func.return_type).into_owned(),
        },
        _ => csharp_type(&func.return_type).into_owned(),
    };

    out.push_str(&render(
        "bridge_field_setup.jinja",
        minijinja::context! {
            field_name,
            options_param_camel,
            field_name_pascal,
            options_pascal,
            trait_pascal,
        },
    ));
    let is_visitor_bridge = bridge_match.bridge.context_type.is_some() && bridge_match.bridge.result_type.is_some();
    let register_template = if is_visitor_bridge {
        "bridge_field_register_visitor.jinja"
    } else {
        "bridge_field_register.jinja"
    };
    out.push_str(&render(register_template, minijinja::context! { trait_pascal }));
    out.push_str("                if (bridgeHandle == IntPtr.Zero) throw GetLastError();\n");
    out.push_str("                try\n                {\n");

    let cs_native_name = to_csharp_name(&func.name);
    out.push_str(&render(
        "bridge_field_inject.jinja",
        minijinja::context! { options_pascal, field_name_pascal, options_param_camel },
    ));

    if func.return_type != TypeRef::Unit {
        out.push_str("                    var nativeResult = ");
    } else {
        out.push_str("                    ");
    }

    out.push_str(
        render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        )
        .trim_end_matches('\n'),
    );

    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                format!("{options_param_camel}Handle")
            } else {
                p.name.to_lower_camel_case().to_string()
            }
        })
        .collect();

    for (i, arg) in call_args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg);
    }
    out.push_str(");\n");

    if func.return_type != TypeRef::Unit {
        out.push_str("                    if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
    }

    if func.return_type != TypeRef::Unit {
        out.push_str(&render(
            "bridge_field_json_return.jinja",
            minijinja::context! { indent => "                    ", result_pascal },
        ));
    }

    out.push_str("                }\n");
    out.push_str("                finally\n");
    out.push_str("                {\n");
    let unregister_template = if is_visitor_bridge {
        "bridge_field_unregister_visitor.jinja"
    } else {
        "bridge_field_unregister.jinja"
    };
    out.push_str(&render(unregister_template, minijinja::context! { trait_pascal }));
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("            else\n");
    out.push_str("            {\n");

    if func.return_type != TypeRef::Unit {
        out.push_str("                var nativeResult = ");
    } else {
        out.push_str("                ");
    }

    out.push_str(
        render(
            "native_call_start.jinja",
            minijinja::context! { method_name => &cs_native_name },
        )
        .trim_end_matches('\n'),
    );
    for (i, arg) in call_args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg);
    }
    out.push_str(");\n");

    if func.return_type != TypeRef::Unit {
        out.push_str("                if (nativeResult == IntPtr.Zero) throw GetLastError();\n");
        out.push_str(&render(
            "bridge_field_json_return.jinja",
            minijinja::context! { indent => "                ", result_pascal },
        ));
    }

    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        finally\n");
    out.push_str("        {\n");
    out.push_str(&render(
        "bridge_field_free_options.jinja",
        minijinja::context! { options_pascal, options_param_camel },
    ));
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    out
}
