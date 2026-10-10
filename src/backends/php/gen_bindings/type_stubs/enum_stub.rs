//! PHPStan stub emission for a single enum class.
//!
//! Split out of `type_stubs.rs` to keep that file under the 1,000-line cap; this is the one
//! stub emitter that has to resolve a foreign cfg-gated variant's reachability, so it carries
//! its own `configured_features` plumbing and is a clean concept boundary. ~keep

use super::{
    DocTarget, enum_constant_entries, gen_data_enum_property_declarations, gen_data_enum_variant_constructor_stubs,
    gen_labeled_string_enum_variant_constructor_stubs, is_labeled_string_enum, is_tagged_data_enum,
    sanitize_rust_idioms,
};
use crate::core::ir::EnumDef;
use ahash::AHashSet;
use std::collections::HashSet;

/// Render the complete stub declaration for one enum, dispatching on the very predicate
/// `rust_bindings.rs` dispatches on ([`is_tagged_data_enum`]): a tagged data enum becomes a flat
/// `#[php_class]` via `gen_flat_data_enum` + `gen_flat_data_enum_methods`, anything else becomes a
/// constants-only class via `gen_enum_constants`. Both arms live here so "which enum shapes get a
/// `from_json`?" has exactly one answer site — the runtime emits it for the flat class and for
/// nothing else, and an enum branch that answered that question independently of the runtime is what
/// left `Message::from_json(..)` declared nowhere while the extension defined it.
///
/// The unit-enum arm declares a plain class with `const` members via [`enum_constant_entries`] — the
/// same name/value derivation `gen_enum_constants` uses for the runtime `#[php_impl]` block — NOT a
/// native PHP 8.1 `enum ... : string`. The extension registers no native enum at all
/// (`gen_enum_constants` in `types/enums.rs` emits `#[php_class] pub struct {Name} {}` plus
/// `pub const` members on a `#[php_impl]` block); a stub that instead declared
/// `enum Foo: string { case Bar = 'bar'; }` described an API the extension never provides —
/// `Foo::Bar` does not exist at runtime (PHP class constants are case-sensitive and an enum-case
/// object is not a string), so a static analyser reported the *correct* call (`Foo::BAR`) as an
/// error and the *broken* one as fine. Making the runtime actually register a native PHP enum is a
/// much larger change to the ext-php-rs registration path and is deliberately not made here — the
/// stub describes the runtime as it exists, not the other way round. ~keep
pub(super) fn gen_enum_stub(
    enum_def: &EnumDef,
    enum_names: &AHashSet<String>,
    core_import: &str,
    configured_features: &HashSet<&str>,
) -> String {
    let mut content = String::new();
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);
    if !is_tagged_data_enum(enum_def) {
        content.push_str(&crate::backends::php::template_env::render(
            "php_record_class_stub_declaration.jinja",
            crate::alef_context! { class_name => &enum_def.name },
        ));
        for (const_name, wire_value) in enum_constant_entries(enum_def, is_host_enum, Some(configured_features)) {
            content.push_str(&crate::backends::php::template_env::render(
                "php_enum_constant_stub.jinja",
                crate::alef_context! {
                    const_name => const_name,
                    value => &wire_value,
                },
            ));
        }
        content.push_str("}\n\n");
        return content;
    }

    if !enum_def.doc.is_empty() {
        content.push_str("/**\n");
        let sanitized = sanitize_rust_idioms(&enum_def.doc, DocTarget::PhpDoc);
        content.push_str(&crate::backends::php::template_env::render(
            "php_phpdoc_lines.jinja",
            crate::alef_context! {
                doc_lines => sanitized.lines().collect::<Vec<_>>(),
                indent => "",
            },
        ));
        content.push_str(" */\n");
    }
    content.push_str(&crate::backends::php::template_env::render(
        "php_record_class_stub_declaration.jinja",
        crate::alef_context! { class_name => &enum_def.name },
    ));

    for declaration in gen_data_enum_property_declarations(enum_def, enum_names) {
        content.push_str(&declaration);
    }

    content.push_str(
        "    /**\n     * Construct from a JSON string — the flat class's only whole-value \
         constructor. The payload's tag field selects the variant; an unrecognised tag throws.\n     */\n",
    );
    content.push_str(&crate::backends::php::template_env::render(
        "php_stub_method_definition.jinja",
        crate::alef_context! {
            static_kw => "static ",
            method_name => "from_json",
            params => "string $json",
            return_type => "self",
            stub_body => "{ throw new \\RuntimeException('Not implemented — provided by the native extension.'); }",
        },
    ));

    for ctor in gen_data_enum_variant_constructor_stubs(enum_def, enum_names, is_host_enum) {
        content.push_str(&ctor);
    }
    if is_labeled_string_enum(enum_def) {
        for ctor in gen_labeled_string_enum_variant_constructor_stubs(enum_def, is_host_enum) {
            content.push_str(&ctor);
        }
    }
    content.push_str("}\n\n");
    content
}
