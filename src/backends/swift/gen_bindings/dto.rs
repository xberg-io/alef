use crate::backends::swift::naming::{swift_rust_shim_ident as swift_ident, swift_source_ident as swift_case_ident};
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{DefaultValue, FieldDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{AsSnakeCase, ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;

pub(super) fn can_emit_first_class_struct(
    ty: &TypeDef,
    _mapper: &SwiftMapper,
    _exclude_fields: &HashSet<String>,
    known_dto_names: &HashSet<String>,
) -> bool {
    !ty.is_opaque
        && ty.has_serde
        && !ty.fields.is_empty()
        && binding_fields(&ty.fields).all(|field| first_class_field_supported(&field.ty, known_dto_names))
        // Note: we no longer require emit_into_rust_direct_call to succeed.  Types whose
        // fields include Vec<Named>, Named, or Map fall back to the JSON roundtrip path in
        // intoRust() automatically (the same `{type_snake}_from_json` shim is emitted by
        // the Rust crate side for all types where has_constructor_extern returns false).
        // The exclude_fields parameter is retained in the signature for the intoRust emitter.
        && !ty.fields.iter().all(|f| f.binding_excluded)
}

/// Returns `true` when a field type can be represented as a stored property in a first-class
/// Swift struct and marshaled through the RustBridge FFI layer.
///
/// Accepted:
/// - Primitives and Bool
/// - String
/// - Named(S) where S is a known DTO or enum in the same API
/// - Vec<T> where T is itself accepted (Vec<Primitive>, Vec<String>, Vec<Named(S)>)
/// - Optional<T> (via TypeRef::Optional) where T is accepted — covers Optional<String>,
///   Optional<Primitive>, Optional<Named>, Optional<Vec<T>>
/// - field.optional = true is handled at the call site; this function only sees the TypeRef
///
/// Rejected (fall through to typealias):
/// - Map<K, V> — bridge layer serialises maps to JSON String; per-field JSON decode is
///   complex and rarely worth the ergonomic gain vs the typealias
/// - Path, Bytes, Duration, Char, Json — not representable as idiomatic Swift stored props
///   without additional infra
pub(super) fn first_class_field_supported(ty: &TypeRef, known_dto_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) | TypeRef::String => true,
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Vec(inner) => first_class_field_supported(inner, known_dto_names),
        TypeRef::Optional(inner) => first_class_field_supported(inner, known_dto_names),
        // Map, Path, Bytes, Duration, Char, Json, Unit — not yet supported
        _ => false,
    }
}

