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

    // ConfigOption type definition
    let go_name = go_type_name(&typ.name);
    out.push_str(&crate::backends::go::template_env::render(
        "config_option_type_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));
    out.push('\n');

    // Generate WithFieldName constructors for each field
    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);

        // Match the struct's special-cased visitor field (typed as the user-facing
        // `Visitor` interface, not the opaque `VisitorHandle`). The With option must
        // accept Visitor too — passing a VisitorHandle and assigning &v yielded a
        // *VisitorHandle, which doesn't satisfy the Visitor interface and broke the
        // Go build whenever the visitor pattern was active.
        let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);

        // For the function parameter, always accept the direct type (not wrapped in optional)
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
        // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults) both
        // store pointer types in the struct, so we must take the address of v when assigning.
        // Exception: slice (Vec) and map types are reference types in Go — go_optional_type
        // returns []T and map[K]V (not *[]T / *map[K]V), so no address-of is needed.
        // Sealed-interface (data enum) fields are also already-nullable interface values; their
        // struct field is `T` (not `*T`), so the assignment must not take the address.
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

    // Generate NewConfig constructor
    out.push_str(&crate::backends::go::template_env::render(
        "config_new_constructor_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));

    // Set default values for fields
    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);
        let default_val = if field.optional || needs_omitempty_pointer(field) {
            // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults)
            // are pointer types. Set to nil so they serialize as absent, letting Rust serde
            // fill in the real default instead of seeing a Go zero value.
            "nil".to_string()
        } else {
            let mut val = crate::codegen::config_gen::default_value_for_field(field, "go");
            // Passthrough json.RawMessage-backed enum: zero value is `nil` (a nil
            // []byte slice). Override unconditionally — config_gen would otherwise
            // return `""` for `String` defaults baked into the IR for these types.
            if let TypeRef::Named(name) = &field.ty {
                if passthrough_enum_names.contains(name.as_str()) {
                    val = "nil".to_string();
                }
            }
            // config_gen returns "nil" for Named types with Empty default, but in Go
            // non-optional Named types are value types. Fix up based on whether the
            // Named type is a string-based enum or a struct.
            if val == "nil" {
                if let TypeRef::Named(name) = &field.ty {
                    if passthrough_enum_names.contains(name.as_str()) {
                        // already handled above; keep nil
                    } else if enum_names.contains(name.as_str()) {
                        // String-typed enum — zero value is empty string
                        val = "\"\"".to_string();
                    } else if data_enum_names.contains(name.as_str()) {
                        // Sealed-interface (data enum) — zero value is nil interface.
                        // Composite literal `T{}` is invalid for interface types.
                        val = "nil".to_string();
                    } else {
                        // Struct — zero value is TypeName{}
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
