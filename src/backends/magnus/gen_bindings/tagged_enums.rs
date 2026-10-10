use crate::backends::magnus::type_map::MagnusMapper;

/// Map a field TypeRef to a Sorbet type string for use in `sig` blocks emitted in `.rb` files.
fn sorbet_type_for_field(ty: &crate::core::ir::TypeRef, optional: bool) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let base = match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "T::Boolean".to_string(),
            PrimitiveType::F32 | PrimitiveType::F64 => "Float".to_string(),
            _ => "Integer".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Bytes => "String".to_string(),
        TypeRef::Vec(inner) => format!("T::Array[{}]", sorbet_type_for_field(inner, false)),
        TypeRef::Map(k, v) => format!(
            "T::Hash[{}, {}]",
            sorbet_type_for_field(k, false),
            sorbet_type_for_field(v, false)
        ),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) => return format!("T.nilable({})", sorbet_type_for_field(inner, false)),
        TypeRef::Duration => "Integer".to_string(),
        TypeRef::Json | TypeRef::Unit => "T.untyped".to_string(),
    };
    if optional { format!("T.nilable({base})") } else { base }
}

/// The body of `from_hash` for a variant serde flattens (see
/// [`crate::codegen::serde_enum_repr::serde_flattens_newtype_payload`]).
///
/// Internal tagging merges the payload's own fields into the tag object, so there is no `_0` key
/// to read and the previous `hash[:_0] || hash["_0"]` resolved to `nil` for every such variant.
/// The payload IS the hash minus the discriminator. Keys are symbolized because the generated
/// Magnus constructor reads them with `kwargs.get(ruby.to_symbol(...))`, which a string-keyed
/// Hash never matches, and `from_hash` accepts both spellings from its callers.
///
/// A payload that is not a plain `TypeRef::Named` has no Ruby class to construct, so the
/// stripped hash is passed through rather than guessing at a constructor — serde itself rejects
/// an internally tagged newtype whose payload is not a map, so this is a degenerate shape. ~keep
fn flattened_newtype_from_hash_body(field: &crate::core::ir::FieldDef, tag_field: &str) -> String {
    use crate::core::ir::TypeRef;

    let payload_expr = match &field.ty {
        TypeRef::Named(payload_type) => format!("{payload_type}.new(payload)"),
        _ => "payload".to_string(),
    };
    let attr_name = if field.name == "_0" {
        "value"
    } else {
        field.name.as_str()
    };
    crate::backends::magnus::template_env::render(
        "tagged_enum_flattened_from_hash.rb.jinja",
        crate::alef_context! {
            tag_field => tag_field,
            attr_name => attr_name,
            payload_expr => payload_expr,
        },
    )
}

