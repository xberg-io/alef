use crate::backends::swift::naming::{swift_rust_shim_ident as swift_ident, swift_source_ident as swift_case_ident};
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
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
        _ => false,
    }
}

/// Compute the set of struct/enum type names that are emitted as first-class Swift Codable
/// values (structs or serde enums), via the same fixed-point iteration used when emitting
/// them in `emit`.
///
/// A type is first-class iff it is non-opaque, has serde, non-trait, non-excluded, has visible
/// fields, and every visible field's type is first-class-supported given the growing set. Unit
/// serde enums and data-variant (tagged/untagged) serde enums seed the set — they are Codable
/// and may appear as fields.
///
/// This is the authoritative classifier shared by the Swift binding emitter (`gen_bindings`) and
/// the swift-bridge Rust-crate getter emitter (`gen_rust_crate`): a `Vec<Named>` getter is
/// JSON-degraded to `Vec<String>` only when the *containing* type is first-class (its Codable
/// wrapper decodes the JSON); an opaque-rendered parent must return a real `Vec<Opaque>` so the
/// opaque element accessors resolve.
pub(crate) fn compute_first_class_dto_names(api: &ApiSurface, exclude_types: &HashSet<String>) -> HashSet<String> {
    let unit_serde_enum_names = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .filter(|e| e.has_serde && e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.clone());
    let data_variant_serde_enum_names = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .filter(|e| e.has_serde && e.variants.iter().any(|v| !v.fields.is_empty()))
        .map(|e| e.name.clone());

    let candidate_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde && !exclude_types.contains(&t.name))
        .filter(|t| !t.fields.is_empty())
        .collect();

    let mut known: HashSet<String> = unit_serde_enum_names.chain(data_variant_serde_enum_names).collect();
    loop {
        let prev_len = known.len();
        for ty in &candidate_types {
            if known.contains(&ty.name) {
                continue;
            }
            if binding_fields(&ty.fields).all(|field| first_class_field_supported(&field.ty, &known)) {
                known.insert(ty.name.clone());
            }
        }
        if known.len() == prev_len {
            break;
        }
    }
    known
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

    let needs_coding_keys = ty.has_default
        || visible_fields.iter().any(|field| {
            let camel = field.name.to_lower_camel_case();
            // `#[serde(rename)]` / `#[serde(rename_all)]`, even if the camelCased identifier matches.
            camel != field.name || field.serde_rename.is_some() || ty.serde_rename_all.is_some()
        });
    let mut coding_keys = String::new();
    if needs_coding_keys {
        for field in &visible_fields {
            let camel = swift_case_ident(&field.name.to_lower_camel_case());
            // `call_type` with `#[serde(rename = "type")]` must decode from `"type"`, not `"call_type"`).
            let wire_key = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                ty.serde_rename_all.as_deref(),
            );
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
    // `#[serde(default)]` / `#[serde(skip_serializing_if = ...)]` decode successfully.
    let mut decoder_init = String::new();
    if ty.has_default {
        emit_decoder_init(mapper, &visible_fields, &mut decoder_init);
    }

    let mut ffi_init_assignments = String::new();
    for field in &visible_fields {
        let swift_field = swift_case_ident(&field.name.to_lower_camel_case());
        let rust_accessor = swift_ident(&field.name.to_lower_camel_case());
        let is_optional = field.optional || matches!(&field.ty, TypeRef::Optional(_));

        let expr = if is_field_unbridgeable_for_init(ty, field, exclude_fields, known_dto_names) {
            if is_optional {
                "nil".to_string()
            } else {
                let swift_ty = mapper.map_type(&field.ty);
                format!("try JSONDecoder().decode({swift_ty}.self, from: Data(\"null\".utf8))")
            }
        } else if is_vec_of_serde_struct(&field.ty, serde_struct_names) {
            let swift_ty = mapper.map_type(&field.ty);
            let swift_ty_with_opt = if is_optional && !matches!(&field.ty, TypeRef::Optional(_)) {
                format!("{swift_ty}?")
            } else {
                swift_ty
            };

            let field_is_optional = is_optional || matches!(&field.ty, TypeRef::Optional(_));
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
                let accessor_with_chain = format!("rb.{rust_accessor}().toString()");
                format!(
                    "try JSONDecoder().decode({swift_ty_with_opt}.self, from: \
                     (({accessor_with_chain}).data(using: .utf8) ?? Data(\"null\".utf8)))"
                )
            } else if let Some(inner_struct_name) = inner_vec_named {
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

    // when the swift-bridge `#[swift_bridge(init)] fn new(...)` extern is emitted for
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
                let s = if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                };
                Some(s)
            }
        }
        DefaultValue::StringLiteral(s) => {
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
    let visible_count = ty.fields.iter().filter(|f| !f.binding_excluded).count();
    if ctor_fields.len() != visible_count {
        return None;
    }

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
    let (inner_ty, is_optional) = match ty {
        TypeRef::Optional(inner) => (inner.as_ref(), true),
        _ => (ty, field_optional),
    };

    match inner_ty {
        TypeRef::Primitive(_) => Some(FieldArg::Direct(format!("self.{self_property}"))),

        TypeRef::String if !is_optional => Some(FieldArg::Direct(format!("RustString(self.{self_property})"))),
        TypeRef::String => Some(FieldArg::Direct(format!("self.{self_property}.map(RustString.init)"))),

        TypeRef::Named(_) if !is_optional => Some(FieldArg::Direct(format!("try self.{self_property}.intoRust()"))),

        TypeRef::Vec(elem) if !is_optional => emit_vec_arg(elem, self_property, local),

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
        TypeRef::String => ("RustString".to_string(), "RustString(__elem)".to_string()),
        TypeRef::Named(name) => (format!("RustBridge.{name}"), "try __elem.intoRust()".to_string()),
        _ => return None,
    };

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
    let opt = field_optional && !matches!(ty, TypeRef::Optional(_));

    match ty {
        TypeRef::String if opt => format!("rb.{accessor}()?.toString()"),
        TypeRef::String => format!("rb.{accessor}().toString()"),

        TypeRef::Named(name) if unit_enum_names.contains(name) && opt => {
            format!("rb.{accessor}().flatMap {{ {name}(rawValue: $0.toString()) }}")
        }
        TypeRef::Named(name) if unit_enum_names.contains(name) => {
            format!(
                "try {{ let rawValue = rb.{accessor}().toString(); \
                 guard let value = {name}(rawValue: rawValue) else {{ \
                 throw {error_type_name}.validation(message: \"Unknown {name} variant\", source: rawValue) \
                 }}; return value }}()"
            )
        }

        TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) && opt => {
            format!("try rb.{accessor}().map {{ try {name}($0) }}")
        }
        TypeRef::Named(name) if known_dto_names.contains(name) && !untagged_enum_names.contains(name) => {
            format!("try {name}(rb.{accessor}())")
        }

        TypeRef::Vec(inner) if opt => match inner.as_ref() {
            TypeRef::Primitive(_) => format!("rb.{accessor}().map {{ Array($0) }}"),
            TypeRef::String => format!("rb.{accessor}()?.map {{ $0.as_str().toString() }}"),
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
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(_) => format!("Array(rb.{accessor}())"),
            TypeRef::String => format!("rb.{accessor}().map {{ $0.as_str().toString() }}"),
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

        _ => format!("rb.{accessor}()"),
    }
}

