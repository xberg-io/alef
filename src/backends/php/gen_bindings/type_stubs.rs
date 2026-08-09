use crate::backends::php::gen_bindings::functions::has_unsupported_static_params;
use crate::backends::php::gen_bindings::php_types::{
    enum_aware_php_phpdoc_type, enum_aware_php_type, php_phpdoc_type_fq, php_property_phpdoc, php_type_fq,
};
use crate::backends::php::gen_bindings::types::{
    enum_constant_entries, flat_field_name, is_php_prop_scalar, is_tagged_data_enum, is_untagged_data_enum,
    php_field_can_be_constructor_param, ty_is_or_wraps_json, ty_references_untagged_data_enum,
};
use crate::backends::php::naming::php_autoload_namespace;
use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::naming::to_php_name;
use crate::codegen::shared::{binding_fields, can_auto_delegate};
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_layout};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, FieldDef, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};
use minijinja::context;
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) fn generate_type_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;

    // The single signal that selects a struct's constructor SHAPE, resolved through the same
    // helper `rust_bindings.rs` uses for the runtime extension. Keying the stub on the per-type
    // `TypeDef::has_serde` instead routes a type down a different branch than the runtime takes
    // (FromJsonOnly vs Kwargs vs ThrowsNoParams), so the stub's whole declared constructor — not
    // merely its `from_json` — can be wrong. It must be `php_crate_requires_serde` and not the bare
    // `php_serde_available` probe, because a tagged data enum forces crate-wide serde on the runtime
    // side; keying the stub on the probe alone reintroduces that same divergence on the
    // regular-struct axis. ~keep
    let serde_available = super::rust_bindings::php_crate_requires_serde(api, config);

    // Mirrors `rust_bindings.rs` so `gen_enum_stub` matches the runtime's reachability verdict. ~keep
    let core_import = config.core_import_name();
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Php);
    let configured_features_set: HashSet<&str> = enabled_features.iter().map(String::as_str).collect();

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
        let is_constructor_param = |f: &FieldDef| {
            f.cfg.is_none()
                && php_field_can_be_constructor_param(&f.ty, &enum_names, &opaque_types, &untagged_data_enum_names)
        };

        // A field that fails `php_field_can_be_constructor_param` also fails `is_php_prop_scalar`,
        // so in a serde-capable crate its struct always qualifies for `from_json` — the field is
        // reachable after all and there is nothing to warn about. Only when the crate has no serde
        // is such a field genuinely write-unreachable, which is why this keys on the crate-level
        // signal rather than on the core type's own derives. ~keep
        if !serde_available {
            for excluded in binding_fields(&typ.fields).filter(|f| !is_constructor_param(f)) {
                tracing::warn!(
                    "php backend stub: {}.{} is not representable as a `#[php(constructor)]` parameter; \
                     the PHPStan stub exposes it via get_{}() only",
                    typ.name,
                    excluded.name,
                    excluded.name,
                );
            }
        }

        match stub_constructor_shape(
            typ,
            &enum_names,
            &opaque_types,
            &untagged_data_enum_names,
            serde_available,
        ) {
            // The runtime's `use_from_json` branch (structs.rs) only ALSO emits a positional
            // `#[php(constructor)]` when at least one representable field is required; when
            // every representable field is optional, `from_json` (rendered below) is the only
            // constructor the extension actually has. Declaring a positional one here would be
            // pure fiction: PHPStan/IDE users would be told they can call `new Type(...)` when
            // the real extension has no such symbol. ~keep
            StubConstructorShape::FromJsonOnly => {}
            // The runtime's `has_named_params && !use_from_json` branch emits a real,
            // zero-param `__construct` that unconditionally throws (a field needs a named/
            // complex param this shape can't represent, and the type doesn't qualify for
            // `from_json` either) -- not the full field-derived parameter list every other
            // shape gets. ~keep
            StubConstructorShape::ThrowsNoParams => {
                content.push_str(&crate::backends::php::template_env::render(
                    "php_constructor_method.jinja",
                    context! { params => "" },
                ));
            }
            // `config_gen::gen_php_kwargs_constructor` derives its parameter list from a
            // DIFFERENT algorithm than the positional shape: `constructor_fields` (only
            // `binding_excluded` is filtered — no `cfg` filter, no representability filter), in
            // raw declaration order, with EVERY param `Option<T>`. ext-php-rs treats every
            // `Option<T>` arg as nullable-and-omittable (`Function::split_args`), so the whole
            // list is optional and PHP's required-before-optional rule imposes no reordering —
            // declaration order IS the positional order callers must use. ~keep
            StubConstructorShape::Kwargs => {
                for declaration in gen_kwargs_property_declarations(typ, &enum_names, serde_available) {
                    content.push_str(&declaration);
                }
                let params = gen_kwargs_constructor_stub_params(typ, &enum_names);
                content.push_str(&crate::backends::php::template_env::render(
                    "php_constructor_method.jinja",
                    context! { params => &params.join(",\n") },
                ));
            }
            StubConstructorShape::Positional => {
                let params = gen_struct_constructor_stub_params(
                    typ,
                    &enum_names,
                    &opaque_types,
                    &untagged_data_enum_names,
                    serde_available,
                );
                content.push_str(&crate::backends::php::template_env::render(
                    "php_constructor_method.jinja",
                    context! { params => &params.join(",\n") },
                ));
            }
        }

        // The real extension additionally emits a `#[php(name = "from_json")]` static
        // constructor (structs.rs's `use_from_json` gate) whenever the struct has a field the
        // `#[php(constructor)]` can't represent (a named/complex param), a `#[derive(Default)]`,
        // or a field with a default. That is the ONLY way to build such a type's nested/complex
        // fields from PHP — ext-php-rs exposes them read-only — so the stub must declare it or
        // the method is invisible to editors and PHPStan. Mirror the exact same gate here.
        if struct_needs_from_json_stub(typ, &enum_names, serde_available) {
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
                let ptype = enum_aware_php_type(&field.ty, &enum_names);
                if php_field_effective_optional(typ, field, serde_available) && !ptype.starts_with('?') {
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

        // A static method's binding, `gen_static_method` (`functions/methods.rs`), silently
        // drops the method (`String::new()`, never registered in the `#[php_impl]` block) when
        // it can't auto-delegate or one of its params has a shape the delegation can't cross
        // (see `has_unsupported_static_params`). Calling that exact predicate here — rather than
        // restating it — is what keeps the stub from promising a static method the runtime
        // extension never defines. Instance methods have no such drop (`gen_instance_method`
        // always emits a body, adapter-backed or delegated), so only statics are filtered. ~keep
        let non_excluded_methods: Vec<&crate::core::ir::MethodDef> = typ
            .methods
            .iter()
            .filter(|m| !m.binding_excluded && !m.sanitized)
            .filter(|m| {
                m.receiver.is_some()
                    || (can_auto_delegate(m, &opaque_types)
                        && !has_unsupported_static_params(&m.params, &opaque_types, &enum_names))
            })
            .collect();
        for method in non_excluded_methods {
            let method_name = method.name.to_lower_camel_case();
            let is_static = method.receiver.is_none();
            let return_type = enum_aware_php_type(&method.return_type, &enum_names);
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
                    let ptype = enum_aware_php_type(&p.ty, &enum_names);
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
        content.push_str(&gen_enum_stub(
            enum_def,
            &enum_names,
            &core_import,
            &configured_features_set,
        ));
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
            // The stub must document the signature the binding actually emits: a `&mut T` DTO
            // parameter on a unit-returning sync function makes the binding return the updated
            // `T` instead of `void` (see `codegen::mut_writeback` and `gen_function_as_static_method`). ~keep
            let stub_return_type: std::borrow::Cow<'_, TypeRef> = if !func.is_async {
                match crate::codegen::mut_writeback::effective_return_type(
                    &func.params,
                    &func.return_type,
                    &opaque_types,
                ) {
                    Some(ty) => std::borrow::Cow::Owned(ty),
                    None => std::borrow::Cow::Borrowed(&func.return_type),
                }
            } else {
                std::borrow::Cow::Borrowed(&func.return_type)
            };
            let return_type = php_type_fq(&stub_return_type, &namespace);
            let return_phpdoc = php_phpdoc_type_fq(&stub_return_type, &namespace);
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
            // The stub declares the `…Api` extension methods `rust_bindings` emits, and that
            // emitter skips a bridge this returns `None` for — declaring one it skipped would
            // describe an extension method PHP never receives. ~keep
            if crate::backends::php::trait_bridge::active_bridge_trait(bridge_cfg, api).is_none() {
                continue;
            }
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
            let layout = resolve_output_layout(config.output_paths.get("php"), &config.name, "packages/php/src/");
            layout.root.join("stubs").to_string_lossy().into_owned()
        });

    Ok(vec![GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}_extension.php", extension_name)),
        content,
        generated_header: false,
    }])
}

