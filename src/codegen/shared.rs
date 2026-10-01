use crate::codegen::field_init::struct_field_init;
use crate::core::ir::{DefaultValue, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use ahash::AHashSet;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub use super::crate_attributes::{format_crate_attributes, format_extra_clippy_allows};

/// Matches a bare zero-argument path call — `a::b::c()` — and nothing else: no trailing
/// `.into()`, no trailing field access, no arguments inside the parens. Anchored at both ends
/// so `downstream::Policy::from_env().into()` and
/// `serde_json::from_str::<T>(r#"{}"#).field` (both of which need the closure to stay, the
/// former for its conversion and the latter for its side-effecting expect/deserialize call)
/// never match. ~keep
static BARE_ZERO_ARG_CALL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*\(\)$").expect("valid regex")
});

/// `unwrap_or_else(|| default_val)` is redundant when `default_val` is exactly a bare
/// zero-argument path call: `unwrap_or_else(path::to::fn)` passes the function itself, which
/// clippy's `redundant_closure` lint requires. Any other shape (arguments, a trailing `.into()`
/// or field access) must keep the closure, since only a call expression is a valid function-item
/// substitute for `unwrap_or_else`. ~keep
fn unwrap_or_else_default(binding_name: &str, field_name: &str, default_val: &str) -> String {
    if BARE_ZERO_ARG_CALL.is_match(default_val) {
        let path = default_val
            .strip_suffix("()")
            .expect("BARE_ZERO_ARG_CALL guarantees a `()` suffix");
        return format!("{binding_name}: {field_name}.unwrap_or_else({path})");
    }
    format!("{binding_name}: {field_name}.unwrap_or_else(|| {default_val})")
}

/// Recursively replace `Named(n)` references where `n` is excluded from the binding's public surface
/// (e.g. `InternalDocument`) with `TypeRef::Json`. An excluded type is never emitted as a binding
/// declaration, so a trait-bridge interface/stub method referencing it would be an undefined name;
/// the runtime bridge marshals such values as JSON, so `Json` is the faithful stand-in. Shared by
/// the go/pyo3/napi/magnus excluded-type handling so the substitution stays identical across them.
pub fn substitute_excluded_types(ty: &TypeRef, excluded: &HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if excluded.contains(name.as_str()) => TypeRef::Json,
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_excluded_types(inner, excluded))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_excluded_types(inner, excluded))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(substitute_excluded_types(k, excluded)),
            Box::new(substitute_excluded_types(v, excluded)),
        ),
        other => other.clone(),
    }
}

/// Recursively replace `Named(n)` references where `n` is a trait exposed via a host-implementable
/// RBS/language interface (e.g. `DocumentExtractor`) with `TypeRef::Named("_{n}")`. Traits are never
/// emitted as a class/struct declaration themselves — the trait-bridge backend surfaces them as an
/// `interface _TraitName` (or the target language's equivalent) that host code implements, so a
/// signature referencing the bare trait name would be an undeclared type. Shared so the substitution
/// stays identical across generators that need it (currently Ruby/Magnus RBS stubs).
pub fn substitute_trait_interfaces(ty: &TypeRef, trait_interfaces: &HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if trait_interfaces.contains(name.as_str()) => TypeRef::Named(format!("_{name}")),
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_trait_interfaces(inner, trait_interfaces))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_trait_interfaces(inner, trait_interfaces))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(substitute_trait_interfaces(k, trait_interfaces)),
            Box::new(substitute_trait_interfaces(v, trait_interfaces)),
        ),
        other => other.clone(),
    }
}

/// Fields that should be emitted in generated binding structs.
///
/// Source-level binding exclusions (`#[doc(hidden)]` / `#[cfg_attr(alef, alef(skip))]`)
/// keep the field in IR so conversion code can still default the core field, but public
/// language DTOs must not expose it.
pub fn binding_fields(fields: &[FieldDef]) -> impl Iterator<Item = &FieldDef> {
    fields.iter().filter(|field| !field.binding_excluded)
}

/// Returns true if this parameter is required but must be promoted to optional
/// because it follows an optional parameter in the list.
/// PyO3 requires that required params come before all optional params.
pub fn is_promoted_optional(params: &[ParamDef], idx: usize) -> bool {
    if params[idx].optional {
        return false;
    }
    params[..idx].iter().any(|p| p.optional)
}

