use minijinja::context;

use crate::backends::go::type_map::go_type;
use crate::codegen::naming::{go_type_name, to_go_name};
use crate::codegen::shared::binding_fields;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{TypeDef, TypeRef};

use super::helpers::{is_tuple_field, needs_omitempty_pointer};
use super::structs::is_options_field_bridge_field;

pub(in crate::backends::go::gen_bindings) fn gen_config_options(
    typ: &TypeDef,
    enum_names: &std::collections::HashSet<&str>,
    passthrough_enum_names: &std::collections::HashSet<&str>,
    data_enum_names: &std::collections::HashSet<&str>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(2048);

    let go_name = go_type_name(&typ.name);
    out.push_str(&crate::backends::go::template_env::render(
        "config_option_type_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));
    out.push('\n');

    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);

        let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);

        let param_type = if is_visitor_field {
            std::borrow::Cow::Borrowed("Visitor")
        } else {
            go_type(&field.ty)
        };

        out.push_str(&crate::backends::go::template_env::render(
            "config_with_option_comment.jinja",
            context! {
                go_name => &go_name,
                field_go_name => &field_go_name,
                field_name => &field.name,
            },
        ));
        let is_slice_or_map = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
        let is_sealed_interface = matches!(&field.ty, TypeRef::Named(n) if data_enum_names.contains(n.as_str()));
        let use_ptr = !is_visitor_field
            && (field.optional || needs_omitempty_pointer(field))
            && !is_slice_or_map
            && !is_sealed_interface;
        let assign_val = if use_ptr { "&v" } else { "v" };
        out.push_str(&crate::backends::go::template_env::render(
            "config_with_option_signature.jinja",
            context! {
                go_name => &go_name,
                field_go_name => &field_go_name,
                param_type => param_type.as_ref(),
                assign_val => assign_val,
            },
        ));
        out.push('\n');
    }

    out.push_str(&crate::backends::go::template_env::render(
        "config_new_constructor_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));

    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);
        let default_val = if field.optional || needs_omitempty_pointer(field) {
            "nil".to_string()
        } else {
            let mut val = crate::codegen::config_gen::default_value_for_field(field, "go");
            if let TypeRef::Named(name) = &field.ty {
                if passthrough_enum_names.contains(name.as_str()) {
                    val = "nil".to_string();
                }
            }
            if val == "nil" {
                if let TypeRef::Named(name) = &field.ty {
                    if passthrough_enum_names.contains(name.as_str()) {
                    } else if enum_names.contains(name.as_str()) {
                        val = "\"\"".to_string();
                    } else if data_enum_names.contains(name.as_str()) {
                        val = "nil".to_string();
                    } else {
                        val = format!("{}{{}}", go_type_name(name));
                    }
                }
            }
            val
        };
        out.push_str(&crate::backends::go::template_env::render(
            "config_default_field.jinja",
            context! {
                field_go_name => &field_go_name,
                default_val => &default_val,
            },
        ));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "config_new_constructor_footer.jinja",
        minijinja::Value::default(),
    ));

    out
}
