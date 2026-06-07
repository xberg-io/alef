use crate::backends::swift::gen_bindings::bridge_artifacts::already_emitted_top_level_names;
use crate::backends::swift::gen_bindings::client::emit_doc_comment;
use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

pub(super) struct ForwarderArg {
    setup_line: Option<String>,
    arg_expr: String,
}

fn forwarder_param_signature(
    ty: &TypeRef,
    swift_param_name: &str,
    optional: bool,
    known_dto_names: &std::collections::HashSet<String>,
) -> (String, ForwarderArg) {
    let inner_ty = if optional {
        if let TypeRef::Optional(inner) = ty {
            inner.as_ref().clone()
        } else {
            ty.clone()
        }
    } else {
        ty.clone()
    };
    let make_optional = |inner: &str| -> String {
        if optional || matches!(ty, TypeRef::Optional(_)) {
            format!("{inner}?")
        } else {
            inner.to_string()
        }
    };
    match &inner_ty {
        TypeRef::Bytes => {
            let swift_ty = make_optional("[UInt8]");
            let local = format!("_rb_{swift_param_name}");
            let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                Some(format!(
                    "let {local} = {swift_param_name}.map {{ bytes -> RustVec<UInt8> in let v = RustVec<UInt8>(); for b in bytes {{ v.push(value: b) }}; return v }}"
                ))
            } else {
                Some(format!(
                    "let {local}: RustVec<UInt8> = {{ let v = RustVec<UInt8>(); for b in {swift_param_name} {{ v.push(value: b) }}; return v }}()"
                ))
            };
            (
                swift_ty,
                ForwarderArg {
                    setup_line: setup,
                    arg_expr: local,
                },
            )
        }
        TypeRef::Vec(elem) => match elem.as_ref() {
            TypeRef::String => {
                let swift_ty = make_optional("[String]");
                let local = format!("_rb_{swift_param_name}");
                let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                    Some(format!(
                        "let {local} = {swift_param_name}.map {{ strs -> RustVec<RustString> in let v = RustVec<RustString>(); for s in strs {{ v.push(value: RustString(s)) }}; return v }}"
                    ))
                } else {
                    Some(format!(
                        "let {local}: RustVec<RustString> = {{ let v = RustVec<RustString>(); for s in {swift_param_name} {{ v.push(value: RustString(s)) }}; return v }}()"
                    ))
                };
                (
                    swift_ty,
                    ForwarderArg {
                        setup_line: setup,
                        arg_expr: local,
                    },
                )
            }
            TypeRef::Primitive(_) => {
                let inner = super::overloads::swift_type_name(elem);
                let swift_ty = make_optional(&format!("[{inner}]"));
                let local = format!("_rb_{swift_param_name}");
                let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                    Some(format!(
                        "let {local} = {swift_param_name}.map {{ xs -> RustVec<{inner}> in let v = RustVec<{inner}>(); for x in xs {{ v.push(value: x) }}; return v }}"
                    ))
                } else {
                    Some(format!(
                        "let {local}: RustVec<{inner}> = {{ let v = RustVec<{inner}>(); for x in {swift_param_name} {{ v.push(value: x) }}; return v }}()"
                    ))
                };
                (
                    swift_ty,
                    ForwarderArg {
                        setup_line: setup,
                        arg_expr: local,
                    },
                )
            }
            TypeRef::Named(name) if known_dto_names.contains(name) => {
                let swift_ty = make_optional(&format!("[{name}]"));
                let local = format!("_rb_{swift_param_name}");
                let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                    Some(format!(
                        "let {local} = try {swift_param_name}.map {{ items -> RustVec<RustString> in let v = RustVec<RustString>(); for item in items {{ let data = try JSONEncoder().encode(item); let json = String(data: data, encoding: .utf8) ?? \"null\"; v.push(value: RustString(json)) }}; return v }}"
                    ))
                } else {
                    Some(format!(
                        "let {local}: RustVec<RustString> = try ({{ () throws -> RustVec<RustString> in let v = RustVec<RustString>(); for item in {swift_param_name} {{ let data = try JSONEncoder().encode(item); let json = String(data: data, encoding: .utf8) ?? \"null\"; v.push(value: RustString(json)) }}; return v }}())"
                    ))
                };
                (
                    swift_ty,
                    ForwarderArg {
                        setup_line: setup,
                        arg_expr: local,
                    },
                )
            }
            TypeRef::Named(name) => {
                let swift_ty = make_optional(&format!("[{name}]"));
                let local = format!("_rb_{swift_param_name}");
                let inner = super::overloads::swift_type_name(elem);
                let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                    Some(format!(
                        "let {local} = {swift_param_name}.map {{ xs -> RustVec<{inner}> in let v = RustVec<{inner}>(); for x in xs {{ v.push(value: x) }}; return v }}"
                    ))
                } else {
                    Some(format!(
                        "let {local}: RustVec<{inner}> = {{ let v = RustVec<{inner}>(); for x in {swift_param_name} {{ v.push(value: x) }}; return v }}()"
                    ))
                };
                (
                    swift_ty,
                    ForwarderArg {
                        setup_line: setup,
                        arg_expr: local,
                    },
                )
            }
            _ => {
                let swift_ty = make_optional(&super::overloads::swift_type_name(ty));
                (
                    swift_ty,
                    ForwarderArg {
                        setup_line: None,
                        arg_expr: swift_param_name.to_string(),
                    },
                )
            }
        },
        TypeRef::Named(name) if known_dto_names.contains(name) => {
            let swift_ty = make_optional(&super::overloads::swift_type_name(&inner_ty));
            let local = format!("_rb_{swift_param_name}");
            let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                Some(format!("let {local} = try {swift_param_name}?.intoRust()"))
            } else {
                Some(format!("let {local} = try {swift_param_name}.intoRust()"))
            };
            (
                swift_ty,
                ForwarderArg {
                    setup_line: setup,
                    arg_expr: local,
                },
            )
        }
        TypeRef::String => {
            let swift_ty = make_optional("String");
            let local = format!("_rb_{swift_param_name}");
            let setup = if optional || matches!(ty, TypeRef::Optional(_)) {
                Some(format!("let {local} = {swift_param_name}.map {{ RustString($0) }}"))
            } else {
                Some(format!("let {local} = RustString({swift_param_name})"))
            };
            (
                swift_ty,
                ForwarderArg {
                    setup_line: setup,
                    arg_expr: local,
                },
            )
        }
        _ => {
            let swift_ty = make_optional(&super::overloads::swift_type_name(&inner_ty));
            (
                swift_ty,
                ForwarderArg {
                    setup_line: None,
                    arg_expr: swift_param_name.to_string(),
                },
            )
        }
    }
}