/// Emits a first-class Swift struct (`public struct Foo: Codable, Sendable, Hashable`)
/// for a non-opaque DTO that has serde derives and at least one field.
///
/// Layer 1: public struct declaration with `public let` stored properties,
///          memberwise `public init` with default-nil optionals, and `CodingKeys`
///          when field names differ from their wire (serde snake_case) keys.
/// Layer 2: `internal extension Foo` with `init(_ rb: RustBridge.Foo) throws` and
///          `func intoRust() throws -> RustBridge.Foo` for FFI marshaling.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_first_class_struct(
    ty: &TypeDef,
    mapper: &SwiftMapper,
    exclude_fields: &HashSet<String>,
    known_dto_names: &HashSet<String>,
    unit_enum_names: &HashSet<String>,
    untagged_enum_names: &HashSet<String>,
    serde_struct_names: &HashSet<String>,
    error_type_name: &str,
    configured_features: &std::collections::HashSet<&str>,
    out: &mut String,
) {
    let type_name = &ty.name;
    let type_snake = AsSnakeCase(type_name.as_str()).to_string();

    // Emit `public let` stored properties.
    // The extractor unwraps Option<T> into (ty: T, optional: true) -- check field.optional
    // in addition to TypeRef::Optional to handle both IR representations correctly.
    //
    // Note: `swift_case_ident` is used here (and at every other site that emits a
    // Swift-side identifier) so that fields named after Swift reserved keywords
    // are wrapped in backticks (`` `default` ``) rather than escaped with a
    // trailing underscore.
    let visible_fields: Vec<_> = binding_fields(&ty.fields).collect();
    let mut properties = String::new();
    for field in &visible_fields {
        super::client::emit_doc_comment(&field.doc, "    ", &mut properties);
        let camel = swift_case_ident(&field.name.to_lower_camel_case());
        let already_optional = matches!(&field.ty, TypeRef::Optional(_));
        let swift_ty = mapper.map_type(&field.ty);
        let property_type = if field.optional && !already_optional {
            format!("{swift_ty}?")
        } else {
            swift_ty
        };
        properties.push_str(&crate::backends::swift::template_env::render(
            "swift_struct_property.swift.jinja",
            minijinja::context! {
                name => camel,
                ty => property_type,
            },
        ));
    }

    // Memberwise init with default-nil for Optional fields.
    let params: Vec<String> = visible_fields
        .iter()
        .map(|field| {
            let camel = swift_case_ident(&field.name.to_lower_camel_case());
            let already_optional = matches!(&field.ty, TypeRef::Optional(_));
            let swift_ty = mapper.map_type(&field.ty);
            if field.optional && !already_optional {
                format!("{camel}: {swift_ty}? = nil")
            } else if already_optional {
                format!("{camel}: {swift_ty} = nil")
            } else {
                format!("{camel}: {swift_ty}")
            }
        })
        .collect();
    let init_params = params.join(", ");
    let mut init_assignments = String::new();
    for field in &visible_fields {
        let camel = swift_case_ident(&field.name.to_lower_camel_case());
        init_assignments.push_str(&crate::backends::swift::template_env::render(
            "swift_self_assignment.swift.jinja",
            minijinja::context! {
                field => camel,
                expr => camel,
            },
        ));
    }

    // CodingKeys: emit when at least one field's camelCase name differs from the
    // serde wire key, OR when `ty.has_default` is true (the custom decoder below
    // references `CodingKeys.<camel>` for every field).
    let needs_coding_keys = ty.has_default
        || visible_fields.iter().any(|field| {
            let camel = field.name.to_lower_camel_case();
            camel != field.name
        });
    let mut coding_keys = String::new();
    if needs_coding_keys {
        for field in &visible_fields {
            let camel = swift_case_ident(&field.name.to_lower_camel_case());
            let wire_key = &field.name;
            coding_keys.push_str(&crate::backends::swift::template_env::render(
                "swift_coding_key.swift.jinja",
                minijinja::context! {
                    name => camel,
                    wire_key => wire_key,
                },
            ));
        }
    }

    // Custom `init(from decoder:)` when the Rust source has `#[derive(Default)]` or
    // a manual `impl Default`. Swift's auto-synthesized Codable decoder rejects JSON
    // that omits non-Optional declared properties; the custom init uses
    // `decodeIfPresent + ?? <fallback>` so JSON inputs from Rust serializers using
    // `#[serde(default)]` / `#[serde(skip_serializing_if = ...)]` decode successfully.
    let mut decoder_init = String::new();
    if ty.has_default {
        emit_decoder_init(mapper, &visible_fields, &mut decoder_init);
    }

    // init(_ rb: RustBridge.FooRef) throws
    //
    // Accept the `Ref` base class rather than the owned class so callers can
    // pass either: an owned `RustBridge.Foo` (which is-a `RustBridge.FooRef`
    // via swift-bridge's class hierarchy `Foo: FooRefMut: FooRef`) or a
    // `FooRef` borrowed out of a `RustVec<Foo>` iteration. Without this, code
    // like `try rb.layouts().map { try Layout($0) }` fails to compile with
    // `cannot convert value of type 'LayoutRef' to expected argument type 'Layout'`.
    //
    // The Swift-side property name uses backtick escape (`swift_case_ident`),
    // while the RustBridge accessor uses the trailing-underscore form
    // (`swift_ident`) to match the swift-bridge-generated Rust method name —
    // see `gen_rust_crate::extern_block::emit_extern_block_for_type`.
    let mut ffi_init_assignments = String::new();
    for field in &visible_fields {
        let swift_field = swift_case_ident(&field.name.to_lower_camel_case());
        // The RustBridge accessor name now follows lowerCamelCase because the
        // swift-bridge getter extern emits `swift_name = "<camel>"` (see
        // `gen_rust_crate::extern_block::emit_extern_block_for_type`).
        let rust_accessor = swift_ident(&field.name.to_lower_camel_case());
        let is_optional = field.optional || matches!(&field.ty, TypeRef::Optional(_));

        // If the wrapper-side getter is unbridgeable (skipped by
        // `is_unbridgeable_getter`), the Rust crate emits no accessor at all —
        // calling `rb.{field}()` would fail to compile. Fall back to a sensible
        // default: nil for optional fields, JSONDecoder roundtrip of an empty
        // value for required ones (the public struct still surfaces the field
        // for direct construction / JSON decode paths).
        let expr = if is_field_unbridgeable_for_init(ty, field, exclude_fields, known_dto_names) {
            if is_optional {
                "nil".to_string()
            } else {
                // Fields here are non-optional and unbridgeable — extraordinarily rare
                // in practice. Emit a JSONDecoder-of-default placeholder so the file
                // compiles; the JSON path on `intoRust()` covers the round-trip.
                let swift_ty = mapper.map_type(&field.ty);
                format!("try JSONDecoder().decode({swift_ty}.self, from: Data(\"null\".utf8))")
            }
        } else if is_vec_of_serde_struct(&field.ty, serde_struct_names) {
            // Vec<Struct> where Struct has serde: the Rust getter returns Vec<String> (JSON-encoded).
            // swift-bridge marshals this as RustVec<RustString>. Decode each element via JSONDecoder.
            //
            // Optional<Vec<Struct>> (field.optional=true with TypeRef::Vec) is bridged as plain String
            // (single JSON-encoded array, with "null" representing None).
            // Decode the entire array via JSONDecoder with optional handling.
            let swift_ty = mapper.map_type(&field.ty);
            let swift_ty_with_opt = if is_optional && !matches!(&field.ty, TypeRef::Optional(_)) {
                format!("{swift_ty}?")
            } else {
                swift_ty
            };

            // The field is optional if the field flag is set OR the type itself is `Optional(...)`.
            // `Option<Vec<Struct>>` reaches this arm in both shapes: `Vec` with the optional flag
            // (how most DTOs model it) and an explicit `Optional(Vec<Struct>)` type.
            let field_is_optional = is_optional || matches!(&field.ty, TypeRef::Optional(_));
            // The inner `Vec<Named>` struct name, reached through an optional wrapper if present.
            let inner_vec_named: Option<&str> = match &field.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(name) => Some(name.as_str()),
                    _ => None,
                },
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Vec(inner) => match inner.as_ref() {
                        TypeRef::Named(name) => Some(name.as_str()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };

            if field_is_optional {
                // Optional<Vec<Struct>> is bridged as String (single JSON-encoded array or "null").
                // The getter returns RustString (NOT optional), so no optional chaining on the getter.
                // Decode the whole array via JSONDecoder, which handles the "null" JSON value as None.
                let accessor_with_chain = format!("rb.{rust_accessor}().toString()");
                format!(
                    "try JSONDecoder().decode({swift_ty_with_opt}.self, from: \
                     (({accessor_with_chain}).data(using: .utf8) ?? Data(\"null\".utf8)))"
                )
            } else if let Some(inner_struct_name) = inner_vec_named {
                // Vec<Struct> (non-optional) is bridged as Vec<String>, which marshals as RustVec<RustString>.
                // Each element is a RustStringRef containing JSON, so decode each one.
                format!(
                    "try rb.{rust_accessor}().map {{ (s: RustStringRef) -> {inner_struct_name} in \
                     let d = s.as_str().toString().data(using: .utf8) ?? Data(); \
                     return try JSONDecoder().decode({inner_struct_name}.self, from: d) }}"
                )
            } else {
                format!("rb.{rust_accessor}()")
            }
        } else if is_untagged_enum_type(&field.ty, untagged_enum_names) {
            // Untagged-enum field (`#[serde(untagged)]`): the swift-bridge accessor returns a
            // `RustString` containing the JSON-serialized value rather than an opaque
            // `RustBridge.{Name}Ref`. There is no `init(_ rb: RustBridge.{Name}Ref)` for
            // untagged enums, so calling `try {Name}(rb.{field}())` produces a compile error
            // ("missing argument label 'from:'" / "RustString does not conform to Decoder").
            // Decode via JSONDecoder from the JSON payload instead.
            //
            // The accessor is `Optional<RustString>` only when the source field itself is
            // `Option<T>` — for non-optional fields, the bridge returns plain `RustString`
            // and applying `?.toString()` produces the inverse compile error
            // ("cannot use optional chaining on non-optional value of type 'RustString'").
            let swift_ty = mapper.map_type(&field.ty);
            let swift_ty_with_opt = if is_optional && !matches!(&field.ty, TypeRef::Optional(_)) {
                format!("{swift_ty}?")
            } else {
                swift_ty
            };
            let accessor_with_chain = if is_optional {
                format!("rb.{rust_accessor}()?.toString() ?? \"null\"")
            } else {
                format!("rb.{rust_accessor}().toString()")
            };
            format!(
                "try JSONDecoder().decode({swift_ty_with_opt}.self, from: \
                 (({accessor_with_chain}).data(using: .utf8) ?? Data(\"null\".utf8)))"
            )
        } else if needs_json_bridge_for_swift(&field.ty) {
            // Field is bridged as a JSON string at the Rust boundary — the getter
            // always returns a plain `RustString` whose contents are the JSON-encoded
            // value (`"null"` when the source field was None). Decode through
            // JSONDecoder so the Swift property receives the typed value.
            //
            // The accessor is plain `RustString` (NOT Optional) for JSON-bridged
            // fields regardless of whether the source field is `Option<T>` —
            // optionality is encoded as the JSON string "null". Applying `?.toString()`
            // produces a compile error ("cannot use optional chaining on non-optional
            // value of type 'RustString'").
            let swift_ty = mapper.map_type(&field.ty);
            let swift_ty_with_opt = if is_optional && !matches!(&field.ty, TypeRef::Optional(_)) {
                format!("{swift_ty}?")
            } else {
                swift_ty
            };
            let accessor_with_chain = format!("rb.{rust_accessor}().toString()");
            format!(
                "try JSONDecoder().decode({swift_ty_with_opt}.self, from: \
                 (({accessor_with_chain}).data(using: .utf8) ?? Data(\"null\".utf8)))"
            )
        } else {
            swift_ffi_read_expr(
                &field.ty,
                is_optional,
                &rust_accessor,
                known_dto_names,
                unit_enum_names,
                untagged_enum_names,
                error_type_name,
            )
        };
        ffi_init_assignments.push_str(&crate::backends::swift::template_env::render(
            "swift_self_assignment.swift.jinja",
            minijinja::context! {
                field => swift_field,
                expr => expr,
            },
        ));
    }

    // func intoRust() — prefer a direct `RustBridge.{Type}(...)` bulk-constructor call
    // when the swift-bridge `#[swift_bridge(init)] fn new(...)` extern is emitted for
    // this type AND every constructor field can be converted to its bridge argument
    // without a JSON detour. Falls back to the JSON roundtrip via the `{type}_from_json`
    // shim otherwise (e.g. fields needing JSON bridge, types without a Default impl).
    //
    // The bulk path mirrors the symmetric direct-field-access pattern in
    // `init(_ rb: RustBridge.{Type}) throws` above and avoids the JSONEncoder + Rust-side
    // `serde_json::from_str` work for primitive-only DTOs.
    let mut into_rust_body = String::new();
    let direct_call = emit_into_rust_direct_call(ty, mapper, exclude_fields, type_name, configured_features);
    match direct_call {
        Some(call) => into_rust_body.push_str(&call),
        None => {
            let from_json_fn = format!("{type_snake}_from_json").to_lower_camel_case();
            into_rust_body.push_str("        let data = try JSONEncoder().encode(self)\n");
            into_rust_body.push_str("        let json = String(data: data, encoding: .utf8) ?? \"{}\"\n");
            into_rust_body.push_str(&crate::backends::swift::template_env::render(
                "swift_into_rust_json_return.swift.jinja",
                minijinja::context! {
                    from_json_fn => from_json_fn,
                },
            ));
        }
    }

    // Emit public methods for inherent instance methods.
    let (instance_methods, _static_methods) = crate::codegen::shared::partition_methods(&ty.methods);
    let mut methods_source = String::new();
    for method in instance_methods {
        if method.sanitized || method.is_static {
            continue;
        }
        emit_instance_method_for_first_class_struct(method, type_name, mapper, &mut methods_source);
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "first_class_struct.swift.jinja",
        minijinja::context! {
            type_name => type_name,
            properties => properties,
            init_params => init_params,
            init_assignments => init_assignments,
            coding_keys => coding_keys,
            decoder_init => decoder_init,
            ffi_init_assignments => ffi_init_assignments,
            into_rust_body => into_rust_body,
            methods => methods_source,
        },
    ));
}

