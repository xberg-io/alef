use crate::adapters::AdapterBodies;
use crate::backends::php::type_map::PhpMapper;
use crate::codegen::doc_emission;
use crate::codegen::generators;
use crate::codegen::shared;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToLowerCamelCase;
use minijinja::context;

use super::super::helpers::{
    gen_php_call_args, gen_php_call_args_with_let_bindings, gen_php_function_params,
    gen_php_lossy_binding_to_core_fields, gen_php_named_let_bindings, param_conversion_is_fallible, php_wrap_return,
};
use super::params::{
    PhpEnumReturnSets, PhpParamTypeSets, apply_bridge_none_substitutions, apply_default_param_substitutions,
    bridge_param_names, gen_php_serde_let_bindings, has_ref_named_params, has_sanitized_recoverable,
    override_bytes_return_type, promote_default_params, promoted_default_param_names, return_type_sig,
};
use super::stubs::gen_stub_return;

#[allow(clippy::too_many_arguments)]
/// Generate an instance method binding for an opaque struct.
pub(crate) fn gen_instance_method(
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

    let params_str = if params.is_empty() { String::new() } else { params };

    // Non-opaque Named params are received as `&T` (ext-php-rs doesn't support owned T via
    // FromZvalMut), so gen_php_function_params uses &T and gen_php_call_args emits
    // `.clone().into()`.  This means we CAN delegate opaque methods even with non-opaque Named
    // params — the `&T` → clone → `.into()` chain handles the conversion correctly.
    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else if can_delegate && is_opaque {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let is_owned_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
        // For opaque types, self.inner is Arc<T> or Arc<Mutex<T>>.
        // When the type is in mutex_types, we need .lock().unwrap() to access the inner value,
        // regardless of whether the method is &self or &mut self. The Mutex protects the inner value.
        let needs_lock = mutex_types.contains(type_name);
        let core_call = if is_owned_receiver {
            format!("(*self.inner).clone().{}({})", method.name, call_args)
        } else if needs_lock {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                crate::backends::php::template_env::render(
                    "php_result_unit_body.jinja",
                    context! {
                        core_call => &core_call,
                    },
                )
            } else {
                let wrap = php_wrap_return(
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
                crate::backends::php::template_env::render(
                    "php_result_wrapped_body.jinja",
                    context! {
                        core_call => &core_call,
                        wrap => &wrap,
                    },
                )
            }
        } else {
            php_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                mutex_types,
                json_string_enum_names,
                string_enum_names,
            )
        }
    } else if is_opaque {
        // Not auto-delegatable opaque instance method — use let-binding conversion
        let let_bindings = gen_php_named_let_bindings(&method.params, opaque_types, enum_names, core_import);
        let call_args = gen_php_call_args_with_let_bindings(&method.params, opaque_types, mutex_types);
        // When the type is in mutex_types, we need .lock().unwrap() to access the inner value,
        // regardless of whether the method is &self or &mut self. The Mutex protects the inner value.
        let needs_lock = mutex_types.contains(type_name);
        let core_call = if needs_lock {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                crate::backends::php::template_env::render(
                    "php_result_unit_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &let_bindings,
                        core_call => &core_call,
                    },
                )
            } else {
                let wrap = php_wrap_return(
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
                crate::backends::php::template_env::render(
                    "php_result_wrapped_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &let_bindings,
                        core_call => &core_call,
                        wrap => &wrap,
                    },
                )
            }
        } else {
            format!(
                "{let_bindings}{}",
                php_wrap_return(
                    &core_call,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    mutex_types,
                    json_string_enum_names,
                    string_enum_names,
                )
            )
        }
    } else {
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let mut out = String::new();
    // The `#[php_impl]` facade is Rust source, so doc text must be emitted as Rust line
    // doc-comments (`///`). PHPDoc `/** … */` blocks are unsafe here: Rust block comments
    // nest, so doc text containing `/*` (e.g. `image/*`) opens a nested comment that the
    // intended closing `*/` never balances → `error[E0758]: unterminated block doc-comment`.
    doc_emission::emit_rustdoc(&mut out, &method.doc, "    ");
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    // ext-php-rs's `ZendClassObject<T>` has an inherent `initialize(&mut self, T)` method
    // that collides with a user-written `pub fn initialize(&self)` during `#[php_impl]`
    // expansion. Rename the Rust-side fn to `initialize_plugin` and preserve the PHP-facing
    // name via `#[php(name = "initialize")]`.
    let (rust_name, php_rename_attr) = match method.name.as_str() {
        "initialize" => (
            "initialize_plugin".to_string(),
            "#[php(name = \"initialize\")]\n    ".to_string(),
        ),
        _ => (method.name.clone(), String::new()),
    };
    let ret_sig = return_type_sig(&return_annotation);
    out.push_str("    ");
    out.push_str(&php_rename_attr);
    out.push_str(trait_allow);
    if params_str.is_empty() {
        out.push_str(&crate::backends::php::template_env::render(
            "php_method_definition_no_params.jinja",
            context! {
                name => &rust_name,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    } else {
        out.push_str(&crate::backends::php::template_env::render(
            "php_method_definition_with_params.jinja",
            context! {
                name => &rust_name,
                params => &params_str,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
/// Generate an instance method binding for a non-opaque struct (uses gen_lossy_binding_to_core_fields).
pub(crate) fn gen_instance_method_non_opaque(
    method: &MethodDef,
    mapper: &PhpMapper,
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    enums: &[EnumDef],
    bridge_type_aliases: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    let string_enum_names = &mapper.enum_names;
    let json_string_enum_names = &mapper.json_string_enum_names;
    let params = gen_php_function_params(&method.params, mapper, opaque_types, bridge_type_aliases);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    // RefMut methods can be delegated if the type is Mutex-wrapped (present in mutex_types).
    // Arc<T> doesn't support &mut T directly, but Arc<Mutex<T>> does via lock().
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
    let has_mut_methods = mutex_types.contains(&typ.name);

    let can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || has_mut_methods)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let params_str = if params.is_empty() { String::new() } else { params };

    let body = if can_delegate {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let field_conversions = gen_php_lossy_binding_to_core_fields(
            typ,
            core_import,
            &mapper.enum_names,
            &mapper.untagged_data_enum_names,
            enums,
        );
        let core_call = format!("core_self.{}({})", method.name, call_args);

        // Use php_wrap_return for proper type conversions
        let wrapped_call = php_wrap_return(
            &core_call,
            &method.return_type,
            &typ.name,
            opaque_types,
            typ.is_opaque,
            method.returns_ref,
            method.returns_cow,
            mutex_types,
            json_string_enum_names,
            string_enum_names,
        );

        let is_enum_return = matches!(&method.return_type, TypeRef::Named(n) if mapper.enum_names.contains(n.as_str()));

        if method.error_type.is_some() {
            if is_enum_return {
                crate::backends::php::template_env::render(
                    "php_result_debug_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &field_conversions,
                        core_call => &core_call,
                    },
                )
            } else {
                let wrap = php_wrap_return(
                    "result",
                    &method.return_type,
                    &typ.name,
                    opaque_types,
                    typ.is_opaque,
                    method.returns_ref,
                    method.returns_cow,
                    mutex_types,
                    json_string_enum_names,
                    string_enum_names,
                );
                crate::backends::php::template_env::render(
                    "php_result_wrapped_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &field_conversions,
                        core_call => &core_call,
                        wrap => &wrap,
                    },
                )
            }
        } else if is_enum_return {
            crate::backends::php::template_env::render(
                "php_debug_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &field_conversions,
                    core_call => &core_call,
                },
            )
        } else {
            crate::backends::php::template_env::render(
                "php_wrapped_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &field_conversions,
                    wrapped_call => &wrapped_call,
                },
            )
        }
    } else {
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    // ext-php-rs's `ZendClassObject` has an inherent `initialize(&mut self, T)` method;
    // emitting a user `pub fn initialize(&self)` collides during `#[php_impl]` expansion.
    // Rename the Rust-side fn to avoid the collision while keeping the PHP method name.
    let (rust_name, php_rename_attr) = match method.name.as_str() {
        "initialize" => (
            "initialize_plugin".to_string(),
            "    #[php(name = \"initialize\")]\n".to_string(),
        ),
        _ => (method.name.clone(), String::new()),
    };
    let ret_sig = return_type_sig(&return_annotation);
    if params_str.is_empty() {
        format!(
            "{php_rename_attr}{trait_allow}pub fn {rust_name}(&self){ret_sig} {{\n    \
             {body}\n\
             }}"
        )
    } else {
        format!(
            "{php_rename_attr}{trait_allow}pub fn {rust_name}(&self, {params_str}){ret_sig} {{\n    \
             {body}\n\
             }}"
        )
    }
}

/// Generate a static method binding.
pub(crate) fn gen_static_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    typ: &TypeDef,
    mutex_types: &AHashSet<String>,
) -> String {
    let string_enum_names = &mapper.enum_names;
    let json_string_enum_names = &mapper.json_string_enum_names;
    let empty_bridges = AHashSet::new();
    let params = gen_php_function_params(&method.params, mapper, opaque_types, &empty_bridges);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);
    let core_type_path = typ.rust_path.replace('-', "_");
    let call_args = gen_php_call_args(&method.params, opaque_types);

    // Exclude methods with unsupported parameter types for PHP bindings.
    // ext-php-rs has limited FromZval support:
    // - Vec<T> where T is a struct: not supported
    // - Map<K,V>: not directly supported
    // - Non-opaque Named params: not supported (no owned FromZval)
    let has_unsupported_params = method.params.iter().any(|p| {
        match &p.ty {
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => true, // non-opaque struct
            TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())),
            TypeRef::Map(_, _) => true, // Maps not directly supported
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str()))
                    || matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())))
            }
            _ => false,
        }
    });

    let body = if can_delegate && !has_unsupported_params {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        let is_enum_return = matches!(&method.return_type, TypeRef::Named(n) if mapper.enum_names.contains(n.as_str()));
        if method.error_type.is_some() {
            if is_enum_return {
                format!(
                    "{core_call}.map(|val| format!(\"{{:?}}\", val)).map_err(|e| PhpException::default(e.to_string()))"
                )
            } else {
                let wrap = php_wrap_return(
                    "val",
                    &method.return_type,
                    &typ.name,
                    opaque_types,
                    typ.is_opaque,
                    method.returns_ref,
                    method.returns_cow,
                    mutex_types,
                    json_string_enum_names,
                    string_enum_names,
                );
                if wrap == "val" {
                    format!("{core_call}.map_err(|e| PhpException::default(e.to_string()))")
                } else {
                    format!("{core_call}.map(|val| {wrap}).map_err(|e| PhpException::default(e.to_string()))")
                }
            }
        } else if is_enum_return {
            format!("format!(\"{{:?}}\", {core_call})")
        } else {
            php_wrap_return(
                &core_call,
                &method.return_type,
                &typ.name,
                opaque_types,
                typ.is_opaque,
                method.returns_ref,
                method.returns_cow,
                mutex_types,
                json_string_enum_names,
                string_enum_names,
            )
        }
    } else {
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let mut out = String::new();
    // Rust source: emit `///` line doc-comments, not PHPDoc `/** … */` (which would break
    // compilation when doc text contains `/*`, e.g. `image/*`, because Rust block comments nest).
    doc_emission::emit_rustdoc(&mut out, &method.doc, "    ");
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    let ret_sig = return_type_sig(&return_annotation);
    // The Rust fn ident stays snake_case; the PHP-facing name is camelCase so callers
    // (userland facade, stubs) can invoke it via PSR-12 camelCase method names.
    let php_name = method.name.to_lower_camel_case();
    out.push_str("    ");
    out.push_str(trait_allow);
    if params.is_empty() {
        out.push_str(&crate::backends::php::template_env::render(
            "php_static_method_definition_no_params.jinja",
            context! {
                name => &method.name,
                php_name => &php_name,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    } else {
        out.push_str(&crate::backends::php::template_env::render(
            "php_static_method_definition_with_params.jinja",
            context! {
                name => &method.name,
                php_name => &php_name,
                params => &params,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    }
    out
}

/// Generate a free function binding as a static method body (no `#[php_function]` attribute).
/// Used when functions are placed inside a `#[php_impl]` facade class.
pub(crate) fn gen_function_as_static_method(
    func: &FunctionDef,
    mapper: &PhpMapper,
    type_sets: PhpParamTypeSets<'_>,
    core_import: &str,
    bridges: &[TraitBridgeConfig],
    has_serde: bool,
    mutex_types: &AHashSet<String>,
) -> String {
    let body = gen_function_body(
        func,
        &type_sets,
        core_import,
        &PhpEnumReturnSets {
            string_enum_names: &mapper.enum_names,
            json_string_enum_names: &mapper.json_string_enum_names,
        },
        bridges,
        has_serde,
        mutex_types,
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
    // A fallible param conversion (e.g. a `Vec<Struct>` decoded element-by-element) emits
    // `return Err(...)`, so the function must return `PhpResult<T>` even when the core fn is
    // infallible. `gen_function_body` Ok-wraps the success path in the same condition.
    let has_fallible_param = func
        .params
        .iter()
        .any(|p| param_conversion_is_fallible(p, type_sets.opaque, type_sets.enums));
    let mut return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some() || has_fallible_param);
    // For Bytes returns, convert Vec<u8> to String in the type annotation
    if matches!(&func.return_type, TypeRef::Bytes) {
        return_annotation = override_bytes_return_type(&return_annotation);
    }

    let mut out = String::new();
    // Rust source: emit `///` line doc-comments, not PHPDoc `/** … */` (which would break
    // compilation when doc text contains `/*`, e.g. `image/*`, because Rust block comments nest).
    doc_emission::emit_rustdoc(&mut out, &func.doc, "    ");
    let ret_sig = return_type_sig(&return_annotation);
    // The Rust fn ident stays snake_case; the PHP-facing name is camelCase so callers
    // (userland facade, stubs) can invoke it via PSR-12 camelCase method names.
    let php_name = func.name.to_lower_camel_case();
    if params.is_empty() {
        out.push_str(&crate::backends::php::template_env::render(
            "php_static_method_definition_no_params.jinja",
            context! {
                name => &func.name,
                php_name => &php_name,
                ret_sig => &ret_sig,
                body => &body,
            },
        ));
    } else {
        out.push_str(&crate::backends::php::template_env::render(
            "php_static_method_definition_with_params.jinja",
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

/// Shared body generation for sync free functions.
fn gen_function_body(
    func: &FunctionDef,
    type_sets: &PhpParamTypeSets<'_>,
    core_import: &str,
    enum_returns: &PhpEnumReturnSets<'_>,
    bridges: &[TraitBridgeConfig],
    has_serde: bool,
    mutex_types: &AHashSet<String>,
) -> String {
    let bridge_names = bridge_param_names(bridges);
    let can_delegate = shared::can_auto_delegate_function(func, type_sets.opaque);
    // When the core fn is infallible but a param conversion can `return Err(...)`, the binding
    // signature is still `PhpResult<T>` (see `gen_function_as_static_method`), so the success
    // path must be `Ok(...)`-wrapped.
    let force_ok = func.error_type.is_none()
        && func
            .params
            .iter()
            .any(|p| param_conversion_is_fallible(p, type_sets.opaque, type_sets.enums));
    if can_delegate {
        let promoted_params = promote_default_params(&func.params, type_sets.default, type_sets.opaque);
        let promoted_names = promoted_default_param_names(&func.params, type_sets.default, type_sets.opaque);
        let let_bindings = gen_php_named_let_bindings(&promoted_params, type_sets.opaque, type_sets.enums, core_import);
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
        let is_enum_return =
            matches!(&func.return_type, TypeRef::Named(n) if enum_returns.string_enum_names.contains(n.as_str()));
        if func.error_type.is_some() {
            if is_enum_return {
                crate::backends::php::template_env::render(
                    "php_result_debug_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &let_bindings,
                        core_call => &core_call,
                    },
                )
            } else {
                let wrap = php_wrap_return(
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
                );
                crate::backends::php::template_env::render(
                    "php_result_wrapped_body_with_let_bindings.jinja",
                    context! {
                        let_bindings => &let_bindings,
                        core_call => &core_call,
                        wrap => &wrap,
                    },
                )
            }
        } else if is_enum_return {
            crate::backends::php::template_env::render(
                "php_debug_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                },
            )
        } else {
            let wrapped_call = php_wrap_return(
                &core_call,
                &func.return_type,
                "",
                type_sets.opaque,
                false,
                func.returns_ref,
                false,
                mutex_types,
                enum_returns.json_string_enum_names,
                enum_returns.string_enum_names,
            );
            let body_template = if force_ok {
                "php_ok_wrapped_body_with_let_bindings.jinja"
            } else {
                "php_wrapped_body_with_let_bindings.jinja"
            };
            crate::backends::php::template_env::render(
                body_template,
                context! {
                    let_bindings => &let_bindings,
                    wrapped_call => &wrapped_call,
                },
            )
        }
    } else if func.sanitized
        && !has_sanitized_recoverable(&func.params)
        && !(has_serde && func.error_type.is_some() && has_ref_named_params(&func.params, type_sets.opaque))
    {
        // Sanitized functions cannot be auto-delegated AND we have no recoverable serde path —
        // emit a safe stub: Err(...) when the signature is PhpResult<T>, default value otherwise.
        gen_stub_return(&func.return_type, func.error_type.is_some(), &func.name)
    } else {
        // Not auto-delegatable: use serde round-trip for Named params with is_ref=true and for
        // sanitized Vec<tuple> params (decoded as Vec<String>). The serde path requires the
        // function to return Result (uses `?` operator).
        let promoted_params = promote_default_params(&func.params, type_sets.default, type_sets.opaque);
        let promoted_names = promoted_default_param_names(&func.params, type_sets.default, type_sets.opaque);
        let needs_serde = func.error_type.is_some()
            && (has_ref_named_params(&func.params, type_sets.opaque) || has_sanitized_recoverable(&func.params));
        let let_bindings = if has_serde && needs_serde {
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
        if func.error_type.is_some() {
            let wrap = php_wrap_return(
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
            );
            crate::backends::php::template_env::render(
                "php_result_wrapped_body_with_let_bindings.jinja",
                context! {
                    let_bindings => &let_bindings,
                    core_call => &core_call,
                    wrap => &wrap,
                },
            )
        } else {
            let wrapped_call = php_wrap_return(
                &core_call,
                &func.return_type,
                "",
                type_sets.opaque,
                false,
                func.returns_ref,
                false,
                mutex_types,
                enum_returns.json_string_enum_names,
                enum_returns.string_enum_names,
            );
            let body_template = if force_ok {
                "php_ok_wrapped_body_with_let_bindings.jinja"
            } else {
                "php_wrapped_body_with_let_bindings.jinja"
            };
            crate::backends::php::template_env::render(
                body_template,
                context! {
                    let_bindings => &let_bindings,
                    wrapped_call => &wrapped_call,
                },
            )
        }
    }
}
