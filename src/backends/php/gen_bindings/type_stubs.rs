use crate::backends::php::gen_bindings::enum_helpers::{php_enum_case_value, sanitize_php_enum_case};
use crate::backends::php::gen_bindings::php_types::{
    php_phpdoc_type, php_phpdoc_type_fq, php_property_phpdoc, php_type, php_type_fq,
};
use crate::backends::php::gen_bindings::types::is_tagged_data_enum;
use crate::backends::php::naming::php_autoload_namespace;
use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::naming::to_php_name;
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::path::PathBuf;

pub(super) fn generate_type_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;

    let extension_name = config.php_extension_name();
    let class_name = extension_name.to_pascal_case();

    let namespace = php_autoload_namespace(config);
    let php_config = config.php.as_ref();
    let exclude_functions: AHashSet<String> = php_config
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let exclude_types: AHashSet<String> = php_config
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();

    let mut content = String::new();
    content.push_str(&crate::backends::php::template_env::render(
        "php_file_header.jinja",
        minijinja::Value::default(),
    ));
    content.push_str(&hash::header(CommentStyle::DoubleSlash));
    content.push_str("// Type stubs for the native PHP extension — declares classes\n");
    content.push_str("// provided at runtime by the compiled Rust extension (.so/.dll).\n");
    content.push_str("// Include this in phpstan.neon scanFiles for static analysis.\n\n");
    content.push_str(&crate::backends::php::template_env::render(
        "php_declare_strict_types.jinja",
        minijinja::Value::default(),
    ));
    content.push('\n');
    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace_block_begin.jinja",
        context! { namespace => &namespace },
    ));

    content.push_str(&crate::backends::php::template_env::render(
        "php_exception_class_declaration.jinja",
        context! { class_name => &class_name },
    ));
    content.push_str("    public function getErrorCode(): int { throw new \\RuntimeException('Not implemented.'); }\n");
    // These are backed by #[php_method] impls in the generated native extension.
    let has_status_code = api
        .errors
        .iter()
        .any(|e| e.methods.iter().any(|m| m.name == "status_code"));
    let has_is_transient = api
        .errors
        .iter()
        .any(|e| e.methods.iter().any(|m| m.name == "is_transient"));
    let has_error_type = api
        .errors
        .iter()
        .any(|e| e.methods.iter().any(|m| m.name == "error_type"));
    if has_status_code {
        content.push_str(
            "    /** HTTP status code for this error (0 means no associated status). */\n    \
                 public function statusCode(): int { throw new \\RuntimeException('Not implemented.'); }\n",
        );
    }
    if has_is_transient {
        content.push_str(
            "    /** Returns true if the error is transient and a retry may succeed. */\n    \
                 public function isTransient(): bool { throw new \\RuntimeException('Not implemented.'); }\n",
        );
    }
    if has_error_type {
        content.push_str(
            "    /** Machine-readable error category string for matching and logging. */\n    \
                 public function errorType(): string { throw new \\RuntimeException('Not implemented.'); }\n",
        );
    }
    content.push_str("}\n\n");

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
    {
        if typ.is_opaque || typ.fields.is_empty() {
            continue;
        }
        if !typ.doc.is_empty() {
            content.push_str("/**\n");
            let sanitized = sanitize_rust_idioms(&typ.doc, DocTarget::PhpDoc);
            content.push_str(&crate::backends::php::template_env::render(
                "php_phpdoc_lines.jinja",
                context! {
                    doc_lines => sanitized.lines().collect::<Vec<_>>(),
                    indent => "",
                },
            ));
            content.push_str(" */\n");
        }
        content.push_str(&crate::backends::php::template_env::render(
            "php_record_class_stub_declaration.jinja",
            context! { class_name => &typ.name },
        ));

        let mut sorted_fields: Vec<&crate::core::ir::FieldDef> = binding_fields(&typ.fields).collect();
        sorted_fields.sort_by_key(|f| f.optional);

        let params: Vec<String> = sorted_fields
            .iter()
            .map(|f| {
                let ptype = php_type(&f.ty);
                let nullable = if f.optional && !ptype.starts_with('?') {
                    format!("?{ptype}")
                } else {
                    ptype
                };
                let default = if f.optional { " = null" } else { "" };
                let php_name = to_php_name(&f.name);
                let phpdoc_type = php_phpdoc_type(&f.ty);
                let var_type = if f.optional && !phpdoc_type.starts_with('?') {
                    format!("?{phpdoc_type}")
                } else {
                    phpdoc_type
                };
                let phpdoc = php_property_phpdoc(&var_type, &f.doc, "        ");
                format!("{phpdoc}        public readonly {nullable} ${php_name}{default}",)
            })
            .collect();
        content.push_str(&crate::backends::php::template_env::render(
            "php_constructor_method.jinja",
            context! { params => &params.join(",\n") },
        ));

        let non_excluded_methods: Vec<&crate::core::ir::MethodDef> = typ
            .methods
            .iter()
            .filter(|m| !m.binding_excluded && !m.sanitized)
            .collect();
        for method in non_excluded_methods {
            let method_name = method.name.to_lower_camel_case();
            let is_static = method.receiver.is_none();
            let return_type = php_type(&method.return_type);
            // `Option`), emit an `@return array<T>` PHPDoc so PHPStan sees the iterable
            let return_inner = match &method.return_type {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(return_inner, TypeRef::Vec(_) | TypeRef::Map(_, _)) {
                let return_phpdoc = php_phpdoc_type_fq(&method.return_type, &namespace);
                content.push_str(&format!("    /** @return {return_phpdoc} */\n"));
            }
            let first_optional_idx = method.params.iter().position(|p| p.optional);
            let params: Vec<String> = method
                .params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type(&p.ty);
                    if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                        let nullable = if ptype.starts_with('?') { "" } else { "?" };
                        format!("{nullable}{ptype} ${} = null", p.name)
                    } else {
                        format!("{} ${}", ptype, p.name)
                    }
                })
                .collect();
            let static_kw = if is_static { "static " } else { "" };
            let is_void = matches!(&method.return_type, TypeRef::Unit);
            let stub_body = if is_void {
                "{ }".to_string()
            } else {
                "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }".to_string()
            };
            content.push_str(&crate::backends::php::template_env::render(
                "php_stub_method_definition.jinja",
                context! {
                    static_kw => static_kw,
                    method_name => &method_name,
                    params => &params.join(", "),
                    return_type => &return_type,
                    stub_body => &stub_body,
                },
            ));
        }

        content.push_str("}\n\n");
    }

    for enum_def in &api.enums {
        if is_tagged_data_enum(enum_def) {
            if !enum_def.doc.is_empty() {
                content.push_str("/**\n");
                let sanitized = sanitize_rust_idioms(&enum_def.doc, DocTarget::PhpDoc);
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_lines.jinja",
                    context! {
                        doc_lines => sanitized.lines().collect::<Vec<_>>(),
                        indent => "",
                    },
                ));
                content.push_str(" */\n");
            }
            content.push_str(&crate::backends::php::template_env::render(
                "php_record_class_stub_declaration.jinja",
                context! { class_name => &enum_def.name },
            ));
            for ctor in gen_data_enum_variant_constructor_stubs(enum_def) {
                content.push_str(&ctor);
            }
            content.push_str("}\n\n");
        } else {
            content.push_str(&crate::backends::php::template_env::render(
                "php_tagged_enum_declaration.jinja",
                context! { enum_name => &enum_def.name },
            ));
            for variant in &enum_def.variants {
                let case_name = sanitize_php_enum_case(&variant.name);
                content.push_str(&crate::backends::php::template_env::render(
                    "php_enum_variant_stub.jinja",
                    context! {
                        variant_name => case_name,
                        value => &php_enum_case_value(enum_def, variant),
                    },
                ));
            }
            content.push_str("}\n\n");
        }
    }

    // issue on macOS (cdylib builds do not collect `#[php_function]` entries there).
    if api.functions.iter().any(|f| !exclude_functions.contains(&f.name)) || !config.trait_bridges.is_empty() {
        let bridge_param_names_stubs: ahash::AHashSet<&str> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.as_deref())
            .collect();

        content.push_str(&crate::backends::php::template_env::render(
            "php_api_class_declaration.jinja",
            context! { class_name => &class_name },
        ));
        for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
            let return_type = php_type_fq(&func.return_type, &namespace);
            let return_phpdoc = php_phpdoc_type_fq(&func.return_type, &namespace);
            let visible_params: Vec<_> = func
                .params
                .iter()
                .filter(|p| !bridge_param_names_stubs.contains(p.name.as_str()))
                .collect();
            let has_array_params = visible_params
                .iter()
                .any(|p| matches!(&p.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)));
            let has_array_return = matches!(&func.return_type, TypeRef::Vec(_) | TypeRef::Map(_, _))
                || matches!(&func.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_) | TypeRef::Map(_, _)));
            let first_optional_idx = visible_params.iter().position(|p| p.optional);
            if has_array_params || has_array_return {
                content.push_str("    /**\n");
                for (idx, p) in visible_params.iter().enumerate() {
                    let ptype = php_phpdoc_type_fq(&p.ty, &namespace);
                    let nullable_prefix = if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                        "?"
                    } else {
                        ""
                    };
                    content.push_str(&crate::backends::php::template_env::render(
                        "php_phpdoc_static_param.jinja",
                        context! {
                            nullable_prefix => nullable_prefix,
                            ptype => &ptype,
                            param_name => &p.name,
                        },
                    ));
                }
                content.push_str(&crate::backends::php::template_env::render(
                    "php_phpdoc_static_return.jinja",
                    context! { return_phpdoc => &return_phpdoc },
                ));
                content.push_str("     */\n");
            }
            let params: Vec<String> = visible_params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type_fq(&p.ty, &namespace);
                    if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                        let nullable_ptype = if ptype.starts_with('?') {
                            ptype
                        } else {
                            format!("?{ptype}")
                        };
                        format!("{} ${} = null", nullable_ptype, p.name)
                    } else {
                        format!("{} ${}", ptype, p.name)
                    }
                })
                .collect();
            let stub_method_name = func.name.to_lower_camel_case();
            let is_void_stub = return_type == "void";
            let stub_body = if is_void_stub {
                "{ }".to_string()
            } else {
                "{ throw new \\RuntimeException('Not implemented.'); }".to_string()
            };
            content.push_str(&crate::backends::php::template_env::render(
                "php_static_method_stub.jinja",
                context! {
                    method_name => &stub_method_name,
                    params => &params.join(", "),
                    return_type => &return_type,
                    stub_body => &stub_body,
                },
            ));
        }
        for bridge_cfg in &config.trait_bridges {
            if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
                let method_name = register_fn.to_lower_camel_case();
                let interface_name = php_type_fq(&TypeRef::Named(bridge_cfg.trait_name.clone()), &namespace);
                let params = format!("{interface_name} $backend");
                content.push_str(&crate::backends::php::template_env::render(
                    "php_static_method_stub.jinja",
                    context! {
                        method_name => &method_name,
                        params => &params,
                        return_type => "void",
                        stub_body => "{ }",
                    },
                ));
            }
            if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
                let method_name = unregister_fn.to_lower_camel_case();
                content.push_str(&crate::backends::php::template_env::render(
                    "php_static_method_stub.jinja",
                    context! {
                        method_name => &method_name,
                        params => "string $name",
                        return_type => "void",
                        stub_body => "{ }",
                    },
                ));
            }
            if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
                let method_name = clear_fn.to_lower_camel_case();
                content.push_str(&crate::backends::php::template_env::render(
                    "php_static_method_stub.jinja",
                    context! {
                        method_name => &method_name,
                        params => "",
                        return_type => "void",
                        stub_body => "{ }",
                    },
                ));
            }
        }
        content.push_str("}\n\n");
    }

    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace_block_end.jinja",
        minijinja::Value::default(),
    ));

    let output_dir = config
        .php
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| s.output.to_string_lossy().to_string())
        .unwrap_or_else(|| "packages/php/stubs/".to_string());

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}_extension.php", extension_name)),
        content,
        generated_header: false,
    }])
}

