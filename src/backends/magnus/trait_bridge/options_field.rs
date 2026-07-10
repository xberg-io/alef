use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::ApiSurface;

/// Find a bridge config that uses options_field binding and a parameter of the options_type.
/// This complements find_bridge_param which only handles FunctionParam bindings.
pub fn find_options_field_binding<'a>(
    func: &crate::core::ir::FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for bridge in bridges {
        if bridge.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        if let Some(options_type) = &bridge.options_type {
            for (idx, param) in func.params.iter().enumerate() {
                let matches = match &param.ty {
                    crate::core::ir::TypeRef::Named(n) => n == options_type,
                    crate::core::ir::TypeRef::Optional(inner) => {
                        if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                            n == options_type
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if matches {
                    return Some((idx, bridge));
                }
            }
        }
    }
    None
}

/// Generate an options_field visitor bridge function for Magnus.
///
/// This function accepts the configured bridge object as an optional argument separate from options.
/// Since the bridge handle is excluded from the binding, we create options internally and wire
/// the bridge object directly into it.
pub fn gen_options_field_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    options_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Rb", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

    let non_option_params: Vec<_> = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != options_param_idx)
        .collect();

    let mut sig_parts = Vec::new();
    for (_, p) in &non_option_params {
        let ty = mapper.map_type(&p.ty);
        sig_parts.push(format!("{}: {}", p.name, ty));
    }
    let bridge_param_name = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    sig_parts.push(format!("{bridge_param_name}: Option<magnus::Value>"));

    let _params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let has_error = func.error_type.is_some();
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), e.to_string()))";

    let options_name = &func.params[options_param_idx].name;
    let Some(options_field) = bridge_cfg.resolved_options_field() else {
        return String::new();
    };
    let Some(options_type) = ({
        let raw = &func.params[options_param_idx].ty;
        let inner = match raw {
            TypeRef::Optional(b) => b.as_ref(),
            other => other,
        };
        if let TypeRef::Named(n) = inner {
            Some(n.as_str())
        } else {
            bridge_cfg.options_type.as_deref()
        }
    }) else {
        return String::new();
    };
    let visitor_extract = format!(
        "let {options_name}_core = match {bridge_param_name} {{\n    \
         Some(v) if !v.is_nil() => {{\n        \
         if magnus::RHash::from_value(v).is_some() {{\n            \
         let json = v.funcall::<_, _, String>(\"to_json\", ()).map_err(|e| {{\n                \
         magnus::Error::new(\n                    \
         unsafe {{ magnus::Ruby::get_unchecked() }}.exception_runtime_error(),\n                    \
         format!(\"failed to serialize Ruby options to JSON: {{}}\", e),\n                \
         )\n            \
         }})?;\n            \
         serde_json::from_str::<{core_import}::{options_type}>(&json).map_err(|e| {{\n                \
         magnus::Error::new(\n                    \
         unsafe {{ magnus::Ruby::get_unchecked() }}.exception_runtime_error(),\n                    \
         format!(\"failed to deserialize options JSON: {{}}\", e),\n                \
         )\n            \
         }})?\n        \
         }} else if let Ok(opts_binding) = <&{options_type} as magnus::TryConvert>::try_convert(v) {{\n            \
         opts_binding.clone().into()\n        \
         }} else {{\n            \
         let bridge = {struct_name}::new(v);\n            \
         let handle = std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path};\n            \
         let mut opts = {core_import}::{options_type}::default();\n            \
         opts.{options_field} = Some(handle);\n            \
         opts\n        \
         }}\n    \
         }},\n    \
         _ => {core_import}::{options_type}::default(),\n    \
         }};",
        struct_name = struct_name,
        handle_path = handle_path,
        core_import = core_import,
        options_name = options_name,
        options_type = options_type,
        options_field = options_field,
        bridge_param_name = bridge_param_name,
    );

    let call_args: String = non_option_params
        .iter()
        .map(|(_, p)| match &p.ty {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if p.optional {
                    format!("{}.as_ref().map(|v| &v.inner)", p.name)
                } else {
                    format!("&{}.inner", p.name)
                }
            }
            TypeRef::Named(_) => format!("{}.into()", p.name),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if opaque_types.contains(n.as_str()) {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("{}.map(Into::into)", p.name)
                    }
                } else {
                    p.name.clone()
                }
            }
            TypeRef::String | TypeRef::Char => {
                if p.is_ref {
                    format!("&{}", p.name)
                } else {
                    p.name.clone()
                }
            }
            _ => p.name.clone(),
        })
        .chain(std::iter::once(format!("Some({options_name}_core)")))
        .collect::<Vec<_>>()
        .join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{visitor_extract}\n    {core_call}{err_conv}")
        } else {
            format!("{visitor_extract}\n    {core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{visitor_extract}\n    {core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str("#[allow(unused_variables)]\n");
    out.push_str("pub fn ");
    out.push_str(func_name);
    out.push_str("(args: &[magnus::Value]) -> ");
    out.push_str(&ret);
    out.push_str(" {\n");
    out.push_str("    let args = magnus::scan_args::scan_args::<\n");
    out.push_str("        (");
    for (_, p) in &non_option_params {
        out.push_str(&mapper.map_type(&p.ty));
        out.push_str(", ");
    }
    out.push_str("), (Option<magnus::Value>,), (), (), (), ()\n");
    out.push_str("    >(args)?;\n");
    out.push_str("    let (");
    for (i, (_, p)) in non_option_params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&p.name);
    }
    out.push_str(",) = args.required;\n");
    out.push_str("    let (");
    out.push_str(bridge_param_name);
    out.push_str(",) = args.optional;\n");
    out.push_str("    ");
    out.push_str(&body);
    out.push('\n');
    out.push_str("}\n");

    out
}