/// Check if a free function can be auto-delegated to the core crate.
/// Opaque Named params are allowed (unwrapped via Arc). Non-opaque Named params are not
/// (require From impls that may not exist for types with sanitized fields).
///
/// For extendr R backend: slice params `&[T]` (represented as `Vec<T>` with `is_ref=true`)
/// are delegatable because extendr can convert them to `Vec<T>` at the boundary.
pub fn can_auto_delegate_function(func: &crate::core::ir::FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func.params.iter().all(|p| {
            !p.sanitized
                && is_delegatable_param_with_slices(&p.ty, opaque_types)
                && !is_named_ref_param(p, opaque_types)
        })
        && is_delegatable_return(&func.return_type)
}

/// Check if all params and return type are delegatable.
/// For opaque types, skip methods with RefMut receiver (cannot borrow Arc mutably).
///
/// For extendr R backend: slice params `&[T]` (represented as `Vec<T>` with `is_ref=true`)
/// are delegatable because extendr can convert them to `Vec<T>` at the boundary.
pub fn can_auto_delegate(method: &MethodDef, opaque_types: &AHashSet<String>) -> bool {
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() {
        return false;
    }
    !method.sanitized
        && method.params.iter().all(|p| {
            !p.sanitized
                && is_delegatable_param_with_slices(&p.ty, opaque_types)
                && !is_named_ref_param(p, opaque_types)
        })
        && is_delegatable_return(&method.return_type)
}

/// Like [`can_auto_delegate`] but permits non-opaque Named `&T` params (and `&[T]` / `&[&str]`
/// slices), because the caller emits owned `_core` let-bindings and passes a borrow of them.
///
/// This is the method-level analogue of the free-function `can_delegate_with_named_let_bindings`
/// check. It must only be used by generators whose delegation body wires up
/// `gen_named_let_bindings_pub` + `gen_call_args_with_let_bindings` (e.g. the shared static
/// method generator); generators that emit inline `.into()` call args must keep using
/// [`can_auto_delegate`].
pub fn can_auto_delegate_with_named_let_bindings(method: &MethodDef, opaque_types: &AHashSet<String>) -> bool {
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() {
        return false;
    }
    !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && is_delegatable_param_with_slices(&p.ty, opaque_types))
        && is_delegatable_return(&method.return_type)
}

/// A Named param with is_ref=true needs a let-binding (can't inline .into() + borrow).
/// A `Vec<String>` param with is_ref=true needs conversion to `Vec<&str>`.
/// A `Vec<NonOpaqueNamed>` param with is_ref=true needs a let-binding (gen_php_call_args emits
/// `&{name}_core[..]` which is only valid when a let binding for `{name}_core` exists).
/// Public alias for use by backend-specific codegen (e.g. napi types.rs opaque delegate check).
pub fn is_named_ref_param_pub(p: &crate::core::ir::ParamDef, opaque_types: &AHashSet<String>) -> bool {
    is_named_ref_param(p, opaque_types)
}

fn is_named_ref_param(p: &crate::core::ir::ParamDef, opaque_types: &AHashSet<String>) -> bool {
    if !p.is_ref {
        return false;
    }
    match &p.ty {
        TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char => true,
            TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
            _ => false,
        },
        _ => false,
    }
}

/// A param type is delegatable if it's simple, or a Named type (opaque → Arc unwrap, non-opaque → .into()).
///
/// `Json` is delegatable: the binding takes a JSON string and `gen_call_args` emits
/// `serde_json::from_str(...)` to bridge it into the core `serde_json::Value` parameter.
/// All Rust-based bindings already depend on serde_json (Json field round-tripping uses it).
pub fn is_delegatable_param(ty: &TypeRef, _opaque_types: &AHashSet<String>) -> bool {
    is_delegatable_param_with_slices(ty, _opaque_types)
}

