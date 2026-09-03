//! ~keep DELIBERATE DUPLICATION: this module is a narrowed port of
//! `backends::php::gen_bindings::serde_defaults`, which solves the identical problem for the php
//! backend. It keeps only the literal-value and resolved-optional-function-call cases (the
//! shapes this defect actually has for pyo3) and drops php's enum-lowered-to-`String` wire-value
//! handling and its `NamedFunctionPath` cross-type source, neither of which apply here -- pyo3
//! keeps real enums as pyclass enums instead of lowering them to strings, and every core
//! primitive already commits to a pyo3-friendly width (see `primitive_return_type`), so there is
//! no width-narrowing cast to synthesize. The two copies WILL drift if edited independently and
//! must be kept in sync by hand until a third backend needs this exact mechanism -- per this
//! repo's own convention (extract shared logic after the third repetition, not the second), that
//! third backend is the signal to hoist both into `codegen::generators`, not before. Without this
//! note the drift is silent, which is exactly the failure php's own module documents guarding
//! against (the `#305` history in `serde_default_field_attr`'s doc comment).
//!
//! Synthesizes a `crate::serde_defaults::<...>` function for a struct field whose core
//! `#[serde(default = "path")]` cannot be mirrored verbatim onto the pyo3 binding struct.
//!
//! `path` frequently names a private, module-local free function next to the struct it defaults
//! for (e.g. `ExtractionConfig::use_cache` and `ExtractionConfig::enable_quality_processing`
//! both use `#[serde(default = "default_true")]`, `fn default_true() -> bool { true }` declared
//! in the same module). `postprocess::resolve_public_default_functions` only promotes
//! `Owner::method` paths to `DefaultValue::PublicFunctionCall`, so a bare free-function path
//! never becomes one -- but
//! `extract::extractor::defaults::function_default::fold_constant_default_functions` already
//! folds such a function's single-statement body into a literal `DefaultValue` (`BoolLiteral`,
//! `StringLiteral`, `IntLiteral`, `FloatLiteral`) when it can prove it, regardless of the
//! function's visibility. The shared `codegen::generators::structs::serde_default_field_attr`
//! fallback only ever re-emits the *original* `#[serde(default = "path")]` text verbatim (sound
//! only for `PublicFunctionCall`), so it correctly declines to copy an unresolvable free-function
//! path onto the mirror rather than emit code that fails to compile -- but that leaves the field
//! required, rejecting a partial JSON payload (e.g. `ExtractionConfig.from_json('{"chunking":
//! {...}}')`) that the core type itself accepts. Synthesizing our *own* function from the
//! already-folded literal sidesteps the path-resolution problem entirely: the mirror crate owns
//! the function it references, so there is nothing left to resolve.
//!
//! Also covers an `Option<T>` scalar field whose default is a *resolved* `PublicFunctionCall`
//! (e.g. `ExtractionConfig::extraction_timeout_secs`, `#[serde(default =
//! "ExtractionConfig::default_extraction_timeout")]`, `fn default_extraction_timeout() ->
//! Option<u64> { Some(600) }`): the function body is not a bare literal (`Some(600)` is a call
//! expression, not a literal `DefaultValue` this module folds), so the literal branch above
//! cannot cover it, and the shared `serde_default_field_attr` fallback's `PublicFunctionCall`
//! bare-copy path never gets tried for an `Option`-typed field at all -- both `codegen::
//! generators::structs::serde_default_field_attr` and this module's own literal branch treat an
//! `Option<T>` field as out of scope for the same reason a bare-literal body can't be re-wrapped
//! into `Some(...)` without guessing. A resolved `PublicFunctionCall`, in contrast, already
//! returns the field's *exact* `Option<T>` type (that is what `#[serde(default = "path")]`
//! requires of `path`), so calling it directly needs no wrapping or casting.

use crate::codegen::naming::pascal_to_snake;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn default_fn_ident(type_name: &str, field_name: &str) -> String {
    format!("{}_{}", pascal_to_snake(type_name), pascal_to_snake(field_name))
}

fn serde_default_path(default: Option<&str>) -> Option<&str> {
    let default = default?;
    let marker = "serde(default = \"";
    let start = default.find(marker)? + marker.len();
    let rest = &default[start..];
    let end = rest.find('"')?;
    let path = rest[..end].trim();
    (!path.is_empty()).then_some(path)
}

/// The pyo3 mirror field's exact Rust type for a core primitive. Core public types already
/// commit to pyo3-friendly widths (see `types::TesseractConfig`'s own doc comment: "Public API
/// uses i32 for PyO3 compatibility"), and alef performs no further narrowing for this backend
/// (`cast_uints_to_i32`/`cast_large_ints_to_f64` are both `false` in `config::binding_config`),
/// so this is a direct, non-narrowing mapping -- unlike php's `primitive_return_type`, which
/// narrows every wide integer to `i64`. ~keep
fn primitive_return_type(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
        PrimitiveType::Bool => "bool",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
    }
}