/// Renders a typed `DefaultValue` into a Swift literal expression suitable for use as
/// a `??` fallback. Returns `None` for variants that have no direct Swift literal
/// (`Empty`, `None`, `EnumVariant`), so callers fall back to a type-based default.
///
/// `FloatLiteral` values that are NaN or infinite are also rejected so generated code
/// stays parseable — callers handle those by falling back to a type-based default.
pub(super) fn swift_typed_default_literal(dv: &DefaultValue) -> Option<String> {
    match dv {
        DefaultValue::BoolLiteral(true) => Some("true".to_string()),
        DefaultValue::BoolLiteral(false) => Some("false".to_string()),
        DefaultValue::IntLiteral(n) => Some(n.to_string()),
        DefaultValue::FloatLiteral(f) => {
            if f.is_nan() || f.is_infinite() {
                None
            } else {
                // Emit at least one decimal so the literal parses as a Swift
                // floating-point literal rather than an integer literal.
                let s = if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                };
                Some(s)
            }
        }
        DefaultValue::StringLiteral(s) => {
            // Conservative escape for Swift string literal: backslash, double quote,
            // newline, carriage return, tab. Anything else passes through.
            let mut escaped = String::with_capacity(s.len() + 2);
            escaped.push('"');
            for ch in s.chars() {
                match ch {
                    '\\' => escaped.push_str("\\\\"),
                    '"' => escaped.push_str("\\\""),
                    '\n' => escaped.push_str("\\n"),
                    '\r' => escaped.push_str("\\r"),
                    '\t' => escaped.push_str("\\t"),
                    c => escaped.push(c),
                }
            }
            escaped.push('"');
            Some(escaped)
        }
        // EnumVariant requires knowledge of the target enum's Swift case name and is
        // not safely renderable from the variant string alone — fall back to a plain
        // `decode(T.self, ...)` (no `??`).
        DefaultValue::EnumVariant(_) => None,
        DefaultValue::Empty | DefaultValue::None => None,
    }
}

