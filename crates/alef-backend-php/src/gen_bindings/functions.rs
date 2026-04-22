use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_adapters::AdapterBodies;
use alef_codegen::generators;
use alef_codegen::shared;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};

use super::helpers::{
    gen_php_call_args, gen_php_call_args_with_let_bindings, gen_php_function_params,
    gen_php_lossy_binding_to_core_fields, gen_php_named_let_bindings, php_wrap_return,
};

/// Format the `-> ReturnType` part of a function signature.
/// Returns an empty string for unit `()` return types to avoid
/// emitting `-> ()` which triggers `clippy::unused_unit`.
fn return_type_sig(annotation: &str) -> String {
    if annotation == "()" {
        String::new()
    } else {
        format!(" -> {annotation}")
    }
}

/// Build the set of parameter names that are trait bridge params.
/// Bridge params are sanitized to a String/Option<String> in the IR but must be
/// passed as `None` to the core function (the PHP backend has no bridge implementation).
fn bridge_param_names(bridges: &[TraitBridgeConfig]) -> AHashSet<&str> {
    bridges.iter().filter_map(|b| b.param_name.as_deref()).collect()
}

/// Replace the argument expression for each bridge param with `None` in the comma-separated
/// call args string.  The replacement is done term-by-term so partial-name matches are avoided.
fn apply_bridge_none_substitutions(call_args: &str, func: &FunctionDef, bridge_names: &AHashSet<&str>) -> String {
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

/// Returns true if any Named (non-opaque) param with `is_ref=true` is present.
/// These are the params that would fail the `.clone().into()` path when no `From` impl exists,
/// and for which the serde round-trip is a viable recovery path.
fn has_ref_named_params(params: &[alef_core::ir::ParamDef], opaque_types: &AHashSet<String>) -> bool {
    params
        .iter()
        .any(|p| p.is_ref && matches!(&p.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())))
}

