use std::borrow::Cow;

use crate::backends::go::type_map::{go_field_type, go_optional_field_type};
use crate::codegen::c_consumer;
use crate::codegen::naming::{go_type_name, to_go_name, wire_field_name};
use crate::codegen::shared::binding_fields;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};

use super::helpers::{emit_type_doc, is_tuple_field, needs_omitempty_pointer};

pub(in crate::backends::go::gen_bindings) fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let type_snake = crate::backends::go::c_symbols::type_component(&typ.name);
    let go_name = go_type_name(&typ.name);
    let c_type = format!("{}{}", c_consumer::export_type_prefix(ffi_prefix), typ.name);

    crate::backends::go::template_env::render(
        "opaque_type.jinja",
        crate::alef_context! {
            go_name => go_name,
            ffi_prefix => ffi_prefix,
            type_snake => type_snake,
            c_type => c_type,
        },
    )
}

/// Generate only the `Free()` method for an opaque handle type whose struct definition
/// was already emitted by `gen_go_error_types`.
///
/// Error types share their name with their corresponding opaque handle (the C layer allocates
/// a `SampleLlmError*` handle that the Go binding holds as an opaque pointer). However the Go
/// error struct uses `Code`/`Message` string fields rather than a raw `ptr unsafe.Pointer`, so
/// we cannot generate the normal `Free()` using `h.ptr`. Instead we emit an unexported stub
/// that references the C symbols to keep them from being pruned, but does nothing at runtime —
/// Go error values are not heap-allocated C objects from the binding's perspective.
pub(in crate::backends::go::gen_bindings) fn gen_opaque_type_free_only(typ: &TypeDef, _ffi_prefix: &str) -> String {
    let _ = typ;
    String::new()
}

/// Exported Go field names that [`gen_struct_type`] emits for `typ`.
///
/// Go rejects a struct that has both a field and a method named `X`, so the method-wrapper
/// pass consults this set and drops instance methods that would shadow a field. The filtering
/// here must stay in step with the field loop in [`gen_struct_type`]. ~keep
pub(in crate::backends::go::gen_bindings) fn go_struct_field_names(typ: &TypeDef) -> std::collections::HashSet<String> {
    binding_fields(&typ.fields)
        .filter(|field| !is_tuple_field(field))
        .map(|field| to_go_name(&field.name))
        .collect()
}

pub(crate) fn go_struct_field_type(
    typ: &TypeDef,
    field: &FieldDef,
    enum_names: &std::collections::HashSet<&str>,
    passthrough_enum_names: &std::collections::HashSet<&str>,
    data_enum_names: &std::collections::HashSet<&str>,
    struct_names: &std::collections::HashSet<&str>,
) -> Cow<'static, str> {
    let use_default_pointer = !field.optional && needs_omitempty_pointer(typ, field, struct_names);
    let named_type = direct_named_field_type(&field.ty);
    let is_sealed_interface = named_type.is_some_and(|name| data_enum_names.contains(name));
    let is_unresolved_named = named_type.is_some_and(|name| {
        !enum_names.contains(name)
            && !passthrough_enum_names.contains(name)
            && !data_enum_names.contains(name)
            && !struct_names.contains(name)
    });

    if is_unresolved_named {
        Cow::Borrowed("*json.RawMessage")
    } else if let Some(name) = named_type.filter(|_| is_sealed_interface) {
        Cow::Owned(go_type_name(name))
    } else if field.optional || use_default_pointer {
        go_optional_field_type(field)
    } else {
        go_field_type(field)
    }
}

