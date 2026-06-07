use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::ToLowerCamelCase;

use super::streaming::render_streaming_chunk_decode;

pub(super) fn emit_doc_comment(doc: &str, indent: &str, out: &mut String) {
    if doc.is_empty() {
        return;
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "doc_comment.jinja",
        minijinja::context! {
            indent => indent,
            lines => doc.lines().collect::<Vec<_>>(),
        },
    ));
}

pub(super) fn first_param_is(func_def: &FunctionDef, ty: &TypeRef) -> bool {
    func_def.params.first().map(|p| &p.ty == ty).unwrap_or(false)
}

pub(super) fn emit_convenience_wrappers(api: &ApiSurface, out: &mut String) {
    let all_names: std::collections::HashSet<&str> = api.functions.iter().map(|f| f.name.as_str()).collect();

    let bytes_candidates: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| {
            first_param_is(f, &TypeRef::Bytes) && !f.is_async && !super::overloads::convenience_name_shadows_bridge(f)
        })
        .collect();

    let path_candidates: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| {
            first_param_is(f, &TypeRef::Path) && !f.is_async && !super::overloads::convenience_name_shadows_bridge(f)
        })
        .collect();

    if bytes_candidates.is_empty() && path_candidates.is_empty() {
        return;
    }

    out.push_str("// MARK: - Convenience Wrapper Functions\n");
    out.push_str("// These wrappers bridge String / [UInt8] inputs to RustBridge's\n");
    out.push_str("// RustVec<UInt8> requirement. The config parameter must be a fully\n");
    out.push_str("// constructed opaque type (built via the generated initializer);\n");
    out.push_str("// JSON-config decoding is not available because swift-bridge opaque\n");
    out.push_str("// proxy classes are not Codable Swift structs.\n\n");

    if !bytes_candidates.is_empty() {
        out.push_str("/// Converts a Swift `[UInt8]` array to a `RustVec<UInt8>` by pushing each byte.\n");
        out.push_str("/// swift-bridge's `RustVec<T>` runtime only exposes `init()` and `push(value:)`;\n");
        out.push_str("/// no array-initializer shorthand exists.\n");
        out.push_str("private func makeByteVec(_ bytes: [UInt8]) -> RustVec<UInt8> {\n");
        out.push_str("    let vec = RustVec<UInt8>()\n");
        out.push_str("    for b in bytes { vec.push(value: b) }\n");
        out.push_str("    return vec\n");
        out.push_str("}\n\n");
    }

    for func in &bytes_candidates {
        super::overloads::emit_bytes_overloads(func, &all_names, out);
    }

    for func in &path_candidates {
        super::overloads::emit_path_overload(func, &all_names, out);
    }

    let _ = api;
}

