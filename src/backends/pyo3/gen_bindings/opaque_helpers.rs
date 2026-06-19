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

/// Whether to emit an `impl Default for Type` block for an opaque wrapper.
///
/// Returns true when:
/// - The type has `has_default = true` (indicating it has impl Default in core Rust), and
/// - No existing `impl Default` is already present in the impl_block.
///
/// The body is chosen by [`emit_default_impl`]: a no-arg static `new()` delegates to
/// `Self::new()` (idiomatic, satisfies clippy's `new_without_default`); otherwise the
/// core type's own `Default` is forwarded through the `inner` field. The latter case is
/// required: non-opaque parent structs that derive `Default` and hold this wrapper as a
/// field (e.g. `OcrRequest { document: OcrDocument }`) will not compile unless the
/// wrapper also implements `Default`, even when the wrapper has no no-arg constructor.
pub(super) fn should_emit_default_impl(typ: &crate::core::ir::TypeDef, impl_block: &str) -> bool {
    // Only emit if the core Rust type has impl Default
    if !typ.has_default {
        return false;
    }

    // Check if Default impl already exists
    !impl_block.contains("impl Default")
}

/// True when the type exposes a no-arg static `pub fn new() -> Self`.
fn has_no_arg_static_new(typ: &crate::core::ir::TypeDef) -> bool {
    typ.methods
        .iter()
        .any(|m| m.name == "new" && m.params.is_empty() && m.receiver.is_none())
}

/// Generate an `impl Default for Type` block. When the wrapper has a no-arg static
/// `new()`, delegate to it (`Self::new()`); otherwise forward the core type's `Default`
/// through the opaque `inner` field (`Self { inner: Default::default() }`).
pub(super) fn emit_default_impl(typ: &crate::core::ir::TypeDef) -> String {
    let body = if has_no_arg_static_new(typ) {
        "Self::new()".to_string()
    } else {
        "Self { inner: Default::default() }".to_string()
    };
    format!(
        "impl Default for {} {{\n    fn default() -> Self {{\n        {body}\n    }}\n}}\n",
        typ.name
    )
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
