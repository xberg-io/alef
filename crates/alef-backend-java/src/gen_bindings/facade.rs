use crate::type_map::{java_boxed_type, java_return_type, java_type};
use alef_codegen::naming::to_java_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, PrimitiveType, TypeRef};
use std::collections::HashSet;

use super::helpers::{emit_javadoc, is_bridge_param_java};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_facade_class(
    api: &ApiSurface,
    package: &str,
    public_class: &str,
    raw_class: &str,
    _prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    _has_visitor_pattern: bool,
) -> String {
    // Build per-function context objects for the facade_class template.
    let functions: Vec<minijinja::Value> = api
        .functions
        .iter()
        .map(|func| {
            let params: Vec<String> = func
                .params
                .iter()
                .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    let ptype = if p.optional {
                        java_boxed_type(&p.ty)
                    } else {
                        java_type(&p.ty)
                    };
                    format!("final {} {}", ptype, to_java_name(&p.name))
                })
                .collect();

            let return_type = java_return_type(&func.return_type).to_string();
            let is_void = matches!(func.return_type, TypeRef::Unit);
            let is_optional = matches!(func.return_type, TypeRef::Optional(_));
            let java_name = to_java_name(&func.name);

            let mut javadoc = String::new();
            emit_javadoc(&mut javadoc, &func.doc, "    ");

            let null_checks: Vec<String> = func
                .params
                .iter()
                .filter(|p| !p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    let pname = to_java_name(&p.name);
                    format!("java.util.Objects.requireNonNull({pname}, \"{pname} must not be null\");")
                })
                .collect();

            // Delegate to raw FFI class — bridge params stripped from raw class signature.
            let call_args: Vec<String> = func
                .params
                .iter()
                .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| to_java_name(&p.name))
                .collect();

            let has_optional_overload = func
                .params
                .iter()
                .any(|p| p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases));

            let required_params: Vec<String> = if has_optional_overload {
                func.params
                    .iter()
                    .filter(|p| !p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                    .map(|p| format!("final {} {}", java_type(&p.ty), to_java_name(&p.name)))
                    .collect()
            } else {
                vec![]
            };

            // Build call to raw class: bridge params excluded; optional params use defaults.
            let full_args: Vec<String> = if has_optional_overload {
                func.params
                    .iter()
                    .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                    .map(|p| {
                        if p.optional {
                            match &p.ty {
                                TypeRef::Primitive(prim) => match prim {
                                    PrimitiveType::I8
                                    | PrimitiveType::I16
                                    | PrimitiveType::I32
                                    | PrimitiveType::U8
                                    | PrimitiveType::U16
                                    | PrimitiveType::U32 => "0".to_string(),
                                    PrimitiveType::I64
                                    | PrimitiveType::Isize
                                    | PrimitiveType::U64
                                    | PrimitiveType::Usize => "0L".to_string(),
                                    PrimitiveType::F32 => "0.0f".to_string(),
                                    PrimitiveType::F64 => "0.0".to_string(),
                                    PrimitiveType::Bool => "false".to_string(),
                                },
                                _ => "null".to_string(),
                            }
                        } else {
                            to_java_name(&p.name)
                        }
                    })
                    .collect()
            } else {
                vec![]
            };

            minijinja::context! {
                javadoc => javadoc,
                return_type => return_type,
                is_void => is_void,
                is_optional => is_optional,
                java_name => java_name,
                params => params,
                null_checks => null_checks,
                call_args => call_args,
                has_optional_overload => has_optional_overload,
                required_params => required_params,
                full_args => full_args,
            }
        })
        .collect();

    let class_body = crate::template_env::render(
        "facade_class.jinja",
        minijinja::context! {
            class_name => public_class,
            raw_class => raw_class,
            functions => functions,
        },
    );

    let header = hash::header(CommentStyle::DoubleSlash);
    let has_list = class_body.contains("List<");
    let has_map = class_body.contains("Map<");
    let has_optional = class_body.contains("Optional<");

    crate::template_env::render(
        "facade_file.jinja",
        minijinja::context! {
            header => header,
            package => package,
            has_list => has_list,
            has_map => has_map,
            has_optional => has_optional,
            body => class_body,
        },
    )
}