/// Generate a Ruby marker module and Data.define variants for an internally-tagged enum.
///
/// Emits:
/// - A marker module with `interface!` and `abstract!` Sorbet annotations, plus a
///   dispatcher `from_hash(hash)` that routes to the appropriate variant constructor
///   based on the discriminator field.
/// - A `Data.define(...)` per variant that includes the marker module, with typed
///   attribute accessors, variant predicate methods, and a per-variant `from_hash` factory.
///
/// This is the Ruby 3.2+ idiomatic pattern for sealed sum types using Data classes
/// mixed into marker modules. Each variant instance `is_a?(MarkerModule)` returns true.
pub(super) fn gen_tagged_enum_ruby_classes(
    enum_def: &crate::core::ir::EnumDef,
    module_name: &str,
    types: &[crate::core::ir::TypeDef],
) -> String {
    use crate::codegen::doc_emission::emit_yard_doc;
    let mut out = String::new();

    let class_name = &enum_def.name;
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let tag_field_symbol = crate::backends::magnus::ruby_symbol_literal(tag_field);

    let mut doc_comment = String::new();
    if !enum_def.doc.is_empty() {
        emit_yard_doc(&mut doc_comment, &enum_def.doc, "  ");
    } else {
        doc_comment.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_marker_doc.rb.jinja",
            crate::alef_context! {
                class_name => class_name,
            },
        ));
    }
    let mut dispatch_arms = String::new();
    for variant in &enum_def.variants {
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let variant_const = format!("{}{}", class_name, variant.name);
        dispatch_arms.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_dispatch_arm.rb.jinja",
            crate::alef_context! {
                wire_name => wire_name,
                variant_const => variant_const,
            },
        ));
    }
    out.push_str(&crate::backends::magnus::template_env::render(
        "tagged_enum_marker_module.rb.jinja",
        crate::alef_context! {
            module_name => module_name,
            class_name => class_name,
            doc_comment => doc_comment,
            tag_field => tag_field,
            tag_field_symbol => tag_field_symbol,
            dispatch_arms => dispatch_arms,
        },
    ));

    for variant in &enum_def.variants {
        let variant_class = format!("{}{}", class_name, variant.name);
        let field_names: Vec<&str> = variant.fields.iter().map(|f| f.name.as_str()).collect();

        let mut doc_comment = String::new();
        if !variant.doc.is_empty() {
            emit_yard_doc(&mut doc_comment, &variant.doc, "  ");
        } else {
            doc_comment.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_variant_doc.rb.jinja",
                crate::alef_context! {
                    variant_class => &variant_class,
                    class_name => class_name,
                },
            ));
        }

        let symbol_args = if field_names.is_empty() {
            String::new()
        } else {
            variant
                .fields
                .iter()
                .map(|f| {
                    let attr_name = if f.name == "_0" { "value" } else { f.name.as_str() };
                    format!(":{attr_name}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        let mut field_accessors = String::new();
        for field in &variant.fields {
            let attr_name = if field.name == "_0" {
                "value"
            } else {
                field.name.as_str()
            };
            let sorbet_t = sorbet_type_for_field(&field.ty, field.optional);
            let mut doc_comment = String::new();
            if !field.doc.is_empty() {
                emit_yard_doc(&mut doc_comment, &field.doc, "    ");
            }
            // Data's reader lives on this class, not its superclass; `super` cannot reach it. ~keep
            field_accessors.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_field_accessor.rb.jinja",
                crate::alef_context! {
                    doc_comment => doc_comment,
                    sorbet_t => sorbet_t,
                    attr_name => attr_name,
                },
            ));
        }

        let mut predicate_methods = String::new();
        for variant_name in &variant_names {
            let v_snake = crate::codegen::naming::pascal_to_snake(variant_name);
            // minijinja renders a bool Jinja2-style as `True`/`False`, which Ruby parses as ~keep
            // constant lookups, so the literal is spelled out here instead of passing a bool. ~keep
            let ruby_boolean_literal = if *variant_name == variant.name { "true" } else { "false" };
            predicate_methods.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_predicate_method.rb.jinja",
                crate::alef_context! {
                    predicate_name => v_snake,
                    ruby_boolean_literal => ruby_boolean_literal,
                },
            ));
        }

        let from_hash_body =
            if crate::codegen::serde_enum_repr::serde_flattens_newtype_payload(enum_def, variant, types) {
                flattened_newtype_from_hash_body(&variant.fields[0], tag_field)
            } else {
                // Under adjacent tagging (`tag = "t", content = "c"`) serde puts a newtype payload
                // under the CONTENT key, never under the synthesized positional name -- `_0` is an
                // alef-internal field name that appears on no wire. Reading `hash[:_0]` there yields
                // nil for every such variant, the same defect the flattened branch above fixes for
                // internal tagging (e.g. a diff-line enum with `tag="kind", content="text"`). ~keep
                let positional_wire_key = crate::codegen::serde_enum_repr::serde_enum_repr(enum_def)
                    .content()
                    .map(str::to_string);
                let field_args: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let is_positional = f.name == "_0";
                        let key_string = match (is_positional, positional_wire_key.as_deref()) {
                            (true, Some(content)) => content,
                            (true, None) => "_0",
                            (false, _) => f.name.as_str(),
                        };
                        let param_name = if is_positional {
                            "value".to_string()
                        } else {
                            f.name.clone()
                        };
                        let val_expr = format!("hash[:{key_string}] || hash[\"{key_string}\"]");
                        format!("{param_name}: {val_expr}")
                    })
                    .collect();
                let call = if field_args.is_empty() {
                    "new".to_string()
                } else {
                    format!("new({})", field_args.join(", "))
                };
                format!("      {call}\n")
            };

        let doc_comment = doc_comment.replace("  # ", "  ## ");

        out.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_variant_class.rb.jinja",
            crate::alef_context! {
                doc_comment => doc_comment,
                symbol_args => symbol_args,
                variant_class => variant_class,
                class_name => class_name,
                field_accessors => field_accessors,
                predicate_methods => predicate_methods,
                from_hash_body => from_hash_body,
            },
        ));
    }

    if out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str("end\n");
    out
}