pub(super) fn emit_free_function_forwarders(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    known_dto_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    client_class_names: &HashSet<String>,
    out: &mut String,
) {
    let mut exclude_functions: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    for contract in &api.handler_contracts {
        if let Some(adapter) = contract.response_adapter.as_deref() {
            if let Some(short) = adapter.rsplit("::").next() {
                exclude_functions.insert(short.to_string());
            }
        }
    }

    let already = already_emitted_top_level_names(api);
    let mut emitted_any = false;

    for func in &api.functions {
        if func.binding_excluded || exclude_functions.contains(&func.name) {
            continue;
        }
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        let swift_name = swift_ident(&func.name.to_lower_camel_case());
        if already.contains(&swift_name) {
            continue;
        }
        if !emitted_any {
            out.push_str("// MARK: - Free-function Forwarders\n");
            out.push_str(
                "// Re-export every public free function on the source Rust crate as a\n\
                 // top-level `public func` on the host module so consumers do not need to\n\
                 // `import RustBridge` directly. Forwarders take Swift-native parameter\n\
                 // types and convert to the swift-bridge runtime types internally.\n\n",
            );
            emitted_any = true;
        }
        if func.is_async {
            emit_async_free_function_forwarder(func, &swift_name, known_dto_names, enum_names, out);
        } else {
            emit_single_free_function_forwarder(func, &swift_name, known_dto_names, client_class_names, out);
        }
    }
}