fn direct_named_field_type(field_type: &TypeRef) -> Option<&str> {
    match field_type {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn go_struct_field_json_tag(
    typ: &TypeDef,
    field: &FieldDef,
    sets: &GoStructTypeSets<'_>,
    bytes_shadow: bool,
) -> String {
    let use_default_pointer = !field.optional && needs_omitempty_pointer(typ, field, sets.structs);
    let named_enum_default = !field.optional
        && !use_default_pointer
        && (field.default.is_some() || typ.serde_container_default)
        && matches!(&field.ty, TypeRef::Named(name) if sets.enums.contains(name.as_str()));
    let unresolved_named = direct_named_field_type(&field.ty).is_some_and(|name| {
        !sets.enums.contains(name)
            && !sets.passthrough_enums.contains(name)
            && !sets.data_enums.contains(name)
            && !sets.structs.contains(name)
    });
    let collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
    let omit = !(bytes_shadow && matches!(&field.ty, TypeRef::Bytes))
        && (field.optional || collection || use_default_pointer || named_enum_default || unresolved_named);
    let json_name = wire_field_name(
        &field.name,
        field.serde_rename.as_deref(),
        typ.serde_rename_all.as_deref(),
    );
    format!("json:\"{json_name}{}\"", if omit { ",omitempty" } else { "" })
}

struct GoStructTypeSets<'a> {
    enums: &'a std::collections::HashSet<&'a str>,
    passthrough_enums: &'a std::collections::HashSet<&'a str>,
    data_enums: &'a std::collections::HashSet<&'a str>,
    structs: &'a std::collections::HashSet<&'a str>,
}

fn render_struct_field(typ: &TypeDef, field: &FieldDef, sets: &GoStructTypeSets<'_>) -> String {
    let field_type = go_struct_field_type(
        typ,
        field,
        sets.enums,
        sets.passthrough_enums,
        sets.data_enums,
        sets.structs,
    );
    let json_tag = go_struct_field_json_tag(typ, field, sets, false);
    let doc_lines: Vec<&str> = field.doc.lines().map(str::trim).collect();
    crate::backends::go::template_env::render(
        "struct_field.jinja",
        crate::alef_context! {
            doc_lines => doc_lines,
            field_name => to_go_name(&field.name),
            field_type => &field_type,
            json_tag => &json_tag,
        },
    )
}

fn render_struct_fields(typ: &TypeDef, sets: &GoStructTypeSets<'_>, trait_bridges: &[TraitBridgeConfig]) -> String {
    let mut out = String::new();
    for field in binding_fields(&typ.fields).filter(|field| !is_tuple_field(field)) {
        if is_options_field_bridge_field(typ, field, trait_bridges) {
            let doc_lines: Vec<&str> = field.doc.lines().map(str::trim).collect();
            if !doc_lines.is_empty() {
                out.push_str(&crate::backends::go::template_env::render(
                    "visitor_field_doc.jinja",
                    crate::alef_context! { doc_lines => &doc_lines },
                ));
            }
            out.push_str(&crate::backends::go::template_env::render(
                "visitor_field.jinja",
                crate::alef_context! { field_name => to_go_name(&field.name) },
            ));
            out.push('\n');
        } else {
            out.push_str(&render_struct_field(typ, field, sets));
        }
    }
    out
}

fn render_marshal_aux_fields(
    typ: &TypeDef,
    sets: &GoStructTypeSets<'_>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::new();
    for field in binding_fields(&typ.fields).filter(|field| !is_tuple_field(field)) {
        if is_options_field_bridge_field(typ, field, trait_bridges) {
            continue;
        }
        let field_type = if matches!(&field.ty, TypeRef::Bytes) {
            Cow::Borrowed("[]int")
        } else {
            go_struct_field_type(
                typ,
                field,
                sets.enums,
                sets.passthrough_enums,
                sets.data_enums,
                sets.structs,
            )
        };
        let json_tag = go_struct_field_json_tag(typ, field, sets, true);
        out.push_str(&crate::backends::go::template_env::render(
            "struct_marshal_aux_field.jinja",
            crate::alef_context! { field_name => to_go_name(&field.name), field_type => &field_type, json_tag => &json_tag },
        ));
    }
    out
}

fn render_marshal_aux_assignments(
    typ: &TypeDef,
    sets: &GoStructTypeSets<'_>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::new();
    for field in binding_fields(&typ.fields).filter(|field| !is_tuple_field(field)) {
        if is_options_field_bridge_field(typ, field, trait_bridges) {
            continue;
        }
        let go_field = to_go_name(&field.name);
        let template = if matches!(&field.ty, TypeRef::Bytes) {
            let use_default_pointer = !field.optional && needs_omitempty_pointer(typ, field, sets.structs);
            if field.optional || use_default_pointer {
                "struct_marshal_bytes_field_pointer.jinja"
            } else {
                "struct_marshal_bytes_field_nonpointer.jinja"
            }
        } else {
            "struct_marshal_regular_field.jinja"
        };
        out.push_str(&crate::backends::go::template_env::render(
            template,
            crate::alef_context! { go_field => &go_field },
        ));
    }
    out
}

fn render_struct_marshal_json(
    typ: &TypeDef,
    go_name: &str,
    sets: &GoStructTypeSets<'_>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    if !binding_fields(&typ.fields).any(|field| !is_tuple_field(field) && matches!(&field.ty, TypeRef::Bytes)) {
        return String::new();
    }
    let mut out = String::new();
    out.push('\n');
    out.push_str(&crate::backends::go::template_env::render(
        "struct_marshal_json_header.jinja",
        crate::alef_context! { go_name => go_name },
    ));
    out.push_str(&render_marshal_aux_fields(typ, sets, trait_bridges));
    out.push_str(&crate::backends::go::template_env::render(
        "struct_marshal_aux_init.jinja",
        minijinja::Value::default(),
    ));
    out.push_str(&render_marshal_aux_assignments(typ, sets, trait_bridges));
    out.push_str(&crate::backends::go::template_env::render(
        "struct_marshal_json_footer.jinja",
        minijinja::Value::default(),
    ));
    out
}