/// Like `is_delegatable_param` but aware of slice parameters `&[T]` (represented as `Vec<T>` with `is_ref=true`).
/// Extendr R backend can auto-delegate slices by converting them to owned `Vec<T>` at the boundary.
fn is_delegatable_param_with_slices(ty: &TypeRef, _opaque_types: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => is_delegatable_param_with_slices(inner, _opaque_types),
        TypeRef::Vec(inner) => is_delegatable_param_with_slices(inner, _opaque_types),
        TypeRef::Map(k, v) => {
            is_delegatable_param_with_slices(k, _opaque_types) && is_delegatable_param_with_slices(v, _opaque_types)
        }
    }
}

/// Return types are more permissive — Named types work via .into() (core→binding From exists).
///
/// `Json` is delegatable: the binding returns a JSON string and the core `serde_json::Value`
/// is serialized via `.to_string()` by `wrap_return_with_mutex_mapped`.
pub fn is_delegatable_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_return(inner),
        TypeRef::Map(k, v) => is_delegatable_return(k) && is_delegatable_return(v),
    }
}

/// A type is delegatable if it can cross the binding boundary without From impls.
/// Named types are NOT delegatable as function params (may lack From impls).
/// For opaque methods, Named types are handled separately via Arc wrap/unwrap.
pub fn is_delegatable_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Named(_) => false,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_type(inner),
        TypeRef::Map(k, v) => is_delegatable_type(k) && is_delegatable_type(v),
        TypeRef::Json => false,
    }
}

/// Check if a type is delegatable in the opaque method context.
/// Opaque methods can handle Named params via Arc unwrap and Named returns via Arc wrap.
///
/// `Json` is delegatable: for params, `gen_call_args` emits `serde_json::from_str(&name)` to
/// bridge the binding's `String` into the core's `serde_json::Value`; for return types,
/// `wrap_return_with_mutex_mapped` serializes the `Value` back to a `String` via `.to_string()`.
/// All Rust-based bindings already depend on serde_json (Json field round-tripping uses it).
pub fn is_opaque_delegatable_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_opaque_delegatable_type(inner),
        TypeRef::Map(k, v) => is_opaque_delegatable_type(k) && is_opaque_delegatable_type(v),
    }
}

/// Check if a type is "simple" — can be passed without any conversion.
pub fn is_simple_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_simple_type(inner),
        TypeRef::Map(k, v) => is_simple_type(k) && is_simple_type(v),
        TypeRef::Named(_) | TypeRef::Json => false,
    }
}

/// Partition methods into (instance, static).
pub fn partition_methods(methods: &[MethodDef]) -> (Vec<&MethodDef>, Vec<&MethodDef>) {
    let instance: Vec<_> = methods.iter().filter(|m| m.receiver.is_some()).collect();
    let statics: Vec<_> = methods.iter().filter(|m| m.receiver.is_none()).collect();
    (instance, statics)
}

/// Build a constructor parameter list string.
/// Returns (param_list, signature_with_defaults, field_assignments).
/// If param_list exceeds 100 chars, uses multiline format with trailing commas.
pub fn constructor_parts(fields: &[FieldDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> (String, String, String) {
    constructor_parts_with_renames_and_cfg_restore(fields, type_mapper, None, &[])
}

/// Like `constructor_parts` but with optional field renames for keyword escaping.
/// `field_renames` maps original field name → binding field name (e.g. "class" → "class_").
/// Parameters keep the original name (valid in Rust), struct literal uses the renamed field.
pub fn constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    constructor_parts_with_renames_and_cfg_restore(fields, type_mapper, field_renames, &[])
}

/// Like `constructor_parts_with_renames` but also includes assignments for cfg-gated fields
/// that have been force-restored via trait-bridge `bind_via = "options_field"`. Such fields
/// are absent from the constructor parameter list but must be present in the `Self { ... }`
/// struct literal — emitted as `field: Default::default()` so the binding struct compiles.
pub fn constructor_parts_with_renames_and_cfg_restore(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
) -> (String, String, String) {
    let mut sorted_fields: Vec<&FieldDef> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name))
        .collect();
    sorted_fields.sort_by_key(|f| (f.optional || f.cfg.is_some()) as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let is_optional = f.optional || f.cfg.is_some();
            let ty = if is_optional {
                match &f.ty {
                    TypeRef::Optional(_) => type_mapper(&f.ty),
                    _ => format!("Option<{}>", type_mapper(&f.ty)),
                }
            } else {
                type_mapper(&f.ty)
            };
            format!("{}: {}", f.name, ty)
        })
        .collect();

    let defaults: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            if f.optional || f.cfg.is_some() {
                format!("{}=None", f.name)
            } else {
                f.name.clone()
            }
        })
        .collect();

    let assignments: Vec<String> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() && !never_skip_cfg_field_names.contains(&f.name) {
                return format!("{}: Default::default()", binding_name);
            }
            if binding_name != f.name {
                return binding_name.to_string();
            }
            f.name.clone()
        })
        .collect();

    let single_line = params.join(", ");
    let param_list = if single_line.len() > 100 {
        format!("\n        {},\n    ", params.join(",\n        "))
    } else {
        single_line
    };

    (param_list, defaults.join(", "), assignments.join(", "))
}

