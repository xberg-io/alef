use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_java_name;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use std::collections::HashSet;

use super::helpers::{emit_javadoc_with_throws, is_bridge_param_java, render_nullable_type};

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
    config: &crate::core::config::ResolvedCrateConfig,
) -> String {
    use crate::core::config::extras::AdapterPattern;

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
                    let annotated = render_nullable_type(&ptype, p.optional);
                    format!("final {annotated} {}", to_java_name(&p.name))
                })
                .collect();

            let capsule_host_type = if let TypeRef::Named(name) = &func.return_type {
                config
                    .java
                    .as_ref()
                    .and_then(|java| java.capsule_types.get(name.as_str()))
                    .map(|capsule| {
                        if capsule.host_type.is_empty() {
                            name.clone()
                        } else {
                            capsule.host_type.clone()
                        }
                    })
            } else {
                None
            };

            let return_type = if let Some(host_type) = capsule_host_type {
                host_type
            } else if let TypeRef::Optional(inner) = &func.return_type {
                let inner_type = java_boxed_type(inner);
                render_nullable_type(&inner_type, true)
            } else {
                java_return_type(&func.return_type).to_string()
            };
            let is_void = matches!(func.return_type, TypeRef::Unit);
            let needs_optional_unwrap =
                matches!(&func.return_type, TypeRef::Optional(inner) if !matches!(inner.as_ref(), TypeRef::Bytes));
            let is_optional = matches!(func.return_type, TypeRef::Optional(_));
            let java_name = to_java_name(&func.name);

            let mut javadoc = String::new();
            let exception_class = format!("{raw_class}Exception");
            emit_javadoc_with_throws(&mut javadoc, &func.doc, "    ", &exception_class);

            let null_checks: Vec<String> = func
                .params
                .iter()
                .filter(|p| !p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    let pname = to_java_name(&p.name);
                    format!("java.util.Objects.requireNonNull({pname}, \"{pname} must not be null\");")
                })
                .collect();

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
                    .map(|p| {
                        let ptype = java_type(&p.ty);
                        format!("final {} {}", ptype, to_java_name(&p.name))
                    })
                    .collect()
            } else {
                vec![]
            };

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
                needs_optional_unwrap => needs_optional_unwrap,
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

    let api_function_names: HashSet<String> = api.functions.iter().map(|f| to_java_name(&f.name)).collect();

    let mut trait_bridge_wrappers = String::new();
    for bridge in &config.trait_bridges {
        if bridge
            .exclude_languages
            .contains(&crate::core::config::Language::Java.to_string())
        {
            continue;
        }
        let trait_pascal = bridge.trait_name.as_str().to_string();
        let bridge_class = format!("{}Bridge", trait_pascal);

        if let Some(register_fn) = &bridge.register_fn {
            let java_register_fn = to_java_name(register_fn);
            if !api_function_names.contains(&java_register_fn) {
                let trait_ident = format!("I{}", trait_pascal);
                let method_code = format!(
                    "    public static void {}(final {} impl) throws {}Exception {{\n        try {{\n            {}.{}(impl);\n        }} catch (Exception e) {{\n            throw new {}Exception(e.getMessage(), e);\n        }}\n    }}\n\n",
                    java_register_fn, trait_ident, raw_class, bridge_class, java_register_fn, raw_class
                );
                trait_bridge_wrappers.push_str(&method_code);
            }
        }

        if let Some(unregister_fn) = &bridge.unregister_fn {
            let java_unregister_fn = to_java_name(unregister_fn);
            if !api_function_names.contains(&java_unregister_fn) {
                let method_code = format!(
                    "    public static void {}(final String name) throws {}Exception {{\n        try {{\n            {}.{}(name);\n        }} catch (Exception e) {{\n            throw new {}Exception(e.getMessage(), e);\n        }}\n    }}\n\n",
                    java_unregister_fn, raw_class, bridge_class, java_unregister_fn, raw_class
                );
                trait_bridge_wrappers.push_str(&method_code);
            }
        }

        if let Some(clear_fn) = &bridge.clear_fn {
            let java_clear_fn = to_java_name(clear_fn);
            if !api_function_names.contains(&java_clear_fn) {
                let method_code = format!(
                    "    public static void {}() throws {}Exception {{\n        try {{\n            {}.{}();\n        }} catch (Exception e) {{\n            throw new {}Exception(e.getMessage(), e);\n        }}\n    }}\n\n",
                    java_clear_fn, raw_class, bridge_class, java_clear_fn, raw_class
                );
                trait_bridge_wrappers.push_str(&method_code);
            }
        }
    }

    let mut streaming_wrappers = String::new();
    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        if adapter.owner_type.is_none() || adapter.item_type.is_none() {
            continue;
        }

        let adapter_name = &adapter.name;
        let java_name = to_java_name(adapter_name);
        let owner_type = adapter.owner_type.as_deref().unwrap();
        let item_type = adapter.item_type.as_deref().unwrap();

        let short_item_type = item_type.rsplit("::").next().unwrap_or(item_type);

        let param_parts: Vec<String> = adapter
            .params
            .iter()
            .map(|p| {
                let java_type = match p.ty.as_str() {
                    "String" | "&str" | "&'static str" => "String",
                    "Vec<String>" => "List<String>",
                    "()" => "Void",
                    other => other.rsplit("::").next().unwrap_or(other),
                };
                let annotation = if p.optional { "@Nullable " } else { "" };
                format!("final {}{} {}", annotation, java_type, to_java_name(&p.name))
            })
            .collect();

        let method_call = if param_parts.is_empty() {
            format!(
                "    public static java.util.stream.Stream<{short_item_type}> {java_name}(final {owner_type} engine) throws {raw_class}Exception {{\n        return engine.{java_name}();\n    }}\n"
            )
        } else {
            let param_str = format!("final {owner_type} engine, {}", param_parts.join(", "));
            let call_args = adapter
                .params
                .iter()
                .map(|p| to_java_name(&p.name))
                .collect::<Vec<_>>()
                .join(", ");

            format!(
                "    public static java.util.stream.Stream<{short_item_type}> {java_name}({param_str}) throws {raw_class}Exception {{\n        return engine.{java_name}({call_args});\n    }}\n"
            )
        };

        streaming_wrappers.push_str(&method_call);
        streaming_wrappers.push('\n');
    }

    let class_body = crate::backends::java::template_env::render(
        "facade_class.jinja",
        minijinja::context! {
            class_name => public_class,
            raw_class => raw_class,
            functions => functions,
            trait_bridge_wrappers => trait_bridge_wrappers,
            streaming_wrappers => streaming_wrappers,
        },
    );

    let header = hash::header(CommentStyle::DoubleSlash);
    let has_list = class_body.contains("List<");
    let has_map = class_body.contains("Map<");
    let has_optional = class_body.contains("Optional<");
    let has_nullable = class_body.contains("@Nullable");

    crate::backends::java::template_env::render(
        "facade_file.jinja",
        minijinja::context! {
            header => header,
            package => package,
            has_list => has_list,
            has_map => has_map,
            has_optional => has_optional,
            has_nullable => has_nullable,
            body => class_body,
        },
    )
}
