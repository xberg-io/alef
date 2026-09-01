//! Field initialisers for the `#[php(constructor)] pub fn new(...)` a PHP-mirrored struct gets.
//!
//! Split out of `structs.rs` so the rule deciding what a field the constructor *cannot accept as
//! a parameter* is initialised with lives in one place, next to the tests that pin it.

use std::borrow::Cow;

use ahash::AHashSet;

use super::php_field_can_be_constructor_param;
use crate::core::ir::{DefaultValue, FieldDef, TypeDef, TypeRef};

pub(crate) fn php_binding_keeps_field(field: &FieldDef, never_skip_cfg_field_names: &[String]) -> bool {
    !field.binding_excluded && (field.cfg.is_none() || never_skip_cfg_field_names.contains(&field.name))
}

pub(crate) fn php_binding_type<'a>(typ: &'a TypeDef, never_skip_cfg_field_names: &[String]) -> Cow<'a, TypeDef> {
    if typ
        .fields
        .iter()
        .all(|field| php_binding_keeps_field(field, never_skip_cfg_field_names))
    {
        return Cow::Borrowed(typ);
    }
    let mut binding_type = typ.clone();
    binding_type
        .fields
        .retain(|field| php_binding_keeps_field(field, never_skip_cfg_field_names));
    Cow::Owned(binding_type)
}

/// Starting point for the local the generated constructor binds the core type's `Default` to, so
/// a constructor that omits several fields reads them all off one delegating call instead of one
/// call per field. Only a starting point: [`core_defaults_local`] lengthens it until it cannot
/// collide, because a consumer crate's identifiers are adversarial input and no fixed prefix is a
/// guarantee. ~keep
const CORE_DEFAULTS_BASE: &str = "__alef_core_defaults";

/// Every identifier that is, or could become, a binding in the generated constructor's scope.
///
/// Deliberately a superset of what any single constructor shape emits — the raw field name (used
/// by the shorthand initialiser), the PHP parameter name, and the `*_core` / `*_core_result`
/// locals the `Vec<Named>` let-binding template introduces — computed for *every* field rather
/// than only the ones this shape turns into parameters. Narrowing it to the exact emitted set
/// would re-couple the reservation to the parameter filter, so a later change to that filter
/// could reintroduce a collision without touching this function. ~keep
fn reserved_constructor_identifiers(typ: &TypeDef) -> AHashSet<String> {
    let mut reserved = AHashSet::new();
    for field in typ.fields.iter().filter(|f| !f.binding_excluded) {
        let php_param_name = crate::codegen::naming::to_php_name(&field.name);
        reserved.insert(format!("{php_param_name}_core_result"));
        reserved.insert(format!("{php_param_name}_core"));
        reserved.insert(php_param_name);
        reserved.insert(field.name.clone());
    }
    reserved
}

/// The name of the local holding `<Self as Default>::default()`, chosen so it cannot shadow any
/// parameter or local in the generated constructor.
///
/// A fixed name would be a convention, not a guarantee: if a consumer field or parameter ever
/// spelled it, the `let` would shadow the parameter and `Self { .. }` would bind the wrong value
/// at the wrong type — silently, because the shadowed binding still type-checks whenever the
/// types happen to agree. Appending `_` terminates because the reserved set is finite. ~keep
fn core_defaults_local(typ: &TypeDef) -> String {
    let reserved = reserved_constructor_identifiers(typ);
    let mut local = CORE_DEFAULTS_BASE.to_string();
    while reserved.contains(local.as_str()) {
        local.push('_');
    }
    local
}

/// The `Self { .. }` initialiser list for the named constructor, plus the statement it needs
/// when at least one omitted field is recovered from the core type's `Default`.
///
/// `Debug` is required, not decorative: the refusal tests assert with `expect_err`, whose Ok arm
/// formats the success value into the panic message. Both fields are plain generated-source
/// text, so there is nothing sensitive or large to leak into a failure report. ~keep
#[derive(Debug)]
pub(crate) struct ConstructorInit {
    /// Emitted immediately before `Self { .. }`. Empty when no field needed the recovery, so a
    /// constructor that does not use the local never binds it (and never trips `unused_variables`).
    pub(crate) prelude: String,
    /// Comma-joined `a, b: b_php, c: <core-defaults local>.c`.
    pub(crate) field_inits: String,
}

/// Where a constructor-omitted field's value comes from.
///
/// Every variant is a value alef can defend. `Default::default()` — the unconditional answer this
/// replaced — is defensible only under [`OmittedInit::TypeZero`], where the IR states the default
/// *is* the field type's own zero. Everywhere else it invents a value the source crate never
/// wrote, and the invention is not cosmetic: for an allow-list field the invented value is an
/// empty allow-list, which is the fail-*open* direction (nothing is on the list, so nothing is
/// checked against it). A deny-list fabricated empty fails open too, by denying nothing. Neither
/// is recoverable at runtime, because the caller has no way to see that a value was invented.
/// Anything this enum cannot account for fails generation instead. ~keep
#[derive(Debug, PartialEq, Eq)]
enum OmittedInit {
    /// `DefaultValue::Empty` / `DefaultValue::None`: the IR asserts the default is exactly this
    /// type's zero, so the target-language zero is exact rather than a guess.
    TypeZero,
    /// Read back off `<Self as Default>::default()`, which for a type with a core `Default` is
    /// the delegating impl (see [`classify_omitted_field`]).
    FromCoreDefault,
}

