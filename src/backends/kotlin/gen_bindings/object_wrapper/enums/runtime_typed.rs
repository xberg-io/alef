//! Emit Jackson writes that keep a polymorphic type id through a container payload.
//!
//! `mapper.writeValue(gen, payload)` hands Jackson a value whose *static* type is gone. For a
//! collection that erasure moves to the elements: Jackson resolves each element's serializer
//! dynamically (`findValueSerializer` on the element's class) and calls `serialize`, never
//! `serializeWithType`. Both ways a sealed base carries its wire form are lost by that call —
//! `@JsonTypeInfo` writes its discriminator only from `serializeWithType`, and a custom base
//! serializer is cancelled on every struct-variant subclass with `JsonSerializer.None`, which is
//! exactly what the dynamic per-element lookup finds. The array comes out as bare payload objects
//! and Rust rejects it ("data did not match any variant").
//!
//! So a container payload has to be written out element by element, each element through
//! `findTypedValueSerializer(<runtime class>, true, null)`: the type-wrapping serializer, resolved
//! from the RUNTIME class. Passing the declared base class instead emits the discriminator but
//! drops the payload. ~keep

use crate::core::ir::TypeRef;

const INDENT_STEP: &str = "    ";

/// Indent of a `when`-arm body inside a generated `StdSerializer.serialize` override.
pub(super) const PAYLOAD_INDENT: &str = "                ";

/// Emit the runtime-typed write of `expr` (a newtype variant's payload of IR type `ty`) at
/// `indent`, or return `false` without emitting anything when the shape does not need one and the
/// caller's plain `mapper.writeValue` is correct.
pub(super) fn try_emit_runtime_typed_payload_write(
    out: &mut String,
    indent: &str,
    expr: &str,
    ty: &TypeRef,
    optional: bool,
) -> bool {
    let wrapped = if optional {
        TypeRef::Optional(Box::new(ty.clone()))
    } else {
        ty.clone()
    };
    if !needs_runtime_typed_write(&wrapped) {
        return false;
    }
    emit_write(out, indent, expr, &wrapped, 0);
    true
}

/// True when `ty` is a container Jackson would erase that holds a `Named` type it could erase.
///
/// A bare `Named` payload is excluded on purpose: `mapper.writeValue` resolves the root value
/// from its own runtime class, so a non-container payload already keeps its wire form. ~keep
fn needs_runtime_typed_write(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => holds_named(inner),
        TypeRef::Map(key, value) => map_key_is_writable(key) && holds_named(value),
        _ => false,
    }
}

fn holds_named(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => holds_named(inner),
        TypeRef::Map(key, value) => map_key_is_writable(key) && holds_named(value),
        _ => false,
    }
}

/// A JSON member name is a string, so a map key only round-trips when `toString()` already yields
/// the wire form serde would write. That holds for strings and numbers; a `Named` key's Kotlin
/// `toString()` is its class/constant name rather than its serde rename, so those maps stay on the
/// `mapper.writeValue` path where Jackson's own key serializer handles them. ~keep
fn map_key_is_writable(key: &TypeRef) -> bool {
    matches!(key, TypeRef::String | TypeRef::Char | TypeRef::Primitive(_))
}

fn element_name(depth: usize) -> String {
    if depth == 0 {
        "elem".to_string()
    } else {
        format!("elem{depth}")
    }
}

fn entry_name(depth: usize) -> String {
    if depth == 0 {
        "entry".to_string()
    } else {
        format!("entry{depth}")
    }
}

fn emit_write(out: &mut String, indent: &str, expr: &str, ty: &TypeRef, depth: usize) {
    match ty {
        TypeRef::Named(_) => out.push_str(&crate::backends::kotlin::template_env::render(
            "runtime_typed_value_write.jinja",
            minijinja::context! { indent => indent, expr => expr },
        )),
        TypeRef::Optional(inner) if holds_named(inner) => emit_nullable(out, indent, expr, inner, depth),
        TypeRef::Vec(inner) if holds_named(inner) => emit_array(out, indent, expr, inner, depth),
        TypeRef::Map(key, value) if map_key_is_writable(key) && holds_named(value) => {
            emit_object(out, indent, expr, value, depth);
        }
        _ => {
            out.push_str(indent);
            out.push_str("mapper.writeValue(gen, ");
            out.push_str(expr);
            out.push_str(")\n");
        }
    }
}

/// Bind a local before the null check rather than testing `expr` in place: `expr` may be a property
/// access, and Kotlin only smart-casts a local or a stable `val`. ~keep
fn emit_nullable(out: &mut String, indent: &str, expr: &str, inner: &TypeRef, depth: usize) {
    let local = element_name(depth);
    let inner_indent = format!("{indent}{INDENT_STEP}");
    out.push_str(&format!("{indent}val {local} = {expr}\n"));
    out.push_str(&format!("{indent}if ({local} == null) {{\n"));
    out.push_str(&format!("{inner_indent}gen.writeNull()\n"));
    out.push_str(&format!("{indent}}} else {{\n"));
    emit_write(out, &inner_indent, &local, inner, depth + 1);
    out.push_str(&format!("{indent}}}\n"));
}

fn emit_array(out: &mut String, indent: &str, expr: &str, inner: &TypeRef, depth: usize) {
    let local = element_name(depth);
    let inner_indent = format!("{indent}{INDENT_STEP}");
    out.push_str(&format!("{indent}gen.writeStartArray()\n"));
    out.push_str(&format!("{indent}for ({local} in {expr}) {{\n"));
    emit_write(out, &inner_indent, &local, inner, depth + 1);
    out.push_str(&format!("{indent}}}\n"));
    out.push_str(&format!("{indent}gen.writeEndArray()\n"));
}

