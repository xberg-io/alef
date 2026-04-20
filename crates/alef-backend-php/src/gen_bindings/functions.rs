use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_codegen::generators;
use alef_codegen::shared;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};

use super::helpers::{
    gen_php_call_args, gen_php_call_args_with_let_bindings, gen_php_function_params,
    gen_php_lossy_binding_to_core_fields, gen_php_named_let_bindings, php_wrap_return,
};

/// Build the set of parameter names that are trait bridge params.
/// Bridge params are sanitized to a String/Option<String> in the IR but must be
/// passed as `None` to the core function (the PHP backend has no bridge implementation).
fn bridge_param_names(bridges: &[TraitBridgeConfig]) -> AHashSet<&str> {
    bridges
        .iter()
        .filter_map(|b| b.param_name.as_deref())
        .collect()
}

/// Replace the argument expression for each bridge param with `None` in the comma-separated
/// call args string.  The replacement is done term-by-term so partial-name matches are avoided.
fn apply_bridge_none_substitutions(
    call_args: &str,
    func: &FunctionDef,
    bridge_names: &AHashSet<&str>,
) -> String {
    if bridge_names.is_empty() || call_args.is_empty() {
        return call_args.to_string();
    }
    // Split on ", " then zip with params to identify which slot to replace.
    let terms: Vec<&str> = call_args.split(", ").collect();
    let result: Vec<String> = terms
        .into_iter()
        .zip(func.params.iter())
        .map(|(term, param)| {
            if bridge_names.contains(param.name.as_str()) {
                "None".to_string()
            } else {
                term.to_string()
            }
        })
        .collect();
    result.join(", ")
}
/// Generate an instance method binding for an opaque struct.
pub(crate) fn gen_instance_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    is_opaque: bool,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let params_str = if params.is_empty() { String::new() } else { params };

    // Non-opaque Named params are received as `&T` (ext-php-rs doesn't support owned T via
    // FromZvalMut), so gen_php_function_params uses &T and gen_php_call_args emits
    // `.clone().into()`.  This means we CAN delegate opaque methods even with non-opaque Named
    // params — the `&T` → clone → `.into()` chain handles the conversion correctly.
    let body = if can_delegate && is_opaque {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let is_owned_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::Owned));
        let core_call = if is_owned_receiver {
            format!("(*self.inner).clone().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!(
                    "{core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok(())"
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
                );
                format!(
                    "let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok({wrap})"
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
            )
        }
    } else if is_opaque {
        // Not auto-delegatable opaque instance method — use let-binding conversion
        let let_bindings = gen_php_named_let_bindings(&method.params, opaque_types, core_import);
        let call_args = gen_php_call_args_with_let_bindings(&method.params, opaque_types);
        let core_call = format!("self.inner.{}({})", method.name, call_args);
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!(
                    "{let_bindings}{core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok(())"
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
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok({wrap})"
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
                    method.returns_cow
                )
            )
        }
    } else {
        gen_php_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    if params_str.is_empty() {
        format!(
            "{trait_allow}pub fn {}(&self) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}(&self, {params_str}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate an instance method binding for a non-opaque struct (uses gen_lossy_binding_to_core_fields).
pub(crate) fn gen_instance_method_non_opaque(
    method: &MethodDef,
    mapper: &PhpMapper,
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    enums: &[EnumDef],
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    // Skip RefMut receivers — can't delegate because we don't have a mutable reference.
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::RefMut));

    let can_delegate = !method.sanitized
        && !is_ref_mut_receiver
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let params_str = if params.is_empty() { String::new() } else { params };

    let body = if can_delegate {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let field_conversions = gen_php_lossy_binding_to_core_fields(typ, core_import, &mapper.enum_names, enums);
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
        );

        let is_enum_return = matches!(&method.return_type, TypeRef::Named(n) if mapper.enum_names.contains(n.as_str()));

        if method.error_type.is_some() {
            if is_enum_return {
                format!(
                    "{field_conversions}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok(format!(\"{{:?}}\", result))"
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
                );
                format!(
                    "{field_conversions}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok({wrap})"
                )
            }
        } else if is_enum_return {
            format!("{field_conversions}format!(\"{{:?}}\", {core_call})")
        } else {
            format!("{field_conversions}{wrapped_call}")
        }
    } else {
        gen_php_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    if params_str.is_empty() {
        format!(
            "{trait_allow}pub fn {}(&self) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}(&self, {params_str}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate a static method binding.
pub(crate) fn gen_static_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    typ: &TypeDef,
    _core_import: &str,
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
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
            )
        }
    } else {
        gen_php_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    if params.is_empty() {
        format!(
            "{trait_allow}pub fn {}() -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}({params}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate a free function binding as a static method body (no `#[php_function]` attribute).
/// Used when functions are placed inside a `#[php_impl]` facade class.
pub(crate) fn gen_function_as_static_method(
    func: &FunctionDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    bridges: &[TraitBridgeConfig],
) -> String {
    let body = gen_function_body(func, opaque_types, core_import, &mapper.enum_names, bridges);
    let bridge_names = bridge_param_names(bridges);
    let visible_params: Vec<_> = func.params.iter().filter(|p| !bridge_names.contains(p.name.as_str())).cloned().collect();
    let params = gen_php_function_params(&visible_params, mapper, opaque_types);
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    if params.is_empty() {
        format!(
            "pub fn {}() -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    } else {
        format!(
            "pub fn {}({params}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    }
}

/// Shared body generation for sync free functions.
fn gen_function_body(
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    enum_names: &AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let bridge_names = bridge_param_names(bridges);
    let can_delegate = shared::can_auto_delegate_function(func, opaque_types);
    if can_delegate {
        let let_bindings = gen_php_named_let_bindings(&func.params, opaque_types, core_import);
        let raw_call_args = gen_php_call_args_with_let_bindings(&func.params, opaque_types);
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
        if func.error_type.is_some() {
            if is_enum_return {
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok(format!(\"{{:?}}\", result))"
                )
            } else {
                let wrap = php_wrap_return(
                    "result",
                    &func.return_type,
                    "",
                    opaque_types,
                    false,
                    func.returns_ref,
                    false,
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok({wrap})"
                )
            }
        } else if is_enum_return {
            format!("{let_bindings}format!(\"{{:?}}\", {core_call})")
        } else {
            format!(
                "{let_bindings}{}",
                php_wrap_return(
                    &core_call,
                    &func.return_type,
                    "",
                    opaque_types,
                    false,
                    func.returns_ref,
                    false
                )
            )
        }
    } else {
        // Not auto-delegatable but try serde-based conversion
        let let_bindings = gen_php_named_let_bindings(&func.params, opaque_types, core_import);
        let raw_call_args = gen_php_call_args_with_let_bindings(&func.params, opaque_types);
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
                opaque_types,
                false,
                func.returns_ref,
                false,
            );
            format!(
                "{let_bindings}let result = {core_call}.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n    Ok({wrap})"
            )
        } else {
            format!(
                "{let_bindings}{}",
                php_wrap_return(
                    &core_call,
                    &func.return_type,
                    "",
                    opaque_types,
                    false,
                    func.returns_ref,
                    false
                )
            )
        }
    }
}

/// Generate an async free function binding as a static method body (no `#[php_function]` attribute).
/// Used when functions are placed inside a `#[php_impl]` facade class.
pub(crate) fn gen_async_function_as_static_method(
    func: &FunctionDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    bridges: &[TraitBridgeConfig],
) -> String {
    let body = gen_async_function_body(func, opaque_types, core_import, &mapper.enum_names, bridges);
    let bridge_names = bridge_param_names(bridges);
    let visible_params: Vec<_> = func.params.iter().filter(|p| !bridge_names.contains(p.name.as_str())).cloned().collect();
    let params = gen_php_function_params(&visible_params, mapper, opaque_types);
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    if params.is_empty() {
        format!(
            "pub fn {}_async() -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    } else {
        format!(
            "pub fn {}_async({params}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    }
}

/// Shared body generation for async free functions (block_on variant).
fn gen_async_function_body(
    func: &FunctionDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    enum_names: &AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let bridge_names = bridge_param_names(bridges);
    let can_delegate = shared::can_auto_delegate_function(func, opaque_types);
    if can_delegate {
        let let_bindings = gen_php_named_let_bindings(&func.params, opaque_types, core_import);
        let raw_call_args = gen_php_call_args_with_let_bindings(&func.params, opaque_types);
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
                opaque_types,
                false,
                func.returns_ref,
                false,
            )
        };
        if func.error_type.is_some() {
            format!(
                "{let_bindings}WORKER_RUNTIME.block_on(async {{\n        \
                 let result = {core_call}.await.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n        \
                 Ok({result_wrap})\n    }})"
            )
        } else {
            format!(
                "{let_bindings}let result = WORKER_RUNTIME.block_on(async {{ {core_call}.await }});\n    {result_wrap}"
            )
        }
    } else {
        gen_php_unimplemented_body(
            &func.return_type,
            &format!("{}_async", func.name),
            func.error_type.is_some(),
        )
    }
}

/// Generate an async instance method binding for PHP (block on runtime).
pub(crate) fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    is_opaque: bool,
    type_name: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate && is_opaque {
        let call_args = gen_php_call_args(&method.params, opaque_types);
        let inner_clone = "let inner = self.inner.clone();\n    ";
        let core_call = format!("inner.{}({})", method.name, call_args);
        let result_wrap = php_wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            true,
            method.returns_ref,
            method.returns_cow,
        );
        if method.error_type.is_some() {
            format!(
                "{inner_clone}WORKER_RUNTIME.block_on(async {{\n        \
                 let result = {core_call}.await.map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))?;\n        \
                 Ok({result_wrap})\n    }})"
            )
        } else {
            format!(
                "{inner_clone}let result = WORKER_RUNTIME.block_on(async {{ {core_call}.await }});\n    {result_wrap}"
            )
        }
    } else {
        gen_php_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            method.error_type.is_some(),
        )
    };

    if params.is_empty() {
        format!(
            "pub fn {}_async(&self) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "pub fn {}_async(&self, {params}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate an async static method binding for PHP (block on runtime).
pub(crate) fn gen_async_static_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let body = gen_php_unimplemented_body(
        &method.return_type,
        &format!("{}_async", method.name),
        method.error_type.is_some(),
    );

    if params.is_empty() {
        format!(
            "pub fn {}_async() -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "pub fn {}_async({params}) -> {return_annotation} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate a type-appropriate unimplemented body for PHP (no todo!()).
pub(crate) fn gen_php_unimplemented_body(
    return_type: &alef_core::ir::TypeRef,
    fn_name: &str,
    has_error: bool,
) -> String {
    use alef_core::ir::TypeRef;
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(ext_php_rs::exception::PhpException::default(\"{err_msg}\".to_string()))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                alef_core::ir::PrimitiveType::F32 => "0.0f32".to_string(),
                alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!("panic!(\"alef: {fn_name} not auto-delegatable\")"),
            TypeRef::Duration => "std::time::Duration::default()".to_string(),
        }
    }
}
