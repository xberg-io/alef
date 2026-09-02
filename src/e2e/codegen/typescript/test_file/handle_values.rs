//! Rendering of handle-config values into TypeScript expressions.
//!
//! A handle argument's config object maps a fixture JSON key to the binding class that key's
//! value must be constructed with (`nested_types` overrides merged over the classes derived from
//! the config type's own IR fields). This module is the single authority on how that map is
//! applied: both the emitted setter statements and the class names the import block must carry
//! run the same traversal, so they cannot disagree about which classes a fixture references.

use super::*;

/// The builder context a rendered object needs, bundled so the traversal below stays a recursion
/// over the only two things that actually change as it descends: the JSON key and its value.
/// `nested_types` is the raw call override and `effective_nested_types` is that override merged
/// over the IR-derived classes — [`ts_builder_expression_inner`] re-derives its own merge for
/// whatever type it is handed, so it must keep receiving the raw override, not the merged map.
///
/// `owner_type` is the un-prefixed IR name of the struct whose fields are being rendered at the
/// current recursion depth (the handle's config type at the top level), so a scalar field can be
/// resolved back to its IR [`crate::core::ir::TypeRef`] and routed through the same enum/bigint
/// typing [`ts_builder_expression_inner`] already applies to nested objects. It is cleared to
/// `None` when recursion descends into a plain object this generator has no type mapping for —
/// that object's fields belong to some other, unknown struct, and resolving them against the
/// outer `owner_type` would attribute a field to the wrong owner. ~keep
#[derive(Clone, Copy)]
pub(in crate::e2e::codegen::typescript::test_file) struct HandleConfigContext<'a> {
    pub nested_types: &'a std::collections::HashMap<String, String>,
    pub effective_nested_types: &'a std::collections::HashMap<String, String>,
    pub lang: &'a str,
    pub enum_fields: &'a std::collections::HashMap<String, String>,
    pub bigint_fields: &'a std::collections::BTreeSet<String>,
    pub type_defs: &'a [TypeDef],
    pub enums: &'a [EnumDef],
    pub wasm_type_prefix: &'a str,
    pub owner_type: Option<&'a str>,
}

/// Render one handle-config field value as a TypeScript expression.
pub(in crate::e2e::codegen::typescript::test_file) fn build_handle_config_value(
    key: &str,
    value: &serde_json::Value,
    context: &HandleConfigContext<'_>,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    let mut used_types = std::collections::BTreeSet::new();
    render_value(key, value, context, &mut used_types, referenced_enums)
}

/// Record every binding class the rendered form of `value` will construct.
///
/// Implemented by rendering and discarding the expression rather than by walking the map a second
/// time: an import list derived independently from the emitted body is exactly how a nested class
/// ends up constructed but never imported (`ReferenceError: X is not defined` at run time). ~keep
pub(in crate::e2e::codegen::typescript::test_file) fn collect_used_handle_config_types(
    key: &str,
    value: &serde_json::Value,
    context: &HandleConfigContext<'_>,
    used_types: &mut std::collections::BTreeSet<String>,
) {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let _rendered = render_value(key, value, context, used_types, &mut referenced_enums);
}

/// Render `value`, which hangs off JSON key `key`, recording the classes constructed along the way.
///
/// The key is the only lookup into `effective_nested_types`, at every depth. An array has no key
/// of its own — the map is keyed by field name — so its elements inherit the array's key, which is
/// what makes an object inside a class-typed list get the same constructor a directly nested
/// object gets. `derive_nested_types_for_wasm` already unwraps `Vec<Named>` when it builds that
/// map, so the element's class is present at exactly the point it used to be discarded. A key
/// absent from the map renders as a plain literal, so genuinely untyped objects and lists keep the
/// bare form `json_to_js_camel` gave them. ~keep
///
/// Recursion is unbounded by design: the map's semantics are name-keyed, not path-keyed, so there
/// is no depth at which a key stops meaning what it means. Termination comes from the JSON value
/// being a finite tree.
fn render_value(
    key: &str,
    value: &serde_json::Value,
    context: &HandleConfigContext<'_>,
    used_types: &mut std::collections::BTreeSet<String>,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    match value {
        serde_json::Value::Array(elements) => {
            let rendered: Vec<String> = elements
                .iter()
                .map(|element| render_value(key, element, context, used_types, referenced_enums))
                .collect();
            format!("[{}]", rendered.join(", "))
        }
        serde_json::Value::Object(fields) => match context.effective_nested_types.get(key) {
            Some(type_name) => {
                used_types.insert(type_name.clone());
                ts_builder_expression_inner(
                    fields,
                    type_name,
                    context.nested_types,
                    context.lang,
                    context.enum_fields,
                    context.bigint_fields,
                    context.type_defs,
                    context.enums,
                    context.wasm_type_prefix,
                    &[],
                    "",
                    0,
                    referenced_enums,
                )
            }
            None => {
                // This object has no known IR type, so its fields cannot be resolved against
                // `context.owner_type` — that name belongs to a different struct. ~keep
                let inner_context = HandleConfigContext {
                    owner_type: None,
                    ..*context
                };
                let entries: Vec<String> = fields
                    .iter()
                    .map(|(field, field_value)| {
                        let rendered = render_value(field, field_value, &inner_context, used_types, referenced_enums);
                        format!("{}: {rendered}", js_object_key(&underscore_camel_case(field)))
                    })
                    .collect();
                format!("{{ {} }}", entries.join(", "))
            }
        },
        scalar => {
            let camel_key = underscore_camel_case(key);
            wasm_scalar_value_expression(
                context.owner_type,
                key,
                &camel_key,
                scalar,
                context.lang,
                context.enum_fields,
                context.bigint_fields,
                context.type_defs,
                context.enums,
                context.wasm_type_prefix,
                referenced_enums,
            )
        }
    }
}