/// Returns the element-level Swift expression used in a `.map { ... }` closure when
/// converting a `RustVec<T>` element to its first-class Swift equivalent.
pub(super) fn vec_elem_convert_expr(inner: &TypeRef, known_dto_names: &HashSet<String>) -> String {
    match inner {
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

pub(crate) fn needs_json_bridge_for_swift(ty: &TypeRef) -> bool {
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
        TypeRef::Vec(inner) => !is_leaf(inner),
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
    if let TypeRef::Vec(inner) = &field.ty
        && field.sanitized
        && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes)
    {
        return true;
    }
    if !ty.has_serde
        && let TypeRef::Vec(inner) = &field.ty
        && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes)
    {
        return true;
    }
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
/// The method serializes self to JSON, calls a Rust wrapper extern, and deserializes the result.
fn emit_instance_method_for_first_class_struct(
    method: &MethodDef,
    type_name: &str,
    mapper: &SwiftMapper,
    out: &mut String,
) {
    if method.is_static {
        return;
    }

    let method_swift_name = swift_case_ident(&method.name.to_lower_camel_case());
    let method_snake = method.name.to_snake_case();
    let type_snake = type_name.to_snake_case();
    let extern_fn_name = format!("{type_snake}_{method_snake}_from_json");
    let extern_swift_name = swift_ident(&extern_fn_name.to_lower_camel_case());

    let mut param_decls = Vec::new();
    let mut param_names = Vec::new();
    for param in &method.params {
        let param_swift_type = mapper.map_type(&param.ty);
        let param_swift_name = swift_case_ident(&param.name.to_lower_camel_case());

        let param_type = if param.optional && !matches!(&param.ty, TypeRef::Optional(_)) {
            format!("{param_swift_type}?")
        } else {
            param_swift_type
        };

        param_decls.push(format!("{param_swift_name}: {param_type}"));
        param_names.push(param_swift_name);
    }
    let params_signature = param_decls.join(", ");

    let return_swift_type = if matches!(method.return_type, TypeRef::Unit) {
        "Void".to_string()
    } else {
        mapper.map_type(&method.return_type)
    };

    let return_clause = if return_swift_type == "Void" {
        String::new()
    } else {
        format!(" -> {return_swift_type}")
    };

    let throws_clause = "throws ";

    out.push_str(&format!(
        "    public func {method_swift_name}({params_signature}) {throws_clause}{return_clause} {{\n",
    ));

    out.push_str("        let jsonSelf = try JSONEncoder().encode(self)\n");
    out.push_str("        let selfString = String(data: jsonSelf, encoding: .utf8) ?? \"{}\"\n");

    let call_args = if param_names.is_empty() {
        "selfString".to_string()
    } else {
        format!("selfString, {}", param_names.join(", "))
    };

    if matches!(method.return_type, TypeRef::Unit) {
        out.push_str(&format!(
            "        _ = try RustBridge.{extern_swift_name}({call_args})\n"
        ));
    } else {
        out.push_str(&format!(
            "        let resultJson = try RustBridge.{extern_swift_name}({call_args}).toString()\n"
        ));
        out.push_str("        let data = resultJson.data(using: .utf8) ?? Data()\n");
        out.push_str(&format!(
            "        return try JSONDecoder().decode({}.self, from: data)\n",
            return_swift_type
        ));
    }

    out.push_str("    }\n\n");
}
