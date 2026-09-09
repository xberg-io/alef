use crate::backends::rustler::gen_bindings::helpers::elixir_safe_param_name;
use crate::backends::rustler::gen_bindings::public_api_args::{
    json_encode_param_indices, method_deserialization_introduces_result, nif_arg,
};
use crate::backends::rustler::template_env;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

#[allow(clippy::too_many_arguments)]
pub(super) fn append_top_level_opaque_methods(
    content: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<String>,
    exclude_types: &AHashSet<&str>,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    native_mod: &str,
    app_module: &str,
) {
    let type_names: AHashSet<&str> = api
        .types
        .iter()
        .filter(|typ| typ.is_opaque && !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
        .map(|typ| typ.name.as_str())
        .collect();
    let streaming_methods: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|adapter| {
            adapter
                .owner_type
                .as_deref()
                .map(|owner| format!("{owner}.{}", adapter.name))
        })
        .collect();

    for opaque_type in api.types.iter().filter(|typ| type_names.contains(typ.name.as_str())) {
        append_type_methods(
            content,
            opaque_type,
            &streaming_methods,
            exclude_functions,
            opaque_types,
            default_types,
            native_mod,
            app_module,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_type_methods(
    content: &mut String,
    opaque_type: &TypeDef,
    streaming_methods: &AHashSet<String>,
    exclude_functions: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    native_mod: &str,
    app_module: &str,
) {
    for method in opaque_type
        .methods
        .iter()
        .filter(|method| !exclude_functions.contains(method.name.as_str()))
        .filter(|method| !streaming_methods.contains(&format!("{}.{}", opaque_type.name, method.name)))
    {
        append_method_wrapper(
            content,
            opaque_type,
            method,
            opaque_types,
            default_types,
            native_mod,
            app_module,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_method_wrapper(
    content: &mut String,
    opaque_type: &TypeDef,
    method: &MethodDef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    native_mod: &str,
    app_module: &str,
) {
    let type_lower = opaque_type.name.to_lowercase();
    let method_name = method.name.to_snake_case();
    let nif_function = if method.is_async && !method.name.ends_with("_async") {
        format!("{type_lower}_{method_name}_async")
    } else {
        format!("{type_lower}_{method_name}")
    };
    let (definition_arguments, call_arguments) = method_arguments(method, opaque_types, default_types);
    let doc_first = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
    let returns_self = matches!(&method.return_type, TypeRef::Named(name) if name == &opaque_type.name);
    let unwrap_result = method_deserialization_introduces_result(method, true, opaque_types, default_types);
    content.push_str(&template_env::render(
        "elixir_top_level_opaque_method_wrapper.ex.jinja",
        minijinja::context! {
            doc_first => &doc_first,
            func_name => &nif_function,
            def_args => &definition_arguments.join(", "),
            call_args => &call_arguments.join(", "),
            native_mod => native_mod,
            unwrap_result => unwrap_result,
            preserve_result => method.is_async || method.error_type.is_some(),
            returns_self => returns_self,
            has_receiver => method.receiver.is_some(),
            app_module => app_module,
            type_name => &opaque_type.name,
        },
    ));
    content.push('\n');
}

fn method_arguments(
    method: &MethodDef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut definitions = Vec::new();
    let mut calls = Vec::new();
    if method.receiver.is_some() {
        definitions.push("obj".to_string());
        calls.push("obj".to_string());
    }
    let json_params = json_encode_param_indices(&method.params, opaque_types, default_types);
    let tagged_params = AHashMap::new();
    for (index, parameter) in method.params.iter().enumerate() {
        let safe_name = elixir_safe_param_name(&parameter.name);
        definitions.push(safe_name.clone());
        calls.push(nif_arg(index, &safe_name, &json_params, &tagged_params));
    }
    (definitions, calls)
}