/// Which shape of runtime constructor `gen_bindings/types/structs.rs` emits for a struct.
/// Mirrors its own branching (`use_from_json`, whether a representable required field
/// exists, `has_named_params`, `typ.has_default`) so the stub can match what the extension
/// actually exposes instead of assuming every struct gets the common positional shape.
#[derive(Debug, PartialEq, Eq)]
enum StubConstructorShape {
    /// `#[php(constructor)] pub fn new(...)`, built from representable non-cfg fields,
    /// required-before-optional — the shape `gen_struct_constructor_stub_params` computes.
    Positional,
    /// The `use_from_json` branch with no representable *required* field: the extension
    /// emits only `#[php(name = "from_json")]`, no positional constructor at all.
    FromJsonOnly,
    /// A real, zero-param `__construct` that always throws. Two runtime paths produce it:
    /// `structs.rs`'s explicit `pub fn __construct() -> PhpResult<Self> { Err(...) }` (a field
    /// needs a named/complex param the positional shape can't represent and the type doesn't
    /// qualify for `from_json`), and a type with a hand-written static `new`, for which
    /// `structs.rs` emits no constructor at all and ext-php-rs's `ClassBuilder` registers its
    /// own zero-arg `__construct` throwing "You cannot instantiate this class from PHP."
    ThrowsNoParams,
    /// A `#[derive(Default)]` type routed through `config_gen::gen_php_kwargs_constructor`:
    /// every non-`binding_excluded` field in declaration order (no `cfg` filter, no
    /// representability filter), each param `Option<T>` and therefore nullable-and-omittable.
    /// Derived by [`gen_kwargs_constructor_stub_params`], NOT by the positional shape's
    /// required-before-optional derivation. ~keep
    Kwargs,
}

