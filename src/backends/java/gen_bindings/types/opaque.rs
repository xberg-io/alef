use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_class_name;
use crate::core::config::{AdapterConfig, AdapterPattern};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{MethodDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToSnakeCase};

use crate::backends::java::gen_bindings::helpers::{emit_javadoc, safe_java_method_name};
use crate::backends::java::gen_bindings::marshal::{is_ffi_string_return, java_ffi_return_cast, java_ffi_return_expr};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_opaque_handle_class(
    package: &str,
    typ: &TypeDef,
    prefix: &str,
    adapters: &[AdapterConfig],
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) -> String {
    let class_name = &typ.name;
    let type_snake = class_name.to_snake_case();
    let header = hash::header(CommentStyle::DoubleSlash);

    let streaming_adapters: Vec<&AdapterConfig> = adapters
        .iter()
        .filter(|a| {
            matches!(a.pattern, AdapterPattern::Streaming)
                && a.owner_type.as_deref() == Some(class_name.as_str())
                && a.item_type.is_some()
                && a.params.first().is_some_and(|p| !p.ty.is_empty())
                && !a.skip_languages.iter().any(|l| l == "java")
        })
        .collect();
    let has_streaming = !streaming_adapters.is_empty();

    let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.to_snake_case()).collect();
    let instance_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.is_static)
        .filter(|m| !streaming_method_names.contains(&m.name.to_snake_case()))
        .collect();
    let static_factory_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| m.receiver.is_none())
        .filter(|m| !matches!(m.name.as_str(), "default" | "to_json" | "from_json"))
        .filter(|m| !m.returns_ref_to_owner(&typ.name))
        .collect();
    let has_instance_methods = !instance_methods.is_empty();
    let has_static_factories = !static_factory_methods.is_empty();
    let needs_helpers = has_streaming || has_instance_methods;

    let mut body = String::new();

    emit_javadoc(&mut body, &typ.doc, "");
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_header.jinja",
        minijinja::context! { class_name => class_name },
    ));

    for adapter in &streaming_adapters {
        gen_streaming_method(&mut body, adapter, prefix, &type_snake, main_class, to_json_type_names);
    }

    for method in &instance_methods {
        gen_instance_method(
            &mut body,
            method,
            prefix,
            &type_snake,
            main_class,
            opaque_type_names,
            to_json_type_names,
        );
    }

    if has_static_factories {
        for method in &static_factory_methods {
            gen_static_factory_method(
                &mut body,
                method,
                class_name,
                prefix,
                &type_snake,
                main_class,
                enum_names,
            );
        }
    }

    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_close.jinja",
        minijinja::context! {
            free_handle => free_handle,
            class_name => class_name,
        },
    ));

    if needs_helpers {
        gen_streaming_helpers(&mut body, prefix, main_class);
    }

    body.push_str("}\n");

    let mut imports: Vec<&str> = vec!["java.lang.foreign.MemorySegment"];
    if needs_helpers || has_static_factories {
        if body.contains("Arena") {
            imports.push("java.lang.foreign.Arena");
        }
        if body.contains("ValueLayout") {
            imports.push("java.lang.foreign.ValueLayout");
        }
        if body.contains("ObjectMapper") {
            imports.push("com.fasterxml.jackson.databind.ObjectMapper");
        }
        if body.contains("JsonNode") {
            imports.push("com.fasterxml.jackson.databind.JsonNode");
        }
    }
    let _ = has_streaming;
    if body.contains("List<") {
        imports.push("java.util.List");
    }
    if body.contains("Optional<") {
        imports.push("java.util.Optional");
    }
    if body.contains("Map<") {
        imports.push("java.util.Map");
    }

    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&body);
    out
}