/// Returns a Swift literal expression for the "natural" default of a `TypeRef`, used
/// as a fallback when the field has `typed_default = Empty/None` or no typed default
/// at all.
///
/// Returns `None` for `Named(_)`, `Bytes`, `Path`, `Duration`, `Json`, `Char`, `Unit` —
/// the caller then emits a plain `decode(T.self, ...)` which relies on the nested
/// type's own decoder.
pub(super) fn swift_type_based_default(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => Some("false".to_string()),
            PrimitiveType::U8
            | PrimitiveType::I8
            | PrimitiveType::U16
            | PrimitiveType::I16
            | PrimitiveType::U32
            | PrimitiveType::I32
            | PrimitiveType::U64
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => Some("0".to_string()),
            PrimitiveType::F32 | PrimitiveType::F64 => Some("0".to_string()),
        },
        TypeRef::String => Some("\"\"".to_string()),
        TypeRef::Vec(_) => Some("[]".to_string()),
        TypeRef::Map(_, _) => Some("[:]".to_string()),
        TypeRef::Optional(_) => Some("nil".to_string()),
        // Named: cannot synthesize a default value without knowing the target type.
        // Bytes/Path/Duration/Json/Char/Unit are out of scope — caller falls through
        // to a plain decode.
        _ => None,
    }
}

/// Emits a custom `public init(from decoder: any Decoder) throws` body that uses
/// `decodeIfPresent + ?? <fallback>` for every non-Optional field with a known
/// default, `decodeIfPresent ?? nil` for Optional fields, and plain
/// `decode(T.self, ...)` for non-Optional fields with no safe Swift fallback
/// (e.g. nested `Named` structs).
pub(super) fn emit_decoder_init(mapper: &SwiftMapper, visible_fields: &[&FieldDef], out: &mut String) {
    out.push_str("    public init(from decoder: any Decoder) throws {\n");
    out.push_str("        let container = try decoder.container(keyedBy: CodingKeys.self)\n");
    for field in visible_fields {
        let camel = swift_case_ident(&field.name.to_lower_camel_case());
        let already_optional = matches!(&field.ty, TypeRef::Optional(_));
        let is_optional = field.optional || already_optional;
        let swift_ty = mapper.map_type(&field.ty);

        if is_optional {
            // Optional fields decode to nil when the key is missing OR when
            // present-and-null. Strip a trailing `?` so the type passed to
            // `decodeIfPresent` is the inner (non-optional) form —
            // `decodeIfPresent(T.self, ...)` already returns `T?`.
            let inner_ty = swift_ty.strip_suffix('?').unwrap_or(&swift_ty);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_decode_optional_assignment.swift.jinja",
                minijinja::context! {
                    field => camel,
                    ty => inner_ty,
                },
            ));
            continue;
        }

        // Non-Optional field: pick a fallback literal in this priority order:
        //   1. Typed default literal (BoolLiteral / IntLiteral / FloatLiteral /
        //      StringLiteral).
        //   2. Type-based default for collections / primitives / strings:
        //      `[]`, `[:]`, `false`, `0`, `""`.
        //   3. None → emit plain `decode(T.self, ...)` with no fallback.
        let fallback = field
            .typed_default
            .as_ref()
            .and_then(swift_typed_default_literal)
            .or_else(|| swift_type_based_default(&field.ty));

        match fallback {
            Some(fb) => {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_default_assignment.swift.jinja",
                    minijinja::context! {
                        field => camel,
                        ty => swift_ty,
                        fallback => fb,
                    },
                ));
            }
            None => {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_decode_required_assignment.swift.jinja",
                    minijinja::context! {
                        field => camel,
                        ty => swift_ty,
                    },
                ));
            }
        }
    }
    out.push_str("    }\n");
}