/// - Constructor bridge fn: `create_<snake_type_name>` → camelCase `create<PascalTypeName>`
/// - Method bridge fn: `<snake_type_name>_<method_name>` → camelCase `<camelTypeName><PascalMethod>`
pub(super) fn emit_client_class(
    type_name: &str,
    methods: &[MethodDef],
    mapper: &impl TypeMapper,
    config: &ResolvedCrateConfig,
    first_class_types: &std::collections::HashSet<String>,
    out: &mut String,
) {
    use heck::ToSnakeCase;

    let snake_name = type_name.to_snake_case();
    let constructor_fn = swift_ident(&format!("create_{snake_name}").to_lower_camel_case());

    let mut methods_body = String::new();
    for method in methods {
        if method.sanitized {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let method_camel = swift_ident(&method_snake.to_lower_camel_case());
        let bridge_fn_snake = format!("{snake_name}_{method_snake}");
        let bridge_fn_camel = swift_ident(&bridge_fn_snake.to_lower_camel_case());

        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let swift_name = swift_ident(&p.name.to_lower_camel_case());
                let ty_str = if p.optional {
                    format!("{}?", mapper.map_type(&p.ty))
                } else {
                    mapper.map_type(&p.ty)
                };
                format!("_ {swift_name}: {ty_str}")
            })
            .collect();
        let params_str = params.join(", ");

        let has_dto_param = method
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(n) if first_class_types.contains(n)));
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let swift_name = swift_ident(&p.name.to_lower_camel_case());
                match &p.ty {
                    TypeRef::Named(n) if first_class_types.contains(n) => {
                        if p.optional {
                            format!("try {swift_name}?.intoRust()")
                        } else {
                            format!("try {swift_name}.intoRust()")
                        }
                    }
                    _ => swift_name,
                }
            })
            .collect();
        let args_str = if args.is_empty() {
            String::new()
        } else {
            format!(", {}", args.join(", "))
        };

        let return_ty = mapper.map_type(&method.return_type);
        let needs_return_init = matches!(&method.return_type, TypeRef::Named(n) if first_class_types.contains(n));
        let needs_throws = method.error_type.is_some() || has_dto_param || needs_return_init;
        let throws_clause = if needs_throws { " throws" } else { "" };
        let async_clause = if method.is_async { " async" } else { "" };
        let return_clause = if matches!(method.return_type, TypeRef::Unit) {
            String::new()
        } else {
            format!(" -> {return_ty}")
        };

        emit_doc_comment(&method.doc, "    ", &mut methods_body);
        let mut method_body = String::new();
        if matches!(method.return_type, TypeRef::Unit) {
            method_body.push_str(&crate::backends::swift::template_env::render(
                "swift_client_method_unit_body.swift.jinja",
                minijinja::context! {
                    throws_kw => if needs_throws { "try " } else { "" },
                    bridge_function => &bridge_fn_camel,
                    args => &args_str,
                },
            ));
        } else {
            let await_kw = if method.is_async { "await " } else { "" };
            let try_kw = if needs_throws { "try " } else { "" };
            let bytes_suffix = if matches!(method.return_type, TypeRef::Bytes) {
                ".map { Data($0.map { $0 }) }"
            } else {
                ""
            };
            if bytes_suffix.is_empty() {
                if needs_return_init {
                    method_body.push_str(&crate::backends::swift::template_env::render(
                        "swift_client_method_dto_return_body.swift.jinja",
                        minijinja::context! {
                            return_type => &return_ty,
                            try_kw => try_kw,
                            await_kw => await_kw,
                            bridge_function => &bridge_fn_camel,
                            args => &args_str,
                        },
                    ));
                } else {
                    method_body.push_str(&crate::backends::swift::template_env::render(
                        "swift_client_method_return_body.swift.jinja",
                        minijinja::context! {
                            try_kw => try_kw,
                            await_kw => await_kw,
                            bridge_function => &bridge_fn_camel,
                            args => &args_str,
                        },
                    ));
                }
            } else {
                method_body.push_str(&crate::backends::swift::template_env::render(
                    "swift_client_method_bytes_body.swift.jinja",
                    minijinja::context! {
                        try_kw => try_kw,
                        await_kw => await_kw,
                        bridge_function => &bridge_fn_camel,
                        args => &args_str,
                    },
                ));
            }
        }
        methods_body.push_str(&crate::backends::swift::template_env::render(
            "swift_client_method.swift.jinja",
            minijinja::context! {
                method_name => &method_camel,
                params => &params_str,
                async_clause => async_clause,
                throws_clause => throws_clause,
                return_clause => return_clause,
                body => method_body,
            },
        ));
    }

    let mut streaming_methods = String::new();
    for adapter in config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| a.owner_type.as_deref() == Some(type_name))
    {
        emit_streaming_client_method(adapter, &snake_name, first_class_types, &mut streaming_methods);
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_client_class.swift.jinja",
        minijinja::context! {
            type_name => type_name,
            constructor_fn => constructor_fn,
            methods => methods_body,
            streaming_methods => streaming_methods,
        },
    ));
}

/// Emit an `AsyncThrowingStream<Item, Error>` wrapper for a streaming adapter
/// method on a Swift client class.
pub(super) fn emit_streaming_client_method(
    adapter: &AdapterConfig,
    owner_snake: &str,
    first_class_types: &std::collections::HashSet<String>,
    out: &mut String,
) {
    use heck::{AsSnakeCase, ToLowerCamelCase};
    let method_camel = swift_ident(&adapter.name.to_lower_camel_case());
    let start_fn_snake = format!("{owner_snake}_{}_start", adapter.name);
    let start_fn_camel = swift_ident(&start_fn_snake.to_lower_camel_case());

    let item_type = adapter.item_type.as_deref().unwrap_or("String");
    let item_type_from_json = swift_ident(&format!("{}_from_json", AsSnakeCase(item_type)).to_lower_camel_case());

    let params: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            let swift_name = swift_ident(&p.name.to_lower_camel_case());
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            format!("_ {swift_name}: {simple_ty}")
        })
        .collect();
    let params_str = params.join(", ");

    let call_args: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            let swift_name = swift_ident(&p.name.to_lower_camel_case());
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            if first_class_types.contains(simple_ty) {
                format!("(try {swift_name}.intoRust())")
            } else {
                swift_name
            }
        })
        .collect();
    let call_args_str = if call_args.is_empty() {
        String::new()
    } else {
        format!(", {}", call_args.join(", "))
    };

    let chunk_decode = render_streaming_chunk_decode(
        item_type,
        &item_type_from_json,
        first_class_types.contains(item_type),
        "                        ",
    );
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_streaming_client_method.swift.jinja",
        minijinja::context! {
            method_name => &method_camel,
            params => &params_str,
            item_type => item_type,
            start_fn => &start_fn_camel,
            call_args => call_args_str,
            chunk_decode => chunk_decode,
        },
    ));
}

