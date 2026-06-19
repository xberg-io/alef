//! Post-processing rewriter for flutter_rust_bridge-generated `lib.dart`.
//!
//! flutter_rust_bridge emits sealed-class tagged-union variants with positional
//! parameter names (`field0`, `field1`, ...) when the underlying Rust variant
//! is a tuple variant:
//!
//! ```dart
//! const factory FormatMetadata.pdf({required PdfMetadata field0}) =
//!     FormatMetadata_Pdf;
//! ```
//!
//! These positional names are awkward for callers and inconsistent with the
//! kotlin/swift/etc. binding surface, which derives payload-informed names
//! (`metadata`, `value`, `value0`, ...) using the shared algorithm defined in
//! `alef-backend-kotlin::gen_bindings::shared::kotlin_field_name_with_type`.
//!
//! [`rewrite_frb_sealed_variants`] post-processes the frb-generated source and
//! rewrites variant parameter names to match the payload-derived convention.
//! Other code in the file is left untouched.
//!
//! Algorithm (per variant declaration line(s)):
//! 1. Match the canonical frb sealed-variant signature:
//!    `const factory <Enum>.<variantCamel>({required <PayloadType> field<N>, ...}) = <Enum>_<VariantPascal>;`
//! 2. Recover the `VariantPascal` token from the trailing assignment so that
//!    the variant name is unambiguous (the dotted form is lowerCamel, which
//!    cannot be reliably inverted back to PascalCase for multi-word variants).
//! 3. For each `field<N>` parameter, derive its new name from the payload type
//!    using the payload-derived helper (see [`payload_param_name`]).

#[path = "frb_rewrite/external_library_loader.rs"]
mod external_library_loader;
#[path = "frb_rewrite/imports_helpers.rs"]
mod imports_helpers;
#[path = "frb_rewrite/sealed_variants.rs"]
mod sealed_variants;
#[cfg(test)]
#[path = "frb_rewrite/tests.rs"]
mod tests;
#[path = "frb_rewrite/text_transformations.rs"]
mod text_transformations;

pub use sealed_variants::rewrite_frb_sealed_variants;
pub use text_transformations::{
    filter_excluded_functions, fix_handler_executor_calls, inject_display_as_text_methods,
    make_struct_fields_with_defaults_optional,
};