/// Returns the Swift body of `intoRust()` as a direct `RustBridge.{Type}(...)` call when
/// every constructor field can be converted without JSON, or `None` when at least one
/// field requires the JSON fallback (e.g. JSON-bridged Map/Json fields, types without a
/// Default impl that prevent the bulk constructor extern from being emitted).
///
/// Conversions supported in this PoC:
/// - Primitive (Int/UInt/Bool/Float/Double): pass `self.{camel}` directly.
/// - String: pass `self.{camel}` (Swift `String` conforms to swift-bridge's `IntoRustString`).
/// - Named(struct): recurse via `try self.{camel}.intoRust()`.
/// - Vec<Primitive | String | Named>: build a `RustVec<T>` and push each converted element.
/// - Optional<Primitive | String>: pass `self.{camel}` (swift-bridge handles native nullable).
///
/// Returns `None` for anything else (Map, Json, Path, Bytes, Duration, Char, Optional<Vec>,
/// Optional<Named>, Vec<Vec<...>>) so the caller can use the JSON fallback.
pub(super) fn emit_into_rust_direct_call(
    ty: &TypeDef,
    _mapper: &SwiftMapper,
    exclude_fields: &HashSet<String>,
    type_name: &str,
    configured_features: &std::collections::HashSet<&str>,
) -> Option<String> {
    use crate::backends::swift::gen_rust_crate::extern_block::{constructor_fields, has_constructor_extern};

    if !has_constructor_extern(ty, exclude_fields, configured_features) {
        return None;
    }

    let ctor_fields = constructor_fields(ty, exclude_fields, configured_features);
    // If any visible (binding-visible) field is dropped from the constructor — e.g.
    // listed in `[crates.<crate>.swift] exclude_fields` — the bulk constructor would
    // silently default that field while the JSON roundtrip preserves whatever serde
    // encoded. Fall back to JSON in that case to avoid a behaviour change.
    let visible_count = ty.fields.iter().filter(|f| !f.binding_excluded).count();
    if ctor_fields.len() != visible_count {
        return None;
    }

    // Build per-field conversion expressions. Bail to None (JSON fallback) on the first
    // field type we don't yet support.
    let mut prelude = String::new();
    let mut args: Vec<String> = Vec::with_capacity(ctor_fields.len());
    for field in &ctor_fields {
        let camel = swift_case_ident(&field.name.to_lower_camel_case());
        let local = format!("__{}", field.name.to_lower_camel_case());
        match field_intorust_arg(&field.ty, field.optional, &camel, &local) {
            Some(FieldArg::Direct(expr)) => args.push(expr),
            Some(FieldArg::WithPrelude { prelude: p, arg }) => {
                prelude.push_str(&p);
                args.push(arg);
            }
            None => return None,
        }
    }

    // Emit: `return RustBridge.{TypeName}(arg1, arg2, ...)` — swift-bridge's
    // convenience init takes positional `_` parameters in field-declaration order.
    let mut body = String::new();
    body.push_str(&prelude);
    body.push_str(&crate::backends::swift::template_env::render(
        "swift_bridge_constructor_return.swift.jinja",
        minijinja::context! {
            type_name => type_name,
            args => args.join(", "),
        },
    ));
    Some(body)
}

/// One field's contribution to the `RustBridge.{Type}(...)` argument list.
pub(super) enum FieldArg {
    /// Inline expression — no temporaries needed (`self.foo`, etc.).
    Direct(String),
    /// Multi-statement prelude that materialises a temporary, plus the argument
    /// referencing that temporary (`__foo`).
    WithPrelude { prelude: String, arg: String },
}

