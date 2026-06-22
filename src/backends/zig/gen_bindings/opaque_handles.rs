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
use self::streaming::emit_streaming_struct;
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
            let struct_name = format!("{}Stream", item_type);
            if !emitted_stream_structs.contains(&struct_name) {
                emit_streaming_struct(method, ty, prefix, type_snake, item_type, declared_errors, out);
                out.push('\n');
                emitted_stream_structs.insert(struct_name);
            }
        }
    }
}

fn emit_static_methods(
    ty: &TypeDef,
    prefix: &str,
    declared_errors: &[String],
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    // A static method returning a borrowed reference to its own opaque type (e.g.
    // `Registry::global() -> &'static Registry`) has no FFI symbol — the FFI backend
    // cannot box a borrow into an owned `*mut T` handle, so it skips the export. Mirror
    // that omission here so the Zig wrapper does not call a missing C symbol.
    for method in ty
        .methods
        .iter()
        .filter(|m| m.is_static && !m.returns_ref_to_owner(&ty.name))
    {
        emit_opaque_static_method(method, ty, prefix, declared_errors, struct_names, enum_names, out);
        out.push('\n');
    }
}