pub(super) fn emit_single_free_function_forwarder(
    func: &FunctionDef,
    swift_name: &str,
    known_dto_names: &HashSet<String>,
    client_class_names: &HashSet<String>,
    out: &mut String,
) {
    let return_conversion_throws = return_value_conversion_throws(&func.return_type, known_dto_names);
    let any_param_throws = func
        .params
        .iter()
        .any(|p| param_conversion_throws(&p.ty, known_dto_names));
    let throws_clause = if func.error_type.is_some() || return_conversion_throws || any_param_throws {
        " throws"
    } else {
        ""
    };
    let return_ty = forwarder_return_type(&func.return_type);
    let return_clause = if matches!(&func.return_type, TypeRef::Unit) {
        String::new()
    } else {
        format!(" -> {return_ty}")
    };

    let mut sig_params: Vec<String> = Vec::with_capacity(func.params.len());
    let mut conversion_lines: Vec<String> = Vec::new();
    let mut call_args: Vec<String> = Vec::with_capacity(func.params.len());

    for param in &func.params {
        let swift_param_name = swift_ident(&param.name.to_lower_camel_case());
        let (swift_ty, local_expr) =
            forwarder_param_signature(&param.ty, &swift_param_name, param.optional, known_dto_names);
        let param_default = if param.optional { " = nil" } else { "" };
        sig_params.push(format!("{swift_param_name}: {swift_ty}{param_default}"));
        if let Some(line) = local_expr.setup_line.clone() {
            conversion_lines.push(line);
        }
        call_args.push(local_expr.arg_expr);
    }
    let sig = sig_params.join(", ");
    let args = call_args.join(", ");

    if !func.doc.is_empty() {
        emit_doc_comment(&func.doc, "", out);
    }
    let mut conversion_body = String::new();
    for line in &conversion_lines {
        conversion_body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_conversion_line.swift.jinja",
            minijinja::context! { line => line, },
        ));
    }
    let return_suffix =
        forwarder_return_conversion_suffix_with_throws(&func.return_type, known_dto_names, func.error_type.is_some());
    let effective_try = if func.error_type.is_some() || return_conversion_throws {
        "try "
    } else {
        ""
    };
    let _ = effective_try;

    let body = if func.error_type.is_some() && return_uses_json_bridge(&func.return_type) {
        let decode_ty = forwarder_return_type(&func.return_type);
        crate::backends::swift::template_env::render(
            "swift_sync_forwarder_decode_json_body.swift.jinja",
            minijinja::context! {
                function_name => swift_name,
                args => &args,
                decode_type => &decode_ty,
            },
        )
    } else if non_throwing_optional_string_uses_json_bridge(&func.return_type, func.error_type.is_some()) {
        let decode_ty = forwarder_return_type(&func.return_type);
        crate::backends::swift::template_env::render(
            "swift_sync_forwarder_decode_optional_json_body.swift.jinja",
            minijinja::context! {
                function_name => swift_name,
                args => &args,
                decode_type => &decode_ty,
            },
        )
    } else if matches!(&func.return_type, TypeRef::Named(n) if client_class_names.contains(n)) {
        let bridge_call_try = if func.error_type.is_some() { "try " } else { "" };
        let class_name = swift_type_name(&func.return_type);
        crate::backends::swift::template_env::render(
            "swift_sync_forwarder_client_return_body.swift.jinja",
            minijinja::context! {
                bridge_call_try => bridge_call_try,
                function_name => swift_name,
                args => &args,
                class_name => &class_name,
            },
        )
    } else if bare_named_dto_return(&func.return_type, known_dto_names) {
        let bridge_call_try = if func.error_type.is_some() { "try " } else { "" };
        let dto_name = swift_type_name(&func.return_type);
        crate::backends::swift::template_env::render(
            "swift_sync_forwarder_dto_return_body.swift.jinja",
            minijinja::context! {
                bridge_call_try => bridge_call_try,
                function_name => swift_name,
                args => &args,
                dto_name => &dto_name,
            },
        )
    } else {
        crate::backends::swift::template_env::render(
            "swift_sync_forwarder_result_return_body.swift.jinja",
            minijinja::context! {
                effective_try => effective_try,
                function_name => swift_name,
                args => &args,
                return_suffix => &return_suffix,
            },
        )
    };
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_sync_forwarder.swift.jinja",
        minijinja::context! {
            function_name => swift_name,
            params => &sig,
            throws_clause => throws_clause,
            return_clause => return_clause,
            conversion_lines => conversion_body,
            body => body,
        },
    ));
}