/// Emit a static-factory stub for each per-variant constructor the flat PHP enum class exposes.
///
/// The runtime binding exposes these under the camelCase host name (`to_php_name(<snake>)`), so the
/// stub declares the same public name. Each param maps through the stub's [`php_type`] mapper — the
/// same one DTO field/method stubs use — and the return type is the enum class. Optional fields gain
/// a `?` prefix and a `= null` default, mirroring DTO method stubs. `collect_variant_constructors`
/// owns the skip rules (unit / tuple / `binding_excluded` / sanitized-field variants and hand-written
/// method collisions) so the stub and runtime binding stay aligned.
fn gen_data_enum_variant_constructor_stubs(enum_def: &crate::core::ir::EnumDef) -> Vec<String> {
    use crate::codegen::generators::collect_variant_constructors;

    collect_variant_constructors(enum_def)
        .iter()
        .map(|ctor| {
            let first_optional_idx = ctor.params.iter().position(|p| p.optional);
            let params: Vec<String> = ctor
                .params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = php_type(&p.ty);
                    if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                        let nullable = if ptype.starts_with('?') { "" } else { "?" };
                        format!("{nullable}{ptype} ${} = null", to_php_name(&p.name))
                    } else {
                        format!("{} ${}", ptype, to_php_name(&p.name))
                    }
                })
                .collect();
            // Emit `@param array<...>` PHPDoc for array/map parameters so PHPStan (level max)
            let phpdoc_params: Vec<String> = ctor
                .params
                .iter()
                .filter(|p| matches!(&p.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)))
                .map(|p| format!("@param {} ${}", php_phpdoc_type(&p.ty), to_php_name(&p.name)))
                .collect();
            let doc_block = match phpdoc_params.len() {
                0 => String::new(),
                1 => format!("    /** {} */\n", phpdoc_params[0]),
                _ => {
                    let mut block = String::from("    /**\n");
                    for line in &phpdoc_params {
                        block.push_str(&format!("     * {line}\n"));
                    }
                    block.push_str("     */\n");
                    block
                }
            };
            let method = crate::backends::php::template_env::render(
                "php_static_method_stub.jinja",
                context! {
                    method_name => to_php_name(&ctor.snake_name),
                    params => &params.join(", "),
                    return_type => &enum_def.name,
                    stub_body => "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }",
                },
            );
            format!("{doc_block}{method}")
        })
        .collect()
}

#[cfg(test)]
mod type_stubs_tests;
