use crate::backends::php::gen_bindings::opaque_files::gen_php_opaque_class_file;
use crate::backends::php::gen_bindings::php_types::{php_phpdoc_type, php_type};
use crate::backends::php::naming::php_autoload_namespace;
use crate::codegen::doc_emission;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::path::PathBuf;

pub(super) fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;

    let escape_phpdoc_line = |s: &str| s.replace("*/", "* /");

    let extension_name = config.php_extension_name();
    let class_name = extension_name.to_pascal_case();

    let mut content = String::new();
    content.push_str(&crate::backends::php::template_env::render(
        "php_file_header.jinja",
        minijinja::Value::default(),
    ));
    content.push_str(&hash::header(CommentStyle::DoubleSlash));
    content.push_str(&crate::backends::php::template_env::render(
        "php_declare_strict_types.jinja",
        minijinja::Value::default(),
    ));
    content.push('\n');

    let namespace = php_autoload_namespace(config);

    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => &namespace },
    ));
    content.push('\n');
    content.push_str(&crate::backends::php::template_env::render(
        "php_facade_class_declaration.jinja",
        context! { class_name => &class_name },
    ));

    let bridge_param_names_pub: ahash::AHashSet<&str> = config
        .trait_bridges
        .iter()
        .filter_map(|b| b.param_name.as_deref())
        .collect();

    let php_exclude_functions: AHashSet<String> = config
        .php
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    let no_arg_constructor_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && t.fields.iter().all(|f| f.optional))
        .map(|t| t.name.clone())
        .collect();

    for func in &api.functions {
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        if php_exclude_functions.contains(&func.name) {
            continue;
        }
        let method_name = func.name.to_lower_camel_case();
        let return_php_type = php_type(&func.return_type);

        let visible_params: Vec<_> = func
            .params
            .iter()
            .filter(|p| !bridge_param_names_pub.contains(p.name.as_str()))
            .collect();

        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_block_start.jinja",
            minijinja::Value::default(),
        ));
        if func.doc.is_empty() {
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_text_line.jinja",
                context! { text => &format!("{}.", method_name) },
            ));
        } else {
            let sections = doc_emission::parse_rustdoc_sections(&func.doc);
            for line in sections.summary.lines() {
                content.push_str("     * ");
                content.push_str(&escape_phpdoc_line(line));
                content.push('\n');
            }
            // Skip Arguments, Returns, Errors, Example — they're emitted as @param/@return/@throws below.
        }
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_empty_line.jinja",
            minijinja::Value::default(),
        ));
        for p in &visible_params {
            let ptype = php_phpdoc_type(&p.ty);
            let nullable_prefix = if p.optional && !ptype.starts_with('?') { "?" } else { "" };
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_param_line.jinja",
                context! {
                    nullable_prefix => nullable_prefix,
                    param_type => &ptype,
                    param_name => &p.name,
                },
            ));
        }
        let return_phpdoc = php_phpdoc_type(&func.return_type);
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_return_line.jinja",
            context! { return_type => &return_phpdoc },
        ));
        if func.error_type.is_some() {
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_throws_line.jinja",
                context! {
                    namespace => namespace.as_str(),
                    class_name => &class_name,
                },
            ));
        }
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_block_end.jinja",
            minijinja::Value::default(),
        ));

        let is_optional_default_constructible_param = |p: &crate::core::ir::ParamDef| -> bool {
            if let TypeRef::Named(name) = &p.ty {
                no_arg_constructor_types.contains(name.as_str())
            } else {
                false
            }
        };

        let mut tail_optional = vec![true; visible_params.len()];
        let mut later_required = false;
        for (idx, p) in visible_params.iter().enumerate().rev() {
            if later_required {
                tail_optional[idx] = false;
            }
            let is_required = !(p.optional || is_optional_default_constructible_param(p));
            if is_required {
                later_required = true;
            }
        }
        let params: Vec<String> = visible_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let ptype = php_type(&p.ty);
                let type_is_nullable = ptype.starts_with('?');
                let is_optional_in_ir = p.optional;
                let can_be_optional =
                    type_is_nullable || is_optional_in_ir || is_optional_default_constructible_param(p);

                let can_emit_default = tail_optional[idx]
                    && (type_is_nullable || is_optional_in_ir || is_optional_default_constructible_param(p));

                if can_be_optional && can_emit_default {
                    if ptype.starts_with('?') {
                        format!("{} ${} = null", ptype, p.name)
                    } else {
                        format!("?{} ${} = null", ptype, p.name)
                    }
                } else if can_be_optional {
                    if ptype.starts_with('?') {
                        format!("{} ${}", ptype, p.name)
                    } else {
                        format!("?{} ${}", ptype, p.name)
                    }
                } else {
                    format!("{} ${}", ptype, p.name)
                }
            })
            .collect();

        if params.is_empty() {
            content.push_str(&format!(
                "    public static function {}(): {} {{\n",
                method_name, return_php_type
            ));
        } else {
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_signature_start.jinja",
                context! { method_name => &method_name },
            ));
            content.push_str(&params.join(", "));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_signature_end.jinja",
                context! { return_type => &return_php_type },
            ));
        }
        let ext_method_name = func.name.to_lower_camel_case();
        let is_void = matches!(&func.return_type, TypeRef::Unit);
        // in the original IR order (as registered via #[php_impl]).
        let call_params = visible_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                if (p.optional || is_optional_default_constructible_param(p))
                    && is_optional_default_constructible_param(p)
                    && tail_optional[idx]
                {
                    if let TypeRef::Named(type_name) = &p.ty {
                        return format!("${} ?? new {}()", p.name, type_name);
                    }
                }
                format!("${}", p.name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let call_expr = format!("\\{namespace}\\{class_name}Api::{ext_method_name}({call_params})");
        if is_void {
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_call_statement.jinja",
                context! { call_expr => &call_expr },
            ));
        } else {
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_call_return.jinja",
                context! { call_expr => &call_expr },
            ));
        }
        content.push_str(&crate::backends::php::template_env::render(
            "php_method_end.jinja",
            minijinja::Value::default(),
        ));
    }

    for bridge_cfg in &config.trait_bridges {
        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            let method_name = register_fn.to_lower_camel_case();
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_start.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_text_line.jinja",
                context! { text => &format!("{}.", method_name) },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_empty_line.jinja",
                minijinja::Value::default(),
            ));
            let interface_name = &bridge_cfg.trait_name;
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_param_line.jinja",
                context! {
                    nullable_prefix => "",
                    param_type => interface_name,
                    param_name => "backend",
                },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_return_line.jinja",
                context! { return_type => "void" },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_end.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_signature_start.jinja",
                context! { method_name => &method_name },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_trait_bridge_api_method.jinja",
                context! { interface_name => interface_name },
            ));
            let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}($backend)");
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_call_statement.jinja",
                context! { call_expr => &call_expr },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_end.jinja",
                minijinja::Value::default(),
            ));
        }
        if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
            let method_name = unregister_fn.to_lower_camel_case();
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_start.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_text_line.jinja",
                context! { text => &format!("{}.", method_name) },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_empty_line.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_param_line.jinja",
                context! {
                    nullable_prefix => "",
                    param_type => "string",
                    param_name => "name",
                },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_return_line.jinja",
                context! { return_type => "void" },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_end.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_signature_start.jinja",
                context! { method_name => &method_name },
            ));
            content.push_str("string $name) : void\n    {\n");
            let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}($name)");
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_call_statement.jinja",
                context! { call_expr => &call_expr },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_end.jinja",
                minijinja::Value::default(),
            ));
        }
        if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
            let method_name = clear_fn.to_lower_camel_case();
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_start.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_text_line.jinja",
                context! { text => &format!("{}.", method_name) },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_empty_line.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_return_line.jinja",
                context! { return_type => "void" },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_block_end.jinja",
                minijinja::Value::default(),
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_signature_start.jinja",
                context! { method_name => &method_name },
            ));
            content.push_str(") : void\n    {\n");
            let call_expr = format!("\\{namespace}\\{class_name}Api::{method_name}()");
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_call_statement.jinja",
                context! { call_expr => &call_expr },
            ));
            content.push_str(&crate::backends::php::template_env::render(
                "php_method_end.jinja",
                minijinja::Value::default(),
            ));
        }
    }

    content.push_str(&crate::backends::php::template_env::render(
        "php_class_end.jinja",
        minijinja::Value::default(),
    ));

    let output_dir = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| s.output.to_string_lossy().to_string())
        .unwrap_or_else(|| "packages/php/src/".to_string());

    let mut files: Vec<GeneratedFile> = Vec::new();
    files.push(GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}.php", class_name)),
        content,
        generated_header: false,
    });

    let mut handler_contract_map: ahash::AHashMap<(String, String, String), String> = ahash::AHashMap::new();
    for service in &api.services {
        for reg in &service.registrations {
            handler_contract_map.insert(
                (service.name.clone(), reg.method.clone(), reg.callback_param.clone()),
                reg.callback_contract.clone(),
            );
        }
    }

    for typ in api.types.iter().filter(|t| t.is_opaque && !t.is_trait) {
        let streaming_adapters: Vec<&crate::core::config::AdapterConfig> = config
            .adapters
            .iter()
            .filter(|a| {
                matches!(a.pattern, crate::core::config::AdapterPattern::Streaming)
                    && a.owner_type.as_deref() == Some(&typ.name)
                    && !a.skip_languages.iter().any(|l| l == "php")
            })
            .collect();
        let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.clone()).collect();
        let opaque_file = gen_php_opaque_class_file(
            typ,
            &namespace,
            &streaming_adapters,
            &streaming_method_names,
            &config.trait_bridges,
            &handler_contract_map,
        );
        files.push(GeneratedFile {
            path: PathBuf::from(&output_dir).join(format!("{}.php", typ.name)),
            content: opaque_file,
            generated_header: false,
        });
    }

    Ok(files)
}