fn stub_constructor_shape(
    typ: &crate::core::ir::TypeDef,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
    untagged_data_enum_names: &AHashSet<String>,
    serde_available: bool,
) -> StubConstructorShape {
    // `gen_struct_methods_impl` (structs.rs) gates its ENTIRE constructor block on
    // `!has_explicit_static_new`: a type with a hand-written static `new` gets no
    // `#[php(constructor)]`, no `from_json`, and no explicit `__construct` — the static `new`
    // reaches the stub through the ordinary method loop instead. ext-php-rs still registers a
    // public zero-arg `__construct` for every `#[php_class]` (the `else` arm of
    // `T::constructor()` in `builders/class.rs`) whose body throws "You cannot instantiate this
    // class from PHP.", so the accurate stub is the zero-param throwing shape — not the
    // field-derived parameter list every branch below would otherwise produce. ~keep
    if typ.methods.iter().any(|m| m.is_static && m.name == "new") {
        return StubConstructorShape::ThrowsNoParams;
    }
    if struct_needs_from_json_stub(typ, enum_names, serde_available) {
        let has_representable_required = typ.fields.iter().filter(|f| !f.binding_excluded).any(|f| {
            !f.optional && php_field_can_be_constructor_param(&f.ty, enum_names, opaque_types, untagged_data_enum_names)
        });
        return if has_representable_required {
            StubConstructorShape::Positional
        } else {
            StubConstructorShape::FromJsonOnly
        };
    }
    let has_named_params = typ.fields.iter().any(|f| !is_php_prop_scalar(&f.ty, enum_names));
    if has_named_params {
        StubConstructorShape::ThrowsNoParams
    } else if typ.has_default {
        StubConstructorShape::Kwargs
    } else {
        StubConstructorShape::Positional
    }
}

/// Mirrors the runtime constructor's own widening (`gen_bindings/types/structs.rs`'s
/// `let optional = f.optional || (has_serde && typ.has_default && matches!(f.ty,
/// TypeRef::Duration));`): a `Duration` field on a type with a `Default` impl becomes an optional,
/// nullable param at the FFI boundary even when the field itself is required in the IR, so callers
/// can omit it and get the default. Used for both the constructor stub's param list (order AND
/// nullability) and each field's getter return type — the two must agree with each other and with
/// the runtime, or the stub disagrees with what the extension actually does.
///
/// `serde_available` is the runtime's `has_serde` — the crate-level probe, not `TypeDef::has_serde`.
/// The widening exists because the mirror struct carries `#[serde(skip_serializing_if)]` /
/// `#[serde(default)]` only when the binding crate has serde, so it is the crate-level signal that
/// decides it. ~keep
fn php_field_effective_optional(typ: &crate::core::ir::TypeDef, f: &FieldDef, serde_available: bool) -> bool {
    f.optional || (serde_available && typ.has_default && matches!(f.ty, TypeRef::Duration))
}