/// Emit a non-streaming instance method on an opaque-handle owner.
fn gen_instance_method(
    out: &mut String,
    method: &MethodDef,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) {
    let method_name = safe_java_method_name(&method.name);
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let method_upper = method.name.to_snake_case().to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let ffi_handle = format!("NativeLib.{prefix_upper}_{owner_upper}_{method_upper}");

    let params_sig: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty).to_string()
            } else {
                java_type(&p.ty).to_string()
            };
            format!("final {} {}", ptype, p.name.to_lower_camel_case())
        })
        .collect();

    let is_bytes_result = method.error_type.is_some()
        && (matches!(method.return_type, TypeRef::Bytes)
            || matches!(&method.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes)));

    let (is_optional_return, dispatch_return) = match &method.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    let return_type_java = if is_bytes_result {
        if is_optional_return {
            "java.util.Optional<byte[]>"
        } else {
            "byte[]"
        }
        .to_string()
    } else {
        java_return_type(&method.return_type).to_string()
    };

    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public ");
    out.push_str(&return_type_java);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push(')');

    if method.name != "clone" {
        out.push_str(" throws ");
        out.push_str(&exception_class);
    }

    out.push_str(" {\n");

    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    let needs_arena = method.params.iter().any(|p| match &p.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner)
            if matches!(
                inner.as_ref(),
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_)
            ) =>
        {
            true
        }
        _ => false,
    });

    if needs_arena {
        out.push_str("        try (Arena arena = Arena.ofShared()) {\n");
    } else {
        out.push_str("        try {\n");
    }

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    for p in &method.params {
        let pname = p.name.to_lower_camel_case();
        let cname = format!("c{}", to_class_name(&p.name));
        match &p.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Json => {
                call_args.push(pname);
            }
            TypeRef::Path => {
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_optional_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json) => {
                call_args.push(pname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_optional_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Named(type_name) => {
                let req_snake = type_name.to_snake_case();
                let req_upper = req_snake.to_uppercase();
                let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                if p.optional {
                    out.push_str(&crate::backends::java::template_env::render(
                        "stream_method_optional_named_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            param_name => pname,
                            from_json => from_json,
                            exception_class => exception_class,
                            method_name => method_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::java::template_env::render(
                        "stream_method_named_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            param_name => pname,
                            from_json => from_json,
                            exception_class => exception_class,
                            method_name => method_name,
                        },
                    ));
                }
                named_ptr_frees.push((cname.clone(), req_free));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                let type_name = match inner.as_ref() {
                    TypeRef::Named(n) => n,
                    _ => unreachable!(),
                };
                let req_snake = type_name.to_snake_case();
                let req_upper = req_snake.to_uppercase();
                let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_optional_named_param.jinja",
                    minijinja::context! {
                        c_name => cname,
                        param_name => pname,
                        from_json => from_json,
                        exception_class => exception_class,
                        method_name => method_name,
                    },
                ));
                named_ptr_frees.push((cname.clone(), req_free));
                call_args.push(cname);
            }
            TypeRef::Primitive(_) | TypeRef::Duration => {
                call_args.push(pname);
            }
            _ => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_unsupported_param.jinja",
                    minijinja::context! {
                        param_name => pname,
                        exception_class => exception_class,
                        method_name => method_name,
                    },
                ));
                return;
            }
        }
    }

    let render_named_frees = |indent: &str| -> String {
        let mut frees = String::new();
        for (cname, free_handle) in &named_ptr_frees {
            frees.push_str(&crate::backends::java::template_env::render(
                "stream_method_free_named_ptr.jinja",
                minijinja::context! {
                    indent => indent,
                    c_name => cname,
                    free_handle => free_handle,
                },
            ));
        }
        frees
    };

    let mut call_args_full = vec!["this.handle".to_string()];
    call_args_full.extend(call_args);
    let args_joined = call_args_full.join(", ");

    if is_bytes_result {
        let free_bytes = format!("NativeLib.{prefix_upper}_FREE_BYTES");
        let empty_return = if is_optional_return {
            "return java.util.Optional.empty();"
        } else {
            "return null;"
        };
        let success_return = if is_optional_return {
            "java.util.Optional.of(result)"
        } else {
            "result"
        };
        out.push_str(&crate::backends::java::template_env::render(
            "stream_method_bytes_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                empty_return => empty_return,
                free_bytes => free_bytes,
                success_return => success_return,
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Named(_)) {
        let return_type_name = match &dispatch_return {
            TypeRef::Named(n) => n.clone(),
            _ => unreachable!(),
        };

        if opaque_type_names.contains(&return_type_name) || !to_json_type_names.contains(&return_type_name) {
            if opaque_type_names.contains(&return_type_name) {
                let empty_return = if is_optional_return {
                    "java.util.Optional.empty()".to_string()
                } else {
                    "null".to_string()
                };
                let success_return = if is_optional_return {
                    format!("java.util.Optional.of(new {return_type_name}(resultPtr))")
                } else {
                    format!("new {return_type_name}(resultPtr)")
                };
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_opaque_handle_result.jinja",
                    minijinja::context! {
                        ffi_handle => ffi_handle,
                        args_joined => args_joined,
                        named_frees => render_named_frees("            "),
                        empty_return => empty_return,
                        success_return => success_return,
                    },
                ));
            } else {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_unsupported_return.jinja",
                    minijinja::context! {
                        named_frees => render_named_frees("            "),
                        method_name => method_name,
                        exception_class => exception_class,
                    },
                ));
            }
        } else {
            let ret_snake = return_type_name.to_snake_case();
            let ret_upper = ret_snake.to_uppercase();
            let ret_free = format!("NativeLib.{prefix_upper}_{ret_upper}_FREE");
            let ret_to_json = format!("NativeLib.{prefix_upper}_{ret_upper}_TO_JSON");
            let (empty_return, success_return) = if is_optional_return {
                (
                    "java.util.Optional.empty()".to_string(),
                    format!("return java.util.Optional.of(STREAM_MAPPER.readValue(json, {return_type_name}.class));"),
                )
            } else {
                (
                    "null".to_string(),
                    format!("return STREAM_MAPPER.readValue(json, {return_type_name}.class);"),
                )
            };

            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_named_result.jinja",
                minijinja::context! {
                    ffi_handle => ffi_handle,
                    args_joined => args_joined,
                    named_frees => render_named_frees("            "),
                    to_json => ret_to_json,
                    exception_class => exception_class,
                    method_name => method_name,
                    prefix_upper => prefix_upper,
                    return_type_name => return_type_name,
                    ret_free => ret_free,
                    empty_return => empty_return,
                    success_return => success_return,
                },
            ));
        }
    } else if is_ffi_string_return(&dispatch_return) {
        let template = if is_optional_return {
            "stream_method_optional_string_result.jinja"
        } else {
            "stream_method_string_result.jinja"
        };
        out.push_str(&crate::backends::java::template_env::render(
            template,
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                prefix_upper => prefix_upper,
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Primitive(_) | TypeRef::Duration) {
        let template = if is_optional_return {
            "stream_method_optional_primitive_result.jinja"
        } else {
            "stream_method_primitive_result.jinja"
        };
        out.push_str(&crate::backends::java::template_env::render(
            template,
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                java_primitive_type => java_ffi_return_cast(&dispatch_return),
                java_primitive_expr => java_ffi_return_expr(&dispatch_return, "result"),
                is_optional_long => matches!(dispatch_return, TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Isize | PrimitiveType::Usize) | TypeRef::Duration),
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Unit) {
        out.push_str(&crate::backends::java::template_env::render(
            "stream_method_unit_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
            },
        ));
    } else {
        out.push_str(&crate::backends::java::template_env::render(
            "stream_method_unsupported_return.jinja",
            minijinja::context! {
                named_frees => render_named_frees("            "),
                method_name => method_name,
                exception_class => exception_class,
            },
        ));
    }

    let catch_template = if method.name == "clone" {
        "stream_method_catch_unchecked.jinja"
    } else {
        "stream_method_catch.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        catch_template,
        minijinja::context! {
            exception_class => exception_class,
            method_name => method_name,
        },
    ));
}