/// Translate a single field into a `RustBridge.{Type}(...)` argument.
///
/// Returns `None` when the field's Swift type cannot yet be passed directly to the
/// swift-bridge convenience init (Map, Json, Path, Bytes, Duration, Char, etc.) — the
/// caller falls back to the JSON roundtrip.
pub(super) fn field_intorust_arg(
    ty: &TypeRef,
    field_optional: bool,
    self_property: &str,
    local: &str,
) -> Option<FieldArg> {
    // Unwrap Optional(T) and merge with field.optional — both encode "nullable" in
    // different parts of the IR.
    let (inner_ty, is_optional) = match ty {
        TypeRef::Optional(inner) => (inner.as_ref(), true),
        _ => (ty, field_optional),
    };

    match inner_ty {
        // Primitives and bools pass straight through — Swift Int/UInt/Bool/Float/Double
        // map directly onto swift-bridge's bridged Rust primitives, including the
        // `Optional<T>` variants.
        TypeRef::Primitive(_) => Some(FieldArg::Direct(format!("self.{self_property}"))),

        // String fields must be wrapped with `RustString(...)`. swift-bridge 0.1.59 emits
        // the convenience init using a *single* `GenericIntoRustString: IntoRustString`
        // type parameter shared across every String-like argument in the signature
        // (including `RustVec<GenericIntoRustString>` for `Vec<String>` fields). When any
        // field forces `RustString` (via the Vec path in `emit_vec_arg`), every other
        // String arg must also be `RustString` — Swift `String` cannot unify with
        // `RustString` under the shared generic. Wrapping unconditionally keeps the
        // init linkable regardless of which other fields appear.
        // Optional `String?` -> `self.foo.map(RustString.init)` — preserves nil.
        TypeRef::String if !is_optional => Some(FieldArg::Direct(format!("RustString(self.{self_property})"))),
        TypeRef::String => Some(FieldArg::Direct(format!("self.{self_property}.map(RustString.init)"))),

        // Nested struct field: recurse via the symmetric `intoRust()` method on the
        // first-class struct. swift-bridge takes the wrapper class (e.g. `RustBridge.Span`).
        TypeRef::Named(_) if !is_optional => Some(FieldArg::Direct(format!("try self.{self_property}.intoRust()"))),

        // Vec<T>: build a `RustVec<U>` and push each converted element. The element
        // converter is the inner type's own intoRust handling.
        TypeRef::Vec(elem) if !is_optional => emit_vec_arg(elem, self_property, local),

        // Other forms (Map, Path, Bytes, Duration, Char, Json, Optional<Vec>,
        // Optional<Named>, Vec<Vec<…>>) take the JSON fallback for now.
        _ => None,
    }
}

/// Emit a `RustVec<U>` materialisation prelude and the argument referencing it.
///
/// Supports inner types: Primitive (`RustVec<Int>` etc.), String (`RustVec<RustString>`,
/// because swift-bridge's `Vectorizable` is implemented for `RustString` not `String`),
/// and Named (`RustVec<RustBridge.Foo>` via per-element `intoRust()`).
pub(super) fn emit_vec_arg(elem: &TypeRef, self_property: &str, local: &str) -> Option<FieldArg> {
    let (rust_vec_param, elem_expr): (String, String) = match elem {
        TypeRef::Primitive(prim) => {
            let swift_prim = SwiftMapper.primitive(prim).into_owned();
            (swift_prim, "__elem".to_string())
        }
        // swift-bridge: `RustVec<T>` requires `T: Vectorizable`. Only `RustString` satisfies
        // that for textual data — bare Swift `String` does not. Wrap each element with
        // `RustString(__elem)` so the resulting `RustVec<RustString>` matches the Rust
        // `Vec<String>` ABI expected by the bridge.
        TypeRef::String => ("RustString".to_string(), "RustString(__elem)".to_string()),
        TypeRef::Named(name) => (format!("RustBridge.{name}"), "try __elem.intoRust()".to_string()),
        _ => return None,
    };

    // RustVec.push(value:) consumes one element at a time. The literal Swift array
    // (`self.{prop}`) iterates fine in a `for`-loop. The Rust-side extern takes
    // ownership of the resulting RustVec, so we don't need any extra cleanup.
    let prelude = format!(
        "        let {local} = RustVec<{rust_vec_param}>()\n        \
         for __elem in self.{self_property} {{ {local}.push(value: {elem_expr}) }}\n",
    );
    Some(FieldArg::WithPrelude {
        prelude,
        arg: local.to_string(),
    })
}