/// Emit an `extension RustBridge.{Owner}{Adapter}StreamHandle: @unchecked Sendable {}`
/// declaration for the given streaming adapter.
pub(super) fn emit_stream_handle_sendable(adapter: &AdapterConfig, owner_type: &str, out: &mut String) {
    use heck::ToPascalCase;
    let owner_pascal = owner_type.to_pascal_case();
    let adapter_pascal = adapter.name.to_pascal_case();
    let handle_name = format!("{owner_pascal}{adapter_pascal}StreamHandle");
    super::emit_sendable_conformance(
        out,
        &handle_name,
        None,
        &[
            "swift-bridge opaque types are not automatically Sendable.  The Rust",
            "side uses Mutex<stream> + tokio Runtime — both Send + Sync — so",
            "@unchecked is correct: thread-safety is enforced by Rust.",
        ],
    );
}

/// Emit top-level `public func` streaming wrappers for adapters whose owner type
/// is **not** wrapped in a Swift client class (i.e. no `client_constructor_body` in
/// alef.toml).
pub(super) fn emit_streaming_free_functions(
    config: &ResolvedCrateConfig,
    first_class_types: &std::collections::HashSet<String>,
    sendable_emitted: &mut std::collections::HashSet<String>,
    out: &mut String,
) {
    use heck::{AsSnakeCase, ToLowerCamelCase, ToSnakeCase};

    let client_constructor_types: std::collections::HashSet<&str> = config
        .swift
        .as_ref()
        .map(|c| c.client_constructor_body.keys().map(String::as_str).collect())
        .unwrap_or_default();

    let orphan_adapters: Vec<&AdapterConfig> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| !a.skip_languages.iter().any(|l| l == "swift"))
        .filter(|a| {
            a.owner_type
                .as_deref()
                .map(|t| !client_constructor_types.contains(t))
                .unwrap_or(false)
        })
        .collect();

    if orphan_adapters.is_empty() {
        return;
    }

    out.push_str("// MARK: - Streaming free functions\n");
    out.push_str(
        "// These adapters are owned by opaque handle types that do not have a\n\
         // Swift class wrapper (no client_constructor_body in alef.toml).  The\n\
         // streaming methods are therefore exposed as module-level free functions\n\
         // that accept the owner handle as their first parameter.\n\n",
    );
    for adapter in &orphan_adapters {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        if sendable_emitted.insert(owner_type.to_string()) {
            super::emit_sendable_conformance(
                out,
                owner_type,
                Some("streaming owner"),
                &[
                    "swift-bridge opaque types are not automatically Sendable.",
                    "Captured by Task.detached in streaming free functions — Rust type is Send + Sync.",
                ],
            );
            out.push('\n');
        }
        emit_stream_handle_sendable(adapter, owner_type, out);
        out.push('\n');
        for param in &adapter.params {
            let simple_ty = param.ty.rsplit("::").next().unwrap_or(&param.ty);
            if !first_class_types.contains(simple_ty) {
                let key = format!("param:{simple_ty}");
                if sendable_emitted.insert(key) {
                    super::emit_sendable_conformance(
                        out,
                        simple_ty,
                        Some("streaming request param"),
                        &[
                            "swift-bridge opaque types are not automatically Sendable.",
                            "Passed into Task.detached for streaming — Rust type is Send + Sync.",
                        ],
                    );
                    out.push('\n');
                }
            }
        }
    }

    for adapter in &orphan_adapters {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let owner_snake = owner_type.to_snake_case();
        let method_camel = swift_ident(&adapter.name.to_lower_camel_case());
        let start_fn_snake = format!("{owner_snake}_{}_start", adapter.name);
        let start_fn_camel = swift_ident(&start_fn_snake.to_lower_camel_case());

        let item_type = adapter.item_type.as_deref().unwrap_or("String");
        let item_type_from_json = swift_ident(&format!("{}_from_json", AsSnakeCase(item_type)).to_lower_camel_case());

        let owner_camel = swift_ident(&owner_type.to_lower_camel_case());
        let mut sig_params: Vec<String> = vec![format!("_ {owner_camel}: {owner_type}")];
        let mut call_args: Vec<String> = vec![];

        for param in &adapter.params {
            let swift_name = swift_ident(&param.name.to_lower_camel_case());
            let simple_ty = param.ty.rsplit("::").next().unwrap_or(&param.ty);
            sig_params.push(format!("_ {swift_name}: {simple_ty}"));
            if first_class_types.contains(simple_ty) {
                call_args.push(format!("(try {swift_name}.intoRust())"));
            } else {
                call_args.push(swift_name);
            }
        }

        let params_str = sig_params.join(", ");
        let call_args_str = if call_args.is_empty() {
            String::new()
        } else {
            format!(", {}", call_args.join(", "))
        };

        let chunk_decode = render_streaming_chunk_decode(
            item_type,
            &item_type_from_json,
            first_class_types.contains(item_type),
            "                    ",
        );
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_streaming_free_function.swift.jinja",
            minijinja::context! {
                function_name => &method_camel,
                params => &params_str,
                item_type => item_type,
                start_fn => &start_fn_camel,
                owner_arg => &owner_camel,
                call_args => call_args_str,
                chunk_decode => chunk_decode,
            },
        ));
        out.push('\n');
    }
}
