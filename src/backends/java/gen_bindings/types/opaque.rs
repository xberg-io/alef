use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_class_name;
use crate::core::config::{AdapterConfig, AdapterPattern};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToSnakeCase};

use crate::backends::java::gen_bindings::helpers::{emit_javadoc, safe_java_method_name};
use crate::backends::java::gen_bindings::marshal::{is_ffi_string_return, java_ffi_return_cast, java_ffi_return_expr};

mod extended;
use extended::{gen_static_factory_method, gen_streaming_helpers, gen_streaming_method, param_needs_null_check};
mod instance;
#[cfg(test)]
mod optional_bytes_result_tests;
use instance::gen_instance_method;

struct OpaqueClassMethods<'a> {
    streaming_adapters: Vec<&'a AdapterConfig>,
    instance_methods: Vec<&'a MethodDef>,
    static_factory_methods: Vec<&'a MethodDef>,
}

fn select_opaque_class_methods<'a>(typ: &'a TypeDef, adapters: &'a [AdapterConfig]) -> OpaqueClassMethods<'a> {
    let streaming_adapters: Vec<_> = adapters
        .iter()
        .filter(|adapter| {
            matches!(adapter.pattern, AdapterPattern::Streaming)
                && adapter.owner_type.as_deref() == Some(typ.name.as_str())
                && adapter.item_type.is_some()
                && adapter.params.first().is_some_and(|param| !param.ty.is_empty())
                && !adapter.skip_languages.iter().any(|language| language == "java")
        })
        .collect();
    let streaming_names: AHashSet<_> = streaming_adapters
        .iter()
        .map(|adapter| adapter.name.to_snake_case())
        .collect();
    let instance_methods = typ
        .methods
        .iter()
        .filter(|method| !method.is_static && !streaming_names.contains(&method.name.to_snake_case()))
        .collect();
    let static_factory_methods = typ
        .methods
        .iter()
        .filter(|method| method.receiver.is_none())
        .filter(|method| !matches!(method.name.as_str(), "default" | "to_json" | "from_json"))
        .filter(|method| !method.returns_ref_to_owner(&typ.name))
        .collect();
    OpaqueClassMethods {
        streaming_adapters,
        instance_methods,
        static_factory_methods,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_opaque_class_methods(
    body: &mut String,
    methods: &OpaqueClassMethods<'_>,
    typ: &TypeDef,
    prefix: &str,
    type_snake: &str,
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) {
    for adapter in &methods.streaming_adapters {
        gen_streaming_method(body, adapter, prefix, type_snake, main_class, to_json_type_names);
    }
    for method in &methods.instance_methods {
        gen_instance_method(
            body,
            method,
            prefix,
            type_snake,
            main_class,
            enum_names,
            opaque_type_names,
            to_json_type_names,
        );
    }
    for method in &methods.static_factory_methods {
        gen_static_factory_method(
            body,
            method,
            &typ.name,
            prefix,
            type_snake,
            main_class,
            enum_names,
            opaque_type_names,
        );
    }
}

fn opaque_class_imports(body: &str, needs_helpers: bool, has_static_factories: bool) -> Vec<&'static str> {
    let mut imports = vec!["java.lang.foreign.MemorySegment"];
    if needs_helpers || has_static_factories {
        for (needle, import) in [
            ("Arena", "java.lang.foreign.Arena"),
            ("ValueLayout", "java.lang.foreign.ValueLayout"),
            ("ObjectMapper", "com.fasterxml.jackson.databind.ObjectMapper"),
            ("JsonNode", "com.fasterxml.jackson.databind.JsonNode"),
        ] {
            if body.contains(needle) {
                imports.push(import);
            }
        }
    }
    for (needle, import) in [
        ("List<", "java.util.List"),
        ("Optional<", "java.util.Optional"),
        ("Map<", "java.util.Map"),
    ] {
        if body.contains(needle) {
            imports.push(import);
        }
    }
    imports
}

#[allow(clippy::too_many_arguments)]
fn render_opaque_class_body(
    typ: &TypeDef,
    prefix: &str,
    adapters: &[AdapterConfig],
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) -> (String, bool, bool) {
    let type_snake = typ.name.to_snake_case();
    let methods = select_opaque_class_methods(typ, adapters);
    let has_static_factories = !methods.static_factory_methods.is_empty();
    let needs_helpers = !methods.streaming_adapters.is_empty() || !methods.instance_methods.is_empty();
    let mut body = String::new();
    emit_javadoc(&mut body, &typ.doc, "");
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_header.jinja",
        minijinja::context! { class_name => typ.name },
    ));
    emit_opaque_class_methods(
        &mut body,
        &methods,
        typ,
        prefix,
        &type_snake,
        main_class,
        enum_names,
        opaque_type_names,
        to_json_type_names,
    );
    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_close.jinja",
        minijinja::context! { free_handle, class_name => typ.name },
    ));
    if needs_helpers {
        gen_streaming_helpers(&mut body, prefix, main_class);
    }
    body.push_str("}\n");
    (body, needs_helpers, has_static_factories)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_opaque_handle_class(
    package: &str,
    typ: &TypeDef,
    prefix: &str,
    adapters: &[AdapterConfig],
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let (body, needs_helpers, has_static_factories) = render_opaque_class_body(
        typ,
        prefix,
        adapters,
        main_class,
        enum_names,
        opaque_type_names,
        to_json_type_names,
    );
    let imports = opaque_class_imports(&body, needs_helpers, has_static_factories);
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&body);
    out
}

fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn is_opaque_param(param: &crate::core::ir::ParamDef, opaque_type_names: &AHashSet<String>) -> bool {
    named_type_name(&param.ty).is_some_and(|name| opaque_type_names.contains(name))
}

fn java_opaque_method_param_supported(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Json
        | TypeRef::Path
        | TypeRef::Named(_)
        | TypeRef::Primitive(_)
        | TypeRef::Duration => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Json | TypeRef::Path | TypeRef::Named(_)
        ),
        _ => false,
    }
}

fn emit_java_resource_declarations(
    out: &mut String,
    method: &MethodDef,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
) {
    for param in &method.params {
        let cname = format!("c{}", to_class_name(&param.name));
        if is_opaque_param(param, opaque_type_names) {
            let type_name = named_type_name(&param.ty).expect("opaque named parameter");
            out.push_str(&crate::backends::java::template_env::render(
                "opaque_resource_declaration.jinja",
                minijinja::context! { type_name, c_name => cname },
            ));
        } else if named_type_name(&param.ty).is_some_and(|name| !enum_names.contains(name)) {
            out.push_str(&crate::backends::java::template_env::render(
                "opaque_resource_declaration.jinja",
                minijinja::context! { type_name => "", c_name => cname },
            ));
        }
    }
}

fn render_java_resource_cleanup(
    method: &MethodDef,
    prefix_upper: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    indent: &str,
) -> String {
    let mut cleanup_actions = String::new();
    for param in method.params.iter().rev() {
        let cname = format!("c{}", to_class_name(&param.name));
        if is_opaque_param(param, opaque_type_names) {
            cleanup_actions.push_str(&crate::backends::java::template_env::render(
                "opaque_cleanup_lease.jinja",
                minijinja::context! { indent, c_name => cname },
            ));
        } else if let Some(type_name) = named_type_name(&param.ty).filter(|name| !enum_names.contains(*name)) {
            let free_handle = format!(
                "NativeLib.{prefix_upper}_{}_FREE",
                type_name.to_snake_case().to_uppercase()
            );
            cleanup_actions.push_str(&crate::backends::java::template_env::render(
                "opaque_cleanup_handle.jinja",
                minijinja::context! { indent, c_name => cname, free_handle },
            ));
        }
    }
    if cleanup_actions.is_empty() {
        return cleanup_actions;
    }
    format!(
        "{indent}var cleanupFailures = new CleanupFailures();\n{cleanup_actions}{indent}cleanupFailures.throwIfAny(operationFailure);\n"
    )
}