/// Returns the Swift expression to read a field value from a `RustBridge` accessor call.
///
/// swift-bridge exposes Rust struct fields as method calls: `.field_name()`.
///
/// Type-specific conversions:
/// - `String` → `.toString()` (RustString → Swift String)
/// - `Named(S)` → `try S(rb.field())` (RustBridge.S → first-class Swift struct via init)
/// - `Vec<String>` → `rb.field().map { $0.toString() }` (RustVec<RustString> → [String])
/// - `Vec<Primitive>` → `Array(rb.field())` (RustVec<T> → [T], RustVec is a Collection)
/// - `Vec<Named(S)>` → `try rb.field().map { try S($0) }` (RustVec<RustBridge.S> → [S])
/// - Optional variants use `?` chains accordingly.
///
/// `field_optional` is true when `field.optional == true` (extractor-unwrapped IR form).
/// `TypeRef::Optional(inner)` is the TypeRef-wrapped form — both are handled.
pub(super) fn swift_ffi_read_expr(
    ty: &TypeRef,
    field_optional: bool,
    accessor: &str,
    known_dto_names: &HashSet<String>,
    unit_enum_names: &HashSet<String>,
    untagged_enum_names: &HashSet<String>,
    error_type_name: &str,
) -> String {
    // When the field is optional in the extractor-unwrapped IR form (field.optional == true
    // but TypeRef is NOT Optional), the swift-bridge getter returns `T?` natively.
    // We compose the same `?`-chain logic as for TypeRef::Optional.
    let opt = field_optional && !matches!(ty, TypeRef::Optional(_));

    match ty {
        // String fields: getter returns RustString; call .toString().
        TypeRef::String if opt => format!("rb.{accessor}()?.toString()"),
        TypeRef::String => format!("rb.{accessor}().toString()"),

        // Named(S) where S is a unit serde enum: getter returns String (serde-serialized).
        // Convert via raw value init (`:String, Codable` enum).
        // When optional: `rb.field().flatMap { S(rawValue: $0.toString()) }`
        // The `flatMap` is called on the bridge `Optional<RustString>` directly so
        // Swift picks `Optional.flatMap` instead of `Sequence.flatMap` (String is a
        // Sequence with `Character` elements — chaining `?.toString().flatMap`
        // would dispatch to the Sequence variant and break compilation).
        // When required: `S(rawValue: rb.field().toString())` (force-safe: serde guarantees valid value)
        TypeRef::Named(name) if unit_enum_names.contains(name) && opt => {
            format!("rb.{accessor}().flatMap {{ {name}(rawValue: $0.toString()) }}")
        }
        TypeRef::Named(name) if unit_enum_names.contains(name) => {
            // Throw proper error instead of crashing on unknown variants.
            format!(
                "try {{ let rawValue = rb.{accessor}().toString(); \
                 guard let value = {name}(rawValue: rawValue) else {{ \
                 throw {error_type_name}.validation(message: \"Unknown {name} variant\", source: rawValue) \
                 }}; return value }}()"
            )
        }

        // Named(S) where S is a first-class struct: getter returns RustBridge.S (or S?).
        // Convert with the symmetric `init(_ rb:) throws` on the first-class struct.
        // Note: untagged enums are excluded here because they are in `known_dto_names` but
        // bridge as RustString — those cases are handled at the call site before reaching
        // this function (via `is_untagged_enum_type` check in the field init loop).
        TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) && opt => {
            format!("try rb.{accessor}().map {{ try {name}($0) }}")
        }
        TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) => {
            format!("try {name}(rb.{accessor}())")
        }

        // Vec<T>: getter returns RustVec<T> (a Swift Collection/Sequence).
        TypeRef::Vec(inner) if opt => {
            // The getter return type for optional Vec is RustVec<T>? when the field is
            // declared optional via `field.optional` (not TypeRef::Optional).
            // Use `?.map` for the optional chain.
            match inner.as_ref() {
                TypeRef::Primitive(_) => format!("rb.{accessor}().map {{ Array($0) }}"),
                TypeRef::String => format!("rb.{accessor}()?.map {{ $0.as_str().toString() }}"),
                // Vec<UntaggedEnum>: each element is a JSON-encoded RustString.
                // Decode each element via JSONDecoder in the map closure.
                TypeRef::Named(name) if untagged_enum_names.contains(name) => {
                    format!(
                        "try rb.{accessor}()?.map {{ (s: RustStringRef) -> {name} in \
                         let d = s.as_str().toString().data(using: .utf8) ?? Data(); \
                         return try JSONDecoder().decode({name}.self, from: d) }}"
                    )
                }
                TypeRef::Named(name) if known_dto_names.contains(name) => {
                    format!("try rb.{accessor}()?.map {{ try {name}($0) }}")
                }
                _ => {
                    let map_expr = vec_elem_convert_expr(inner, known_dto_names);
                    format!("rb.{accessor}()?.map {{ {map_expr} }}")
                }
            }
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(_) => format!("Array(rb.{accessor}())"),
            TypeRef::String => format!("rb.{accessor}().map {{ $0.as_str().toString() }}"),
            // Vec<UntaggedEnum>: each element is a JSON-encoded RustString.
            // Decode each element via JSONDecoder in the map closure.
            TypeRef::Named(name) if untagged_enum_names.contains(name) => {
                format!(
                    "try rb.{accessor}().map {{ (s: RustStringRef) -> {name} in \
                     let d = s.as_str().toString().data(using: .utf8) ?? Data(); \
                     return try JSONDecoder().decode({name}.self, from: d) }}"
                )
            }
            TypeRef::Named(name) if known_dto_names.contains(name) => {
                format!("try rb.{accessor}().map {{ try {name}($0) }}")
            }
            _ => format!("rb.{accessor}()"),
        },

        // TypeRef::Optional(inner) — the extractor-wrapped nullable form.
        // For Optional<Named(unit_enum)>: getter returns Option<String> (serde-serialized).
        // Note: Optional<Named(untagged_enum)> is handled at the call site via
        // `is_untagged_enum_type` before reaching this function.
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String => format!("rb.{accessor}()?.toString()"),
            TypeRef::Named(name) if unit_enum_names.contains(name) => {
                format!("rb.{accessor}().flatMap {{ {name}(rawValue: $0.toString()) }}")
            }
            TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) => {
                format!("try rb.{accessor}().map {{ try {name}($0) }}")
            }
            TypeRef::Vec(elem) => match elem.as_ref() {
                TypeRef::String => format!("rb.{accessor}()?.map {{ $0.as_str().toString() }}"),
                TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) => {
                    format!("try rb.{accessor}()?.map {{ try {name}($0) }}")
                }
                TypeRef::Primitive(_) => format!("rb.{accessor}().map {{ Array($0) }}"),
                _ => format!("rb.{accessor}()"),
            },
            _ => format!("rb.{accessor}()"),
        },

        // Primitives, and anything else the bridge passes through directly.
        _ => format!("rb.{accessor}()"),
    }
}