struct DataEnumField {
    go_name: String,
    enum_go_name: String,
    is_slice: bool,
}

fn data_enum_fields(
    typ: &TypeDef,
    data_enum_names: &std::collections::HashSet<&str>,
    trait_bridges: &[TraitBridgeConfig],
) -> Vec<DataEnumField> {
    binding_fields(&typ.fields)
        .filter(|field| !is_tuple_field(field))
        .filter(|field| !is_options_field_bridge_field(typ, field, trait_bridges))
        .filter_map(|field| {
            let (name, is_slice) = match &field.ty {
                TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => (name, false),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => (name, false),
                    _ => return None,
                },
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => (name, true),
                    _ => return None,
                },
                _ => return None,
            };
            Some(DataEnumField {
                go_name: to_go_name(&field.name),
                enum_go_name: go_type_name(name),
                is_slice,
            })
        })
        .collect()
}

fn render_unmarshal_raw_fields(
    typ: &TypeDef,
    fields: &[DataEnumField],
    sets: &GoStructTypeSets<'_>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::new();
    for field in binding_fields(&typ.fields).filter(|field| !is_tuple_field(field)) {
        if is_options_field_bridge_field(typ, field, trait_bridges) {
            continue;
        }
        out.push_str(&render_unmarshal_raw_field(typ, field, fields, sets));
    }
    out
}

fn render_unmarshal_raw_field(
    typ: &TypeDef,
    field: &FieldDef,
    fields: &[DataEnumField],
    sets: &GoStructTypeSets<'_>,
) -> String {
    let go_name = to_go_name(&field.name);
    let data_enum = fields.iter().find(|definition| definition.go_name == go_name);
    let field_type = data_enum.map_or_else(
        || {
            go_struct_field_type(
                typ,
                field,
                sets.enums,
                sets.passthrough_enums,
                sets.data_enums,
                sets.structs,
            )
        },
        |definition| {
            Cow::Borrowed(if definition.is_slice {
                "[]json.RawMessage"
            } else {
                "json.RawMessage"
            })
        },
    );
    let json_tag = data_enum.map_or_else(
        || go_struct_field_json_tag(typ, field, sets, false),
        |_| {
            let json_name = wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                typ.serde_rename_all.as_deref(),
            );
            format!("json:\"{json_name},omitempty\"")
        },
    );
    crate::backends::go::template_env::render(
        "struct_unmarshal_raw_field.jinja",
        crate::alef_context! { go_field_name => &go_name, field_type => &field_type, json_tag => &json_tag },
    )
}