/// How a diagnostic names the type that owns an unrenderable field: the full Rust path when the
/// IR carries one, so the reader is sent to the definition rather than to a bare short name.
fn owning_type_path(typ: &TypeDef) -> &str {
    if typ.rust_path.is_empty() {
        &typ.name
    } else {
        &typ.rust_path
    }
}

/// Decide where a constructor-omitted field's value comes from, or fail generation.
///
/// `<Self as Default>::default()` is safe to read here because PHP emits a *delegating* `Default`
/// impl for every serde-mirrored type whose core type has one (`gen_php_struct` sets
/// `emit_delegating_default_impl` from `typ.has_default`, and PHP passes no
/// `emit_delegating_default_for_types` allow-list, so the impl is always the delegating one).
/// That impl is `<core::T as Default>::default().into()`, so reading a field off it needs no
/// per-field core-to-binding conversion — the whole struct has already been converted.
///
/// `DefaultValue::Unresolved` normally never reaches here: `unreadable_field_default_diagnostics`
/// (`cli::pipeline::generate::validation`) already fails the whole run for it. It does reach here
/// for a crate that suppresses that code, and the delegating read is the right answer then too —
/// alef could not *spell* the value, but the compiled `Default` impl still produces it. ~keep
///
/// `field.optional` is deliberately NOT an exemption. `None` for an omitted `Option` looks
/// principled — it is what an empty `Option` looks like anyway — but the generated stub promises
/// callers only that such a field is "not settable via the constructor" and says nothing about
/// its value, so `None` is a claim no part of the source crate made. The sibling PHP constructor
/// (`codegen::config_gen::php::gen_php_kwargs_constructor`) never invents it either: for an
/// optional field it passes the caller's own `Option` straight through. The one place alef does
/// map optional to `None` (`gen_struct_default_impl`) is guarded by `has_default`, which is the
/// branch above. An omitted `Option` with no `Default` anywhere is the same fabrication as an
/// empty allow-list, wearing a type that makes it look like absence. ~keep
///
/// Nor does alef fall back to `Default::default()` when the type has no `Default` impl: the
/// invented empty value — empty allow-list, empty deny-list, zero limit, null policy — is the
/// fail-open direction for a security control. Hence the bail rather than a synthesised value. ~keep
fn classify_omitted_field(typ: &TypeDef, field: &FieldDef) -> anyhow::Result<OmittedInit> {
    if matches!(field.typed_default, Some(DefaultValue::Empty | DefaultValue::None)) {
        return Ok(OmittedInit::TypeZero);
    }
    if typ.has_default {
        return Ok(OmittedInit::FromCoreDefault);
    }
    anyhow::bail!(
        "php backend: cannot initialise `{type_path}.{field_name}` in the generated \
         `#[php(constructor)] new(...)`: the field's type is not representable as a PHP constructor \
         parameter, and `{type_name}` has no `Default` impl to read the value from. Fix one of:\n  \
         - add `#[derive(Default)]` or `impl Default for {type_name}`;\n  \
         - mark the field `#[alef(skip)]` if PHP callers must never set it;\n  \
         - give the field a PHP-representable type so it becomes a real constructor parameter.",
        type_path = owning_type_path(typ),
        type_name = typ.name,
        field_name = field.name,
    )
}

/// True for a field type that `gen_php_function_params` (`helpers/params.rs`) renders as a
/// by-reference `#[php_class]` parameter — a bare `Named` (or `Optional<Named>`) type that is
/// neither an enum (moved as an owned PHP `string`) nor opaque (bridged separately). Such a
/// param is a *reference* to the caller's already-constructed mirror object, so the field
/// initialiser must `.clone()` it rather than move it. ~keep
fn is_named_struct_by_ref(ty: &TypeRef, enum_names: &AHashSet<String>, opaque_types: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => !enum_names.contains(name.as_str()) && !opaque_types.contains(name.as_str()),
        TypeRef::Optional(inner) => is_named_struct_by_ref(inner, enum_names, opaque_types),
        _ => false,
    }
}