/// A folded `DefaultValue` literal rendered as `(return type, body expression)`, when `ty` is a
/// primitive or `String` field whose folded default is a literal of the matching kind. `None`
/// for every other combination (a `Named`/`Vec`/`Map`/`Optional` field, or a `DefaultValue` this
/// module has no literal rendering for) -- those are not this defect's shape and are left exactly
/// as the shared `serde_default_field_attr` fallback already leaves them. ~keep
fn typed_default_fn(default: &DefaultValue, ty: &TypeRef) -> Option<(&'static str, String)> {
    match (default, ty) {
        (DefaultValue::BoolLiteral(value), TypeRef::Primitive(PrimitiveType::Bool)) => {
            Some(("bool", value.to_string()))
        }
        (DefaultValue::StringLiteral(value) | DefaultValue::EnumVariant(value), TypeRef::String) => {
            Some(("String", format!("{value:?}.to_string()")))
        }
        (DefaultValue::IntLiteral(value), TypeRef::Primitive(primitive)) => {
            if matches!(primitive, PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64) {
                return None;
            }
            Some((primitive_return_type(primitive), value.to_string()))
        }
        (
            DefaultValue::FloatLiteral(value),
            TypeRef::Primitive(primitive @ (PrimitiveType::F32 | PrimitiveType::F64)),
        ) => {
            let rendered = format!("{value}");
            let body = if rendered.contains('.') || rendered.contains('e') {
                rendered
            } else {
                format!("{rendered}.0")
            };
            Some((primitive_return_type(primitive), body))
        }
        _ => None,
    }
}

/// The mirror field's full return type for an `Option<T>` scalar field (`Option<u64>`,
/// `Option<String>`, ...), or `None` when `field.ty` (already `Option`-unwrapped, per
/// `FieldDef::ty`'s own contract) is not a primitive or `String` -- a `Named`/`Vec`/`Map` inner
/// type is out of scope for the same reason `typed_default_fn` excludes it. ~keep
fn optional_scalar_return_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Primitive(primitive) => Some(format!("Option<{}>", primitive_return_type(primitive))),
        TypeRef::String => Some("Option<String>".to_string()),
        _ => None,
    }
}

/// The single decision point for "does `typ.field` get a `crate::serde_defaults::*` function?".
/// Both emitters go through it: [`gen_serde_defaults_module`] to write the `pub fn`, and the
/// pyo3 struct field-attribute closure (via [`serde_default_fn_name`]) to write the
/// `#[serde(default = "crate::serde_defaults::…")]` that references it -- so the reference side
/// can never name a function the definition side declines to emit. ~keep
fn serde_default_body(typ: &TypeDef, field: &FieldDef) -> Option<(String, String)> {
    if !typ.has_default || field.binding_excluded {
        return None;
    }
    // Only the valued form: a bare `#[serde(default)]` sentinel already renders correctly via
    // the shared `serde_default_field_attr` fallback and must not get a second, colliding
    // `serde(default...)` attribute from this module. This only proves a valued default exists
    // at all -- the callable path below is `field.typed_default`'s own resolved string once it
    // is a `PublicFunctionCall`, never this raw attribute text.
    // `resolve_public_default_functions` (`extract::extractor::postprocess`) rebuilds that
    // string from `typ.rust_path` (the owner's fully-qualified core path), which is very often
    // a *different* string from the bare `Owner::method` text written at the field's
    // `#[serde(default = "...")]` -- calling the raw text from inside `mod serde_defaults` (a
    // module nested below the crate root, not the core crate) would reference the wrong item or
    // fail to resolve at all. ~keep
    serde_default_path(field.default.as_deref())?;
    let default = field.typed_default.as_ref()?;

    if !field.optional
        && let Some((return_type, body)) = typed_default_fn(default, &field.ty)
    {
        return Some((return_type.to_string(), body));
    }

    // `Option<T>` field: only a resolved `PublicFunctionCall` is sound here -- its return type
    // is already the field's exact `Option<T>` (see this module's doc comment), so no wrapping
    // or casting is needed, and there is nothing left to guess. Uses the resolved, fully
    // -qualified call target from `typed_default` itself, never the raw attribute text above.
    // ~keep
    if field.optional
        && let DefaultValue::PublicFunctionCall(resolved) = default
        && let Some(return_type) = optional_scalar_return_type(&field.ty)
    {
        return Some((return_type, format!("{resolved}()")));
    }

    None
}

/// The `crate::serde_defaults::…` function name for a field, or `None` when no function will be
/// generated for it. The reference side must not emit an attribute when this returns `None`.
pub(super) fn serde_default_fn_name(typ: &TypeDef, field: &FieldDef) -> Option<String> {
    serde_default_body(typ, field).map(|_| default_fn_ident(&typ.name, &field.name))
}

