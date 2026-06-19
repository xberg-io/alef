use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, TypeDef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::{dart_safe_ident, dart_safe_type_name};
use crate::backends::dart::template_env;
use crate::backends::dart::type_map::DartMapper;

use super::render_type::render_type;

#[allow(dead_code)]
pub(super) fn emit_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !ty.doc.is_empty() {
        let doc_lines: Vec<String> = ty.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    let visible_fields: Vec<_> = binding_fields(&ty.fields).collect();
    if visible_fields.is_empty() {
        out.push_str(&template_env::render(
            "class_empty.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        return;
    }
    out.push_str(&template_env::render(
        "class_open.jinja",
        minijinja::context! {
            name => ty.name.as_str(),
        },
    ));
    for field in &visible_fields {
        let ty_str = if field.optional {
            format!("{}?", render_type(&field.ty, imports))
        } else {
            render_type(&field.ty, imports)
        };
        let name = dart_safe_ident(&field.name.to_lower_camel_case());
        if !field.doc.is_empty() {
            let doc_lines: Vec<String> = field.doc.lines().map(ToString::to_string).collect();
            out.push_str(&template_env::render(
                "doc_comment.jinja",
                minijinja::context! {
                    indent => "  ",
                    lines => doc_lines,
                },
            ));
        }
        out.push_str(&template_env::render(
            "final_field_decl.jinja",
            minijinja::context! {
                ty_str => ty_str,
                name => name.as_str(),
            },
        ));
    }
    // Constructor
    if visible_fields.len() == 1 {
        let field = visible_fields[0];
        let name = dart_safe_ident(&field.name.to_lower_camel_case());
        let ty_str = if field.optional {
            format!("{}?", render_type(&field.ty, imports))
        } else {
            render_type(&field.ty, imports)
        };
        out.push_str(&template_env::render(
            "single_param_constructor.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                param_name => name.as_str(),
            },
        ));
        let _ = ty_str; // used above for field emission, constructor uses `this.`
    } else {
        out.push_str(&template_env::render(
            "multi_param_constructor_open.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        for field in &visible_fields {
            let name = dart_safe_ident(&field.name.to_lower_camel_case());
            out.push_str(&template_env::render(
                "constructor_required_param.jinja",
                minijinja::context! {
                    name => name.as_str(),
                },
            ));
        }
        out.push_str(&template_env::render("constructor_close.jinja", minijinja::context! {}));
    }
    out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
}

#[allow(dead_code)]
pub(super) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        let doc_lines: Vec<String> = en.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&template_env::render(
            "enum_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            if !variant.doc.is_empty() {
                let doc_lines: Vec<String> = variant.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "  ",
                        lines => doc_lines,
                    },
                ));
            }
            let vname = dart_safe_ident(&variant.name.to_lower_camel_case());
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    vname => vname.as_str(),
                    suffix => suffix,
                },
            ));
        }
        out.push_str(&template_env::render("enum_close.jinja", minijinja::context! {}));
    } else {
        out.push_str(&template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        for variant in &en.variants {
            // Use dart_safe_type_name to avoid shadowing Dart core types (e.g. `List`, `Map`).
            let safe_variant_name = dart_safe_type_name(&variant.name, Some(&en.name));
            if !variant.doc.is_empty() {
                let doc_lines: Vec<String> = variant.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "",
                        lines => doc_lines,
                    },
                ));
            }
            if variant.fields.is_empty() {
                out.push_str(&template_env::render(
                    "final_class_extends.jinja",
                    minijinja::context! {
                        name => safe_variant_name.as_str(),
                        parent => en.name.as_str(),
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "final_class_header.jinja",
                    minijinja::context! {
                        name => safe_variant_name.as_str(),
                        parent => en.name.as_str(),
                    },
                ));
                for f in variant.fields.iter() {
                    let ty_str = DartMapper.map_type(&f.ty);
                    let fname = dart_safe_ident(&f.name.to_lower_camel_case());
                    out.push_str(&template_env::render(
                        "final_field_decl.jinja",
                        minijinja::context! {
                            ty_str => ty_str,
                            name => fname.as_str(),
                        },
                    ));
                }
                if variant.fields.len() == 1 {
                    let fname = dart_safe_ident(&variant.fields[0].name.to_lower_camel_case());
                    out.push_str(&template_env::render(
                        "single_param_constructor.jinja",
                        minijinja::context! {
                            name => safe_variant_name.as_str(),
                            param_name => fname.as_str(),
                        },
                    ));
                } else {
                    out.push_str(&template_env::render(
                        "multi_param_constructor_open.jinja",
                        minijinja::context! {
                            name => safe_variant_name.as_str(),
                        },
                    ));
                    for f in variant.fields.iter() {
                        let fname = dart_safe_ident(&f.name.to_lower_camel_case());
                        out.push_str(&template_env::render(
                            "constructor_required_param.jinja",
                            minijinja::context! {
                                name => fname.as_str(),
                            },
                        ));
                    }
                    out.push_str(&template_env::render("constructor_close.jinja", minijinja::context! {}));
                }
                out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
    use std::collections::BTreeSet;

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    fn make_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            variants,
            methods: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
            has_default: false,
        }
    }

    fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }
    }

    // ── (a) sealed class keyword in tagged enum emission ────────────────────

    #[test]
    fn tagged_enum_emits_sealed_class_keyword() {
        let en = make_enum(
            "Message",
            vec![
                make_variant("System", vec![make_field("content", TypeRef::String)]),
                make_variant("User", vec![make_field("content", TypeRef::String)]),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);
        assert!(
            out.contains("sealed class Message {}"),
            "tagged enum must open with `sealed class`: {out}"
        );
    }

    #[test]
    fn tagged_enum_variants_emit_final_class_extends() {
        let en = make_enum(
            "Shape",
            vec![
                make_variant(
                    "Circle",
                    vec![make_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
                ),
                make_variant("Rect", vec![]),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);
        assert!(
            out.contains("final class Circle extends Shape {"),
            "data variant must be `final class ... extends`: {out}"
        );
        assert!(
            out.contains("final class Rect extends Shape {}"),
            "unit variant must be `final class ... extends ... {{}}`: {out}"
        );
    }

    // ── (b) const constructors on DTOs ──────────────────────────────────────

    #[test]
    fn dto_single_field_emits_const_constructor() {
        let ty = make_type("Point", vec![make_field("x", TypeRef::Primitive(PrimitiveType::I32))]);
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_type(&ty, &mut out, &mut imports);
        assert!(
            out.contains("const Point(this.x);"),
            "single-field DTO must have `const` constructor: {out}"
        );
    }

    #[test]
    fn dto_multi_field_emits_const_constructor() {
        let ty = make_type(
            "Pair",
            vec![
                make_field("first", TypeRef::String),
                make_field("second", TypeRef::Primitive(PrimitiveType::I32)),
            ],
        );
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_type(&ty, &mut out, &mut imports);
        assert!(
            out.contains("const Pair({"),
            "multi-field DTO must have `const` constructor: {out}"
        );
        assert!(
            out.contains("required this.first"),
            "multi-field DTO must use named required params: {out}"
        );
    }

    // ── (c) final fields throughout ─────────────────────────────────────────

    #[test]
    fn dto_fields_are_final() {
        let ty = make_type(
            "Config",
            vec![
                make_field("name", TypeRef::String),
                make_field("count", TypeRef::Primitive(PrimitiveType::I32)),
            ],
        );
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_type(&ty, &mut out, &mut imports);
        assert!(out.contains("final String name;"), "DTO fields must be `final`: {out}");
        assert!(out.contains("final int count;"), "DTO fields must be `final`: {out}");
    }

    #[test]
    fn tagged_enum_variant_fields_are_final() {
        let en = make_enum(
            "Event",
            vec![make_variant(
                "Clicked",
                vec![make_field("button", TypeRef::Primitive(PrimitiveType::I32))],
            )],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);
        assert!(
            out.contains("final int button;"),
            "sealed variant fields must be `final`: {out}"
        );
    }

    // ── (d) const constructors on sealed variant classes ────────────────────

    #[test]
    fn tagged_enum_data_variant_emits_const_constructor() {
        let en = make_enum(
            "Cmd",
            vec![make_variant("Quit", vec![make_field("reason", TypeRef::String)])],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);
        assert!(
            out.contains("const Quit(this.reason);"),
            "sealed data variant must have `const` constructor: {out}"
        );
    }

    #[test]
    fn dto_class_uses_final_class_modifier() {
        let ty = make_type("Token", vec![make_field("value", TypeRef::String)]);
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_type(&ty, &mut out, &mut imports);
        assert!(
            out.contains("final class Token {"),
            "DTO must use `final class` modifier: {out}"
        );
    }
}