/// The initialiser for a field the constructor *does* accept as a parameter, keyed off the same
/// param-name convention `gen_php_function_params` establishes for the parameter list.
///
/// `Vec<Named>` DOES have a `{param}_core` arm here, unlike a stale version of this comment used
/// to claim: `php_field_can_be_constructor_param`'s `Vec` arm now accepts a plain (non-opaque,
/// non-enum) struct element too, not only an opaque or enum one, so the two conditions this
/// comment used to call "mutually exclusive" are not. The sibling constructor in `structs.rs`
/// already emits the `{param}_core` let-binding (`php_vec_named_struct_let_binding.jinja`) for
/// every representable `Vec<Named>` field before this function runs; this arm only has to
/// reference the local it created, by the SAME `{php_param_name}_core` name.
fn representable_field_init(
    field: &FieldDef,
    php_param_name: &str,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> String {
    let is_bytes = matches!(&field.ty, TypeRef::Bytes)
        || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes));
    if is_bytes {
        if field.optional {
            return format!("{}: {php_param_name}.map(|b| b.0)", field.name);
        }
        return format!("{}: {php_param_name}.0", field.name);
    }
    // Symmetric with the Json GETTER (`ty_is_or_wraps_json` in `types.rs`), which returns a
    // serialized JSON `String` because `serde_json::Value` has no ext-php-rs `FromZval` impl: the
    // constructor takes the SAME `String` shape (`gen_php_function_params`'s `Json` arm) and
    // decodes it back with `serde_json::from_str`. A malformed string is refused with `?` rather
    // than silently defaulted to `Value::Null` -- fabricating a value the PHP caller's own input
    // never specified is exactly what the no-fabrication check exists to prevent, and unlike an
    // OMITTED field (which never runs this function at all) this value came from the caller, not
    // from alef inventing one, so `?`-propagating the real parse error is the honest outcome, not
    // an over-refusal. `?` type-checks here because `php_field_can_be_constructor_param`'s `Json`
    // arm makes `param_conversion_is_fallible` true for this field, which
    // `gen_struct_methods_impl` (`structs.rs`) reads to wrap the constructor in `PhpResult<Self>`
    // instead of the ordinarily-infallible bare `Self`. ~keep
    if matches!(&field.ty, TypeRef::Json) {
        if field.optional {
            return format!(
                "{}: {php_param_name}.map(|s| serde_json::from_str(&s)).transpose().map_err(|e|                  PhpException::default(e.to_string()))?",
                field.name
            );
        }
        return format!(
            "{}: serde_json::from_str(&{php_param_name}).map_err(|e| PhpException::default(e.to_string()))?",
            field.name
        );
    }
    if let TypeRef::Vec(inner) = &field.ty
        && let TypeRef::Named(name) = inner.as_ref()
        && !enum_names.contains(name.as_str())
        && !opaque_types.contains(name.as_str())
    {
        return format!("{}: {php_param_name}_core", field.name);
    }
    if is_named_struct_by_ref(&field.ty, enum_names, opaque_types) {
        if field.optional {
            // `.cloned()`, not `.map(|v| v.clone())` — same value on `Option<&T>`, but the
            // longer form trips `clippy::map_clone` in the CONSUMER crate this lands in, not in
            // alef. ~keep
            return format!("{}: {php_param_name}.cloned()", field.name);
        }
        return format!("{}: {php_param_name}.clone()", field.name);
    }
    if field.name == php_param_name {
        field.name.clone()
    } else {
        format!("{}: {php_param_name}", field.name)
    }
}

/// Build the `Self { .. }` initialiser list for the named constructor.
///
/// Fails when any omitted field's value is unknowable — see [`classify_omitted_field`].
pub(crate) fn gen_constructor_field_inits(
    typ: &TypeDef,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
    untagged_data_enum_names: &AHashSet<String>,
    never_skip_cfg_field_names: &[String],
) -> anyhow::Result<ConstructorInit> {
    let core_defaults = core_defaults_local(typ);
    let mut field_inits: Vec<String> = Vec::new();
    let mut needs_core_defaults = false;

    for field in typ
        .fields
        .iter()
        .filter(|field| php_binding_keeps_field(field, never_skip_cfg_field_names))
    {
        if field.cfg.is_some() {
            field_inits.push(format!("{}: Default::default()", field.name));
            continue;
        }
        if php_field_can_be_constructor_param(&field.ty, enum_names, opaque_types, untagged_data_enum_names) {
            let php_param_name = crate::codegen::naming::to_php_name(&field.name);
            field_inits.push(representable_field_init(
                field,
                &php_param_name,
                enum_names,
                opaque_types,
            ));
            continue;
        }
        match classify_omitted_field(typ, field)? {
            OmittedInit::TypeZero => field_inits.push(format!("{}: Default::default()", field.name)),
            OmittedInit::FromCoreDefault => {
                needs_core_defaults = true;
                field_inits.push(format!("{}: {core_defaults}.{}", field.name, field.name));
            }
        }
    }

    let prelude = if needs_core_defaults {
        crate::backends::php::template_env::render(
            "php_core_defaults_let_binding.jinja",
            minijinja::context! { binding => core_defaults.as_str() },
        )
    } else {
        String::new()
    };

    Ok(ConstructorInit {
        prelude,
        field_inits: field_inits.join(", "),
    })
}

#[cfg(test)]
#[path = "constructor_init/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "constructor_init/runtime_oracle.rs"]
mod runtime_oracle;