/// Build the parameter list (one entry per line) for a struct's PHPStan `#[php(constructor)]`
/// stub, in the exact order and shape the real extension's `new(...)` declares.
///
/// Field selection mirrors `gen_bindings/types/structs.rs`'s own constructor filter
/// (`php_field_can_be_constructor_param`), and the ordering mirrors its stable
/// required-before-optional sort — both MUST derive from the same predicate or the stub and the
/// runtime constructor drift out of positional agreement.
fn gen_struct_constructor_stub_params(
    typ: &crate::core::ir::TypeDef,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
    untagged_data_enum_names: &AHashSet<String>,
    serde_available: bool,
) -> Vec<String> {
    let is_constructor_param = |f: &FieldDef| {
        f.cfg.is_none() && php_field_can_be_constructor_param(&f.ty, enum_names, opaque_types, untagged_data_enum_names)
    };

    let mut ctor_fields: Vec<&FieldDef> = binding_fields(&typ.fields)
        .filter(|f| is_constructor_param(f))
        .collect();
    ctor_fields.sort_by_key(|f| php_field_effective_optional(typ, f, serde_available));

    ctor_fields
        .iter()
        .map(|f| {
            let optional = php_field_effective_optional(typ, f, serde_available);
            let ptype = enum_aware_php_type(&f.ty, enum_names);
            let nullable = if optional && !ptype.starts_with('?') {
                format!("?{ptype}")
            } else {
                ptype
            };
            let default = if optional { " = null" } else { "" };
            let php_name = to_php_name(&f.name);
            if !is_php_prop_scalar(&f.ty, enum_names) {
                // Constructor-representable but not a real PHP property (no `#[php(prop)]`
                // on the extension's struct field) — a plain, non-promoted parameter.
                return format!("        {nullable} ${php_name}{default}");
            }
            let phpdoc_type = enum_aware_php_phpdoc_type(&f.ty, enum_names);
            let var_type = if optional && !phpdoc_type.starts_with('?') {
                format!("?{phpdoc_type}")
            } else {
                phpdoc_type
            };
            let phpdoc = php_property_phpdoc(&var_type, &f.doc, "        ");
            format!("{phpdoc}        public readonly {nullable} ${php_name}{default}",)
        })
        .collect()
}

/// Build the parameter list (one entry per line) for the `Kwargs` constructor shape, mirroring
/// `codegen::config_gen::php::gen_php_kwargs_constructor` — the function that renders the runtime
/// `pub fn __construct(...)` — parameter for parameter.
///
/// That generator maps `constructor_fields(typ)` (`!binding_excluded`, nothing else) in raw
/// declaration order to `name: Option<{mapped_ty}>`. Three consequences the positional shape's
/// derivation gets wrong here, each of which desynchronises positional slots:
///
/// * No `cfg` filter and no `php_field_can_be_constructor_param` filter — every non-excluded field
///   is a parameter, so dropping any of them shifts every later argument by one slot.
/// * Every parameter is `Option<T>`, which ext-php-rs turns into a nullable, omittable arg
///   (`Function::split_args`: "an argument is optional if it's nullable or has a default"). No
///   parameter is required, so PHP's required-before-optional rule constrains nothing and the
///   required-first sort the positional shape applies is pure divergence, not a PHP necessity.
/// * The PHP-visible parameter name is the raw Rust field ident verbatim (ext-php-rs's
///   `ident_to_php_name` only strips an `r#` prefix), NOT `to_php_name`. Named arguments therefore
///   use the snake_case field name even though the `#[php(prop)]` property is camelCase.
///
/// Constructor-property promotion is unusable for this shape and deliberately not emitted: a
/// promoted parameter must share its name AND its type with the property, but here the parameter is
/// nullable (`?T`) under the snake_case name while the backing field is non-nullable `T` (the
/// runtime body applies `.unwrap_or(default)`) under the camelCase property name. The properties are
/// declared separately by [`gen_kwargs_property_declarations`]. ~keep
fn gen_kwargs_constructor_stub_params(typ: &crate::core::ir::TypeDef, enum_names: &AHashSet<String>) -> Vec<String> {
    binding_fields(&typ.fields)
        .map(|f| {
            let ptype = enum_aware_php_type(&f.ty, enum_names);
            let nullable = if ptype.starts_with('?') {
                ptype
            } else {
                format!("?{ptype}")
            };
            format!("        {nullable} ${} = null", f.name)
        })
        .collect()
}

