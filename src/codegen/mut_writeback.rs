//! Policy for `&mut T` parameters that a generated binding must hand back to its caller.
//!
//! A core function shaped like `fn tag_record(record: &mut Record)` cannot be bound as an
//! owned by-value parameter returning void: every backend converts the host DTO into an owned
//! `_core` intermediate, mutates that, and drops it. The caller's value is untouched and there
//! is no diagnostic — the call silently does nothing.
//!
//! Every backend that lowers such a parameter must consult this module so the eight host
//! languages agree on one answer instead of each inventing its own.
//!
//! The supported shape is **exactly one `&mut T` DTO parameter on a unit-returning function**.
//! The binding then returns the updated `T`. Any other `&mut` DTO shape is rejected at
//! generation time by [`reject_unsupported_writeback`] rather than emitted lossily.

use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;

/// Whether `param` is a `&mut T` parameter whose `T` is a non-opaque (serde DTO) named type.
///
/// Opaque named types are excluded: the host holds a live handle for those, so a backend that
/// mutates through the handle is already correct and must not be rewritten. ~keep
pub fn is_writeback_param(param: &ParamDef, opaque_types: &AHashSet<String>) -> bool {
    if !param.is_ref || !param.is_mut || param.optional {
        return false;
    }
    match &param.ty {
        TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
        _ => false,
    }
}

/// Every `&mut T` DTO parameter in `params`, in declaration order.
pub fn writeback_params<'a>(params: &'a [ParamDef], opaque_types: &AHashSet<String>) -> Vec<&'a ParamDef> {
    params.iter().filter(|p| is_writeback_param(p, opaque_types)).collect()
}

/// The single `&mut T` DTO parameter a binding must write back, when the signature is supported.
///
/// Returns `None` both when there is no such parameter and when the shape is one
/// [`reject_unsupported_writeback`] rejects, so callers that forget to call the rejection
/// helper degrade to "no rewrite" rather than to a half-applied rewrite.
pub fn writeback_param<'a>(
    params: &'a [ParamDef],
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
) -> Option<&'a ParamDef> {
    if !matches!(return_type, TypeRef::Unit) {
        return None;
    }
    let found = writeback_params(params, opaque_types);
    match found.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

/// The named DTO type a binding must return for `param`, e.g. `Record` for `record: &mut Record`.
pub fn writeback_type_name(param: &ParamDef) -> Option<&str> {
    match &param.ty {
        TypeRef::Named(name) => Some(name.as_str()),
        _ => None,
    }
}

/// The return type a binding must declare in place of `return_type`, when a write-back applies.
pub fn effective_return_type(
    params: &[ParamDef],
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
) -> Option<TypeRef> {
    writeback_param(params, return_type, opaque_types).map(|p| p.ty.clone())
}

/// Reject `&mut T` DTO shapes no host language in the matrix can express.
///
/// Two shapes are rejected, both of which would otherwise emit a binding that accepts the
/// argument and silently discards the mutation:
///
/// - more than one `&mut T` DTO parameter — the binding has only one return slot;
/// - a `&mut T` DTO parameter on a function that already returns a value — same reason.
///
/// `function_name` is the core Rust function or method name, so the diagnostic points the
/// consumer at the signature to change.
pub fn reject_unsupported_writeback(
    function_name: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
) -> anyhow::Result<()> {
    let found = writeback_params(params, opaque_types);
    if found.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
    if found.len() > 1 {
        anyhow::bail!(
            "`{function_name}` takes {} `&mut` parameters ({}); a binding has one return slot, so at \
             most one is supported. Change the core signature to take one `&mut` parameter, or to take \
             the values by move and return them.",
            found.len(),
            names.join(", "),
        );
    }
    if !matches!(return_type, TypeRef::Unit) {
        anyhow::bail!(
            "`{function_name}` takes a `&mut` parameter (`{}`) and also returns a value; the single \
             return slot already carries the updated `&mut` value. Change the core signature to return \
             the updated value itself, or to fold both results into one returned type.",
            names[0],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
