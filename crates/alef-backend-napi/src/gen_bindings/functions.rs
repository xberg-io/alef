use crate::type_map::NapiMapper;
use ahash::AHashSet;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::generators::{self, RustBindingConfig};
use alef_codegen::naming::to_node_name;
use alef_codegen::shared::{can_auto_delegate, function_params, partition_methods};
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};
use super::helpers::{gen_vec_f32_conversion_bindings, napi_apply_primitive_casts_to_call_args, napi_gen_call_args, napi_wrap_return, napi_wrap_return_fn, needs_napi_cast, core_prim_str, needs_vec_f32_conversion};

pub(super) fn gen_opaque_struct_methods(typ: &TypeDef, mapper: &NapiMapper, cfg: &RustBindingConfig, opaque_types: &AHashSet<String>, prefix: &str, adapter_bodies: &alef_adapters::AdapterBodies) -> String {
    let mut impl_builder = ImplBuilder::new(&format!("{prefix}{}", typ.name));
    impl_builder.add_attr("napi");
    let (instance, statics) = partition_methods(&typ.methods);
    for method in &instance {
        let adapter_key = format!("{}.{}", typ.name, method.name);
        if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        impl_builder.add_method(&gen_opaque_instance_method(method, mapper, typ, cfg, opaque_types, prefix, adapter_bodies));
    }
    for method in &statics {
        let adapter_key = format!("{}.{}", typ.name, method.name);
        if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        impl_builder.add_method(&gen_static_method(method, mapper, typ, cfg, opaque_types, prefix));
    }
    impl_builder.build()
}

fn gen_opaque_instance_method(method: &MethodDef, mapper: &NapiMapper, typ: &TypeDef, cfg: &RustBindingConfig, opaque_types: &AHashSet<String>, prefix: &str, adapter_bodies: &alef_adapters::AdapterBodies) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());
    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name { format!("(js_name = \"{}\")", js_name) } else { String::new() };
    let async_kw = if method.is_async { "async " } else { "" };
    let type_name = &typ.name;
    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::RefMut));
    let call_args = napi_gen_call_args(&method.params, opaque_types);
    let opaque_can_delegate = !method.sanitized && !is_ref_mut_receiver && (!is_owned_receiver || typ.is_clone) && method.params.iter().all(|p| !p.sanitized && alef_codegen::shared::is_delegatable_param(&p.ty, opaque_types)) && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type);
    let make_async_core_call = |method_name: &str| -> String { format!("inner.{method_name}({call_args})") };
    let async_result_wrap = napi_wrap_return("result", &method.return_type, type_name, opaque_types, true, method.returns_ref, prefix);
    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(adapter_body) = adapter_bodies.get(&adapter_key) { adapter_body.clone() } else if !opaque_can_delegate { if cfg.has_serde && !method.sanitized && generators::has_named_params(&method.params, opaque_types) && method.error_type.is_some() && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type) { let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"; let serde_bindings = generators::gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        "); let serde_call_args = generators::gen_call_args_with_let_bindings(&method.params, opaque_types); let core_call = format!("self.inner.{}({serde_call_args})", method.name); if matches!(method.return_type, TypeRef::Unit) { format!("{serde_bindings}{core_call}{err_conv}?;\n    Ok(())") } else { let wrap = napi_wrap_return("result", &method.return_type, type_name, opaque_types, true, method.returns_ref, prefix); format!("{serde_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})") } } else { generators::gen_unimplemented_body(&method.return_type, &format!("{type_name}.{}", method.name), method.error_type.is_some(), cfg, &method.params, opaque_types) } } else if method.is_async { let inner_clone_line = "let inner = self.inner.clone();\n    "; let core_call_str = make_async_core_call(&method.name); generators::gen_async_body(&core_call_str, cfg, method.error_type.is_some(), &async_result_wrap, true, inner_clone_line, matches!(method.return_type, TypeRef::Unit), Some(&return_type)) } else { let use_let_bindings = generators::has_named_params(&method.params, opaque_types); let (let_bindings, call_args_for_call) = if use_let_bindings { let bindings = generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import); let args = napi_apply_primitive_casts_to_call_args(&generators::gen_call_args_with_let_bindings(&method.params, opaque_types), &method.params); (bindings, args) } else { (String::new(), napi_gen_call_args(&method.params, opaque_types)) }; let core_call = if is_owned_receiver { format!("(*self.inner).clone().{}({})", method.name, call_args_for_call) } else { format!("self.inner.{}({})", method.name, call_args_for_call) }; if method.error_type.is_some() { let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"; if matches!(method.return_type, TypeRef::Unit) { format!("{let_bindings}{core_call}{err_conv}?;\n    Ok(())") } else { let wrap = napi_wrap_return("result", &method.return_type, type_name, opaque_types, true, method.returns_ref, prefix); format!("{let_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})") } } else { format!("{let_bindings}{}", napi_wrap_return(&core_call, &method.return_type, type_name, opaque_types, true, method.returns_ref, prefix)) } } };
    let mut attrs = String::new();
    if method.params.len() + 1 > 7 { attrs.push_str("#[allow(clippy::too_many_arguments)]\n"); }
    if method.error_type.is_some() { attrs.push_str("#[allow(clippy::missing_errors_doc)]\n"); }
    if generators::is_trait_method_name(&method.name) { attrs.push_str("#[allow(clippy::should_implement_trait)]\n"); }
    format!("{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}(&self, {params}) -> {return_annotation} {{\n    {body}\n}}", method.name)
}