/// Renders every synthesized default as a single `mod serde_defaults { ... }` item, or `None`
/// when no field in `api` needs one.
pub(super) fn gen_serde_defaults_module(api: &ApiSurface) -> Option<String> {
    let mut body = String::new();
    for typ in &api.types {
        for field in &typ.fields {
            let Some((return_type, expr)) = serde_default_body(typ, field) else {
                continue;
            };
            body.push_str(&format!(
                "    pub fn {}() -> {return_type} {{ {expr} }}\n",
                default_fn_ident(&typ.name, &field.name)
            ));
        }
    }
    if body.is_empty() {
        return None;
    }
    Some(format!("mod serde_defaults {{\n{body}}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, TypeDef};

    fn use_cache_field() -> FieldDef {
        FieldDef {
            name: "use_cache".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: Some("serde(default = \"default_true\")".to_string()),
            typed_default: Some(DefaultValue::BoolLiteral(true)),
            ..Default::default()
        }
    }

    // The raw attribute text (`default`) and the extractor's resolved, fully-qualified call
    // target (`typed_default`) are deliberately DIFFERENT strings here -- exactly the real
    // shape `resolve_public_default_functions` produces (it rebuilds the path from
    // `typ.rust_path`, not from the bare text written at the field). A regression that reverts
    // to calling the raw text would still pass a fixture where the two strings happen to match,
    // so this fixture must never make them equal. ~keep
    fn extraction_timeout_field() -> FieldDef {
        FieldDef {
            name: "extraction_timeout_secs".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            optional: true,
            default: Some("serde(default = \"ExtractionConfig::default_extraction_timeout\")".to_string()),
            typed_default: Some(DefaultValue::PublicFunctionCall(
                "xberg::core::config::ExtractionConfig::default_extraction_timeout".to_string(),
            )),
            ..Default::default()
        }
    }

    fn config_with_field(field: FieldDef) -> TypeDef {
        TypeDef {
            name: "ExtractionConfig".to_string(),
            has_default: true,
            fields: vec![field],
            ..Default::default()
        }
    }

    #[test]
    fn synthesizes_named_function_for_unresolvable_private_default() {
        let typ = config_with_field(use_cache_field());
        assert_eq!(
            serde_default_fn_name(&typ, &typ.fields[0]),
            Some("extraction_config_use_cache".to_string())
        );
    }

    #[test]
    fn module_defines_the_function_the_reference_side_names() {
        let typ = config_with_field(use_cache_field());
        let api = ApiSurface {
            types: vec![typ],
            ..Default::default()
        };
        let module = gen_serde_defaults_module(&api).expect("module generated");
        assert!(
            module.contains("pub fn extraction_config_use_cache() -> bool { true }"),
            "expected synthesized bool-literal default, got:\n{module}"
        );
    }

    #[test]
    fn bare_serde_default_sentinel_is_left_to_the_shared_fallback() {
        let mut field = use_cache_field();
        field.default = Some("/* serde(default) */".to_string());
        let typ = config_with_field(field);
        assert_eq!(serde_default_fn_name(&typ, &typ.fields[0]), None);
    }

    #[test]
    fn field_without_a_default_gets_no_function() {
        let mut field = use_cache_field();
        field.default = None;
        field.typed_default = None;
        let typ = config_with_field(field);
        assert_eq!(serde_default_fn_name(&typ, &typ.fields[0]), None);
    }

    #[test]
    fn optional_field_with_resolved_function_call_gets_a_wrapping_function() {
        let typ = config_with_field(extraction_timeout_field());
        let api = ApiSurface {
            types: vec![typ.clone()],
            ..Default::default()
        };
        assert_eq!(
            serde_default_fn_name(&typ, &typ.fields[0]),
            Some("extraction_config_extraction_timeout_secs".to_string())
        );
        let module = gen_serde_defaults_module(&api).expect("module generated");
        assert!(
            module.contains(
                "pub fn extraction_config_extraction_timeout_secs() -> Option<u64> { \
                 xberg::core::config::ExtractionConfig::default_extraction_timeout() }"
            ),
            "expected the RESOLVED (fully-qualified) call target, not the raw attribute text, \
             got:\n{module}"
        );
    }

    #[test]
    fn optional_field_with_unresolved_function_call_gets_no_function() {
        // Same shape as `extraction_timeout_field`, but the extractor never promoted the path to
        // `PublicFunctionCall` -- e.g. `postprocess::resolve_public_default_functions` could not
        // find a matching static method. Synthesizing a call here would be exactly the guess
        // this module exists to avoid; the field stays required, same as today. ~keep
        let mut field = extraction_timeout_field();
        field.typed_default = Some(DefaultValue::FunctionCall(
            "ExtractionConfig::default_extraction_timeout".to_string(),
        ));
        let typ = config_with_field(field);
        assert_eq!(serde_default_fn_name(&typ, &typ.fields[0]), None);
    }

    #[test]
    fn optional_named_type_field_gets_no_function() {
        // `Option<OcrConfig>`-shaped: `optional_scalar_return_type` only covers primitives and
        // `String`, so a resolved function call on a `Named` field must not synthesize a
        // (necessarily wrong) return type. ~keep
        let mut field = extraction_timeout_field();
        field.ty = TypeRef::Named("OcrConfig".to_string());
        let typ = config_with_field(field);
        assert_eq!(serde_default_fn_name(&typ, &typ.fields[0]), None);
    }
}
