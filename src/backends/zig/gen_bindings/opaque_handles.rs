mod constructors;
mod instance_methods;
mod params;
mod returns;
mod static_methods;
mod streaming;

use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::TypeDef;
use heck::AsSnakeCase;
use std::collections::{HashMap, HashSet};

use self::constructors::emit_opaque_constructor as emit_constructor_impl;
use self::instance_methods::{emit_opaque_free, emit_opaque_method};
use self::static_methods::emit_opaque_static_method;
use self::streaming::{StreamingContext, emit_streaming_struct, stream_struct_name};
use super::helpers::emit_cleaned_zig_doc;

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::zig::template_env::render(template_name, ctx)
}

/// Emit a top-level `pub fn create_<type_snake>(allocator, params...) !TypeName`
/// constructor that wraps the `c.{prefix}_{type_snake}_new(...)` FFI symbol.
pub(crate) fn emit_opaque_constructor(
    ty: &TypeDef,
    prefix: &str,
    ctor: &ClientConstructorConfig,
    top_level_names: &HashSet<String>,
    out: &mut String,
) {
    emit_constructor_impl(ty, prefix, ctor, top_level_names, out);
}

/// Emit a Zig struct wrapper for an opaque handle type (one with `is_opaque = true`
/// or `has_serde = false`) that has instance methods.
///
/// The emitted struct stores a `*anyopaque` handle obtained from the C FFI and
/// exposes each non-static, non-excluded method as a Zig function that dispatches
/// via `c.{prefix}_{snake_type}_{snake_method}(self._handle, ...)`.
///
/// Static methods are emitted as top-level Zig functions that call the FFI constructor,
/// e.g., `pub fn init(method: Method, path: []const u8) {TypeName}`.
pub(crate) fn emit_opaque_handle(
    ty: &TypeDef,
    prefix: &str,
    declared_errors: &[String],
    struct_names: &HashSet<String>,
    streaming_item_types: &HashMap<String, String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    let type_snake = AsSnakeCase(&ty.name).to_string();
    emit_streaming_structs(ty, prefix, declared_errors, streaming_item_types, &type_snake, out);
    emit_static_methods(ty, prefix, declared_errors, struct_names, enum_names, out);

    emit_cleaned_zig_doc(out, &ty.doc, "");
    out.push_str(&render(
        "opaque_handle_header.jinja",
        minijinja::context! {
            type_name => &ty.name,
        },
    ));
    out.push('\n');

    for method in ty.methods.iter().filter(|m| !m.is_static) {
        emit_opaque_method(
            method,
            ty,
            prefix,
            &type_snake,
            declared_errors,
            struct_names,
            streaming_item_types,
            enum_names,
            out,
        );
        out.push('\n');
    }

    emit_opaque_free(ty, prefix, &type_snake, out);
    out.push_str("};\n");
}

fn emit_streaming_structs(
    ty: &TypeDef,
    prefix: &str,
    declared_errors: &[String],
    streaming_item_types: &HashMap<String, String>,
    type_snake: &str,
    out: &mut String,
) {
    let mut emitted_stream_structs: HashSet<String> = HashSet::new();
    for method in ty.methods.iter().filter(|m| !m.is_static) {
        if let Some(item_type) = streaming_item_types.get(&method.name) {
            let method_snake = AsSnakeCase(&method.name).to_string();
            let struct_name = stream_struct_name(ty, &method_snake, item_type, streaming_item_types);
            if !emitted_stream_structs.contains(&struct_name) {
                emit_streaming_struct(
                    method,
                    &StreamingContext {
                        ty,
                        prefix,
                        type_snake,
                        item_type,
                        declared_errors,
                        streaming_item_types,
                    },
                    out,
                );
                out.push('\n');
                emitted_stream_structs.insert(struct_name);
            }
        }
    }
}