pub(super) fn emit_async_free_function_forwarder(
    func: &FunctionDef,
    swift_name: &str,
    known_dto_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    let return_conversion_throws = return_value_conversion_throws(&func.return_type, known_dto_names);
    let any_param_throws = func
        .params
        .iter()
        .any(|p| param_conversion_throws(&p.ty, known_dto_names));
    let throws_clause = if func.error_type.is_some() || return_conversion_throws || any_param_throws {
        " throws"
    } else {
        ""
    };
    let return_ty = forwarder_return_type(&func.return_type);
    let return_clause = if matches!(&func.return_type, TypeRef::Unit) {
        String::new()
    } else {
        format!(" -> {return_ty}")
    };

    let mut sig_params: Vec<String> = Vec::with_capacity(func.params.len());
    let mut conversion_lines: Vec<String> = Vec::new();
    let mut call_args: Vec<String> = Vec::with_capacity(func.params.len());

    for param in &func.params {
        let swift_param_name = swift_ident(&param.name.to_lower_camel_case());
        let (swift_ty, local_expr) =
            forwarder_param_signature(&param.ty, &swift_param_name, param.optional, known_dto_names);
        let param_default = if param.optional { " = nil" } else { "" };
        sig_params.push(format!("{swift_param_name}: {swift_ty}{param_default}"));
        let is_enum_param = matches!(&param.ty, TypeRef::Named(name) if enum_names.contains(name));
        if !is_enum_param {
            if let Some(line) = local_expr.setup_line.clone() {
                conversion_lines.push(line);
            }
        }
        let arg_expr = if is_enum_param {
            let local = format!("_rb_{swift_param_name}");
            conversion_lines.push(format!(
                "let {local} = try String(data: JSONEncoder().encode({swift_param_name}), encoding: .utf8) ?? \"null\""
            ));
            local
        } else {
            local_expr.arg_expr
        };
        call_args.push(arg_expr);
    }
    let sig = sig_params.join(", ");
    let args = call_args.join(", ");

    if !func.doc.is_empty() {
        emit_doc_comment(&func.doc, "", out);
    }

    let effective_try = if func.error_type.is_some() || return_conversion_throws {
        "try "
    } else {
        ""
    };

    let (bridge_call, return_stmt) = match &func.return_type {
        TypeRef::Named(name) if known_dto_names.contains(name) => {
            let struct_name = swift_ident(name);
            (
                format!("try RustBridge.{swift_name}({args})"),
                format!("        return try {struct_name}(_rb_obj)"),
            )
        }
        _ if return_uses_json_bridge(&func.return_type) && func.error_type.is_some() => {
            let decode_ty = forwarder_return_type(&func.return_type);
            (
                format!("try RustBridge.{swift_name}({args}).toString()"),
                format!(
                    "        let _rb_data = _rb_result.data(using: .utf8) ?? Data()\n        return try JSONDecoder().decode({decode_ty}.self, from: _rb_data)"
                ),
            )
        }
        TypeRef::String => (
            format!("try RustBridge.{swift_name}({args})"),
            "        return result.toString()".to_string(),
        ),
        _ => (
            format!("try RustBridge.{swift_name}({args})"),
            "        return result".to_string(),
        ),
    };

    let mut conversion_body = String::new();
    for line in &conversion_lines {
        conversion_body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_conversion_line.swift.jinja",
            minijinja::context! { line => line, },
        ));
    }

    let mut body = String::new();
    if matches!(&func.return_type, TypeRef::Named(name) if known_dto_names.contains(name)) {
        body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_dto_return_body.swift.jinja",
            minijinja::context! {
                bridge_call => &bridge_call,
                return_statement => &return_stmt,
            },
        ));
    } else if return_uses_json_bridge(&func.return_type) && func.error_type.is_some() {
        let decode_ty = forwarder_return_type(&func.return_type);
        body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_decode_json_body.swift.jinja",
            minijinja::context! {
                bridge_call => &bridge_call,
                decode_type => &decode_ty,
            },
        ));
    } else if matches!(&func.return_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_))) {
        body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_conversion_line.swift.jinja",
            minijinja::context! {
                line => format!("let result = {bridge_call}"),
            },
        ));
        if func.error_type.is_some() {
            if let TypeRef::Vec(inner) = &func.return_type {
                if let TypeRef::Named(name) = inner.as_ref() {
                    let struct_name = swift_ident(name);
                    body.push_str("        var items: [");
                    body.push_str(&struct_name);
                    body.push_str("] = []\n");
                    body.push_str("        for ref in result {\n");
                    if known_dto_names.contains(name) {
                        body.push_str("            items.append(try ");
                        body.push_str(&struct_name);
                        body.push_str("(ref))\n");
                    } else {
                        body.push_str("            var item = try RustBridge.");
                        body.push_str(&struct_name);
                        body.push_str("(ptr: ref.ptr)\n");
                        body.push_str("            item.isOwned = false\n");
                    }
                    body.push_str("            items.append(item)\n");
                    body.push_str("        }\n");
                    body.push_str("        return items\n");
                } else {
                    let suffix =
                        forwarder_return_conversion_suffix_with_throws(&func.return_type, known_dto_names, true);
                    body.push_str(&crate::backends::swift::template_env::render(
                        "swift_forwarder_result_return_body.swift.jinja",
                        minijinja::context! { suffix => &suffix, },
                    ));
                }
            }
        } else {
            let suffix = forwarder_return_conversion_suffix_with_throws(&func.return_type, known_dto_names, false);
            body.push_str(&crate::backends::swift::template_env::render(
                "swift_forwarder_result_return_body.swift.jinja",
                minijinja::context! { suffix => &suffix, },
            ));
        }
    } else if matches!(&func.return_type, TypeRef::Unit) {
        body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_unit_body.swift.jinja",
            minijinja::context! { bridge_call => &bridge_call, },
        ));
    } else {
        body.push_str(&crate::backends::swift::template_env::render(
            "swift_forwarder_let_return_body.swift.jinja",
            minijinja::context! {
                bridge_call => &bridge_call,
                return_statement => &return_stmt,
            },
        ));
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_async_forwarder.swift.jinja",
        minijinja::context! {
            function_name => swift_name,
            params => sig,
            throws_clause => throws_clause,
            return_clause => return_clause,
            effective_try => effective_try,
            conversion_lines => conversion_body,
            body => body,
        },
    ));
    out.push('\n');
}

