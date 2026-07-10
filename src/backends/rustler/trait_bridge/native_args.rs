//! Build native-term callback arguments for the Rustler plugin trait bridge.
//!
//! The plugin bridge dispatches a trait call to the Elixir host by sending a
//! `{:trait_call, method, args, reply_id}` message, where `args` is a NATIVE Erlang term map (built
//! inside `OwnedEnv::send_and_clear`), not a JSON string. To build that map after moving into the
//! dispatch closure, each argument is first materialised into an OWNED Rust value (before the
//! `spawn_blocking`), then encoded through Rustler's [`rustler::Encoder`] inside the closure where
//! an `env` is available.
//!
//! - A known serde-struct param (per the shared
//!   [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] allowlist) is
//!   materialised as the binding's `NifStruct`/`NifMap` via the same `From<core::T>` conversion used
//!   for return values, so the host receives a native struct/map.
//! - Strings, primitives, booleans, and lists are materialised as their natural owned Rust values.
//! - Enums, opaque/handle types, and unknown `Named` params fall back to a debug string.

use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};
use std::collections::HashSet;

/// One marshalled callback argument: the owned binding emitted before the dispatch closure, and the
/// map key under which the encoded value is inserted into the native args map.
pub(super) struct NativeArg {
    /// Map key (the parameter name, stripped of any leading underscore).
    pub key: String,
    /// Local binding name holding the owned value (`{name}_arg`).
    pub binding: String,
    /// Expression that materialises the owned value, evaluated BEFORE `spawn_blocking` while the
    /// borrowed params are still in scope.
    pub owned_expr: String,
}

/// Build the native-arg descriptors for a trait method's params.
///
/// `struct_param_types` is the shared serde-struct allowlist; a `Named` param in this set is
/// materialised as the binding struct (`{Name}::from(...)`) so it encodes as a native term.
pub(super) fn build_native_args(params: &[ParamDef], struct_param_types: &HashSet<String>) -> Vec<NativeArg> {
    params
        .iter()
        .map(|p| {
            let key = p.name.strip_prefix('_').unwrap_or(&p.name).to_string();
            NativeArg {
                key,
                binding: format!("{}_arg", p.name),
                owned_expr: owned_arg_expr(p, struct_param_types),
            }
        })
        .collect()
}

/// Produce the owned, `Encoder`-able value expression for a single param.
fn owned_arg_expr(p: &ParamDef, struct_param_types: &HashSet<String>) -> String {
    let name = &p.name;

    if let TypeRef::Named(n) = &p.ty {
        if struct_param_types.contains(n) {
            if p.is_ref {
                return format!("{n}::from((*{name}).clone())");
            }
            return format!("{n}::from({name}.clone())");
        }
    }

    if p.optional && matches!(&p.ty, TypeRef::String) {
        return format!("{name}.map(|s| s.to_string())");
    }

    match &p.ty {
        TypeRef::String | TypeRef::Char => {
            if p.is_ref {
                format!("{name}.to_string()")
            } else {
                format!("{name}.clone()")
            }
        }
        TypeRef::Primitive(
            PrimitiveType::Bool
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::Usize
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::I64
            | PrimitiveType::Isize
            | PrimitiveType::F32
            | PrimitiveType::F64,
        ) => name.clone(),
        TypeRef::Vec(_) => {
            if p.is_ref {
                format!("{name}.to_vec()")
            } else {
                format!("{name}.clone()")
            }
        }
        _ => format!("format!(\"{{:?}}\", {name})"),
    }
}
