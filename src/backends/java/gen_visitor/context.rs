//! Panama decoding for the FFI backend's `#[repr(C)]` visitor context struct.
//!
//! The layout, the field offsets and the set of fields that actually cross the boundary all come
//! from `codegen::visitor_context_abi`, the same derivation the FFI backend emits the struct
//! from. Nothing about the context shape is decided here. ~keep

use heck::ToShoutySnakeCase;
use minijinja::Value;

use crate::backends::java::gen_bindings::emits_get_value;
use crate::backends::java::template_env::render;
use crate::backends::java::type_map::java_type;
use crate::codegen::shared::binding_fields;
use crate::codegen::visitor_context_abi::{ContextAbi, ContextFieldShape, ContextScalar, context_abi};
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

/// The two generated blocks that carry the context shape into `VisitorBridge`.
pub(super) struct ContextDecoding {
    /// `CTX_LAYOUT` and the offset constant per carried field.
    pub layout: String,
    /// The whole `decodeContext` method.
    pub decode_method: String,
}

pub(super) fn context_decoding(context_def: &TypeDef, api: &ApiSurface, context_type: &str) -> ContextDecoding {
    let abi = context_abi(context_def, api);

    let mut members: Vec<String> = Vec::with_capacity(abi.fields.len());
    for field in &abi.fields {
        if field.leading_padding > 0 {
            members.push(padding_member(field.leading_padding));
        }
        members.push(format!(
            "{}.withName(\"{}\")",
            value_layout(field.scalar),
            field.name.escape_debug()
        ));
    }
    if abi.trailing_padding > 0 {
        members.push(padding_member(abi.trailing_padding));
    }

    let offsets: Vec<Value> = abi
        .fields
        .iter()
        .map(|field| {
            crate::alef_context! {
                constant => offset_constant(&field.name),
                name => field.name.escape_debug().to_string(),
            }
        })
        .collect();

    let arguments: Vec<Value> = binding_fields(&context_def.fields)
        .map(|field| argument(field, &abi, api))
        .collect();

    ContextDecoding {
        layout: render(
            "visitor_context_layout.jinja",
            crate::alef_context! {
                context_type => context_type,
                members => members,
                offsets => offsets,
                byte_size => abi.byte_size,
                byte_alignment => abi.byte_alignment,
            },
        ),
        decode_method: render(
            "visitor_context_decode.jinja",
            crate::alef_context! {
                context_type => context_type,
                arguments => arguments,
            },
        ),
    }
}

/// Describes one argument of the generated `new <Context>(...)` call to the decode template.
///
/// The record component list and the C struct field list are not the same list: the struct drops
/// fields with no C representation, and the record drops binding-excluded ones. Walking the
/// record's components and looking each one up in the ABI is what keeps the two aligned.
fn argument(field: &FieldDef, abi: &ContextAbi, api: &ApiSurface) -> Value {
    let Some(abi_field) = abi.field(&field.name) else {
        return absent_argument(field);
    };
    let constant = offset_constant(&abi_field.name);
    match abi_field.shape {
        ContextFieldShape::RequiredString => crate::alef_context! { kind => "required_string", constant },
        ContextFieldShape::OptionalString => crate::alef_context! { kind => "optional_string", constant },
        ContextFieldShape::Bool => crate::alef_context! { kind => "bool", constant },
        ContextFieldShape::Integer => crate::alef_context! {
            kind => "integer",
            constant,
            value_layout => value_layout(abi_field.scalar),
        },
        ContextFieldShape::Enum => enum_argument(field, api, constant),
    }
}

/// The struct carries a discriminant index, which only reconstructs a variant when the Java
/// binding emitted the type as a plain `enum` — a tagged or untagged union is a sealed interface
/// with no `values()` and no way back from an ordinal, so it takes the absent value instead.
fn enum_argument(field: &FieldDef, api: &ApiSurface, constant: String) -> Value {
    let decodable = matches!(&field.ty, TypeRef::Named(name)
        if api.enums.iter().any(|enum_def| enum_def.name == *name && emits_get_value(enum_def)));
    if decodable {
        crate::alef_context! {
            kind => "enum",
            constant,
            enum_type => java_type(&field.ty).into_owned(),
        }
    } else {
        absent_argument(field)
    }
}

/// A record component the C struct does not carry gets Java's own zero value.
///
/// The FFI backend drops context fields it has no C representation for — floats, collections,
/// nested structs, every optional that is not `Option<String>` — and the visitor still has to be
/// generated, because the options record that holds the callback references the visitor interface
/// whether or not the bridge exists. So the alternatives are a zero value or Java that does not
/// compile. The zero value is deliberately the *Java* default rather than a guess at the Rust
/// one: `null` reads as "the boundary did not carry this", where a fabricated value would not.
/// Widening the FFI struct is what actually fixes such a field; this only keeps the rest of the
/// context decodable. ~keep
fn absent_argument(field: &FieldDef) -> Value {
    crate::alef_context! { kind => "absent", absent_value => absent_value(field) }
}

fn absent_value(field: &FieldDef) -> &'static str {
    if field.optional || matches!(field.ty, TypeRef::Optional(_)) {
        return "null";
    }
    match java_type(&field.ty).as_ref() {
        "boolean" => "false",
        "byte" => "(byte) 0",
        "short" => "(short) 0",
        "int" => "0",
        "long" => "0L",
        "float" => "0.0f",
        "double" => "0.0d",
        _ => "null",
    }
}