/// Emit a static factory method on an opaque-handle class.
///
/// Static factories have no `self` receiver — they allocate a new native object and
/// return it wrapped in the Java class.  Examples: `Parser::default()`,
/// `LanguageRegistry::default()`, `DownloadManager::new(version)`.
///
/// The pattern mirrors `gen_instance_method` but:
///  - the NIF call does NOT prepend `this.handle` to `call_args`
///  - the result is wrapped in `new ClassName(handle)` rather than returned raw
fn gen_static_factory_method(
    out: &mut String,
    method: &MethodDef,
    class_name: &str,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    enum_names: &AHashSet<String>,
) {
    let method_name = safe_java_method_name(&method.name);
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let method_upper = method.name.to_snake_case().to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let ffi_handle = format!("NativeLib.{prefix_upper}_{owner_upper}_{method_upper}");

    let params_sig: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty).to_string()
            } else {
                java_type(&p.ty).to_string()
            };
            format!("final {} {}", ptype, p.name.to_lower_camel_case())
        })
        .collect();

    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public static ");
    out.push_str(class_name);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push_str(") throws ");
    out.push_str(&exception_class);
    out.push_str(" {\n");

    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    let needs_arena = method.params.iter().any(|p| match &p.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner)
            if matches!(
                inner.as_ref(),
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_)
            ) =>
        {
            true
        }
        _ => false,
    });

    if needs_arena {
        out.push_str("        try (Arena arena = Arena.ofShared()) {\n");
    } else {
        out.push_str("        try {\n");
    }

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    for p in &method.params {
        let pname = p.name.to_lower_camel_case();
        let cname = format!("c{}", to_class_name(&p.name));
        match &p.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Json => {
                call_args.push(pname);
            }
            TypeRef::Path => {
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char | TypeRef::Json) => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_optional_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Named(type_name) => {
                if enum_names.contains(type_name.as_str()) {
                    let enum_expr = if p.optional {
                        format!("{pname} != null ? {pname}.ordinal() : -1")
                    } else {
                        format!("{pname}.ordinal()")
                    };
                    out.push_str(&crate::backends::java::template_env::render(
                        "stream_method_enum_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            enum_expr => enum_expr,
                        },
                    ));
                    call_args.push(cname);
                } else {
                    let req_snake = type_name.to_snake_case();
                    let req_upper = req_snake.to_uppercase();
                    let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                    let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                    if p.optional {
                        out.push_str(&crate::backends::java::template_env::render(
                            "stream_method_optional_named_param.jinja",
                            minijinja::context! {
                                c_name => cname,
                                param_name => pname,
                                from_json => from_json,
                                exception_class => exception_class,
                                method_name => method_name,
                            },
                        ));
                    } else {
                        out.push_str(&crate::backends::java::template_env::render(
                            "stream_method_named_param.jinja",
                            minijinja::context! {
                                c_name => cname,
                                param_name => pname,
                                from_json => from_json,
                                exception_class => exception_class,
                                method_name => method_name,
                            },
                        ));
                    }
                    named_ptr_frees.push((cname.clone(), req_free));
                    call_args.push(cname);
                }
            }
            TypeRef::Primitive(_) | TypeRef::Duration => {
                call_args.push(pname);
            }
            _ => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_unsupported_param.jinja",
                    minijinja::context! {
                        param_name => pname,
                        exception_class => exception_class,
                        method_name => method_name,
                    },
                ));
                return;
            }
        }
    }

    let render_named_frees = |indent: &str| -> String {
        let mut frees = String::new();
        for (cname, free_handle) in &named_ptr_frees {
            frees.push_str(&crate::backends::java::template_env::render(
                "stream_method_free_named_ptr.jinja",
                minijinja::context! {
                    indent => indent,
                    c_name => cname,
                    free_handle => free_handle,
                },
            ));
        }
        frees
    };

    let args_joined = call_args.join(", ");

    let named_frees_str = render_named_frees("            ");
    out.push_str(&crate::backends::java::template_env::render(
        "static_factory_return_handle.jinja",
        minijinja::context! {
            ffi_handle => ffi_handle,
            args_joined => args_joined,
            named_frees => named_frees_str,
            exception_class => exception_class,
            method_name => method_name,
            class_name => class_name,
        },
    ));
}

