use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, to_lower_camel, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{HostCapsuleTypeConfig, ResolvedCrateConfig};
use crate::core::ir::{FunctionDef, TypeRef};

use super::super::assemble_kt_content;
use super::capsule_functions::{emit_capsule_function_wrapper, get_capsule_config};
use super::facade_types::{
    bridge_arg, facade_param, facade_return_type, is_dto_named, is_generic_container, render_kotlin_type,
};
use super::helpers::unwrap_optional;

struct FacadeContext<'a> {
    config: &'a ResolvedCrateConfig,
    bridge_name: &'a str,
    opaque_types: &'a HashSet<String>,
    async_method_names: HashSet<String>,
    capsule_types: HashMap<String, HostCapsuleTypeConfig>,
    needs_jackson: bool,
}

struct FunctionProjection {
    method_name: String,
    params: String,
    return_type: String,
    bridge_call: String,
    call_args: String,
    method_already_async: bool,
    emit_suspend_wrapper: bool,
}

pub(super) struct FacadeParams<'a> {
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) kotlin_source_dir: &'a Path,
    pub(super) package: &'a str,
    pub(super) module_name: &'a str,
    pub(super) bridge_name: &'a str,
    pub(super) opaque_types: &'a HashSet<String>,
    pub(super) functions: &'a [&'a FunctionDef],
}

pub(super) fn emit_facade(params: FacadeParams, files: &mut Vec<GeneratedFile>) {
    let FacadeParams {
        config,
        kotlin_source_dir,
        package,
        module_name,
        bridge_name,
        opaque_types,
        functions,
    } = params;
    let context = facade_context(config, bridge_name, opaque_types, functions);
    let imports = facade_imports(functions, &context);
    let mut body = template_env::render(
        "module_object_header.jinja",
        minijinja::context! { module_name => module_name },
    );
    if context.needs_jackson {
        append_jackson_configuration(&mut body);
    }
    for function in functions {
        emit_function(&mut body, function, &context);
    }
    body.push_str("}\n");
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{module_name}.kt")),
        content: assemble_kt_content(package, &imports, &body),
        generated_header: false,
    });
}

fn facade_context<'a>(
    config: &'a ResolvedCrateConfig,
    bridge_name: &'a str,
    opaque_types: &'a HashSet<String>,
    functions: &[&FunctionDef],
) -> FacadeContext<'a> {
    FacadeContext {
        config,
        bridge_name,
        opaque_types,
        async_method_names: functions
            .iter()
            .filter(|function| function.name.ends_with("_async"))
            .map(|function| to_lower_camel(&function.name))
            .collect(),
        capsule_types: config
            .kotlin_android
            .as_ref()
            .map(|android| android.capsule_types.clone())
            .unwrap_or_default(),
        needs_jackson: functions
            .iter()
            .any(|function| function_needs_jackson(function, opaque_types)),
    }
}

fn function_needs_jackson(function: &FunctionDef, opaque_types: &HashSet<String>) -> bool {
    is_dto_named(&function.return_type, opaque_types)
        || is_generic_container(&function.return_type)
        || function.params.iter().any(|param| {
            let ty = unwrap_optional(&param.ty);
            is_dto_named(ty, opaque_types) || is_generic_container(ty)
        })
}

fn facade_imports(functions: &[&FunctionDef], context: &FacadeContext<'_>) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    if context.needs_jackson {
        for import in jackson_imports() {
            imports.insert(import.to_string());
        }
    }
    if functions
        .iter()
        .any(|function| is_generic_container(&function.return_type))
    {
        imports.insert("import com.fasterxml.jackson.core.type.TypeReference".to_string());
    }
    imports
}

fn jackson_imports() -> [&'static str; 9] {
    [
        "import com.fasterxml.jackson.annotation.JsonInclude",
        "import com.fasterxml.jackson.databind.DeserializationFeature",
        "import com.fasterxml.jackson.databind.PropertyNamingStrategies",
        "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module",
        "import com.fasterxml.jackson.module.kotlin.KotlinFeature",
        "import com.fasterxml.jackson.module.kotlin.KotlinModule",
        "import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper",
        "import kotlinx.coroutines.Dispatchers",
        "import kotlinx.coroutines.withContext",
    ]
}

fn append_jackson_configuration(body: &mut String) {
    body.push_str(&template_env::render(
        "android_facade_jackson_config.jinja",
        minijinja::context! {
            duration_millis_module => crate::backends::kotlin::duration_millis_jackson_module(8),
        },
    ));
}

fn emit_function(body: &mut String, function: &FunctionDef, context: &FacadeContext<'_>) {
    if let Some(capsule) = get_capsule_config(function, &context.capsule_types) {
        emit_kdoc_pub(body, &function.doc, "    ");
        emit_capsule_function_wrapper(body, function, context.bridge_name, capsule);
        body.push('\n');
        return;
    }
    emit_kdoc_pub(body, &function.doc, "    ");
    let projection = project_function(function, context);
    if emit_optional_opaque_wrapper(body, function, context.opaque_types, &projection) {
        return;
    }
    if is_dto_named(&function.return_type, context.opaque_types) {
        emit_dto_method(body, function, &projection);
    } else if is_generic_container(&function.return_type) {
        emit_generic_method(body, function, context.opaque_types, &projection);
    } else if direct_opaque_type(function, context.opaque_types).is_some() {
        emit_opaque_method(body, function, &projection);
    } else {
        emit_expression_method(body, &projection, projection.bridge_call.clone());
    }
}

