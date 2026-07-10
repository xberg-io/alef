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
pub(super) fn gen_tagged_enum_ruby_classes(enum_def: &crate::core::ir::EnumDef, module_name: &str) -> String {
    use crate::codegen::doc_emission::emit_yard_doc;
    let mut out = String::new();

    let class_name = &enum_def.name;
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("kind");

    let mut doc_comment = String::new();
    if !enum_def.doc.is_empty() {
        emit_yard_doc(&mut doc_comment, &enum_def.doc, "  ");
    } else {
        doc_comment.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_marker_doc.rb.jinja",
            minijinja::context! {
                class_name => class_name,
            },
        ));
    }
    let mut dispatch_arms = String::new();
    for variant in &enum_def.variants {
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref().or(Some("snake_case")),
        );
        let variant_const = format!("{}{}", class_name, &variant.name);
        dispatch_arms.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_dispatch_arm.rb.jinja",
            minijinja::context! {
                wire_name => wire_name,
                variant_const => variant_const,
            },
        ));
    }
    out.push_str(&crate::backends::magnus::template_env::render(
        "tagged_enum_marker_module.rb.jinja",
        minijinja::context! {
            module_name => module_name,
            class_name => class_name,
            doc_comment => doc_comment,
            tag_field => tag_field,
            dispatch_arms => dispatch_arms,
        },
    ));

    for variant in &enum_def.variants {
        let variant_class = format!("{}{}", class_name, &variant.name);
        let field_names: Vec<&str> = variant.fields.iter().map(|f| f.name.as_str()).collect();

        let mut doc_comment = String::new();
        if !variant.doc.is_empty() {
            emit_yard_doc(&mut doc_comment, &variant.doc, "  ");
        } else {
            doc_comment.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_variant_doc.rb.jinja",
                minijinja::context! {
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
            // `# rubocop:disable Lint/UselessMethodDefinition` keeps `rubocop -a` from
            field_accessors.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_field_accessor.rb.jinja",
                minijinja::context! {
                    doc_comment => doc_comment,
                    sorbet_t => sorbet_t,
                    attr_name => attr_name,
                },
            ));
        }

        let mut predicate_methods = String::new();
        for variant_name in &variant_names {
            let v_snake = crate::codegen::naming::pascal_to_snake(variant_name);
            let returns_true = *variant_name == variant.name;
            predicate_methods.push_str(&crate::backends::magnus::template_env::render(
                "tagged_enum_predicate_method.rb.jinja",
                minijinja::context! {
                    predicate_name => v_snake,
                    returns_true => returns_true,
                },
            ));
        }

        let field_args: Vec<String> = variant
            .fields
            .iter()
            .map(|f| {
                let key_sym = if f.name == "_0" {
                    ":_0".to_string()
                } else {
                    format!(":{}", f.name)
                };
                let param_name = if f.name == "_0" {
                    "value".to_string()
                } else {
                    f.name.clone()
                };
                let key_string = if f.name == "_0" { "_0" } else { f.name.as_str() };
                let val_expr = format!("hash[{key_sym}] || hash[\"{key_string}\"]");
                format!("{param_name}: {val_expr}")
            })
            .collect();
        let from_hash_call = if field_args.is_empty() {
            "new".to_string()
        } else {
            format!("new({})", field_args.join(", "))
        };

        let doc_comment = doc_comment.replace("  # ", "  ## ");

        out.push_str(&crate::backends::magnus::template_env::render(
            "tagged_enum_variant_class.rb.jinja",
            minijinja::context! {
                doc_comment => doc_comment,
                symbol_args => symbol_args,
                variant_class => variant_class,
                class_name => class_name,
                field_accessors => field_accessors,
                predicate_methods => predicate_methods,
                from_hash_call => from_hash_call,
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