/// Declare the `#[php(prop)]` properties of a `Kwargs`-shaped struct, which the constructor stub
/// cannot express through property promotion (see [`gen_kwargs_constructor_stub_params`]).
///
/// Naming and eligibility mirror `gen_php_struct`'s own field-attribute pass exactly: a field gets
/// `#[php(prop, name = to_php_name(field))]` precisely when it is [`is_php_prop_scalar`]. The
/// property type is the field's own type (non-nullable for a required field — the runtime
/// constructor stores `param.unwrap_or(default)`), and the property is writable rather than
/// `readonly`, because the ext-php-rs derive registers a setter alongside the getter for every
/// `#[php(prop)]` field. ~keep
fn gen_kwargs_property_declarations(
    typ: &crate::core::ir::TypeDef,
    enum_names: &AHashSet<String>,
    serde_available: bool,
) -> Vec<String> {
    binding_fields(&typ.fields)
        .filter(|f| is_php_prop_scalar(&f.ty, enum_names))
        .map(|f| {
            // `f.optional` is carried on the FIELD, not always on its `TypeRef`, so `php_type`
            // alone under-reports nullability — the same widening the positional shape and the
            // getters apply.
            let optional = php_field_effective_optional(typ, f, serde_available);
            let ptype = enum_aware_php_type(&f.ty, enum_names);
            let nullable = if optional && !ptype.starts_with('?') {
                format!("?{ptype}")
            } else {
                ptype
            };
            let phpdoc_type = enum_aware_php_phpdoc_type(&f.ty, enum_names);
            let var_type = if optional && !phpdoc_type.starts_with('?') {
                format!("?{phpdoc_type}")
            } else {
                phpdoc_type
            };
            let phpdoc = php_property_phpdoc(&var_type, &f.doc, "    ");
            format!("{phpdoc}    public {nullable} ${};\n", to_php_name(&f.name))
        })
        .collect()
}

/// True when the PHPStan stub for `typ` must declare the `from_json(string $json): self` static
/// constructor, mirroring the exact gate the real extension uses to decide whether to emit
/// `#[php(name = "from_json")]` (see `gen_bindings/types/structs.rs`'s `use_from_json`).
///
/// `from_json` is the only way to build a type's nested/complex fields from PHP — ext-php-rs
/// exposes such fields read-only — so a stub that omits it hides a real, callable method from
/// editors and static analysis (PHPStan reports a false "undefined method").
///
/// `serde_available` is the runtime's own `has_serde` argument: the crate-level
/// `detect_serde_available` probe (see [`super::rust_bindings::php_serde_available`]), NOT the
/// per-type `TypeDef::has_serde`. The emitted body is `serde_json::from_str::<Self>` over alef's
/// generated MIRROR struct, whose `Deserialize` derive `gen_php_struct` attaches under that same
/// crate-level probe — so a core type with no derives of its own still gets a working `from_json`,
/// and a core type that derives both still cannot have one in a crate without serde. Keying this on
/// `TypeDef::has_serde` made the stub disagree with the runtime in both directions. ~keep
pub(super) fn struct_needs_from_json_stub(
    typ: &crate::core::ir::TypeDef,
    enum_names: &AHashSet<String>,
    serde_available: bool,
) -> bool {
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if has_explicit_static_new || typ.fields.is_empty() || !serde_available {
        return false;
    }
    let has_named_params = typ.fields.iter().any(|f| !is_php_prop_scalar(&f.ty, enum_names));
    let has_field_defaults = typ
        .fields
        .iter()
        .any(|f| f.default.is_some() || f.typed_default.is_some());
    has_named_params || typ.has_default || has_field_defaults
}

