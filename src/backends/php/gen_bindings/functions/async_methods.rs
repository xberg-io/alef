use crate::adapters::AdapterBodies;
use crate::backends::php::type_map::PhpMapper;
use crate::codegen::doc_emission;
use crate::codegen::shared;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};
use ahash::AHashSet;
use heck::ToLowerCamelCase;
use minijinja::context;

use super::super::helpers::{
    gen_php_call_args, gen_php_call_args_with_let_bindings, gen_php_function_params, gen_php_named_let_bindings,
    php_wrap_return,
};
use super::params::{
    PhpEnumReturnSets, PhpParamTypeSets, apply_bridge_none_substitutions, apply_default_param_substitutions,
    bridge_param_names, gen_php_serde_let_bindings, has_ref_named_params, has_sanitized_recoverable,
    override_bytes_return_type, promote_default_params, promoted_default_param_names, return_type_sig,
};
use super::stubs::gen_stub_return;

/// Generate an async free function binding as a static method body (no `#[php_function]` attribute).
/// Used when functions are placed inside a `#[php_impl]` facade class.
pub(crate) fn gen_async_function_as_static_method(
    func: &FunctionDef,
    mapper: &PhpMapper,
    type_sets: PhpParamTypeSets<'_>,
    core_import: &str,
    bridges: &[TraitBridgeConfig],
    mutex_types: &AHashSet<String>,
) -> String {
    let enum_returns = PhpEnumReturnSets {
        string_enum_names: &mapper.enum_names,
        json_string_enum_names: &mapper.json_string_enum_names,
    };
    let body = gen_async_function_body(
        func,
        &type_sets,
        core_import,
        &mapper.enum_names,
        bridges,
        mutex_types,
        &enum_returns,
    );
    let bridge_names = bridge_param_names(bridges);
    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !bridge_names.contains(p.name.as_str()))
        .cloned()
        .collect();
    let visible_params = promote_default_params(&visible_params, type_sets.default, type_sets.opaque);
    let params = gen_php_function_params(&visible_params, mapper, type_sets.opaque, &AHashSet::new());
    let return_type = mapper.map_type(&func.return_type);
    let mut return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());
    if matches!(&func.return_type, TypeRef::Bytes) {
        return_annotation = override_bytes_return_type(&return_annotation);
    }

    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &func.doc, "    ");
    let ret_sig = return_type_sig(&return_annotation);
    let php_name = func.name.to_lower_camel_case();
    if params.is_empty() {
        out.push_str(&crate::backends::php::template_env::render(
            "php_async_static_method_definition_no_params.jinja",
            context! {
                name => &func.name,
                php_name => &php_name,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    } else {
        out.push_str(&crate::backends::php::template_env::render(
            "php_async_static_method_definition_with_params.jinja",
            context! {
                name => &func.name,
                php_name => &php_name,
                params => &params,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    }
    out
}

/// Shared body generation for async free functions (block_on variant).
fn gen_async_function_body(
    func: &FunctionDef,
    type_sets: &PhpParamTypeSets<'_>,
    core_import: &str,
    enum_names: &AHashSet<String>,
    bridges: &[TraitBridgeConfig],
    mutex_types: &AHashSet<String>,
    enum_returns: &PhpEnumReturnSets<'_>,
) -> String {
    let bridge_names = bridge_param_names(bridges);
    let can_delegate = shared::can_auto_delegate_function(func, type_sets.opaque);
    let needs_serde = func.error_type.is_some()
        && (has_ref_named_params(&func.params, type_sets.opaque) || has_sanitized_recoverable(&func.params));
    if can_delegate || needs_serde {
        let promoted_params = promote_default_params(&func.params, type_sets.default, type_sets.opaque);
        let promoted_names = promoted_default_param_names(&func.params, type_sets.default, type_sets.opaque);
        let let_bindings = if needs_serde && !can_delegate {
            gen_php_serde_let_bindings(&promoted_params, type_sets.opaque, type_sets.enums, core_import)
        } else {
            gen_php_named_let_bindings(&promoted_params, type_sets.opaque, type_sets.enums, core_import)
        };
        let raw_call_args = gen_php_call_args_with_let_bindings(&promoted_params, type_sets.opaque, mutex_types);
        let raw_call_args = apply_default_param_substitutions(&raw_call_args, &promoted_params, &promoted_names);
        let call_args = apply_bridge_none_substitutions(&raw_call_args, func, &bridge_names);
        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let is_enum_return = matches!(&func.return_type, TypeRef::Named(n) if enum_names.contains(n.as_str()));
        let result_wrap = if is_enum_return {
            "format!(\"{:?}\", result)".to_string()
        } else {
            php_wrap_return(
                "result",
                &func.return_type,
                "",
                type_sets.opaque,
                false,
                func.returns_ref,
                false,
                mutex_types,
                enum_returns.json_string_enum_names,
                enum_returns.string_enum_names,
            )
        };
        if func.error_type.is_some() {
            crate::backends::php::template_env::render(
                "php_async_result_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        } else {
            crate::backends::php::template_env::render(
                "php_async_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        }
    } else {
        gen_stub_return(&func.return_type, func.error_type.is_some(), &func.name)
    }
}

/// Generate an async instance method binding for PHP (block on runtime).
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    is_opaque: bool,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &AdapterBodies,
    mutex_types: &AHashSet<String>,
) -> String {
    let string_enum_names = &mapper.enum_names;
    let json_string_enum_names = &mapper.json_string_enum_names;
    let empty_bridges = AHashSet::new();
    let params = gen_php_function_params(&method.params, mapper, opaque_types, &empty_bridges);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else if can_delegate && is_opaque {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let inner_clone = "let inner = self.inner.clone();\n    ";
        let needs_lock = mutex_types.contains(type_name);
        let inner_name = if needs_lock {
            "self.inner.lock().unwrap()"
        } else {
            "inner"
        };
        let core_call = if needs_lock {
            format!("{inner_name}.{}({})", method.name, call_args)
        } else {
            format!("inner.{}({})", method.name, call_args)
        };
        let result_wrap = php_wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            true,
            method.returns_ref,
            method.returns_cow,
            mutex_types,
            json_string_enum_names,
            string_enum_names,
        );
        if method.error_type.is_some() {
            crate::backends::php::template_env::render(
                "php_async_result_body_with_let_bindings.jinja",
                context! {
                    let_bindings => inner_clone,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        } else {
            crate::backends::php::template_env::render(
                "php_async_body_with_let_bindings.jinja",
                context! {
                    let_bindings => inner_clone,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        }
    } else if is_opaque {
        let named_let_bindings = gen_php_named_let_bindings(&method.params, opaque_types, enum_names, core_import);
        let call_args = gen_php_call_args_with_let_bindings(&method.params, opaque_types, mutex_types);
        let needs_lock = mutex_types.contains(type_name);
        let (let_bindings, core_call) = if needs_lock {
            let core_call = format!("self.inner.lock().unwrap().{}({})", method.name, call_args);
            (named_let_bindings, core_call)
        } else {
            let inner_prefix = "let inner = self.inner.clone();\n    ".to_string();
            let let_bindings = format!("{inner_prefix}{named_let_bindings}");
            let core_call = format!("inner.{}({})", method.name, call_args);
            (let_bindings, core_call)
        };
        let result_wrap = php_wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            true,
            method.returns_ref,
            method.returns_cow,
            mutex_types,
            json_string_enum_names,
            string_enum_names,
        );
        if method.error_type.is_some() {
            crate::backends::php::template_env::render(
                "php_async_result_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        } else {
            crate::backends::php::template_env::render(
                "php_async_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    result_wrap => &result_wrap,
                },
            )
        }
    } else {
        return String::new();
    };

    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &method.doc, "    ");
    let ret_sig = return_type_sig(&return_annotation);
    out.push_str("    ");
    if params.is_empty() {
        out.push_str(&crate::backends::php::template_env::render(
            "php_async_instance_method_definition_no_params.jinja",
            context! {
                name => &method.name,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    } else {
        out.push_str(&crate::backends::php::template_env::render(
            "php_async_instance_method_definition_with_params.jinja",
            context! {
                name => &method.name,
                params => &params,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    }
    out
}

/// Generate an async static method binding for PHP (block on runtime).
pub(crate) fn gen_async_static_method(
    _method: &MethodDef,
    _mapper: &PhpMapper,
    _opaque_types: &AHashSet<String>,
) -> String {
    String::new()
}