/// For a variant-wrapper opaque type (one whose `is_variant_wrapper` flag is set
/// by the extractor), emit a static `pub fn new(...)` on the Magnus binding struct
/// so that `define_singleton_method("new", function!(TypeName::new, N))` in
/// `ruby_init` resolves to a real Rust function.
///
/// The generated `new` creates a core instance via `CoreType::new(args)` and wraps
/// it in `Arc` — matching the opaque struct layout produced by `gen_opaque_struct`.
///
/// Returns `None` when the wrapper has no `new` method in the IR (or its receiver
/// is not `None`), in which case the variant body would not compile either but we
/// silently skip rather than panic so the rest of the surface can still be generated.
pub(super) fn magnus_variant_wrapper_constructor(
    typ: &crate::core::ir::TypeDef,
    mapper: &MagnusMapper,
    core_import: &str,
) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    let needs_into = |t: &crate::core::ir::TypeRef| -> bool {
        matches!(
            t,
            crate::core::ir::TypeRef::Named(_)
                | crate::core::ir::TypeRef::Optional(_)
                | crate::core::ir::TypeRef::Vec(_)
                | crate::core::ir::TypeRef::Map(_, _)
        )
    };
    let call_args = ctor
        .params
        .iter()
        .map(|p| {
            if needs_into(&p.ty) {
                format!("{}.into()", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let body = if call_args.is_empty() {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new()) }}")
    } else {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new({call_args})) }}")
    };
    let fn_sig = if sig_params.is_empty() {
        "pub fn new() -> Self".to_string()
    } else {
        format!("pub fn new({sig_params}) -> Self")
    };
    Some(format!(
        "impl {name} {{\n    {fn_sig} {{\n        {body}\n    }}\n}}\n",
        name = typ.name,
    ))
}

#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod tests {
    use super::gen_tagged_enum_ruby_classes;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Serde's real default (no `#[serde(rename_all = "...")]` on the enum) serializes unit
    /// variants verbatim under their Rust name, e.g. `"KeyValue"` — not snake_cased. The
    /// generated Ruby dispatch must match that wire value exactly, or a Ruby caller building
    /// the discriminator from real JSON payloads (e.g. `"type": "KeyValue"`) never matches any
    /// `when` arm.
    #[test]
    fn dispatch_arm_uses_verbatim_variant_name_when_no_rename_all_declared() {
        let enum_def = EnumDef {
            name: "DataNodeKind".to_string(),
            serde_tag: Some("kind".to_string()),
            serde_rename_all: None,
            variants: vec![variant("KeyValue"), variant("Array")],
            ..Default::default()
        };

        let code = gen_tagged_enum_ruby_classes(&enum_def, "TestLib", &[]);

        assert!(
            code.contains("when \"KeyValue\" then"),
            "expected verbatim wire value \"KeyValue\" in dispatch, got:\n{code}"
        );
        assert!(
            !code.contains("when \"key_value\" then"),
            "must not fabricate a snake_case rename_all that the Rust enum never declared, got:\n{code}"
        );
    }
}