/// Build a function parameter list.
pub fn function_params(params: &[ParamDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    function_params_vec(params, type_mapper).join(", ")
}

/// Per-parameter `name: type` strings, before joining. Callers that render the signature
/// across multiple lines (long-signature wrapping) must reuse this exact list rather than
/// recomputing types — a separate recomputation diverges from the backend-aware mapping used
/// for the single-line form (e.g. extendr's `Nullable<&T>`), producing a signature whose types
/// disagree with the generated body and fail to compile.
pub fn function_params_vec(params: &[ParamDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> Vec<String> {
    let mut seen_optional = false;
    params
        .iter()
        .map(|p| {
            if p.optional {
                seen_optional = true;
            }
            let ty = if p.optional || seen_optional {
                format!("Option<{}>", type_mapper(&p.ty))
            } else {
                type_mapper(&p.ty)
            };
            format!("{}: {}", p.name, ty)
        })
        .collect::<Vec<_>>()
}

/// Build a function signature defaults string (for pyo3 signature etc.).
pub fn function_sig_defaults(params: &[ParamDef]) -> String {
    let mut seen_optional = false;
    params
        .iter()
        .map(|p| {
            if p.optional {
                seen_optional = true;
            }
            if p.optional {
                format!("{}=None", p.name)
            } else if seen_optional {
                let default = match &p.ty {
                    TypeRef::Primitive(PrimitiveType::Bool) => "false",
                    TypeRef::Primitive(_) => "0",
                    _ => "None",
                };
                format!("{}={}", p.name, default)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The digits of a `DefaultValue::FloatLiteral` as a floating-point literal, or `None` when the
/// value has no literal form at all.
///
/// Two failure modes are shared by every curly-brace target language and were, until this
/// existed, re-derived independently in each backend — which is how Kotlin came to emit
/// `val ratio: Double = 1` for a Rust `1.0_f64`:
///
/// - Rust's `Display` for `f64` prints a whole number with no decimal point, and `1` is an
///   *integer* literal in Java, Kotlin, C#, Swift and TypeScript alike. In a boxed or explicitly
///   typed floating-point position that is a type error, not a widening conversion.
/// - `NaN` and the infinities print as `NaN`/`inf`, which name nothing in any of those
///   languages. `None` here means "this default has no literal", which is strictly better than
///   source that does not parse; callers fall back to leaving the field required.
///
/// Type suffixes (`f`, `F`, `d`) stay with the caller: they depend on the target language *and*
/// on whether the field is `f32` or `f64`, which this function deliberately does not know.
/// `src/backends/swift/gen_bindings/dto.rs` still carries its own copy of this rule and should
/// adopt this one; it is the oracle the cross-language default control compares against, so the
/// two must not drift. ~keep
pub fn float_literal_digits(value: f64) -> Option<String> {
    if value.is_nan() || value.is_infinite() {
        return None;
    }
    Some(if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    })
}

/// Format a field's `DefaultValue` as Rust code for the target language.
/// Used by backends generating config constructors with defaults.
///
/// `typ` is the owning type of `field`, needed because a `#[serde(default = "path")]`
/// function (`DefaultValue::FunctionCall`) is frequently private to the source crate and/or
/// `#[cfg(feature = "serde")]`-gated: emitting `path()` directly into a generated binding
/// crate does not compile (`E0425`). `FunctionCall` and `PublicFunctionCall` defaults are
/// therefore both routed through [`crate::codegen::config_gen::default_value_for_field_in_type`],
/// the same recovery mechanism the Magnus/NAPI/PHP/Rustler backends use: `PublicFunctionCall`
/// is already known-callable and is emitted as a direct `path()` call, while `FunctionCall`
/// is recovered by deserializing a minimal JSON stub through the owning type's own
/// `Deserialize` impl (or, when that is not possible, a `compile_error!` naming the
/// unrecoverable field — callers MUST also call
/// [`crate::codegen::config_gen::validate_rust_default_functions`] before generation so that
/// case fails generation instead of shipping uncompilable Rust).
///
/// The declared type of `field` matters for a second, independent reason: serde requires a
/// `#[serde(default = "path")]` function to return exactly the field's type, so when that type
/// is `TypeRef::Named` the recovered expression evaluates to the *core* crate's type. Every
/// caller of `format_default_value` (currently the wasm backend's
/// `config_constructor_parts_with_options`, and — via the shared `gen_constructor` generator —
/// the extendr and pyo3 backends) maps `Named` fields to a distinct binding wrapper type, so
/// the recovered value needs `.into()` to become the type the constructor parameter expects —
/// otherwise the emitted assignment is an `E0308` type mismatch (wrapper vs. core type).
/// `mapped_ty` is that binding-side type as the caller's own type mapper renders it, and it is
/// required because the mapping is not always a wrapper: the wasm backend degrades some `Named`
/// fields to an opaque `JsValue`, which implements no `From<CoreType>`, so `.into()` there is an
/// `E0277` and the value must cross through serde instead. A
/// `compile_error!` recovery failure is left unconverted: it never compiles regardless, and
/// appending `.into()` would only obscure the diagnostic. Every other `DefaultValue` variant
/// already produces a value in the field's own representation and needs no conversion.
/// Render one element of a collection-literal default as Rust source.
///
/// Scalar-only by design: a nested list, an empty marker and a function-call default all need
/// context this element position does not carry, so they return `None` and the caller falls back
/// to `Default::default()` for the collection as a whole. ~keep
fn rust_scalar_default(item: &DefaultValue) -> Option<String> {
    match item {
        DefaultValue::BoolLiteral(b) => Some(format!("{b}")),
        DefaultValue::StringLiteral(s) => Some(format!("\"{}\".to_string()", s.escape_default())),
        DefaultValue::IntLiteral(i) => Some(format!("{i}")),
        DefaultValue::FloatLiteral(f) => {
            let s = format!("{f}");
            Some(if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{s}.0")
            })
        }
        DefaultValue::EnumVariant(v) => Some(v.clone()),
        // Neither variant's own payload is a spellable Rust path on its own (the enclosing enum
        // type is not in reach here), and this position has no `typ` to recover the real value
        // through `core_default_field_access` the way `format_default_value` does. Matches the
        // rest of this all-or-nothing group: the caller falls back to `Default::default()` for
        // the whole collection rather than a partial literal. ~keep
        DefaultValue::TupleVariant(_, _)
        | DefaultValue::StructVariant(_, _)
        | DefaultValue::ListLiteral(_)
        | DefaultValue::Empty
        | DefaultValue::Unresolved(_)
        | DefaultValue::None
        | DefaultValue::FunctionCall(_)
        | DefaultValue::PublicFunctionCall(_) => None,
    }
}

/// True when a backend's type mapper rendered this binding-side type as wasm-bindgen's opaque
/// `JsValue`.
///
/// The binding type — never the IR `TypeRef` on its own — decides which conversion an emitter is
/// allowed to write: `JsValue` implements no `From<CoreType>`, so every `.into()` aimed at it is
/// an `E0277`, and the value has to cross through `serde_wasm_bindgen` instead. The test is
/// textual because a `TypeMapper` hands back a rendered type *string* and nothing richer;
/// comparing only the final `::` segment accepts every spelling a mapper can produce
/// (`JsValue`, `wasm_bindgen::JsValue`, `::wasm_bindgen::JsValue`, `wasm_bindgen::prelude::JsValue`)
/// rather than a fixed pair of literals. It still cannot see through a type alias, which is
/// exactly why every emitter shares this one predicate: a spelling it misses is fixed here once,
/// not once per call site. ~keep
pub fn maps_to_js_value(mapped_ty: &str) -> bool {
    mapped_ty
        .trim()
        .trim_start_matches("::")
        .rsplit("::")
        .next()
        .is_some_and(|segment| segment == "JsValue")
}

/// The value the *source crate's own* `Default` gives `field`, as a Rust expression, or `None`
/// when the owning type cannot be named or has no `Default` to read.
///
/// This is the delegation route for defaults alef can see the shape of but not the value of.
/// [`DefaultValue::EnumVariant`] is the case that needs it: the extractor keeps only the variant
/// name (`SomeEnum::Variant` lowers to `EnumVariant("Variant")` — see
/// `extract::extractor::defaults`), so there is no path to spell in generated Rust, and the
/// binding cannot reconstruct one from `field.ty` because a `Named` type's binding-side spelling
/// is a wrapper, not the core enum. Reading the value back off `<CoreType as Default>::default()`
/// asks the source crate instead of guessing, which is the same principle the NAPI and extendr
/// backends apply by seeding their builders from `CoreType::default()`.
///
/// Direct field access is sound because alef only extracts `pub` fields
/// (`extract::extractor::types` filters on `is_pub`), and it is the same access the serde-stub
/// recovery in `codegen::config_gen::default_value_for_field_in_type` already emits. Unlike that
/// recovery this needs no `Deserialize` impl and cannot panic at runtime — but it is only valid
/// where the owning type really implements `Default`, hence the `has_default` guard. ~keep
pub(crate) fn core_default_field_access(field: &FieldDef, typ: &TypeDef) -> Option<String> {
    if !typ.has_default || typ.rust_path.is_empty() {
        return None;
    }
    let core_path = typ.rust_path.replace('-', "_");
    Some(format!(
        "<{core_path} as ::core::default::Default>::default().{}",
        field.name
    ))
}

/// Wrap a recovered *core-type* expression so it becomes the binding-side type `mapped_ty`.
///
/// A `Named` field usually maps to a per-type binding wrapper reachable by `.into()`, but the
/// wasm backend degrades some of them to an opaque `JsValue` (see `backends::wasm::type_map`).
/// `JsValue` has no `From<CoreType>` impl, so `.into()` there is an E0277; serde is the only
/// bridge, matching what the generated `From<CoreType> for WasmType` bodies already emit for the
/// same fields. Every non-`Named` field already evaluates to its own representation. ~keep
fn convert_core_default_expr(field: &FieldDef, mapped_ty: &str, expr: String) -> String {
    if !matches!(field.ty, TypeRef::Named(_)) {
        return expr;
    }
    if maps_to_js_value(mapped_ty) {
        format!("serde_wasm_bindgen::to_value(&{expr}).unwrap_or(wasm_bindgen::JsValue::NULL)")
    } else {
        format!("{expr}.into()")
    }
}

pub fn format_default_value(field: &FieldDef, typ: &TypeDef, mapped_ty: &str) -> String {
    let default = field
        .typed_default
        .as_ref()
        .expect("format_default_value: caller must have already confirmed field.typed_default is Some");
    match default {
        DefaultValue::BoolLiteral(b) => format!("{}", b),
        DefaultValue::StringLiteral(s) => format!("\"{}\".to_string()", s.escape_default()),
        DefaultValue::IntLiteral(i) => format!("{}", i),
        DefaultValue::FloatLiteral(f) => {
            let s = format!("{}", f);
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{s}.0")
            }
        }
        // The bare variant name is not a Rust path and never compiles on its own; it is kept only
        // as the last-resort spelling for callers that already refuse to use this arm (see
        // `config_constructor_parts_inner`, which falls back to `unwrap_or_default()` when the
        // owning type carries no `Default` to read the real variant off). ~keep
        DefaultValue::EnumVariant(v) => match core_default_field_access(field, typ) {
            Some(access) => convert_core_default_expr(field, mapped_ty, access),
            None => v.clone(),
        },
        // Neither payload is a spellable Rust literal on its own — the enclosing enum's path is
        // not part of either variant, only the bare variant/field names are — but the owning
        // struct's own `Default` impl, when it has one, already computed the real value; reading
        // it back off `<CoreType as Default>::default().field` (the same recovery `EnumVariant`
        // uses above) asks the source crate instead of guessing at a spelling. ~keep
        DefaultValue::TupleVariant(_, _) | DefaultValue::StructVariant(_, _) => {
            match core_default_field_access(field, typ) {
                Some(access) => convert_core_default_expr(field, mapped_ty, access),
                None => "Default::default()".to_string(),
            }
        }
        DefaultValue::ListLiteral(items) => {
            let rendered: Option<Vec<String>> = items.iter().map(rust_scalar_default).collect();
            // A non-scalar element falls back to `Default::default()` rather than a partial
            // literal, matching the extractor's all-or-nothing rule. ~keep
            match rendered {
                Some(values) => format!("vec![{}]", values.join(", ")),
                None => "Default::default()".to_string(),
            }
        }
        // `Unresolved` renders exactly like `Empty` here on purpose. This is a *renderer*, and a
        // renderer has no way to fail; refusing to guess is the validation pass's job
        // (`ValidationCode::UnreadableFieldDefault`), which runs before any backend reaches this
        // code. Reaching here with `Unresolved` therefore means the crate explicitly suppressed
        // that diagnostic and accepted the type-zero. ~keep
        DefaultValue::Empty | DefaultValue::Unresolved(_) => "Default::default()".to_string(),
        DefaultValue::None => "None".to_string(),
        DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => {
            let recovered = crate::codegen::config_gen::default_value_for_field_in_type(field, "rust", typ);
            // A `compile_error!` recovery failure is left unconverted: it never compiles
            // regardless, and appending `.into()` would only obscure the diagnostic. ~keep
            if recovered.starts_with("compile_error!") {
                return recovered;
            }
            convert_core_default_expr(field, mapped_ty, recovered)
        }
    }
}

/// Generate constructor parameter and assignment lists for types with has_default.
/// All fields become `Option<T>` with None defaults for optional fields,
/// or unwrap_or_else with actual defaults for required fields.
///
/// Returns (param_list, signature_defaults, assignments).
/// This is used by PyO3 and similar backends that need signature annotations.
/// Like `config_constructor_parts` but with extra options.
/// When `option_duration_on_defaults` is true, non-optional Duration fields are stored
/// as `Option<u64>` in the binding struct, so the constructor assignment is a passthrough
/// (the From conversion will handle the None → core default mapping).
pub fn config_constructor_parts_with_options(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_with_options_cfg(fields, type_mapper, option_duration_on_defaults, false, typ)
}

/// Generate a constructor that optionalizes only bare serde defaults resolved to enum variants.
/// Required siblings and non-enum defaults stay required without a whole-struct `Default`. ~keep
pub fn constructor_parts_with_field_defaults(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        ConstructorOptionality::FieldDefaults,
        None,
        &[],
        typ,
    )
}

#[derive(Clone, Copy)]
enum ConstructorOptionality {
    All {
        option_duration_on_defaults: bool,
        optionalize_all_defaults: bool,
    },
    FieldDefaults,
}

pub fn config_constructor_parts_with_options_cfg(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    optionalize_all_defaults: bool,
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        ConstructorOptionality::All {
            option_duration_on_defaults,
            optionalize_all_defaults,
        },
        None,
        &[],
        typ,
    )
}

/// Like `config_constructor_parts_with_options` but with field renames for keyword escaping.
pub fn config_constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        ConstructorOptionality::All {
            option_duration_on_defaults,
            optionalize_all_defaults: false,
        },
        field_renames,
        &[],
        typ,
    )
}

