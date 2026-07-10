use super::{OptionsFieldBridges, is_python_builtin_name, python_safe_name, substitute_capsule_type};
use crate::backends::pyo3::type_map::python_type;
use crate::core::ir::{FunctionDef, TypeRef};

pub(super) fn gen_function_stub(
    func: &FunctionDef,
    bridge_param_names: &std::collections::HashSet<&str>,
    capsule_names: &std::collections::HashSet<&str>,
    options_field_bridges: &OptionsFieldBridges<'_>,
    streaming_return_types: &std::collections::HashMap<(Option<String>, String), String>,
) -> String {
    // `#[pyo3(signature = ...)]` (and the api.py wrapper) use: once any param is optional, every
    let mut params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let optional = p.optional || crate::codegen::shared::is_promoted_optional(&func.params, idx);
            let type_str = if bridge_param_names.contains(p.name.as_str()) {
                "object".to_string()
            } else {
                substitute_capsule_type(&python_type(&p.ty), capsule_names)
            };
            if optional {
                let param_type = if type_str.ends_with("| None") {
                    type_str
                } else {
                    format!("{type_str} | None")
                };
                format!("{}: {} = None", p.name, param_type)
            } else {
                format!("{}: {}", p.name, type_str)
            }
        })
        .collect();

    let bridge_kwarg = func.params.iter().find_map(|p| {
        let type_name = match &p.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        }?;
        let (kwarg_name, type_alias, trait_name) = options_field_bridges.get(type_name)?;
        Some((*kwarg_name, *type_alias, *trait_name))
    });
    if let Some((kwarg_name, type_alias, trait_name)) = bridge_kwarg {
        let visitor_type = trait_name.or(type_alias).unwrap_or("object");
        params.push(format!("{kwarg_name}: {visitor_type} | object | None = None"));
    }

    let streaming_key = (None::<String>, func.name.clone());
    let return_type = if let Some(item_type) = streaming_return_types.get(&streaming_key) {
        format!("AsyncIterator[{item_type}]")
    } else {
        substitute_capsule_type(&python_type(&func.return_type), capsule_names)
    };
    let safe_name = python_safe_name(&func.name);
    let def_kw = if func.is_async { "async def" } else { "def" };

    let has_builtin_param = params
        .iter()
        .any(|p| is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
    let single_line = format!(
        "{} {}({}) -> {}: ...",
        def_kw,
        safe_name,
        params.join(", "),
        return_type
    );
    if single_line.len() <= 100 && !has_builtin_param {
        single_line
    } else {
        let mut wrapped = format!("{} {}(\n", def_kw, safe_name);
        for param in &params {
            let name = param.split(':').next().unwrap_or("").trim();
            if is_python_builtin_name(name) {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped_noqa.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            } else {
                wrapped.push_str(&crate::backends::pyo3::template_env::render(
                    "stub_param_wrapped.jinja",
                    minijinja::context! { param => param, indent => "    " },
                ));
            }
        }
        wrapped.push_str(&crate::backends::pyo3::template_env::render(
            "stub_method_signature_end.jinja",
            minijinja::context! { return_type => &return_type },
        ));
        wrapped
    }
}
