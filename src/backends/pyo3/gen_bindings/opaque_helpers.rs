use crate::backends::pyo3::type_map::Pyo3Mapper;

/// For a wrapper type referenced by registration variants (i.e. one whose
/// `is_variant_wrapper` flag is set by the extractor), produce a `#[new]
/// pub fn py_new(...) -> Self { Self::new(...) }` method body suitable for
/// in-place insertion into the type's existing `#[pymethods] impl T { ... }`
/// block via [`inject_into_impl_block`].
///
/// Returns `None` when the wrapper has no `new` method (or the constructor's
/// receiver is not static) — the variant body would not compile in that
/// case either, but we silently skip rather than panic so the rest of the
/// surface can still be generated for diagnosis.
pub(super) fn variant_wrapper_constructor_body(typ: &crate::core::ir::TypeDef, mapper: &Pyo3Mapper) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    let call_args = ctor
        .params
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // The binding wrapper's static `new` already does the
    // `binding-side type → core-side type` conversion for each argument and
    // produces a `Self`; we just delegate.
    let body = if call_args.is_empty() {
        "Self::new()".to_string()
    } else {
        format!("Self::new({call_args})")
    };
    Some(format!(
        "    #[new]\n    pub fn py_new({sig_params}) -> Self {{\n        {body}\n    }}\n"
    ))
}

/// Check if a type has a no-arg `pub fn new() -> Self` method (either static or constructor).
/// This is used to determine whether we should emit an `impl Default for Type` block.
///
/// Returns true only when:
/// - The type has `has_default = true` (indicating it has impl Default in core Rust)
/// - The type has at least one method named "new"
/// - That method takes no parameters and is static (receiver.is_none())
/// - No existing `impl Default` is already present in the impl_block
pub(super) fn should_emit_default_impl(typ: &crate::core::ir::TypeDef, impl_block: &str) -> bool {
    // Only emit if the core Rust type has impl Default
    if !typ.has_default {
        return false;
    }

    // Check if Default impl already exists
    if impl_block.contains("impl Default") {
        return false;
    }

    // Check if there's a no-arg static new() method
    typ.methods.iter().any(|m| {
        m.name == "new" && m.params.is_empty() && m.receiver.is_none() // static method (not &self or &mut self)
    })
}

/// Generate an `impl Default for Type { fn default() -> Self { Self::new() } }` block
/// for a no-arg constructor. This satisfies clippy's `new_without_default` lint.
pub(super) fn emit_default_impl(type_name: &str) -> String {
    format!("impl Default for {type_name} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}\n")
}

/// Inject a method body into the existing `#[pymethods] impl T { ... }`
/// block produced by `gen_opaque_impl_block`. The block ends with a closing
/// `}`; the body is inserted right before it.
pub(super) fn inject_into_impl_block(impl_block: &str, body: &str) -> String {
    let trimmed = impl_block.trim_end();
    let Some(close_idx) = trimmed.rfind('}') else {
        return impl_block.to_string();
    };
    let (head, tail) = trimmed.split_at(close_idx);
    let head_trimmed = head.trim_end();
    format!("{head_trimmed}\n\n{body}{tail}\n")
}
