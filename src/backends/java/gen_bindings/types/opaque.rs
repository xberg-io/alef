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

    // Detect streaming adapters owned by this opaque type. When present we need
    // additional imports (Iterator, NoSuchElementException, ObjectMapper).
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

    // Instance methods on this opaque handle (skip static and any method whose name
    // collides with a streaming adapter — those are emitted by the streaming codegen).
    let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.to_snake_case()).collect();
    let instance_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.is_static)
        .filter(|m| !streaming_method_names.contains(&m.name.to_snake_case()))
        .collect();
    // Static factory methods: receiver is None (no &self). These are constructors /
    // preset factories (e.g. `Parser::default()`, `LanguageRegistry::default()`,
    // `DownloadManager::new(version)`) that return a new instance of the type.
    // The FFI backend never exports `_default` / `_to_json` / `_from_json` for opaque
    // types — those C functions only exist for non-opaque, serde-derivable, non-Update
    // value types. `gen_native_lib` already skips emitting the matching MethodHandle
    // constants; skip the static-factory wrappers here too so we don't reference
    // missing `NativeLib.<PREFIX>_<TYPE>_DEFAULT` constants from `defaultInstance()`.
    let static_factory_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| m.receiver.is_none())
        .filter(|m| !matches!(m.name.as_str(), "default" | "to_json" | "from_json"))
        // A static method returning a borrowed reference to its own opaque type (e.g.
        // `Registry::global() -> &'static Registry`) has no FFI symbol — the FFI backend
        // cannot box a borrow into an owned handle. Skip the wrapper so it does not
        // reference the missing `NativeLib.<PREFIX>_<TYPE>_<METHOD>` MethodHandle.
        .filter(|m| !m.returns_ref_to_owner(&typ.name))
        .collect();
    let has_instance_methods = !instance_methods.is_empty();
    let has_static_factories = !static_factory_methods.is_empty();
    let needs_helpers = has_streaming || has_instance_methods;

    // Build the class body first so we can compute imports from actual usage —
    // Checkstyle's UnusedImports rule fails if we declare an import that
    // never appears in the file body (e.g. when every instance method body
    // is a `Unsupported return shape` stub).
    let mut body = String::new();

    emit_javadoc(&mut body, &typ.doc, "");
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_header.jinja",
        minijinja::context! { class_name => class_name },
    ));

    // Emit streaming iterator methods (e.g. chatStream(req) -> Iterator<ChatCompletionChunk>).
    for adapter in &streaming_adapters {
        gen_streaming_method(&mut body, adapter, prefix, &type_snake, main_class, to_json_type_names);
    }

    // Emit non-streaming instance methods (chat, embed, moderate, …).
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

    // Emit static factory methods (constructors / preset factories with no receiver).
    // These give callers a clean `Parser.ofDefault()`, `LanguageRegistry.ofDefault()`,
    // `DownloadManager.create(version)` API without exposing the raw FFI handle.
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
        // `Arena.ofShared()` is only referenced when method bodies actually use it
        // (e.g., string parameters that allocate via Arena).
        if body.contains("Arena") {
            imports.push("java.lang.foreign.Arena");
        }
        // `ValueLayout` only appears when an instance method, streaming helper, or static
        // factory actually marshals memory; stub methods never reference it.
        if body.contains("ValueLayout") {
            imports.push("java.lang.foreign.ValueLayout");
        }
        // Same reasoning for ObjectMapper — STREAM_MAPPER references it, but
        // not all paths reach STREAM_MAPPER.
        if body.contains("ObjectMapper") {
            imports.push("com.fasterxml.jackson.databind.ObjectMapper");
        }
        // JsonNode is needed when method parameters or returns use it (e.g., requestSchemaJson(JsonNode)).
        if body.contains("JsonNode") {
            imports.push("com.fasterxml.jackson.databind.JsonNode");
        }
    }
    // Streaming method bodies reference java.util.stream.Stream<T> and
    // java.util.stream.StreamSupport via fully-qualified names in the template,
    // so no short-form import is needed. Adding one would trigger Checkstyle's
    // UnusedImports rule (confirmed in sample_crate DefaultClient.java:12).
    let _ = has_streaming;
    // Import collection types from actual body usage (params AND returns), not just return types —
    // e.g. a builder method taking `List<String>` needs the import even with no List return.
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

    // Emit Javadoc derived from the IR method.doc above the method declaration
    // so opaque-handle instance methods carry their source-level rustdoc into
    // the generated Java surface.
    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public ");
    out.push_str(&return_type_java);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push(')');

    // Methods named "clone" cannot declare throws because they override Object.clone()
    // which only throws CloneNotSupportedException. All other methods on opaque types
    // may call FFI functions that fail, so they declare throws.
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

    // Check if any parameters require Arena allocation (String, Path, Named types, etc.)
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

    out.push_str("        try {\n");
    if needs_arena {
        out.push_str("            Arena arena = Arena.ofShared();\n");
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
                // Object (polymorphic JSON) passed directly without marshalling.
                call_args.push(pname);
            }
            TypeRef::Path => {
                // Path → C string requires `.toString()` because Java's SegmentAllocator.allocateFrom
                // accepts String, not java.nio.file.Path. Reuse marshal_path.jinja which already
                // emits the conversion.
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
                // Optional<Object> (polymorphic JSON) passed directly without marshalling.
                call_args.push(pname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                // Optional Path also needs `.toString()` because SegmentAllocator.allocateFrom
                // accepts String, not java.nio.file.Path.
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
                    // Optional Named param (e.g. `query: Option<&BatchListQuery>` in Rust
                    // surfaces as `TypeRef::Named` + `optional: true` in the IR after the
                    // FFI extraction strips the `Option`). Pass MemorySegment.NULL when
                    // the Java arg is null instead of serializing `null` and feeding it
                    // to <Type>_from_json which then errors with "invalid type: null,
                    // expected struct <Type>".
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

        // Check if the return type is opaque or lacks _to_json in the FFI
        if opaque_type_names.contains(&return_type_name) || !to_json_type_names.contains(&return_type_name) {
            // For opaque types, wrap the pointer in a new instance of the return type.
            // For value types without _to_json (shouldn't happen but be defensive), stub the method.
            if opaque_type_names.contains(&return_type_name) {
                // Wrap pointer in new instance: `return new TypeName(resultPtr);`
                // The new wrapper owns the handle and frees it in close(); the
                // method must NOT free resultPtr here (see issue #146 — doing so
                // returned a wrapper around an already-freed handle -> UAF crash).
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
                // Value type without _to_json (defensive stub)
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
            // Normal value type with _to_json — deserialize from JSON
            let ret_snake = return_type_name.to_snake_case();
            let ret_upper = ret_snake.to_uppercase();
            let ret_free = format!("NativeLib.{prefix_upper}_{ret_upper}_FREE");
            let ret_to_json = format!("NativeLib.{prefix_upper}_{ret_upper}_TO_JSON");
            // When the declared return is `Optional<NamedDto>`, the method signature
            // is `Optional<NamedDto>` (from `java_return_type`) but the body builds
            // a bare `NamedDto`; wrap each return site through `Optional.of` /
            // `Optional.empty` so the body matches the signature.  Non-optional
            // named returns keep the historical bare-return shape.
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

    // For clone() methods, wrap exceptions in RuntimeException since the method
    // cannot declare throws (it overrides Object.clone() which only throws
    // CloneNotSupportedException). All other methods can declare throws.
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

    // Null checks for non-optional reference params.
    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    out.push_str("        try {\n");

    // Check if any parameters require Arena allocation (String, Path, Named types, etc.)
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
        out.push_str("            Arena arena = Arena.ofShared();\n");
    }

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    // Marshal parameters (same logic as gen_instance_method but no receiver in call_args).
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
                // Object (polymorphic JSON) passed directly without marshalling.
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
                // Check if this is an enum type (Wave 2 FFI backend emits enums as i32 discriminants)
                if enum_names.contains(type_name.as_str()) {
                    // Enum parameter: convert to ordinal/discriminant value
                    // For Method enum: method.ordinal() gives the i32 discriminant
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
                    // Struct/record parameter: JSON-serialize via _from_json
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
                // Unsupported param type for static factory — emit stub that throws.
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

    // The return type for a static factory is always Self (the class being constructed).
    // Emit: call FFI → check non-null → wrap in new ClassName(handle).
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
    // Strip any leading module path (e.g. `sample_llm::ChatCompletionRequest` → `ChatCompletionRequest`).
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
    // For streaming item types, always derive the to_json symbol from the item type name.
    // Streaming items must have serde derives (checked at adapter validation time);
    // if the FFI symbol is missing, that's a C FFI generation issue, not Java codegen.
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
