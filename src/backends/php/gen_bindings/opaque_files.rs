use crate::backends::php::gen_bindings::php_types::{php_phpdoc_type, php_type};
use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use heck::ToLowerCamelCase;
use minijinja::context;

pub(super) fn gen_php_opaque_class_file(
    typ: &crate::core::ir::TypeDef,
    namespace: &str,
    streaming_adapters: &[&crate::core::config::AdapterConfig],
    streaming_method_names: &AHashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
    handler_contract_map: &ahash::AHashMap<(String, String, String), String>,
) -> String {
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
    content.push_str(&crate::backends::php::template_env::render(
        "php_namespace.jinja",
        context! { namespace => namespace },
    ));
    // PSR-12: blank line between `namespace` and class declaration.
    content.push('\n');

    // Type-level docblock.
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
        "php_final_class_stub_start.jinja",
        context! { class_name => &typ.name },
    ));

    // Instance methods first, static methods second — skip streaming methods
    // (they'll be emitted as Generator wrappers after regular methods).
    let mut method_order: Vec<&crate::core::ir::MethodDef> = Vec::new();
    method_order.extend(
        typ.methods
            .iter()
            .filter(|m| m.receiver.is_some() && !streaming_method_names.contains(&m.name)),
    );
    method_order.extend(
        typ.methods
            .iter()
            .filter(|m| m.receiver.is_none() && !streaming_method_names.contains(&m.name)),
    );

    for method in method_order {
        let method_name = method.name.to_lower_camel_case();
        let return_type = php_type(&method.return_type);
        let is_void = matches!(&method.return_type, TypeRef::Unit);
        let is_static = method.receiver.is_none();

        // PHPDoc block — keep it short to avoid line-width issues.
        let mut doc_lines: Vec<String> = vec![];
        let sanitized = sanitize_rust_idioms(&method.doc, DocTarget::PhpDoc);
        let doc_line = sanitized.lines().next().unwrap_or("").trim();
        if !doc_line.is_empty() {
            doc_lines.push(doc_line.to_string());
        }

        // Add @param PHPDoc for array parameters so PHPStan knows the element type
        let mut phpdoc_params: Vec<String> = vec![];
        for param in &method.params {
            if matches!(&param.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)) {
                let phpdoc_type = php_phpdoc_type(&param.ty);
                phpdoc_params.push(format!("@param {} ${}", phpdoc_type, param.name));
            }
        }
        doc_lines.extend(phpdoc_params);

        // Add @return PHPDoc for array types so PHPStan knows the element type
        let needs_return_phpdoc = matches!(&method.return_type, TypeRef::Vec(_) | TypeRef::Map(_, _));
        if needs_return_phpdoc {
            let phpdoc_type = php_phpdoc_type(&method.return_type);
            doc_lines.push(format!("@return {phpdoc_type}"));
        }

        // Emit PHPDoc if needed
        if !doc_lines.is_empty() {
            content.push_str("    /**\n");
            for line in doc_lines {
                content.push_str(&crate::backends::php::template_env::render(
                    "php_prefixed_phpdoc_line.jinja",
                    context! {
                        indent => "    ",
                        line => &line,
                    },
                ));
            }
            content.push_str("     */\n");
        }

        // Method signature.
        let static_kw = if is_static { "static " } else { "" };
        let first_optional_idx = method.params.iter().position(|p| p.optional);
        let params: Vec<String> = method
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                // PHP has no first-class function-type declarations, so a handler
                // contract that resolves to a callback at the host-language layer
                // can't be referenced by a Rust-side class name (ext-php-rs accepts
                // `callable` for closures passed through Zval). Emit `callable`
                // whenever the handler-contract map identifies this parameter as a
                // contract; phpstan would otherwise flag the phantom class as
                // `class.notFound`.
                let ptype =
                    if handler_contract_map.contains_key(&(typ.name.clone(), method_name.clone(), p.name.clone())) {
                        "callable".to_owned()
                    } else {
                        php_type(&p.ty)
                    };
                if p.optional || first_optional_idx.is_some_and(|first| idx >= first) {
                    let nullable = if ptype.starts_with('?') { "" } else { "?" };
                    format!("{nullable}{ptype} ${} = null", p.name)
                } else {
                    format!("{} ${}", ptype, p.name)
                }
            })
            .collect();
        content.push_str(&crate::backends::php::template_env::render(
            "php_stub_method_definition.jinja",
            context! {
                static_kw => static_kw,
                method_name => &method_name,
                params => &params.join(", "),
                return_type => &return_type,
                stub_body => "",
            },
        ));
        let body = if is_void {
            "    {\n    }\n"
        } else {
            "    {\n        throw new \\RuntimeException('Not implemented — provided by the native extension.');\n    }\n"
        };
        content.push_str(body);
    }

    // Streaming wrapper methods: convert _start/_next/_free Rust functions to PHP Generators.
    for adapter in streaming_adapters {
        let item_type = adapter.item_type.as_deref().unwrap_or("array");
        content.push_str(&gen_php_streaming_method_wrapper(adapter, item_type));
        content.push('\n');
    }

    // Check if this type is a trait bridge type alias (e.g., VisitorHandle)
    for bridge in trait_bridges {
        if let Some(ref type_alias) = bridge.type_alias {
            if type_alias == &typ.name {
                // Emit the from_php_object static method for trait bridge handles
                content.push_str("    /**\n");
                content
                    .push_str("     * Wrap a PHP object implementing the visitor interface as a shareable handle.\n");
                content.push_str("     */\n");
                content.push_str("    public static function from_php_object(object $visitor): self\n");
                content.push_str("    {\n");
                content.push_str(
                    "        throw new \\RuntimeException('Not implemented — provided by the native extension.');\n",
                );
                content.push_str("    }\n");
            }
        }
    }

    content.push_str("}\n");
    content
}

/// Generate a PHP streaming method wrapper for an adapter.
///
/// For PHP, we generate a Generator method that calls the Rust streaming methods directly.
/// Since PHP can't easily pass opaque types as function parameters, we skip the _start/_next/_free
/// pattern and instead keep the streaming logic on the class.
fn gen_php_streaming_method_wrapper(adapter: &crate::core::config::AdapterConfig, _item_type: &str) -> String {
    let method_name = adapter.name.to_lower_camel_case();

    // Build parameter list.
    let mut params_vec: Vec<String> = Vec::new();

    for p in &adapter.params {
        let ptype = php_type(&crate::core::ir::TypeRef::Named(p.ty.clone()));
        let nullable = if p.optional { "?" } else { "" };
        let default = if p.optional { " = null" } else { "" };
        params_vec.push(format!("{nullable}{ptype} ${}{default}", p.name));
    }

    let params_sig = params_vec.join(", ");

    // Generate a stub method that indicates it's provided by the native extension.
    // The actual streaming implementation is on the Rust side; this PHP method
    // is a placeholder for IDE/PHPStan. At runtime, the native extension
    // provides the actual Generator-yielding implementation.
    format!(
        "    public function {method_name}({params_sig}): \\Generator\n    {{\n        \
         throw new \\RuntimeException('Not implemented — provided by the native extension.');\n    \
         }}\n",
        method_name = method_name,
    )
}