/// Like `config_constructor_parts_with_renames` but includes assignments for cfg-gated fields
/// force-restored via `never_skip_cfg_field_names` (emitted as `field: Default::default()`).
pub fn config_constructor_parts_with_renames_and_cfg_restore(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        ConstructorOptionality::All {
            option_duration_on_defaults,
            optionalize_all_defaults: false,
        },
        field_renames,
        never_skip_cfg_field_names,
        typ,
    )
}

pub fn config_constructor_parts(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    typ: &TypeDef,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        ConstructorOptionality::All {
            option_duration_on_defaults: false,
            optionalize_all_defaults: false,
        },
        None,
        &[],
        typ,
    )
}

fn config_constructor_parts_inner(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    optionality: ConstructorOptionality,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
    typ: &TypeDef,
) -> (String, String, String) {
    let (option_duration_on_defaults, optionalize_all_defaults, optionalize_all_fields) = match optionality {
        ConstructorOptionality::All {
            option_duration_on_defaults,
            optionalize_all_defaults,
        } => (option_duration_on_defaults, optionalize_all_defaults, true),
        ConstructorOptionality::FieldDefaults => (false, false, false),
    };
    let mut sorted_fields: Vec<&FieldDef> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name))
        .collect();
    sorted_fields.sort_by_key(|f| {
        if optionalize_all_fields {
            f.optional as u8
        } else {
            (f.optional || f.has_bare_serde_enum_default()) as u8
        }
    });

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let ty = type_mapper(&f.ty);
            let is_optional = optionalize_all_fields || f.optional || f.has_bare_serde_enum_default();
            if is_optional && !matches!(f.ty, TypeRef::Optional(_)) {
                format!("{}: Option<{}>", f.name, ty)
            } else {
                format!("{}: {}", f.name, ty)
            }
        })
        .collect();

    let defaults = sorted_fields
        .iter()
        .filter(|f| optionalize_all_fields || f.optional || f.has_bare_serde_enum_default())
        .map(|f| format!("{}=None", f.name))
        .collect::<Vec<_>>()
        .join(", ");

    let assignments: Vec<String> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() {
                if never_skip_cfg_field_names.contains(&f.name) {
                    if f.optional || matches!(&f.ty, TypeRef::Optional(_)) {
                        return struct_field_init(binding_name, &f.name);
                    }
                    return format!("{}: {}.unwrap_or_default()", binding_name, f.name);
                }
                return format!("{}: Default::default()", binding_name);
            }
            if (option_duration_on_defaults && matches!(f.ty, TypeRef::Duration)) || optionalize_all_defaults {
                return struct_field_init(binding_name, &f.name);
            }
            if f.optional || matches!(&f.ty, TypeRef::Optional(_)) {
                struct_field_init(binding_name, &f.name)
            } else if !optionalize_all_fields && !f.has_bare_serde_enum_default() {
                struct_field_init(binding_name, &f.name)
            } else if let Some(ref typed_default) = f.typed_default {
                match typed_default {
                    // `Empty` *is* `Default::default()`, so the binding type's own default is the
                    // right answer by construction.
                    DefaultValue::Empty => {
                        format!("{}: {}.unwrap_or_default()", binding_name, f.name)
                    }
                    // `EnumVariant` is not. `unwrap_or_default()` here calls the *field type's*
                    // `Default`, which is a different value from the variant this field defaults
                    // to whenever the two disagree — `#[derive(Default)] enum Mode { #[default]
                    // Slow, Fast }` with `mode: Mode::Fast` shipped `Slow` to every wasm, pyo3 and
                    // extendr caller. The variant name alone cannot be spelled as a path (the
                    // extractor drops the enum), so the value is read back off the owning type's
                    // own `Default` instead. Types with no `Default` to read keep the old
                    // rendering: it is the only expression available, not a claim of correctness. ~keep
                    DefaultValue::EnumVariant(_) if core_default_field_access(f, typ).is_none() => {
                        format!("{}: {}.unwrap_or_default()", binding_name, f.name)
                    }
                    _ => {
                        let default_val = format_default_value(f, typ, &type_mapper(&f.ty));
                        // clippy::unnecessary_lazy_evaluations; use unwrap_or_else for heap types.
                        match typed_default {
                            DefaultValue::BoolLiteral(_)
                            | DefaultValue::IntLiteral(_)
                            | DefaultValue::FloatLiteral(_) => {
                                format!("{}: {}.unwrap_or({})", binding_name, f.name, default_val)
                            }
                            _ => unwrap_or_else_default(binding_name, &f.name, &default_val),
                        }
                    }
                }
            } else if optionalize_all_fields {
                format!("{}: {}.unwrap_or_default()", binding_name, f.name)
            } else {
                struct_field_init(binding_name, &f.name)
            }
        })
        .collect();

    let single_line = params.join(", ");
    let param_list = if single_line.len() > 100 {
        format!("\n        {},\n    ", params.join(",\n        "))
    } else {
        single_line
    };

    (param_list, defaults, assignments.join(", "))
}

#[cfg(test)]
mod tests;