#[cfg(test)]
mod discriminator_key_parity_tests {
    use super::gen_tagged_enum_ruby_classes;
    use crate::backends::napi::tagged_enum_discriminant_js_name;
    use crate::codegen::serde_enum_repr::tagged_object_tag_key;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    /// A data-carrying enum whose `#[serde(tag = ...)]` is whatever the caller passes -- including
    /// `None`, the shape whose discriminator key is a convention rather than an IR value and so
    /// the only shape where two emitters could each invent their own answer.
    fn sample_enum(serde_tag: Option<&str>) -> EnumDef {
        EnumDef {
            name: "SampleUnion".to_string(),
            rust_path: "test_core::SampleUnion".to_string(),
            serde_tag: serde_tag.map(str::to_string),
            variants: vec![
                EnumVariant {
                    name: "Basic".to_string(),
                    fields: vec![FieldDef {
                        name: "username".to_string(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Anonymous".to_string(),
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }
    }

    /// The Ruby marker module's `from_hash` dispatcher and the compiled napi binding's
    /// discriminant field are two emitters reading one `EnumDef`. They must name the same key or
    /// a payload produced by one binding is undispatchable by the other.
    ///
    /// `gen_tagged_enum_ruby_classes` spelled its own fallback `"kind"` while every sibling
    /// emitter -- napi, wasm, php, swift, extendr, rustler -- spelled theirs `"type"`. Neither
    /// side's own tests could see it, because each compared its output against its own literal.
    /// Asking [`tagged_object_tag_key`] from both places is what makes the two answers one answer.
    ///
    /// The divergence was LATENT, not observed: `mod.rs`'s only call to
    /// `gen_tagged_enum_ruby_classes` is guarded by `enum_def.serde_tag.is_some()`, so the
    /// `"kind"` fallback could never be reached in production -- verified against the commit that
    /// introduced it, where the guard already read the same. This test therefore pins a trap that
    /// a future relaxation of that guard would have sprung, and the `None` row below is the one
    /// that would have caught it. Do not read it as coverage for a shipped bug.
    #[test]
    fn should_key_the_ruby_dispatcher_on_the_same_field_the_napi_binding_declares() {
        for serde_tag in [None, Some("type"), Some("kind"), Some("discriminator")] {
            let enum_def = sample_enum(serde_tag);

            let expected = tagged_object_tag_key(&enum_def).to_string();
            assert_eq!(
                tagged_enum_discriminant_js_name(&enum_def),
                expected,
                "napi discriminant must come from the shared authority (serde_tag = {serde_tag:?})"
            );

            let ruby = gen_tagged_enum_ruby_classes(&enum_def, "TestLib", &[]);
            assert!(
                ruby.contains(&format!("discriminator = hash[:{expected}] || hash[\"{expected}\"]")),
                "ruby from_hash must read `{expected}` (serde_tag = {serde_tag:?}), got:\n{ruby}"
            );
        }
    }

    /// Pins the direction of the fix, not just the agreement: an enum with no `#[serde(tag)]`
    /// resolves to `type`. Without this a future change could make BOTH emitters say `kind` and
    /// still pass the parity assertion above while disagreeing with php, wasm and swift.
    #[test]
    fn should_fall_back_to_the_conventional_type_key_when_the_enum_declares_no_tag() {
        let enum_def = sample_enum(None);
        assert_eq!(tagged_object_tag_key(&enum_def), "type");

        let ruby = gen_tagged_enum_ruby_classes(&enum_def, "TestLib", &[]);
        assert!(
            !ruby.contains("hash[:kind]"),
            "the untagged fallback must not reintroduce the magnus-only `kind` key, got:\n{ruby}"
        );
    }
}