fn emit_static_methods(
    ty: &TypeDef,
    prefix: &str,
    _declared_errors: &[String],
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    for method in ty
        .methods
        .iter()
        .filter(|m| m.is_static && !m.returns_ref_to_owner(&ty.name))
    {
        emit_opaque_static_method(method, ty, prefix, struct_names, enum_names, out);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{MethodDef, TypeRef};

    #[test]
    fn opaque_handle_rejects_calls_after_idempotent_free() {
        let ty = TypeDef {
            name: "ClientHandle".to_owned(),
            rust_path: "sample::ClientHandle".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "status".to_owned(),
                is_static: false,
                return_type: TypeRef::String,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        };
        let mut output = String::new();

        emit_opaque_handle(
            &ty,
            "sample",
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &mut output,
        );

        assert!(output.contains("_handle: u64"));
        assert!(output.contains("if (handle == 0) return error.HandleClosed;"));
        assert!(output.contains("error{OutOfMemory,HandleClosed}!"));
        assert!(output.contains("if (handle == 0) return;"));
        assert!(output.contains("self._handle = 0;"));
        assert!(!output.contains("@ptrCast(handle)"));
    }

    #[test]
    fn opaque_handle_converts_c_integer_booleans() {
        let ty = TypeDef {
            name: "NodeHandle".to_owned(),
            rust_path: "sample::NodeHandle".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "is_named".to_owned(),
                is_static: false,
                return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        };
        let mut output = String::new();

        emit_opaque_handle(
            &ty,
            "sample",
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &mut output,
        );

        assert!(output.contains("return _result != 0;"), "{output}");
    }

    #[test]
    fn fallible_handle_returns_use_error_codes_and_zero_tokens() {
        let ty = TypeDef {
            name: "ClientHandle".to_owned(),
            rust_path: "sample::ClientHandle".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "child".to_owned(),
                is_static: false,
                return_type: TypeRef::Named("ChildHandle".to_owned()),
                error_type: Some("RequestError".to_owned()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        };
        let mut output = String::new();

        emit_opaque_handle(
            &ty,
            "sample",
            &["RequestError".to_owned()],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &mut output,
        );

        assert!(output.contains("sample_last_error_code() != 0"), "{output}");
        assert!(
            output.contains("if (_result == 0) return error.OutOfMemory"),
            "{output}"
        );
        assert!(!output.contains("_result == null"), "{output}");
    }

    /// Regression test for the emitter shape reported against `Parser.parse` /
    /// `Node.parent` / `Node.child`: a method declared to return `Optional<Named>`
    /// (e.g. `?Tree`, `?Node`) must wrap the raw handle into the struct instead of
    /// returning the raw FFI value bare, which does not type-check in Zig.
    #[test]
    fn optional_handle_returns_wrap_into_struct_instead_of_bare_raw_value() {
        let ty = TypeDef {
            name: "NodeHandle".to_owned(),
            rust_path: "sample::NodeHandle".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "parent".to_owned(),
                is_static: false,
                return_type: TypeRef::Optional(Box::new(TypeRef::Named("NodeHandle".to_owned()))),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        };
        let mut output = String::new();

        emit_opaque_handle(
            &ty,
            "sample",
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &mut output,
        );

        assert!(output.contains("!?NodeHandle"), "{output}");
        assert!(
            output.contains("return if (_result == 0) null else NodeHandle{ ._handle = _result };"),
            "{output}"
        );
        assert!(!output.contains("return _result;"), "{output}");

        // Positive control: `free()` legitimately returns nothing (`void`) and must still
        // emit a bare `return;` on the handle-already-closed path. If this assertion ever
        // fails, the fix above is over-wrapping and the negative assertion is vacuous.
        assert!(output.contains("if (handle == 0) return;"), "{output}");
    }

    /// Regression test for the reported defect: a method with no declared Rust error type,
    /// returning a bare opaque handle (`Tree.root_node() -> Node`, `TreeCursor.node() -> Node`,
    /// `Node.walk() -> TreeCursor`, `Tree.walk() -> TreeCursor`), must declare `OutOfMemory`
    /// alongside `HandleClosed` -- the body unconditionally emits
    /// `if (_result == 0) return error.OutOfMemory;` for this shape (see
    /// `opaque_handles::returns::method_unwrap_return_expr`'s bare-`Named` arm) regardless of
    /// whether a Rust error type was declared. This must fail against the pre-fix emitter, which
    /// declared only `error{HandleClosed}!NodeHandle` -- a set the body's own
    /// `error.OutOfMemory` cannot type-check against. ~keep
    #[test]
    fn infallible_handle_return_declares_out_of_memory_alongside_handle_closed() {
        let ty = TypeDef {
            name: "TreeHandle".to_owned(),
            rust_path: "sample::TreeHandle".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "root_node".to_owned(),
                is_static: false,
                return_type: TypeRef::Named("NodeHandle".to_owned()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        };
        let mut output = String::new();

        emit_opaque_handle(
            &ty,
            "sample",
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &mut output,
        );

        assert!(
            output.contains("error{OutOfMemory,HandleClosed}!NodeHandle"),
            "a handle-returning method with no declared Rust error must still declare \
             OutOfMemory alongside HandleClosed, matching its body's unconditional \
             `error.OutOfMemory`. Got:\n{output}"
        );
        assert!(
            output.contains("if (_result == 0) return error.OutOfMemory;"),
            "Got:\n{output}"
        );
        assert!(
            !output.contains("error{HandleClosed}!NodeHandle"),
            "must not regress to the pre-fix HandleClosed-only declared set. Got:\n{output}"
        );
    }
}
