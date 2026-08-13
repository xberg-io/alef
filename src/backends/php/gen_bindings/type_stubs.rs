use crate::backends::php::gen_bindings::enum_helpers::{php_enum_case_value, sanitize_php_enum_case};
use crate::backends::php::gen_bindings::php_types::{
    php_phpdoc_type, php_phpdoc_type_fq, php_property_phpdoc, php_type, php_type_fq,
};
use crate::backends::php::gen_bindings::types::{
    is_php_prop_scalar, is_tagged_data_enum, is_untagged_data_enum, php_field_can_be_constructor_param,
    ty_is_or_wraps_json, ty_references_untagged_data_enum,
};
use crate::backends::php::naming::php_autoload_namespace;
use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::naming::to_php_name;
use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, FieldDef, TypeRef};
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

    // Derived exactly as `rust_bindings.rs` derives them for the real extension, so the stub's
    // constructor-param filter (`php_field_can_be_constructor_param`) sees the same enum/opaque
    // universe the extension's own constructor filter sees.
    let enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| !is_tagged_data_enum(e) && !is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    let untagged_data_enum_names: AHashSet<String> = api
        .enums
        .iter()
        .filter(|e| is_untagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();
    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

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

        // The real extension's `#[php(constructor)]` only accepts fields that pass
        // `php_field_can_be_constructor_param` (a superset of `is_php_prop_scalar`); a field
        // that fails it is excluded from the constructor entirely. Every OTHER field on the
        // struct (whether or not it made the constructor) still gets a getter, per the real
        // extension's `for field in binding_fields(&typ.fields)` getter loop in structs.rs —
        // so a field can be readable via `get<Field>()` while being unreachable from `new(...)`.
        let is_constructor_param =
            |f: &FieldDef| f.cfg.is_none() && php_field_can_be_constructor_param(&f.ty, &enum_names, &opaque_types);

        if !typ.has_serde {
            for excluded in binding_fields(&typ.fields).filter(|f| !is_constructor_param(f)) {
                tracing::warn!(
                    "php backend stub: {}.{} cannot be represented as a #[php(constructor)] parameter \
                     (its mapped type has no ext-php-rs constructor-param support); the PHPStan stub \
                     omits it from the constructor and exposes it only via get_{}()",
                    typ.name,
                    excluded.name,
                    excluded.name,
                );
            }
        }

        let mut ctor_fields: Vec<&FieldDef> = binding_fields(&typ.fields)
            .filter(|f| is_constructor_param(f))
            .collect();
        ctor_fields.sort_by_key(|f| f.optional);

        let params: Vec<String> = ctor_fields
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
                if !is_php_prop_scalar(&f.ty, &enum_names) {
                    // Constructor-representable but not a real PHP property (no `#[php(prop)]`
                    // on the extension's struct field) — a plain, non-promoted parameter.
                    return format!("        {nullable} ${php_name}{default}");
                }
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

        // The real extension additionally emits a `#[php(name = "from_json")]` static
        // constructor (structs.rs's `use_from_json` gate) whenever the struct has a field the
        // `#[php(constructor)]` can't represent (a named/complex param), a `#[derive(Default)]`,
        // or a field with a default. That is the ONLY way to build such a type's nested/complex
        // fields from PHP — ext-php-rs exposes them read-only — so the stub must declare it or
        // the method is invisible to editors and PHPStan. Mirror the exact same gate here.
        if struct_needs_from_json_stub(typ, &enum_names) {
            content.push_str(
                "    /**\n     * Construct from a JSON string — the only way to build this \
                 type's nested/complex fields from PHP, since ext-php-rs exposes them read-only.\n     */\n",
            );
            content.push_str(&crate::backends::php::template_env::render(
                "php_stub_method_definition.jinja",
                context! {
                    static_kw => "static ",
                    method_name => "from_json",
                    params => "string $json",
                    return_type => "self",
                    stub_body => "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }",
                },
            ));
        }

        // A getter is declared for EVERY binding field, mirroring the extension exactly: the
        // generator's own getter loop (gen_bindings/types/structs.rs, `for field in
        // binding_fields(&typ.fields)`) is unconditional and has no prop filter, so the
        // extension really does expose `get<Field>()` for a prop-scalar field too.
        //
        // Declaring only the non-prop subset was tried and reverted. It looked reasonable --
        // a prop-scalar field is already readable as a promoted `public readonly` property,
        // so its getter is redundant to a human -- but a stub that omits a method the
        // extension has is precisely the drift this file exists to prevent: PHPStan then
        // reports a false "undefined method" on a call that works at runtime. Fidelity to the
        // extension wins over avoiding redundancy. ~keep
        for field in binding_fields(&typ.fields) {
            let getter_method_name = to_php_name(&format!("get_{}", field.name));
            let is_json_getter = ty_is_or_wraps_json(&field.ty)
                || ty_references_untagged_data_enum(&field.ty, &untagged_data_enum_names);
            let return_type = if is_json_getter {
                // The real getter always serializes to `Option<String>` for Json/untagged-enum
                // fields, regardless of the field's own optionality (structs.rs's getter body).
                "?string".to_string()
            } else {
                let ptype = php_type(&field.ty);
                if field.optional && !ptype.starts_with('?') {
                    format!("?{ptype}")
                } else {
                    ptype
                }
            };
            let return_inner = match &field.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if !is_json_getter && matches!(return_inner, TypeRef::Vec(_) | TypeRef::Map(_, _)) {
                let return_phpdoc = php_phpdoc_type_fq(&field.ty, &namespace);
                content.push_str(&format!("    /** @return {return_phpdoc} */\n"));
            }
            if !is_constructor_param(field) {
                content.push_str(
                    "    /**\n     * @readonly Not settable via the constructor — this field's \
                     type has no ext-php-rs #[php(prop)]/constructor-param support, so it can \
                     only be read via this getter.\n     */\n",
                );
            }
            let getter_stub_body =
                "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }";
            content.push_str(&crate::backends::php::template_env::render(
                "php_stub_method_definition.jinja",
                context! {
                    static_kw => "",
                    method_name => &getter_method_name,
                    params => "",
                    return_type => &return_type,
                    stub_body => getter_stub_body,
                },
            ));
        }

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

    if !config.components.is_empty() {
        content.push_str(
            "/** Load a downloadable native component. */\nfunction component_load(string $component): void {}\n\n/** @param list<string>|null $components\n * @return list<string>\n */\nfunction component_prefetch(?array $components = null): array { return []; }\n\n/** Inspect a downloadable native component. */\nfunction component_status(string $component): string { return ''; }\n\n/** Return a downloadable native component cache path. */\nfunction component_cache_path(string $component): string { return ''; }\n\n",
        );
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
        .unwrap_or_else(|| {
            // Stubs are a sibling of the resolved class directory (e.g. `crates/{name}-php/stubs/`
            // next to `crates/{name}-php/src/`) so a co-located crate keeps composer.json,
            // classes, and IDE stubs together. ~keep
            let class_dir = resolve_output_dir(config.output_paths.get("php"), &config.name, "packages/php/src/");
            std::path::Path::new(&class_dir)
                .parent()
                .map(|parent| parent.join("stubs").to_string_lossy().into_owned())
                .unwrap_or_else(|| "packages/php/stubs/".to_string())
        });

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}_extension.php", extension_name)),
        content,
        generated_header: false,
    }])
}

/// True when the PHPStan stub for `typ` must declare the `from_json(string $json): self` static
/// constructor, mirroring the exact gate the real extension uses to decide whether to emit
/// `#[php(name = "from_json")]` (see `gen_bindings/types/structs.rs`'s `use_from_json`).
///
/// `from_json` is the only way to build a type's nested/complex fields from PHP — ext-php-rs
/// exposes such fields read-only — so a stub that omits it hides a real, callable method from
/// editors and static analysis (PHPStan reports a false "undefined method").
pub(super) fn struct_needs_from_json_stub(typ: &crate::core::ir::TypeDef, enum_names: &AHashSet<String>) -> bool {
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if has_explicit_static_new || typ.fields.is_empty() || !typ.has_serde {
        return false;
    }
    let has_named_params = typ.fields.iter().any(|f| !is_php_prop_scalar(&f.ty, enum_names));
    let has_field_defaults = typ
        .fields
        .iter()
        .any(|f| f.default.is_some() || f.typed_default.is_some());
    has_named_params || typ.has_default || has_field_defaults
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