fn gen_static_method(method: &MethodDef, mapper: &NapiMapper, typ: &TypeDef, cfg: &RustBindingConfig, opaque_types: &AHashSet<String>, prefix: &str) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());
    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name { format!("(js_name = \"{}\")", js_name) } else { String::new() };
    let type_name = &typ.name;
    let core_type_path = typ.rust_path.replace('-', "_");
    let call_args = napi_gen_call_args(&method.params, opaque_types);
    let can_delegate_static = can_auto_delegate(method, opaque_types);
    let async_kw = if method.is_async { "async " } else { "" };
    let body = if !can_delegate_static { generators::gen_unimplemented_body(&method.return_type, &format!("{type_name}::{}", method.name), method.error_type.is_some(), cfg, &method.params, opaque_types) } else if method.is_async { let core_call = format!("{core_type_path}::{}({call_args})", method.name); let return_wrap = napi_wrap_return("result", &method.return_type, type_name, opaque_types, typ.is_opaque, method.returns_ref, prefix); generators::gen_async_body(&core_call, cfg, method.error_type.is_some(), &return_wrap, false, "", matches!(method.return_type, TypeRef::Unit), Some(&return_type)) } else { let core_call = format!("{core_type_path}::{}({call_args})", method.name); if method.error_type.is_some() { let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"; let wrapped = napi_wrap_return("val", &method.return_type, type_name, opaque_types, typ.is_opaque, method.returns_ref, prefix); if wrapped == "val" { format!("{core_call}{err_conv}") } else { format!("{core_call}.map(|val| {wrapped}){err_conv}") } } else { napi_wrap_return(&core_call, &method.return_type, type_name, opaque_types, typ.is_opaque, method.returns_ref, prefix) } } };
    let mut attrs = String::new();
    if method.params.len() > 7 { attrs.push_str("#[allow(clippy::too_many_arguments)]\n"); }
    if method.error_type.is_some() { attrs.push_str("#[allow(clippy::missing_errors_doc)]\n"); }
    if generators::is_trait_method_name(&method.name) { attrs.push_str("#[allow(clippy::should_implement_trait)]\n"); }
    format!("{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    {body}\n}}", method.name)
}

pub(super) fn gen_function(func: &FunctionDef, mapper: &NapiMapper, cfg: &RustBindingConfig, opaque_types: &AHashSet<String>, prefix: &str) -> String {
    let params = function_params(&func.params, &|ty| { if let TypeRef::Named(n) = ty { if opaque_types.contains(n.as_str()) { return format!("&{prefix}{n}"); } } mapper.map_type(ty) });
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());
    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name { format!("(js_name = \"{}\")", js_name) } else { String::new() };
    let core_import = cfg.core_import;
    let core_fn_path = { let path = func.rust_path.replace('-', "_"); if path.starts_with(core_import) { path } else { format!("{core_import}::{}", func.name) } };
    let use_let_bindings = generators::has_named_params(&func.params, opaque_types) || func.params.iter().any(|p| needs_vec_f32_conversion(&p.ty));
    let call_args = if use_let_bindings { let base_args = generators::gen_call_args_with_let_bindings(&func.params, opaque_types); napi_apply_primitive_casts_to_call_args(&base_args, &func.params) } else { napi_gen_call_args(&func.params, opaque_types) };
    let can_delegate_fn = alef_codegen::shared::can_auto_delegate_function(func, opaque_types);
    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
    let async_kw = if func.is_async { "async " } else { "" };
    let body = if !can_delegate_fn { if cfg.has_serde && use_let_bindings && func.error_type.is_some() { let serde_bindings = generators::gen_serde_let_bindings(&func.params, opaque_types, core_import, err_conv, "    "); let vec_str_bindings: String = func.params.iter().filter(|p| p.is_ref && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char))).map(|p| format!("let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();\n    ", p.name, p.name)).collect(); let core_call = format!("{core_fn_path}({call_args})"); let await_kw = if func.is_async { ".await" } else { "" }; if matches!(func.return_type, TypeRef::Unit) { format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}?;\n    Ok(())") } else { let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix); if wrapped == "val" { format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}") } else { format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){err_conv}") } } } else { generators::gen_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some(), cfg, &func.params, opaque_types) } } else if func.is_async { let mut let_bindings = if use_let_bindings { generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import) } else { String::new() }; let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params)); let core_call = format!("{core_fn_path}({call_args})"); let return_wrap = napi_wrap_return_fn("result", &func.return_type, opaque_types, func.returns_ref, prefix); let return_type = mapper.map_type(&func.return_type); generators::gen_async_body(&core_call, cfg, func.error_type.is_some(), &return_wrap, false, &let_bindings, matches!(func.return_type, TypeRef::Unit), Some(&return_type)) } else { let core_call = format!("{core_fn_path}({call_args})"); let mut let_bindings = if use_let_bindings { generators::gen_named_let_bindings_pub(&func.params, opaque_types, core_import) } else { String::new() }; let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params)); if func.error_type.is_some() { let wrapped = napi_wrap_return_fn("val", &func.return_type, opaque_types, func.returns_ref, prefix); if wrapped == "val" { format!("{let_bindings}{core_call}{err_conv}") } else { format!("{let_bindings}{core_call}.map(|val| {wrapped}){err_conv}") } } else { format!("{let_bindings}{}", napi_wrap_return_fn(&core_call, &func.return_type, opaque_types, func.returns_ref, prefix)) } } };
    let mut attrs = String::new();
    if func.params.len() > 7 { attrs.push_str("#[allow(clippy::too_many_arguments)]\n"); }
    if func.error_type.is_some() { attrs.push_str("#[allow(clippy::missing_errors_doc)]\n"); }
    format!("{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    {body}\n}}", func.name)
}