/// The Panama value layout for a C scalar.
///
/// Each one is the layout whose Java carrier type is exactly the type `java_type` gives the
/// record component, so the decoded value needs no cast on the way into the constructor.
fn value_layout(scalar: ContextScalar) -> &'static str {
    match scalar {
        ContextScalar::Pointer => "ValueLayout.ADDRESS",
        ContextScalar::I8 | ContextScalar::U8 => "ValueLayout.JAVA_BYTE",
        ContextScalar::I16 | ContextScalar::U16 => "ValueLayout.JAVA_SHORT",
        ContextScalar::I32 | ContextScalar::U32 => "ValueLayout.JAVA_INT",
        ContextScalar::I64 | ContextScalar::U64 | ContextScalar::Isize | ContextScalar::Usize => {
            "ValueLayout.JAVA_LONG"
        }
    }
}

fn padding_member(bytes: u64) -> String {
    format!("MemoryLayout.paddingLayout({bytes})")
}

fn offset_constant(field_name: &str) -> String {
    format!("CTX_OFFSET_{}", field_name.to_shouty_snake_case())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, PrimitiveType};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn optional_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            optional: true,
            ..field(name, ty)
        }
    }

    fn context(fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: "TraversalState".to_string(),
            fields,
            ..TypeDef::default()
        }
    }

    fn simple_enum_api() -> ApiSurface {
        ApiSurface {
            enums: vec![EnumDef {
                name: "TraversalKind".to_string(),
                variants: vec![EnumVariant {
                    name: "Section".to_string(),
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn should_lay_out_fields_with_the_padding_repr_c_inserts() {
        let decoding = context_decoding(
            &context(vec![
                field("label", TypeRef::String),
                field("severity", TypeRef::Primitive(PrimitiveType::U8)),
                field("active", TypeRef::Primitive(PrimitiveType::Bool)),
            ]),
            &ApiSurface::default(),
            "TraversalState",
        );

        assert!(
            decoding.layout.contains("ValueLayout.ADDRESS.withName(\"label\")"),
            "{}",
            decoding.layout
        );
        assert!(decoding.layout.contains("ValueLayout.JAVA_BYTE.withName(\"severity\")"));
        assert!(
            decoding.layout.contains("MemoryLayout.paddingLayout(3)"),
            "{}",
            decoding.layout
        );
        assert!(decoding.layout.contains("CTX_OFFSET_ACTIVE"));
        assert!(!decoding.layout.contains("CTX_OFFSET_TAG_NAME"));
    }

    #[test]
    fn should_decode_each_shape_from_its_own_offset_constant() {
        let decoding = context_decoding(
            &context(vec![
                field("kind", TypeRef::Named("TraversalKind".to_string())),
                field("label", TypeRef::String),
                optional_field("parent", TypeRef::String),
                field("depth", TypeRef::Primitive(PrimitiveType::U64)),
                field("active", TypeRef::Primitive(PrimitiveType::Bool)),
            ]),
            &simple_enum_api(),
            "TraversalState",
        );

        assert!(
            decoding
                .decode_method
                .contains("TraversalKind.values()[ctx.get(ValueLayout.JAVA_INT, CTX_OFFSET_KIND)]")
        );
        assert!(
            decoding
                .decode_method
                .contains("ctx.get(ValueLayout.ADDRESS, CTX_OFFSET_LABEL)")
        );
        assert!(
            decoding
                .decode_method
                .contains("ctx.get(ValueLayout.ADDRESS, CTX_OFFSET_PARENT).equals(MemorySegment.NULL)")
        );
        assert!(
            decoding
                .decode_method
                .contains("ctx.get(ValueLayout.JAVA_LONG, CTX_OFFSET_DEPTH)")
        );
        assert!(
            decoding
                .decode_method
                .contains("ctx.get(ValueLayout.JAVA_INT, CTX_OFFSET_ACTIVE) != 0")
        );
        let separators = decoding
            .decode_method
            .lines()
            .filter(|line| line.trim_end().ends_with(','))
            .count();
        assert_eq!(
            separators, 4,
            "five arguments need four separators:\n{}",
            decoding.decode_method
        );
    }

    #[test]
    fn should_substitute_a_java_zero_for_fields_the_struct_does_not_carry() {
        let decoding = context_decoding(
            &context(vec![
                field("weight", TypeRef::Primitive(PrimitiveType::F64)),
                field("tags", TypeRef::Vec(Box::new(TypeRef::String))),
                optional_field("count", TypeRef::Primitive(PrimitiveType::U32)),
                field("label", TypeRef::String),
            ]),
            &ApiSurface::default(),
            "TraversalState",
        );

        assert!(decoding.decode_method.contains("0.0d,"), "{}", decoding.decode_method);
        assert_eq!(
            decoding.decode_method.matches("null,").count(),
            2,
            "{}",
            decoding.decode_method
        );
        assert!(!decoding.layout.contains("weight"), "{}", decoding.layout);
        assert!(!decoding.layout.contains("tags"));
        assert!(!decoding.layout.contains("count"));
    }

    #[test]
    fn should_skip_binding_excluded_components_that_still_occupy_a_struct_slot() {
        let mut excluded = field("secret", TypeRef::String);
        excluded.binding_excluded = true;
        let decoding = context_decoding(
            &context(vec![excluded, field("label", TypeRef::String)]),
            &ApiSurface::default(),
            "TraversalState",
        );

        assert!(decoding.layout.contains("withName(\"secret\")"), "{}", decoding.layout);
        assert!(
            !decoding.decode_method.contains("CTX_OFFSET_SECRET"),
            "{}",
            decoding.decode_method
        );
        assert!(decoding.decode_method.contains("CTX_OFFSET_LABEL"));
    }
}