/// True when the given `TypeRef` is a reference type whose Java representation may
/// be null (so we should `Objects.requireNonNull` it for non-optional params).
fn param_needs_null_check(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Named(_)
            | TypeRef::Bytes
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _)
    )
}

/// Emit a streaming iterator method body for an opaque-handle owner.
///
/// Generates `public Iterator<Item> <camelName>(Request request)` that calls the
/// FFI iterator-handle trio (`_start`, `_next`, `_free`), deserializing each chunk
/// pointer via `<item>_to_json` + `<item>_free` and rethrowing FFI errors as
/// `<MainClass>Exception`.
///
/// NOTE: Streaming item types must have serde derives in the Rust source.
/// This codegen always emits the `{PREFIX}_{ITEM}_TO_JSON` symbol name, which must
/// exist in the C FFI layer. If a cfg-gated type (e.g. `#[cfg(not(wasm32))]`)
/// lacks the symbol, that indicates a C FFI generation failure, not a Java codegen issue.
fn gen_streaming_method(
    out: &mut String,
    adapter: &AdapterConfig,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    _to_json_type_names: &AHashSet<String>,
) {
    let method_name = adapter.name.to_lower_camel_case();
    let item_type = adapter.item_type.as_deref().unwrap_or("Object");
    let request_type_full = adapter.params[0].ty.as_str();
    let request_type = request_type_full.rsplit("::").next().unwrap_or(request_type_full);
    let request_snake = request_type.to_snake_case();
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let adapter_upper = adapter.name.to_snake_case().to_uppercase();
    let request_upper = request_snake.to_uppercase();
    let item_snake = item_type.to_snake_case();
    let item_upper = item_snake.to_uppercase();
    let exception_class = format!("{main_class}Exception");

    let request_param = adapter.params[0].name.to_lower_camel_case();
    let request_param = if request_param.is_empty() {
        "request".to_string()
    } else {
        request_param
    };

    let start_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_START");
    let next_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_NEXT");
    let free_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_FREE");
    let req_from_json = format!("{prefix_upper}_{request_upper}_FROM_JSON");
    let req_free = format!("{prefix_upper}_{request_upper}_FREE");
    let item_to_json = format!("{prefix_upper}_{item_upper}_TO_JSON");
    let item_free = format!("{prefix_upper}_{item_upper}_FREE");

    out.push_str(&crate::backends::java::template_env::render(
        "streaming_iterator_method.jinja",
        minijinja::context! {
            item_type => item_type,
            method_name => method_name,
            request_type => request_type,
            request_param => request_param,
            exception_class => exception_class,
            req_from_json => req_from_json,
            start_handle => start_handle,
            req_free => req_free,
            next_handle => next_handle,
            prefix_upper => prefix_upper,
            item_to_json => item_to_json,
            item_free => item_free,
            free_handle => free_handle,
        },
    ));
}

/// Emit shared helpers (`STREAM_MAPPER`, `checkLastFfiError`, optionally `readBytesResult`)
/// used by the streaming iterator method bodies above.
fn gen_streaming_helpers(out: &mut String, prefix: &str, main_class: &str) {
    let prefix_upper = prefix.to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let needs_read_bytes_result = out.contains("readBytesResult(");
    let free_bytes = format!("NativeLib.{prefix_upper}_FREE_BYTES");
    let needs_stream_mapper = out.contains("STREAM_MAPPER");

    out.push_str(&crate::backends::java::template_env::render(
        "streaming_helpers.jinja",
        minijinja::context! {
            exception_class => exception_class,
            prefix_upper => prefix_upper,
            needs_read_bytes_result => needs_read_bytes_result,
            free_bytes => free_bytes,
            needs_stream_mapper => needs_stream_mapper,
        },
    ));
}
