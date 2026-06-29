use std::borrow::Cow;

use heck::ToSnakeCase;
use minijinja::context;

use crate::backends::go::type_map::{go_optional_type, go_type};
use crate::codegen::naming::{apply_serde_rename_all, go_type_name, to_go_name};
use crate::codegen::shared::binding_fields;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};

use super::helpers::{emit_type_doc, is_tuple_field, needs_omitempty_pointer};

pub(in crate::backends::go::gen_bindings) fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let go_name = go_type_name(&typ.name);
    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), typ.name);

    crate::backends::go::template_env::render(
        "opaque_type.jinja",
        context! {
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
    // Nothing to emit — the structured error type already has its Error() method and
    // the C-level free function is invoked transparently inside the FFI layer.
    // Returning an empty string avoids a duplicate struct definition and a broken Free().
    let _ = typ;
    String::new()
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
        minijinja::context! {
            name => &go_name,
        },
    ));

    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        // Special handling for Visitor field: use Visitor interface, not a handle type,
        // and mark as json:"-" since it's not serializable
        let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);

        if is_visitor_field {
            let doc_lines: Vec<&str> = if !field.doc.is_empty() {
                field.doc.lines().map(|l| l.trim()).collect()
            } else {
                vec![]
            };
            if !doc_lines.is_empty() {
                out.push_str(&crate::backends::go::template_env::render(
                    "visitor_field_doc.jinja",
                    minijinja::context! {
                        doc_lines => &doc_lines,
                    },
                ));
            }
            out.push_str(&crate::backends::go::template_env::render(
                "visitor_field.jinja",
                minijinja::context! {
                    field_name => to_go_name(&field.name),
                },
            ));
            out.push('\n');
            continue;
        }

        // A non-optional field in a defaulted struct may still need pointer+omitempty when
        // the Go zero value differs from the Rust Default value (e.g., Duration, bool true, int != 0).
        let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);

        // Named types that map to Go string enums must also use omitempty (without pointer),
        // because the Go zero value "" is never a valid Rust enum variant. Without omitempty,
        // marshaling an empty struct sends `"field": ""` which fails Rust serde deserialization.
        let is_named_enum = !field.optional
            && !use_default_pointer
            && typ.has_default
            && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));

        // Sealed-interface enums are already nullable in Go (interface zero value is nil) —
        // they must never be wrapped in a pointer. `*AuthConfig` is "pointer to interface",
        // not "interface", and the two are not assignable. Emit the bare interface name
        // for both optional and non-optional positions.
        let is_sealed_interface = matches!(&field.ty, TypeRef::Named(n) if data_enum_names.contains(n.as_str()));

        // Check if a Named type is unresolved (not in enum_names, passthrough_enum_names,
        // data_enum_names, or struct_names). For unresolved external types, emit
        // *json.RawMessage instead of a non-existent struct. Passthrough enums DO have a
        // generated Go named type (a `json.RawMessage`-backed wrapper), so they are
        // resolved — emitting the raw type here would diverge from the UnmarshalJSON
        // raw-mirror struct, which always uses the resolved named type.
        let is_unresolved_named = matches!(&field.ty, TypeRef::Named(n)
            if !enum_names.contains(n.as_str())
                && !passthrough_enum_names.contains(n.as_str())
                && !data_enum_names.contains(n.as_str())
                && !struct_names.contains(n.as_str()));

        let field_type = if is_unresolved_named {
            // Unresolved external-crate Named types: use *json.RawMessage as fallback
            Cow::Borrowed("*json.RawMessage")
        } else if is_sealed_interface {
            go_type(&field.ty)
        } else if field.optional {
            go_optional_type(&field.ty)
        } else if use_default_pointer {
            // Emit as pointer so that an unset field serializes as absent (omitempty),
            // letting Rust serde fill in the real default instead of seeing a zero value.
            go_optional_type(&field.ty)
        } else {
            go_type(&field.ty)
        };

        // Determine json tag - apply serde rename_all strategy.
        // Use omitempty for optional fields, slice/map types (nil slices serialize to null
        // in Go, which breaks Rust serde deserialization expecting an array), fields
        // where the Go zero value differs from the Rust Default value, and string enum
        // fields where "" is never a valid Rust enum variant.
        // Per-field `#[serde(rename = "...")]` wins over `rename_all`.
        let json_name = field
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_serde_rename_all(&field.name, typ.serde_rename_all.as_deref()));
        let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
        let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum || is_unresolved_named
        {
            format!("json:\"{},omitempty\"", json_name)
        } else {
            format!("json:\"{}\"", json_name)
        };

        let doc_lines: Vec<&str> = if !field.doc.is_empty() {
            field.doc.lines().map(|l| l.trim()).collect()
        } else {
            vec![]
        };
        out.push_str(&crate::backends::go::template_env::render(
            "struct_field.jinja",
            minijinja::context! {
                doc_lines => doc_lines,
                field_name => to_go_name(&field.name),
                field_type => &field_type,
                json_tag => &json_tag,
            },
        ));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "struct_type_end.jinja",
        minijinja::Value::default(),
    ));

    // If any field is a `[]byte` (Vec<u8>), emit custom MarshalJSON so the bytes
    // serialize as a JSON array of integers — matching what Rust's serde
    // `Vec<u8>` deserializer expects. Go's default `json.Marshal([]byte)` emits
    // base64, which Rust's `Deserialize for Vec<u8>` rejects with
    // `invalid type: string "...", expected a sequence`.
    let bytes_fields: Vec<&crate::core::ir::FieldDef> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| !is_tuple_field(f) && matches!(&f.ty, TypeRef::Bytes))
        .collect();
    if !bytes_fields.is_empty() {
        out.push('\n');
        out.push_str(&crate::backends::go::template_env::render(
            "struct_marshal_json_header.jinja",
            context! {
                go_name => &go_name,
            },
        ));
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);
            if is_visitor_field {
                continue;
            }
            let go_field = to_go_name(&field.name);
            // Per-field `#[serde(rename = "...")]` wins over `rename_all`.
            let json_name = field
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_serde_rename_all(&field.name, typ.serde_rename_all.as_deref()));
            let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
            let is_named_enum = !field.optional
                && !use_default_pointer
                && typ.has_default
                && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
            let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
            // Bytes fields must never carry omitempty: an empty Vec<u8> must serialize as `[]`,
            // not be omitted. Rust serde for Vec<u8> rejects a missing/null value differently
            // from an empty sequence, so always emit the tag without omitempty.
            let is_bytes = matches!(&field.ty, TypeRef::Bytes);
            let json_tag = if !is_bytes && (field.optional || is_collection || use_default_pointer || is_named_enum) {
                format!("json:\"{},omitempty\"", json_name)
            } else {
                format!("json:\"{}\"", json_name)
            };
            let go_field_type: String = if matches!(&field.ty, TypeRef::Bytes) {
                "[]int".to_string()
            } else if field.optional || use_default_pointer {
                go_optional_type(&field.ty).to_string()
            } else {
                go_type(&field.ty).to_string()
            };
            out.push_str(&crate::backends::go::template_env::render(
                "struct_marshal_aux_field.jinja",
                context! {
                    field_name => &go_field,
                    field_type => &go_field_type,
                    json_tag => &json_tag,
                },
            ));
        }
        out.push_str(&crate::backends::go::template_env::render(
            "struct_marshal_aux_init.jinja",
            minijinja::Value::default(),
        ));
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);
            if is_visitor_field {
                continue;
            }
            let go_field = to_go_name(&field.name);
            if matches!(&field.ty, TypeRef::Bytes) {
                let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
                let is_pointer = field.optional || use_default_pointer;
                if is_pointer {
                    // Optional `*[]byte` field: only encode when non-nil.
                    out.push_str(&crate::backends::go::template_env::render(
                        "struct_marshal_bytes_field_pointer.jinja",
                        context! {
                            go_field => &go_field,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::go::template_env::render(
                        "struct_marshal_bytes_field_nonpointer.jinja",
                        context! {
                            go_field => &go_field,
                        },
                    ));
                }
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_marshal_regular_field.jinja",
                    context! {
                        go_field => &go_field,
                    },
                ));
            }
        }
        out.push_str(&crate::backends::go::template_env::render(
            "struct_marshal_json_footer.jinja",
            minijinja::Value::default(),
        ));
    }

    // Collect fields whose type is a sealed-interface data enum (either direct or optional).
    // These cannot be unmarshalled by Go's default json.Unmarshal (interface types are opaque),
    // so we emit a custom UnmarshalJSON that reads every data-enum field as json.RawMessage
    // first, then dispatches via the generated UnmarshalX() helper.
    struct DataEnumField {
        go_name: String,
        enum_go_name: String,
        is_optional: bool,
        is_slice: bool,
    }
    let data_enum_fields: Vec<DataEnumField> = binding_fields(&typ.fields)
        .filter(|f| !is_tuple_field(f))
        .filter(|f| !is_options_field_bridge_field(typ, f, trait_bridges))
        .filter_map(|f| {
            // Determine the inner Named type name, and whether the field is optional
            // and/or a slice. Slices of data enums (e.g. `Vec<RerankDocument>` where
            // `RerankDocument` is `#[serde(untagged)]`) need per-element dispatch
            // through the `Unmarshal<Enum>` helper — Go's default unmarshal of a
            // JSON array into `[]<sealed-interface>` fails because Go interfaces
            // are opaque to encoding/json.
            let (enum_name_str, is_optional, is_slice) = match &f.ty {
                TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), false, false),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), true, false),
                    _ => return None,
                },
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), false, true),
                    _ => return None,
                },
                _ => return None,
            };
            Some(DataEnumField {
                go_name: to_go_name(&f.name),
                enum_go_name: go_type_name(enum_name_str),
                is_optional,
                is_slice,
            })
        })
        .collect();

    if !data_enum_fields.is_empty() {
        out.push('\n');
        // Emit: func (s *StructName) UnmarshalJSON(data []byte) error {
        out.push_str(&crate::backends::go::template_env::render(
            "struct_unmarshal_json_header.jinja",
            minijinja::context! {
                go_name => &go_name,
            },
        ));

        // Emit the anonymous helper struct with all fields,
        // replacing data-enum fields with json.RawMessage.
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);
            if is_visitor_field {
                continue;
            }
            let go_field_name = to_go_name(&field.name);
            let json_name = field
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_serde_rename_all(&field.name, typ.serde_rename_all.as_deref()));
            // Check if this field is a data enum field (direct, optional, or slice).
            let data_enum_def = data_enum_fields.iter().find(|def| def.go_name == go_field_name);
            if let Some(def) = data_enum_def {
                // For slice fields we keep the array shape so we can iterate
                // per-element; scalar/optional fields collapse to a single
                // json.RawMessage. Both use omitempty — nil-length checks
                // guard the decode loop below.
                let raw_type = if def.is_slice {
                    "[]json.RawMessage"
                } else {
                    "json.RawMessage"
                };
                let json_tag = format!("json:\"{json_name},omitempty\"");
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_raw_field.jinja",
                    minijinja::context! {
                        go_field_name => &go_field_name,
                        field_type => raw_type,
                        json_tag => &json_tag,
                    },
                ));
            } else {
                // Use the normal field type and tag.
                let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
                let is_named_enum = !field.optional
                    && !use_default_pointer
                    && typ.has_default
                    && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
                let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
                let field_type = if field.optional || use_default_pointer {
                    go_optional_type(&field.ty)
                } else {
                    go_type(&field.ty)
                };
                let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum {
                    format!("json:\"{json_name},omitempty\"")
                } else {
                    format!("json:\"{json_name}\"")
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_raw_field.jinja",
                    minijinja::context! {
                        go_field_name => &go_field_name,
                        field_type => &field_type,
                        json_tag => &json_tag,
                    },
                ));
            }
        }
        out.push_str(&crate::backends::go::template_env::render(
            "struct_unmarshal_after_raw.jinja",
            minijinja::Value::default(),
        ));

        // Copy all non-data-enum fields.
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field = is_options_field_bridge_field(typ, field, trait_bridges);
            if is_visitor_field {
                continue;
            }
            let go_field_name = to_go_name(&field.name);
            let is_data_enum = data_enum_fields.iter().any(|def| def.go_name == go_field_name);
            if !is_data_enum {
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_copy_field.jinja",
                    minijinja::context! {
                        go_field_name => &go_field_name,
                    },
                ));
            }
        }

        // Decode each data-enum field via its UnmarshalX helper.
        for def in &data_enum_fields {
            let unmarshal_fn = format!("Unmarshal{}", def.enum_go_name);
            if def.is_slice {
                // Slice field: iterate over the JSON array and dispatch per element
                // via the generated UnmarshalX helper. The struct field type is
                // `[]<sealed-interface>`, which encoding/json cannot populate
                // directly from a heterogeneous JSON array (interfaces are opaque).
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_data_enum_slice.jinja",
                    minijinja::context! {
                        go_name => &def.go_name,
                        enum_go_name => &def.enum_go_name,
                        unmarshal_fn => &unmarshal_fn,
                    },
                ));
            } else if def.is_optional {
                // Optional field: only decode when the raw bytes are non-nil/non-empty and not "null".
                // The struct field type is the bare sealed-interface (no `*`), since
                // Go interfaces are already nullable — so assign `v` directly.
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_data_enum_value.jinja",
                    minijinja::context! {
                        go_name => &def.go_name,
                        unmarshal_fn => &unmarshal_fn,
                    },
                ));
            } else {
                // Required field: always decode (raw is guaranteed non-nil by the struct unmarshal above).
                out.push_str(&crate::backends::go::template_env::render(
                    "struct_unmarshal_data_enum_value.jinja",
                    minijinja::context! {
                        go_name => &def.go_name,
                        unmarshal_fn => &unmarshal_fn,
                    },
                ));
            }
        }

        out.push_str(&crate::backends::go::template_env::render(
            "struct_unmarshal_json_footer.jinja",
            minijinja::Value::default(),
        ));
    }

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