pub(super) fn emit_trait_bridge_forwarders(config: &ResolvedCrateConfig, out: &mut String) {
    let mut emitted_any = false;
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
        if bridge_cfg.exclude_languages.iter().any(|l| l == "swift") {
            continue;
        }
        if bridge_cfg.register_fn.is_none() && bridge_cfg.unregister_fn.is_none() && bridge_cfg.clear_fn.is_none() {
            continue;
        }
        if !emitted_any {
            out.push_str("// MARK: - Trait Bridge Registration Forwarders\n");
            out.push_str(
                "// Top-level `public func` re-exports of the swift-bridge–generated\n\
                 // `register_*` / `unregister_*` / `clear_*` plugin registration entry\n\
                 // points so consumers do not need to `import RustBridge` for plugin work.\n\n",
            );
            emitted_any = true;
        }
        let trait_name = &bridge_cfg.trait_name;
        let box_type = format!("Swift{trait_name}Box");

        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let camel = register_fn.to_lower_camel_case();
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_trait_forwarder_register.swift.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    box_type => &box_type,
                    function_name => &camel,
                },
            ));
        }
        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let camel = unregister_fn.to_lower_camel_case();
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_trait_forwarder_unregister.swift.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    function_name => &camel,
                },
            ));
        }
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let camel = clear_fn.to_lower_camel_case();
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_trait_forwarder_clear.swift.jinja",
                minijinja::context! {
                    trait_name => trait_name,
                    function_name => &camel,
                },
            ));
        }
    }
}

