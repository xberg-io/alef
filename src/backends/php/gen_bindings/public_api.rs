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
    // Helper: escape `*/` sequences that could close PHPDoc early
    let escape_phpdoc_line = |s: &str| s.replace("*/", "* /");

    let extension_name = config.php_extension_name();
    let class_name = extension_name.to_pascal_case();

    // Generate PHP wrapper class
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
    // PSR-12: blank line between `declare(strict_types=1);` and `namespace`.
    content.push('\n');

    // Determine namespace — delegates to config so [php].namespace overrides are respected.
    let namespace = php_autoload_namespace(config);

    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => &namespace },
    ));
    // PSR-12: blank line between `namespace` and class declaration.
    content.push('\n');
    content.push_str(&crate::backends::php::template_env::render(
        "php_facade_class_declaration.jinja",
        context! { class_name => &class_name },
    ));

    // Build the set of bridge param names so they are excluded from public PHP signatures.
    let bridge_param_names_pub: ahash::AHashSet<&str> = config
        .trait_bridges
        .iter()
        .filter_map(|b| b.param_name.as_deref())
        .collect();

    // Config types whose PHP constructors can be called with zero arguments.
    // Only qualifies when ALL fields are optional (PHP constructor needs no required args).
    // `has_default` (Rust Default impl) is NOT sufficient — the PHP constructor is
    // generated from struct fields and still requires non-optional ones.
    // Opaque types are excluded: their `fields` is empty (no fields exposed to bindings),
    // which would vacuously satisfy `all(optional)` and incorrectly mark required handle
    // parameters as optional in facade method signatures, producing `?Type` and a
    // PHPStan `argument.type` failure when forwarding to the non-nullable native stub.
    let no_arg_constructor_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && t.fields.iter().all(|f| f.optional))
        .map(|t| t.name.clone())
        .collect();

    // Generate wrapper methods for functions
    for func in &api.functions {
        // Skip trait-bridge-managed names (clear_fn) — the trait-bridge loop below
        // emits its own static method, and duplicating it here would cause a
        // PHP fatal "Cannot redeclare" at load time.
        if crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&func.name, &config.trait_bridges) {
            continue;
        }
        // PHP method names are based on the Rust source name (camelCased).
        // Async functions do not get a suffix because PHP blocks on async internally
        // via `block_on`, presenting a synchronous API to callers.
        // For example: `scrape` (async in Rust) → `scrape()` (sync from PHP perspective).
        let method_name = func.name.to_lower_camel_case();
        let return_php_type = php_type(&func.return_type);

        // Visible params exclude bridge params (not surfaced to PHP callers).
        let visible_params: Vec<_> = func
            .params
            .iter()
            .filter(|p| !bridge_param_names_pub.contains(p.name.as_str()))
            .collect();

        // PHPDoc block: translate rustdoc sections to PHPDoc format, stripping Rust-specific syntax.
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
            // Extract and render summary + major rustdoc sections, stripping Rust-specific syntax.
            let sections = doc_emission::parse_rustdoc_sections(&func.doc);
            // Emit summary
            for line in sections.summary.lines() {
                content.push_str("     * ");
                content.push_str(&escape_phpdoc_line(line));
                content.push('\n');
            }
            // Skip Arguments, Returns, Errors, Example — they're emitted as @param/@return/@throws below.
            // This prevents raw Rust syntax from leaking into the docstring.
        }
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_empty_line.jinja",
            minijinja::Value::default(),
        ));
        for p in &visible_params {
            let ptype = php_phpdoc_type(&p.ty);
            // Check if the parameter is optional via the IR flag (indicating Option<T>).
            // php_phpdoc_type() handles TypeRef::Optional by returning a string starting with '?',
            // but parameters can also have p.optional = true without the type being Optional.
            // In that case, we need to prepend '?' to the PHPDoc type.
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

        // Method signature with type hints.
        // Keep parameters in their original Rust order.
        // Since PHP doesn't allow optional params before required ones, and some Rust
        // functions have optional params in the middle, we must make all params after
        // the first optional one also optional (nullable with null default).
        // This ensures e2e generated test code (which uses Rust param order) will work.
        // Treat required named params as optional only when IR metadata proves the
        // target type can be constructed with zero arguments.
        let is_optional_default_constructible_param = |p: &crate::core::ir::ParamDef| -> bool {
            if let TypeRef::Named(name) = &p.ty {
                no_arg_constructor_types.contains(name.as_str())
            } else {
                false
            }
        };

        // Build wrapper signature in RUST parameter order (not reordered).
        // E2e test generator expects Rust param order, so the wrapper must match.
        // This aligns PHP bindings with Python, Ruby, Go, etc. which also preserve
        // Rust parameter order.
        // PHP 8.1 syntax rule: required params must come before optional ones.
        // Optional-default-constructible params (like a no-arg-constructible
        // CrawlConfig) can have `= null` defaults — but ONLY when every later
        // parameter is also optional. Otherwise PHP 8.1 emits a "Required
        // parameter follows optional" deprecation. Walk the param list from
        // the end so a required param resets the optional-tail flag to false.
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
                // Check if the parameter is optional: IR metadata p.optional indicates Option<T>
                // (which php_type renders as ?T), or it's a default-constructible type that can
                // use a null default.
                let type_is_nullable = ptype.starts_with('?');
                let is_optional_in_ir = p.optional;
                let can_be_optional =
                    type_is_nullable || is_optional_in_ir || is_optional_default_constructible_param(p);

                // Only emit `= null` default for parameters that are truly optional.
                // The tail_optional check ensures PHP 8.1 compliance (required params before optional ones).
                let can_emit_default = tail_optional[idx]
                    && (type_is_nullable || is_optional_in_ir || is_optional_default_constructible_param(p));

                if can_be_optional && can_emit_default {
                    // ptype may already be nullable (e.g., "?string" from php_type handling
                    // TypeRef::Optional). Don't double-prepend the nullable prefix.
                    if ptype.starts_with('?') {
                        format!("{} ${} = null", ptype, p.name)
                    } else {
                        format!("?{} ${} = null", ptype, p.name)
                    }
                } else if can_be_optional {
                    // PHP 8.1+ allows `?Type $name` (nullable without default) even
                    // when a non-nullable required parameter follows. Required by
                    // Rust `Option<T>` params that PHP 8.1 ordering forces into a
                    // non-tail position — drop the default but keep the `?` so
                    // callers can pass `null` (matches Rust `None`).
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

        // Emit signature: when params is empty, collapse () to single line.
        // Otherwise, multi-line with params on separate line.
        if params.is_empty() {
            // Single-line signature for no-arg functions
            content.push_str(&format!(
                "    public static function {}(): {} {{\n",
                method_name, return_php_type
            ));
        } else {
            // Multi-line signature for functions with params
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
        // Delegate to the native extension class (registered as `{namespace}\{class_name}Api`).
        // ext-php-rs auto-converts Rust snake_case to PHP camelCase.
        // PHP does not expose async — async behaviour is handled internally via Tokio
        // block_on, so the Rust function name matches the PHP method name exactly.
        let ext_method_name = func.name.to_lower_camel_case();
        let is_void = matches!(&func.return_type, TypeRef::Unit);
        // CRITICAL: Pass parameters to the native function in their ORIGINAL IR order,
        // not in the reordered wrapper signature order.
        // The wrapper signature is reordered for PHP 8.1 compliance (required first),
        // but the native extension method expects parameters
        // in the original IR order (as registered via #[php_impl]).
        // When IR has optional params before required ones, the two orders differ.
        // Example: IR is (engine: Option<Engine>, url: String)
        //   → Wrapper signature: (string $url, ?Engine $engine = null) [reordered]
        //   → Native call: ($engine ?? new Engine(), $url) [IR order]
        // Build call args by iterating visible_params in original IR order.
        let call_params = visible_params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                // Only apply the `?? new Type()` coercion for params that the
                // wrapper actually emits as nullable with `= null` — i.e. params
                // marked optional that also sit in a tail-optional position. A
                // param that became required to satisfy PHP 8.1's "required before
                // optional" rule (`tail_optional[idx] == false`) is non-nullable
                // at the wrapper signature, so `$p ?? new T()` would be a useless
                // null-coalesce (`nullCoalesce.variable` in phpstan).
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

    // Emit trait-bridge registration methods in the PHP facade
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

    // Use PHP stubs output path if configured, otherwise fall back to packages/php/src/.
    // This is intentionally separate from config.output.php, which controls the Rust binding
    // crate output directory (e.g., crates/sample-crawler-php/src/).
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

    // Emit a per-opaque-type PHP class file alongside the facade. These provide
    // method declarations for static analysis (PHPStan) and IDE autocomplete.
    // The native PHP extension registers the same class names at module load
    // (before Composer autoload runs), so these userland files are never
    // included at runtime — the native class always wins.
    // Build a map of (service owner type, method name, param name) -> callback contract
    // to fix generic handler parameter types (e.g., H -> Handler).
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
