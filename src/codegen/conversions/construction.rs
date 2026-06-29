//! Shared construction strategy for binding→core `From` impls when the core type has
//! private (non-`pub`) fields.
//!
//! A core struct with a `pub(crate)`/private field cannot be built with struct-literal
//! syntax from a foreign crate — neither by naming the field (`E0451` / "cannot construct
//! ... due to private fields") nor by patching it via `..Default::default()` (the spread
//! still requires the omitted private fields to be accessible). The only foreign-crate
//! construction path is to seed the core type's `Default` (built inside the defining crate,
//! so it fills the private fields) and assign the public fields onto it.
//!
//! Every backend computes its own per-field conversion expressions, but the *strategy* — when
//! to use the `Default`-seeded builder, and what to emit when the core type has no `Default` —
//! is identical. It lives here so the pyo3/napi/wasm/… shared generator, the Dart mirror-crate
//! generator, and the PHP struct-conversion generator stay in lockstep.

/// A single public field assignment in a `Default`-seeded builder: the core (Rust) field
/// name and the conversion expression that produces its value (referencing the `From` impl's
/// parameter, e.g. `val.content.into()`).
pub struct FieldAssign {
    pub core_field: String,
    pub expr: String,
}

/// Inputs for emitting a private-field `From<Binding> for Core` impl.
pub struct PrivateFieldImpl<'a> {
    /// Fully-qualified core type path (e.g. `sample_core::ExtractionResult`).
    pub core_path: &'a str,
    /// Binding mirror type name (e.g. `JsOcrExtractionResult`, `OcrExtractionResult`).
    pub binding_name: &'a str,
    /// `From::from` parameter name the assignment expressions reference (`val`, `v`, …).
    pub param: &'a str,
    /// Whether the core type implements `Default` (derive or manual).
    pub has_default: bool,
    /// Public-field assignments to apply onto the seeded base.
    pub assignments: &'a [FieldAssign],
    /// Extra `#[allow(...)]` lint groups to emit on the impl (backend-specific).
    pub allow_attrs: &'a [&'a str],
}

/// Emit the full `impl From<Binding> for Core` block for a private-field core type.
///
/// When the core type derives `Default`, emits the `Default`-seeded builder. Otherwise emits a
/// `compile_error!` guiding the core author to derive `Default` (or expose a constructor /
/// exclude the type) — a clear contract violation message instead of code that cannot compile.
pub fn gen_private_field_from_impl(spec: &PrivateFieldImpl) -> String {
    let statements: Vec<String> = spec
        .assignments
        .iter()
        .map(|a| format!("__result.{} = {};", a.core_field, a.expr))
        .collect();

    let construct_error = if spec.has_default {
        String::new()
    } else {
        format!(
            "alef cannot generate From<{binding}> for {core}: the core type has private fields and \
             does not implement Default, so it cannot be constructed from a binding value. Derive \
             Default on {core} (or expose a public constructor / exclude it from this backend).",
            binding = spec.binding_name,
            core = spec.core_path,
        )
    };

    crate::codegen::template_env::render(
        "conversions/private_field_from_impl",
        minijinja::context! {
            core_path => spec.core_path,
            binding_name => spec.binding_name,
            param => spec.param,
            allow_attrs => spec.allow_attrs,
            construct_error => construct_error,
            statements => statements,
        },
    )
}