/// Generate serde-based let bindings for Named (non-opaque) params that have `is_ref=true`.
/// These replace the `.clone().into()` bindings when no `From` impl is available.
/// The round-trip works because PHP binding types derive `Serialize` and core types derive
/// `Deserialize`.
fn gen_php_serde_let_bindings(
    params: &[alef_core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for p in params {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                if p.is_ref {
                    // Serde round-trip: binding type -> JSON -> core type.
                    // Build code lines directly to avoid format-string escaping issues.
                    let pname = &p.name;
                    if p.optional {
                        // Generated code pattern:
                        //   let {pname}_core: Option<{core}::{name}> = {pname}.as_ref()
                        //       .map(|v| {
                        //           let _json = serde_json::to_string(v).map_err(|e| format!("{e}"))?;
                        //           serde_json::from_str::<{core}::{name}>(&_json).map_err(|e| format!("{e}"))
                        //       }).transpose()?;
                        let mut line = String::new();
                        write!(
                            line,
                            "let {pname}_core: Option<{core_import}::{name}> = {pname}.as_ref()"
                        )
                        .ok();
                        write!(line, "\n        .map(|v| {{").ok();
                        write!(line, "\n            let _json = serde_json::to_string(v)").ok();
                        write!(line, ".map_err(|e| format!(\"{{e}}\"))?;").ok();
                        write!(
                            line,
                            "\n            serde_json::from_str::<{core_import}::{name}>(&_json)"
                        )
                        .ok();
                        write!(line, ".map_err(|e| format!(\"{{e}}\"))").ok();
                        write!(line, "\n        }}).transpose()?;").ok();
                        writeln!(out, "{line}").ok();
                    } else {
                        // Generated code pattern:
                        //   let {pname}_json = serde_json::to_string(&{pname}).map_err(|e| format!("{e}"))?;
                        //   let {pname}_core: {core}::{name} = serde_json::from_str(&{pname}_json)
                        //       .map_err(|e| format!("{e}"))?;
                        let mut line = String::new();
                        write!(line, "let {pname}_json = serde_json::to_string(&{pname})").ok();
                        write!(line, "\n        .map_err(|e| format!(\"{{e}}\"))?;").ok();
                        write!(
                            line,
                            "\n    let {pname}_core: {core_import}::{name} = serde_json::from_str(&{pname}_json)"
                        )
                        .ok();
                        write!(line, "\n        .map_err(|e| format!(\"{{e}}\"))?;").ok();
                        writeln!(out, "{line}").ok();
                    }
                } else {
                    // Non-ref Named: use the standard .clone().into() path.
                    if p.optional {
                        writeln!(
                            out,
                            "let {}_core: Option<{core_import}::{name}> = {}.map(|v| v.clone().into());",
                            p.name, p.name
                        )
                        .ok();
                    } else {
                        writeln!(
                            out,
                            "let {}_core: {core_import}::{name} = {}.clone().into();",
                            p.name, p.name
                        )
                        .ok();
                    }
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        if p.optional {
                            writeln!(
                                out,
                                "let {}_core: Option<Vec<{core_import}::{name}>> = {}.as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect());",
                                p.name, p.name
                            )
                            .ok();
                        } else {
                            writeln!(
                                out,
                                "let {}_core: Vec<{core_import}::{name}> = {}.iter().map(|x| x.clone().into()).collect();",
                                p.name, p.name
                            )
                            .ok();
                        }
                    }
                } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref {
                    writeln!(
                        out,
                        "let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();",
                        p.name, p.name
                    )
                    .ok();
                }
            }
            _ => {}
        }
    }
    out
}
/// Generate an instance method binding for an opaque struct.
pub(crate) fn gen_instance_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    is_opaque: bool,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &AdapterBodies,
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
    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else if can_delegate && is_opaque {
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
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    let ret_sig = return_type_sig(&return_annotation);
    if params_str.is_empty() {
        format!(
            "{trait_allow}pub fn {}(&self){ret_sig} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}(&self, {params_str}){ret_sig} {{\n    \
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
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    let ret_sig = return_type_sig(&return_annotation);
    if params_str.is_empty() {
        format!(
            "{trait_allow}pub fn {}(&self){ret_sig} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}(&self, {params_str}){ret_sig} {{\n    \
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
        // Method cannot be auto-delegated — skip it entirely rather than emitting a panic stub.
        return String::new();
    };

    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n"
    } else {
        ""
    };
    let ret_sig = return_type_sig(&return_annotation);
    if params.is_empty() {
        format!(
            "{trait_allow}pub fn {}(){ret_sig} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "{trait_allow}pub fn {}({params}){ret_sig} {{\n    \
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
    has_serde: bool,
) -> String {
    let body = gen_function_body(func, opaque_types, core_import, &mapper.enum_names, bridges, has_serde);
    let bridge_names = bridge_param_names(bridges);
    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !bridge_names.contains(p.name.as_str()))
        .cloned()
        .collect();
    let params = gen_php_function_params(&visible_params, mapper, opaque_types);
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let ret_sig = return_type_sig(&return_annotation);
    if params.is_empty() {
        format!(
            "pub fn {}(){ret_sig} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    } else {
        format!(
            "pub fn {}({params}){ret_sig} {{\n    \
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
    has_serde: bool,
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
    } else if func.sanitized {
        // Sanitized functions cannot be auto-delegated — emit a safe default return value.
        gen_stub_return(&func.return_type)
    } else {
        // Not auto-delegatable: use serde round-trip for Named params with is_ref=true when
        // serde is available (avoids the missing From<BindingType> for core type compile error),
        // otherwise fall back to the .clone().into() let-binding path.
        let let_bindings = if has_serde && has_ref_named_params(&func.params, opaque_types) {
            gen_php_serde_let_bindings(&func.params, opaque_types, core_import)
        } else {
            gen_php_named_let_bindings(&func.params, opaque_types, core_import)
        };
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
    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !bridge_names.contains(p.name.as_str()))
        .cloned()
        .collect();
    let params = gen_php_function_params(&visible_params, mapper, opaque_types);
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let ret_sig = return_type_sig(&return_annotation);
    if params.is_empty() {
        format!(
            "pub fn {}_async(){ret_sig} {{\n    \
             {body}\n\
             }}",
            func.name
        )
    } else {
        format!(
            "pub fn {}_async({params}){ret_sig} {{\n    \
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
        // Cannot auto-delegate — skip entirely.
        String::new()
    }
}

/// Generate an async instance method binding for PHP (block on runtime).
pub(crate) fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &PhpMapper,
    is_opaque: bool,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    adapter_bodies: &AdapterBodies,
) -> String {
    let params = gen_php_function_params(&method.params, mapper, opaque_types);
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else if can_delegate && is_opaque {
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
        // Cannot auto-delegate — skip entirely.
        return String::new();
    };

    let ret_sig = return_type_sig(&return_annotation);
    if params.is_empty() {
        format!(
            "pub fn {}_async(&self){ret_sig} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    } else {
        format!(
            "pub fn {}_async(&self, {params}){ret_sig} {{\n    \
             {body}\n\
             }}",
            method.name
        )
    }
}

/// Generate an async static method binding for PHP (block on runtime).
pub(crate) fn gen_async_static_method(
    _method: &MethodDef,
    _mapper: &PhpMapper,
    _opaque_types: &AHashSet<String>,
) -> String {
    // Async static methods are not auto-delegatable — skip entirely.
    String::new()
}

/// Generate a safe stub return expression for a sanitized function that cannot be auto-delegated.
fn gen_stub_return(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Vec(_) => "Vec::new()".to_string(),
        TypeRef::String => "String::new()".to_string(),
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "false".to_string(),
                PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
                _ => "0".to_string(),
            }
        }
        TypeRef::Map(_, _) => "Default::default()".to_string(),
        _ => "Default::default()".to_string(),
    }
}