/// Declare the read-only properties the flat enum class exposes, mirroring
/// `gen_flat_data_enum_methods`'s `#[php(getter)]` methods one for one.
///
/// `#[php(getter)]` does NOT register a PHP method. ext-php-rs's derive intercepts the method
/// (`impl_.rs::parse_property_method`), strips the literal `get_` prefix off the RAW Rust ident with
/// no case conversion, and registers a `PropertyDescriptor` under that snake_case name — `readonly`
/// whenever there is no paired setter, which for a flat enum is always. So `get_image_url` is
/// `$part->image_url`, not `$part->getImageUrl()`. That is the opposite of the struct path, which
/// deliberately emits PLAIN methods (see `structs.rs`'s historical note) precisely so they land as
/// camelCase `getImageUrl()`; the stub must therefore declare properties here and methods there. ~keep
fn gen_data_enum_property_declarations(enum_def: &EnumDef, enum_names: &AHashSet<String>) -> Vec<String> {
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let mut declarations = vec![format!(
        "    /** @var string The variant discriminator carried by the `{tag_field}` JSON field. */\n    \
         public readonly string ${tag_field}_tag;\n"
    )];

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let flat_name = flat_field_name(variant, idx);
            if !seen.insert(flat_name.clone()) {
                continue;
            }
            // Every flat field is `Option<T>` on the binding struct regardless of the variant
            // field's own optionality — only one variant's fields are populated at a time — so
            // every property is nullable.
            let ptype = enum_aware_php_type(&field.ty, enum_names);
            let nullable = if ptype.starts_with('?') {
                ptype
            } else {
                format!("?{ptype}")
            };
            let phpdoc_type = enum_aware_php_phpdoc_type(&field.ty, enum_names);
            let var_type = if phpdoc_type.starts_with('?') {
                phpdoc_type
            } else {
                format!("?{phpdoc_type}")
            };
            let phpdoc = php_property_phpdoc(&var_type, &field.doc, "    ");
            declarations.push(format!("{phpdoc}    public readonly {nullable} ${flat_name};\n"));
        }
    }
    declarations
}

/// Emit a static-factory stub for each per-variant constructor the flat PHP enum class exposes.
///
/// The runtime binding exposes these under the camelCase host name (`to_php_name(<snake>)`), so the
/// stub declares the same public name. Each param maps through [`enum_aware_php_type`] — the same
/// mapper DTO field/method stubs use, so a param whose own type is a unit-variant enum (e.g.
/// `ContentPart::image(string $url, ImageDetail $detail)`) is typed `string` here exactly as the
/// class's own properties are, rather than promising an object the factory can't actually accept —
/// and the return type is the enum class. Optional fields gain a `?` prefix and a `= null` default,
/// mirroring DTO method stubs. `collect_all_variant_constructors` owns the skip rules (unit / tuple /
/// `binding_excluded` / sanitized-field variants) so the stub and runtime binding
/// (`gen_flat_data_enum_variant_constructors` in `gen_bindings/types/enums.rs`) stay aligned —
/// neither one suppresses a variant on a hand-written-method name collision, since
/// `enum_def.methods` is never forwarded into the generated `#[php_impl]` block.
///
/// `is_host_enum` additionally excludes any variant the runtime factory itself drops as an
/// unreachable FOREIGN cfg-gated variant (see [`variant_constructor_is_reachable`]) — without this
/// the stub would document a static method PHPStan accepts but the extension never registers.
fn gen_data_enum_variant_constructor_stubs(
    enum_def: &crate::core::ir::EnumDef,
    enum_names: &AHashSet<String>,
    is_host_enum: bool,
) -> Vec<String> {
    use crate::codegen::generators::{collect_all_variant_constructors, variant_constructor_is_reachable};

    collect_all_variant_constructors(enum_def)
        .iter()
        .filter(|ctor| {
            enum_def
                .variants
                .iter()
                .find(|v| v.name == ctor.variant_name)
                .is_some_and(|v| variant_constructor_is_reachable(v, is_host_enum))
        })
        .map(|ctor| {
            let first_optional_idx = ctor.params.iter().position(|p| p.optional);
            let params: Vec<String> = ctor
                .params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let ptype = enum_aware_php_type(&p.ty, enum_names);
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
                .map(|p| format!("@param {} ${}", enum_aware_php_phpdoc_type(&p.ty, enum_names), to_php_name(&p.name)))
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

mod enum_stub;
use enum_stub::gen_enum_stub;

#[cfg(test)]
mod enum_stub_declaration_parity_tests;
#[cfg(test)]
mod type_stubs_tests;