fn render_unmarshal_assignments(
    typ: &TypeDef,
    fields: &[DataEnumField],
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::new();
    for field in binding_fields(&typ.fields).filter(|field| !is_tuple_field(field)) {
        if is_options_field_bridge_field(typ, field, trait_bridges) {
            continue;
        }
        let go_name = to_go_name(&field.name);
        if fields.iter().all(|definition| definition.go_name != go_name) {
            out.push_str(&crate::backends::go::template_env::render(
                "struct_unmarshal_copy_field.jinja",
                crate::alef_context! { go_field_name => &go_name },
            ));
        }
    }
    for field in fields {
        let template = if field.is_slice {
            "struct_unmarshal_data_enum_slice.jinja"
        } else {
            "struct_unmarshal_data_enum_value.jinja"
        };
        out.push_str(&crate::backends::go::template_env::render(
            template,
            crate::alef_context! {
                go_name => &field.go_name,
                enum_go_name => &field.enum_go_name,
                unmarshal_fn => format!("Unmarshal{}", field.enum_go_name),
            },
        ));
    }
    out
}

fn render_struct_unmarshal_json(
    typ: &TypeDef,
    go_name: &str,
    sets: &GoStructTypeSets<'_>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let fields = data_enum_fields(typ, sets.data_enums, trait_bridges);
    if fields.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push('\n');
    out.push_str(&crate::backends::go::template_env::render(
        "struct_unmarshal_json_header.jinja",
        crate::alef_context! { go_name => go_name },
    ));
    out.push_str(&render_unmarshal_raw_fields(typ, &fields, sets, trait_bridges));
    out.push_str(&crate::backends::go::template_env::render(
        "struct_unmarshal_after_raw.jinja",
        minijinja::Value::default(),
    ));
    out.push_str(&render_unmarshal_assignments(typ, &fields, trait_bridges));
    out.push_str(&crate::backends::go::template_env::render(
        "struct_unmarshal_json_footer.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a Go struct type definition with json tags for marshaling.
/// Accepts enum_names (unit enums), passthrough_enum_names (untagged enums emitted
/// as `json.RawMessage`-backed named types) and data_enum_names (sealed-interface enums).
/// If any field has a data_enum type, emits custom UnmarshalJSON to dispatch to UnmarshalX().
pub(in crate::backends::go::gen_bindings) fn gen_struct_type(
    typ: &TypeDef,
    enum_names: &std::collections::HashSet<&str>,
    passthrough_enum_names: &std::collections::HashSet<&str>,
    data_enum_names: &std::collections::HashSet<&str>,
    struct_names: &std::collections::HashSet<&str>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(1024);

    let go_name = go_type_name(&typ.name);
    emit_type_doc(&mut out, &go_name, &typ.doc, "is a type.");
    out.push_str(&crate::backends::go::template_env::render(
        "struct_type_decl.jinja",
        crate::alef_context! {
            name => &go_name,
        },
    ));

    let sets = GoStructTypeSets {
        enums: enum_names,
        passthrough_enums: passthrough_enum_names,
        data_enums: data_enum_names,
        structs: struct_names,
    };
    out.push_str(&render_struct_fields(typ, &sets, trait_bridges));

    out.push_str(&crate::backends::go::template_env::render(
        "struct_type_end.jinja",
        minijinja::Value::default(),
    ));

    out.push_str(&render_struct_marshal_json(typ, &go_name, &sets, trait_bridges));
    out.push_str(&render_struct_unmarshal_json(typ, &go_name, &sets, trait_bridges));

    out
}

pub(super) fn is_options_field_bridge_field(
    typ: &TypeDef,
    field: &FieldDef,
    trait_bridges: &[TraitBridgeConfig],
) -> bool {
    let Some(field_type) = named_type_ref(&field.ty) else {
        return false;
    };
    trait_bridges.iter().any(|bridge| {
        bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(typ.name.as_str())
            && bridge.resolved_options_field() == Some(field.name.as_str())
            && bridge.type_alias.as_deref() == Some(field_type)
    })
}

fn named_type_ref(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_ref(inner),
        _ => None,
    }
}