/// Returns the element-level Swift expression used in a `.map { ... }` closure when
/// converting a `RustVec<T>` element to its first-class Swift equivalent.
pub(super) fn vec_elem_convert_expr(inner: &TypeRef, known_dto_names: &HashSet<String>) -> String {
    match inner {
        // RustVec<RustString> iteration yields RustStringRef — see RustStringRef
        // shim comment in `forwarder_return_conversion_suffix_inner` for why we use
        // `as_str().toString()` instead of `toString()` directly.
        TypeRef::String => "$0.as_str().toString()".to_string(),
        TypeRef::Named(name) if known_dto_names.contains(name) => format!("try {name}($0)"),
        TypeRef::Primitive(_) => "$0".to_string(),
        _ => "$0".to_string(),
    }
}

fn is_untagged_enum_type(ty: &TypeRef, untagged_enum_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Named(n) => untagged_enum_names.contains(n),
        TypeRef::Optional(inner) => is_untagged_enum_type(inner, untagged_enum_names),
        _ => false,
    }
}

/// Returns true when `ty` is a `Vec<Named(struct)>` or `Option<Vec<Named(struct)>>` where
/// the struct has serde derives. These are JSON-bridged at the Rust boundary and need
/// JSON decoding on the Swift side instead of the `.map { try Struct($0) }` pattern.
fn is_vec_of_serde_struct(ty: &TypeRef, serde_struct_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(name) = inner.as_ref() {
                serde_struct_names.contains(name)
            } else {
                false
            }
        }
        TypeRef::Optional(inner) => is_vec_of_serde_struct(inner, serde_struct_names),
        _ => false,
    }
}

fn needs_json_bridge_for_swift(ty: &TypeRef) -> bool {
    fn is_leaf(ty: &TypeRef) -> bool {
        matches!(
            ty,
            TypeRef::Primitive(_)
                | TypeRef::String
                | TypeRef::Char
                | TypeRef::Path
                | TypeRef::Json
                | TypeRef::Unit
                | TypeRef::Duration
                | TypeRef::Bytes
                | TypeRef::Named(_),
        )
    }
    match ty {
        TypeRef::Map(_, _) => true,
        // Vec is JSON-bridged only when the inner type is not a "leaf" (Vec<Vec<…>>,
        // Vec<Option<…>>, Vec<Map<…>>). Plain Vec<T> for leaf T crosses as RustVec<T>.
        TypeRef::Vec(inner) => !is_leaf(inner),
        // Plain Optional<T> for primitives/strings crosses natively (`Option<T>`),
        // so it is NOT JSON-bridged. Only Optional wrapping a JSON-bridged inner
        // (Optional<Vec<Vec<…>>>, Optional<Map<…>>) needs JSON decoding.
        TypeRef::Optional(inner) => needs_json_bridge_for_swift(inner),
        _ => false,
    }
}

fn is_field_unbridgeable_for_init(
    ty: &TypeDef,
    field: &FieldDef,
    exclude_fields: &HashSet<String>,
    known_dto_names: &HashSet<String>,
) -> bool {
    let name = field.name.to_snake_case();
    let field_key = format!("{}.{}", ty.name, name);
    if field.binding_excluded || exclude_fields.contains(&field_key) {
        return true;
    }
    // Sanitized Vec field whose inner is not a primitive/bytes — wrapper skips emit.
    if let TypeRef::Vec(inner) = &field.ty
        && field.sanitized
        && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes)
    {
        return true;
    }
    // Vec<Named> on non-serde struct — wrapper skips emit.
    if !ty.has_serde
        && let TypeRef::Vec(inner) = &field.ty
        && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes)
    {
        return true;
    }
    // JSON-bridge with inner Named where the wrapper does not exist.
    if needs_json_bridge_for_swift(&field.ty) {
        let inner_named = match &field.ty {
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        };
        if let Some(n) = inner_named
            && !known_dto_names.contains(n)
        {
            return true;
        }
    }
    false
}

/// Emit a public instance method for a non-opaque first-class struct.
///
/// Calls the Rust bridge extern to invoke the method, converting parameters and return values.
fn emit_instance_method_for_first_class_struct(
    method: &MethodDef,
    type_name: &str,
    mapper: &SwiftMapper,
    out: &mut String,
) {
    // Skip static/associated functions; only emit instance methods with a receiver.
    if method.is_static {
        return;
    }

    let method_name = swift_case_ident(&method.name.to_lower_camel_case());
    let extern_fn_name = format!("{}_{}", AsSnakeCase(type_name), method.name.to_snake_case());

    // Build parameter list (no receiver param; we're calling on self).
    let mut param_strs: Vec<String> = Vec::new();
    for param in &method.params {
        if param.sanitized {
            continue;
        }
        let param_name = swift_ident(&param.name.to_snake_case());
        let param_type = mapper.map_type(&param.ty);
        param_strs.push(format!("{param_name}: {param_type}"));
    }
    let param_list = param_strs.join(", ");

    // Map return type.
    let return_type = mapper.map_type(&method.return_type);

    // For now, emit a basic method body that calls the extern.
    // In a full implementation, this would:
    // 1. Serialize self to JSON (via intoRust()) or reconstruct from fields
    // 2. Call the extern function with self + params
    // 3. Deserialize the return value
    //
    // For this PoC, emit a placeholder that shows the method signature.
    let method_sig = if param_list.is_empty() {
        format!("    public func {method_name}() -> {return_type}")
    } else {
        format!("    public func {method_name}({param_list}) -> {return_type}")
    };

    out.push_str(&method_sig);
    out.push_str(" {\n");
    out.push_str("        fatalError(\"Not yet implemented: ");
    out.push_str(&extern_fn_name);
    out.push_str("\")\n");
    out.push_str("    }\n\n");
}