pub(super) fn param_conversion_throws(ty: &TypeRef, known_dto_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(name) if known_dto_names.contains(name)),
        TypeRef::Vec(elem) => matches!(elem.as_ref(), TypeRef::Named(name) if known_dto_names.contains(name)),
        _ => false,
    }
}

pub(super) fn forwarder_return_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "[UInt8]".to_string(),
        TypeRef::Vec(inner) => format!("[{}]", forwarder_return_type(inner)),
        TypeRef::Optional(inner) => format!("{}?", forwarder_return_type(inner)),
        _ => swift_type_name(ty),
    }
}

pub(super) fn forwarder_return_conversion_suffix_with_throws(
    ty: &TypeRef,
    known_dto_names: &HashSet<String>,
    throws: bool,
) -> String {
    forwarder_return_conversion_suffix_inner(ty, known_dto_names, throws)
}

fn forwarder_return_conversion_suffix_inner(ty: &TypeRef, known_dto_names: &HashSet<String>, _throws: bool) -> String {
    match ty {
        TypeRef::String => ".toString()".to_string(),
        TypeRef::Bytes => ".map { $0 }".to_string(),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::String => ".map { $0.as_str().toString() }".to_string(),
            TypeRef::Primitive(_) => ".map { $0 }".to_string(),
            TypeRef::Named(name) => {
                let struct_name = swift_ident(name);
                if known_dto_names.contains(name) {
                    format!(".map {{ ref in try {struct_name}(ref) }}")
                } else {
                    format!(
                        ".map {{ ref in var item = try RustBridge.{struct_name}(ptr: ref.ptr); item.isOwned = false; return item }}"
                    )
                }
            }
            _ => String::new(),
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if known_dto_names.contains(name) => format!(".map {{ try {name}($0) }}"),
            TypeRef::String => String::new(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

pub(super) fn return_value_conversion_throws(ty: &TypeRef, known_dto_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::Named(name) if known_dto_names.contains(name)
        ),
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(name) if known_dto_names.contains(name)),
        _ => false,
    }
}

pub(super) fn bare_named_dto_return(ty: &TypeRef, known_dto_names: &HashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(name) if known_dto_names.contains(name))
}

pub(super) fn return_uses_json_bridge(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Vec(_) | TypeRef::Map(_, _)),
        TypeRef::Map(_, _) | TypeRef::Json => true,
        TypeRef::Optional(inner) => return_uses_json_bridge(inner),
        _ => false,
    }
}

pub(super) fn non_throwing_optional_string_uses_json_bridge(ty: &TypeRef, throws: bool) -> bool {
    if throws {
        return false;
    }
    matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Primitive(_)))
}

fn swift_type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "[UInt8]".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) => format!("{}?", swift_type_name(inner)),
        TypeRef::Vec(inner) => format!("[{}]", swift_type_name(inner)),
        TypeRef::Map(_, _) => "String".to_string(),
        TypeRef::Primitive(_) => "Int".to_string(),
    }
}