fn project_function(function: &FunctionDef, context: &FacadeContext<'_>) -> FunctionProjection {
    let method_name = to_lower_camel(&function.name);
    let params = function
        .params
        .iter()
        .map(|param| facade_param(param, context.opaque_types))
        .collect::<Vec<_>>()
        .join(", ");
    let bridge_args = function
        .params
        .iter()
        .map(|param| bridge_arg(param, context.opaque_types))
        .collect::<Vec<_>>()
        .join(", ");
    let native_name = format!("native{}", to_pascal_case(&function.name));
    let method_already_async = method_name.ends_with("Async");
    FunctionProjection {
        return_type: facade_return_type(&function.return_type, context.opaque_types),
        bridge_call: format!("{}.{}({bridge_args})", context.bridge_name, native_name),
        call_args: function
            .params
            .iter()
            .map(|param| to_lower_camel(&param.name))
            .collect::<Vec<_>>()
            .join(", "),
        emit_suspend_wrapper: should_emit_suspend_wrapper(&method_name, method_already_async, context),
        method_name,
        params,
        method_already_async,
    }
}

fn should_emit_suspend_wrapper(method_name: &str, method_already_async: bool, context: &FacadeContext<'_>) -> bool {
    let generate = context
        .config
        .generate_overrides
        .get("kotlin_android")
        .unwrap_or(&context.config.generate);
    let wrapper_name = format!("{method_name}Async");
    let wrapper_would_collide = !method_already_async && context.async_method_names.contains(&wrapper_name);
    method_already_async || (generate.async_wrappers && !wrapper_would_collide)
}

fn emit_optional_opaque_wrapper(
    body: &mut String,
    function: &FunctionDef,
    opaque_types: &HashSet<String>,
    projection: &FunctionProjection,
) -> bool {
    let TypeRef::Optional(inner) = &function.return_type else {
        return false;
    };
    let TypeRef::Named(type_name) = inner.as_ref() else {
        return false;
    };
    if !opaque_types.contains(type_name) {
        return false;
    }
    let expression = format!("{}.takeIf {{ it != 0L }}?.let(::{type_name})", projection.bridge_call);
    body.push_str(&template_env::render(
        "android_facade_expr_method.jinja",
        minijinja::context! {
            method_name => projection.method_name,
            params => projection.params,
            return_type => format!("{type_name}?"),
            expression => expression,
        },
    ));
    true
}

fn direct_opaque_type<'a>(function: &'a FunctionDef, opaque_types: &HashSet<String>) -> Option<&'a str> {
    match &function.return_type {
        TypeRef::Named(type_name) if opaque_types.contains(type_name) => Some(type_name),
        _ => None,
    }
}

fn emit_dto_method(body: &mut String, function: &FunctionDef, projection: &FunctionProjection) {
    let TypeRef::Named(return_class) = &function.return_type else {
        unreachable!()
    };
    if !projection.method_already_async {
        body.push_str(&template_env::render(
            "android_facade_dto_method.jinja",
            minijinja::context! {
                method_name => projection.method_name,
                params => projection.params,
                return_type => projection.return_type,
                bridge_call => projection.bridge_call,
                return_class => return_class,
            },
        ));
    }
    if projection.emit_suspend_wrapper {
        emit_kdoc_pub(body, &function.doc, "    ");
        emit_dto_suspend_method(body, projection, return_class);
    }
}

fn emit_dto_suspend_method(body: &mut String, projection: &FunctionProjection, return_class: &str) {
    if !projection.method_already_async {
        emit_async_delegate(body, projection);
        return;
    }
    body.push_str(&template_env::render(
        "android_facade_dto_async_impl.jinja",
        minijinja::context! {
            method_name => projection.method_name,
            params => projection.params,
            return_type => projection.return_type,
            bridge_call => projection.bridge_call,
            return_class => return_class,
        },
    ));
}

fn emit_generic_method(
    body: &mut String,
    function: &FunctionDef,
    opaque_types: &HashSet<String>,
    projection: &FunctionProjection,
) {
    let type_reference = render_kotlin_type(&function.return_type, opaque_types);
    if !projection.method_already_async {
        body.push_str(&template_env::render(
            "android_facade_generic_method.jinja",
            minijinja::context! {
                method_name => projection.method_name,
                params => projection.params,
                return_type => projection.return_type,
                bridge_call => projection.bridge_call,
                type_ref_body => type_reference,
            },
        ));
    }
    if projection.emit_suspend_wrapper {
        emit_kdoc_pub(body, &function.doc, "    ");
        emit_generic_suspend_method(body, projection, &type_reference);
    }
}

fn emit_generic_suspend_method(body: &mut String, projection: &FunctionProjection, type_reference: &str) {
    if !projection.method_already_async {
        emit_async_delegate(body, projection);
        return;
    }
    body.push_str(&template_env::render(
        "android_facade_generic_async_impl.jinja",
        minijinja::context! {
            method_name => projection.method_name,
            params => projection.params,
            return_type => projection.return_type,
            bridge_call => projection.bridge_call,
            type_reference => type_reference,
        },
    ));
}

fn emit_async_delegate(body: &mut String, projection: &FunctionProjection) {
    body.push_str(&template_env::render(
        "android_facade_async_method.jinja",
        minijinja::context! {
            method_name => projection.method_name,
            params => projection.params,
            return_type => projection.return_type,
            args => projection.call_args,
        },
    ));
}

fn emit_opaque_method(body: &mut String, function: &FunctionDef, projection: &FunctionProjection) {
    let TypeRef::Named(type_name) = &function.return_type else {
        unreachable!()
    };
    emit_expression_method(body, projection, format!("{type_name}({})", projection.bridge_call));
}

fn emit_expression_method(body: &mut String, projection: &FunctionProjection, expression: String) {
    body.push_str(&template_env::render(
        "android_facade_expr_method.jinja",
        minijinja::context! {
            method_name => projection.method_name,
            params => projection.params,
            return_type => projection.return_type,
            expression => expression,
        },
    ));
}