fn emit_object(out: &mut String, indent: &str, expr: &str, value: &TypeRef, depth: usize) {
    let local = entry_name(depth);
    let inner_indent = format!("{indent}{INDENT_STEP}");
    out.push_str(&format!("{indent}gen.writeStartObject()\n"));
    out.push_str(&format!("{indent}for ({local} in {expr}) {{\n"));
    out.push_str(&format!("{inner_indent}gen.writeFieldName({local}.key.toString())\n"));
    emit_write(out, &inner_indent, &format!("{local}.value"), value, depth + 1);
    out.push_str(&format!("{indent}}}\n"));
    out.push_str(&format!("{indent}gen.writeEndObject()\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOKUP: &str = "provider.findTypedValueSerializer(elem.javaClass, true, null).serialize(elem, gen, provider)";

    fn write(ty: &TypeRef, optional: bool) -> Option<String> {
        let mut out = String::new();
        try_emit_runtime_typed_payload_write(&mut out, "    ", "value.payload", ty, optional).then_some(out)
    }

    #[test]
    fn should_skip_a_payload_that_holds_no_named_type() {
        assert_eq!(write(&TypeRef::Vec(Box::new(TypeRef::String)), false), None);
        assert_eq!(write(&TypeRef::String, false), None);
    }

    #[test]
    fn should_skip_a_bare_named_payload_because_write_value_resolves_the_root_at_runtime() {
        assert_eq!(write(&TypeRef::Named("Node".to_string()), false), None);
    }

    #[test]
    fn should_type_each_element_of_a_named_vec_by_its_runtime_class() {
        let emitted = write(&TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string()))), false)
            .expect("Vec<Named> needs a runtime-typed write");
        assert_eq!(
            emitted,
            format!(
                "    gen.writeStartArray()\n    for (elem in value.payload) {{\n        {LOOKUP}\n    \
                 }}\n    gen.writeEndArray()\n"
            )
        );
    }

    #[test]
    fn should_null_check_an_optional_named_vec_payload() {
        let emitted = write(&TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string()))), true)
            .expect("Option<Vec<Named>> needs a runtime-typed write");
        assert_eq!(
            emitted,
            "    val elem = value.payload\n    if (elem == null) {\n        gen.writeNull()\n    } else {\n        \
             gen.writeStartArray()\n        for (elem1 in elem) {\n            \
             provider.findTypedValueSerializer(elem1.javaClass, true, null).serialize(elem1, gen, provider)\n        \
             }\n        gen.writeEndArray()\n    }\n"
        );
    }

    #[test]
    fn should_write_a_string_keyed_named_map_as_an_object_of_runtime_typed_values() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Node".to_string())));
        let emitted = write(&ty, false).expect("Map<String, Named> needs a runtime-typed write");
        assert_eq!(
            emitted,
            "    gen.writeStartObject()\n    for (entry in value.payload) {\n        \
             gen.writeFieldName(entry.key.toString())\n        \
             provider.findTypedValueSerializer(entry.value.javaClass, true, null).serialize(entry.value, gen, \
             provider)\n    }\n    gen.writeEndObject()\n"
        );
    }

    #[test]
    fn should_leave_a_named_keyed_map_on_the_mapper_path_because_to_string_is_not_the_wire_key() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::Named("Kind".to_string())),
            Box::new(TypeRef::Named("Node".to_string())),
        );
        assert_eq!(write(&ty, false), None);
    }

    #[test]
    fn should_nest_arrays_for_a_vec_of_vec_of_named() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string())))));
        let emitted = write(&ty, false).expect("Vec<Vec<Named>> needs a runtime-typed write");
        assert!(
            emitted.contains("for (elem in value.payload) {"),
            "outer loop must iterate the payload; got:\n{emitted}"
        );
        assert!(
            emitted.contains("for (elem1 in elem) {"),
            "inner loop must iterate the outer element; got:\n{emitted}"
        );
        assert!(
            emitted.contains("provider.findTypedValueSerializer(elem1.javaClass, true, null)"),
            "the innermost element is the one that needs the typed lookup; got:\n{emitted}"
        );
        assert_eq!(emitted.matches("gen.writeStartArray()").count(), 2, "two array levels");
    }

    #[test]
    fn should_null_check_optional_elements_inside_a_vec() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Named(
            "Node".to_string(),
        )))));
        let emitted = write(&ty, false).expect("Vec<Option<Named>> needs a runtime-typed write");
        assert!(
            emitted.contains("if (elem1 == null) {"),
            "a nullable element must be null-checked before the typed lookup; got:\n{emitted}"
        );
        assert!(
            emitted.contains("provider.findTypedValueSerializer(elem1.javaClass, true, null)"),
            "the non-null branch still needs the typed lookup; got:\n{emitted}"
        );
    }

    #[test]
    fn should_never_emit_the_erased_forms() {
        let shapes = [
            TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string()))),
            TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Named(
                "Node".to_string(),
            ))))),
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Node".to_string()))))),
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Node".to_string()))),
        ];
        for ty in shapes {
            let emitted = write(&ty, false).expect("container of Named needs a runtime-typed write");
            assert!(
                !emitted.contains("mapper.writeValue("),
                "a handled shape must not fall back to the erased mapper.writeValue; got:\n{emitted}"
            );
            assert!(
                !emitted.contains("findValueSerializer("),
                "an untyped findValueSerializer lookup drops the discriminator; got:\n{emitted}"
            );
            assert!(
                !emitted.contains("findTypedValueSerializer(Node::class.java"),
                "the typed lookup must be given the RUNTIME class, not the declared base; got:\n{emitted}"
            );
        }
    }
}
