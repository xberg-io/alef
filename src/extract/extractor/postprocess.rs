use crate::core::ir::{ApiSurface, DefaultValue, EnumDef, FieldDef, TypeRef};
use ahash::AHashMap;

use super::SerdeDefaultsByType;

/// Build a lookup of enum name → the name of its `#[default]`-marked unit variant.
///
/// Deliberately stricter than `default_value_for_enum::default_variant_name`: this lookup is
/// used to assert that two `DefaultValue`s denote the *same* concrete value, so it omits enums
/// with no genuine `#[default]` variant rather than falling back to "the first variant" — a
/// fallback appropriate for backends that must materialize *some* value, but wrong for a
/// comparison that would otherwise manufacture a false agreement between an enum with no
/// declared default and whichever variant happens to be listed first. ~keep
pub(super) fn enum_default_variant_names(enums: &[EnumDef]) -> AHashMap<String, String> {
    enums
        .iter()
        .filter(|enum_def| enum_def.has_default)
        .filter_map(|enum_def| {
            let variant = enum_def.variants.iter().find(|variant| {
                variant.is_default && variant.fields.is_empty() && !variant.originally_had_data_fields
            })?;
            Some((enum_def.name.clone(), variant.name.clone()))
        })
        .collect()
}

pub(super) fn resolve_public_default_functions(surface: &mut ApiSurface) {
    let public_methods: AHashMap<(String, String), String> = surface
        .types
        .iter()
        .flat_map(|typ| {
            typ.methods
                .iter()
                .filter(|method| method.is_static && method.params.is_empty() && !method.binding_excluded)
                .map(|method| {
                    (
                        (typ.name.clone(), method.name.clone()),
                        format!("{}::{}", typ.rust_path.replace('-', "_"), method.name),
                    )
                })
        })
        .collect();

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            let Some(DefaultValue::FunctionCall(path)) = &field.typed_default else {
                continue;
            };
            let segments: Vec<_> = path.split("::").collect();
            let [.., owner, method] = segments.as_slice() else {
                continue;
            };
            if let Some(resolved_path) = public_methods.get(&(owner.to_string(), method.to_string())) {
                field.typed_default = Some(DefaultValue::PublicFunctionCall(resolved_path.clone()));
            }
        }
    }
}

/// Resolve a field's `Empty` typed default to the concrete enum variant it stands for, when the
/// field's own declared type is an enum whose default is known.
///
/// `Empty` already asserts "the value is this field's own type's zero" (see
/// [`DefaultValue::Empty`]) — true whether the initializer was a bare `Default::default()` or
/// `<FieldType>::default()`, since a struct-literal field position can only be filled with a
/// value of the field's own declared type; the two spellings name the same value. For a `Named`
/// field whose type is an enum, "the type's zero" is one specific variant, not a value most
/// backends have an expression for, so this pass makes it concrete: the variant carrying
/// `EnumVariant::is_default`, set either by `#[derive(Default)]`'s `#[default]` attribute or by
/// reading a hand-written `impl Default`'s returned variant (see
/// `extract::extractor::functions::impl_blocks::manual_default_unit_variant`).
///
/// Only a *unit* variant is narrowed. `DefaultValue::EnumVariant` is documented to name a bare
/// unit-variant path with no arguments of its own, so a tuple or struct variant would need
/// `TupleVariant`/`StructVariant` and the payload this pass has no way to read; emitting a bare
/// name for one would fabricate a value that does not compile.
///
/// An enum whose default variant is unknown is left `Empty`, exactly as before this pass runs:
/// downstream backends already treat `Empty` on a `Named` field as "unknown" and fall back to
/// their own honest per-language guard (e.g. C#'s `required`). This pass only narrows what was
/// already unresolved into what most backends can render directly; it never turns a resolvable
/// value into a guess.
///
/// `field.optional` fields are skipped entirely, and the skip is the security-relevant half of
/// this pass. `extract::extractor::types::extract_struct` unwraps `Option<T>` before this runs,
/// so an `Option<Enum>` field reaches here with `field.ty` already collapsed to the bare `Enum`
/// and `field.optional == true` — the same shape as a genuinely required `Enum` field, apart
/// from that flag. `Empty` on such a field means "the field's own type, `Option<Enum>`, is at
/// its zero" — `None` — never "the wrapped `Enum`'s own zero". Narrowing it to `EnumVariant`
/// would materialize a concrete variant for a value the Rust source left deliberately absent;
/// every per-field-literal backend (Python, Kotlin, …) forwards a materialized `Some(variant)`
/// exactly like an explicit caller choice, so a per-file default silently overrides a stricter
/// global policy the caller never meant to relax. Leaving `Empty` in place keeps every
/// downstream consumer on the branch that already renders `None`/`null` for
/// `optional && Empty`. ~keep
pub(super) fn resolve_enum_field_defaults(surface: &mut ApiSurface) {
    let enum_default_variants = enum_default_variant_names(&surface.enums);

    if enum_default_variants.is_empty() {
        return;
    }

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            if field.optional {
                continue;
            }
            if !matches!(&field.typed_default, Some(DefaultValue::Empty)) {
                continue;
            }
            let TypeRef::Named(name) = &field.ty else {
                continue;
            };
            if let Some(variant) = enum_default_variants.get(name) {
                field.typed_default = Some(DefaultValue::EnumVariant(variant.clone()));
            }
        }
    }
}

/// Returns `true` if the type is a simple leaf type (primitive, String, Bytes, Path, etc.)
/// rather than a complex Named, collection, or Optional type.
fn is_simple_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Primitive(_)
            | TypeRef::String
            | TypeRef::Bytes
            | TypeRef::Path
            | TypeRef::Unit
            | TypeRef::Duration
            | TypeRef::Json
    )
}

/// Resolve newtype wrappers in the API surface.
///
/// Single-field tuple structs (`pub struct Foo(T)`) are identified by having exactly
/// one field named `_0`, no methods, and a simple inner type (primitive, String, etc.).
/// For each such newtype, all `TypeRef::Named("Foo")` references throughout the surface
/// are replaced with the inner type `T`, and the newtype TypeDef itself is removed.
/// This makes newtypes fully transparent to backends.
///
/// Tuple structs wrapping complex Named types (e.g., builders) are kept as-is.
pub(super) fn resolve_newtypes(surface: &mut ApiSurface) {
    let newtype_map: AHashMap<String, TypeRef> = surface
        .types
        .iter()
        .filter(|t| t.fields.len() == 1 && t.fields[0].name == "_0" && is_simple_type(&t.fields[0].ty))
        .map(|t| (t.name.clone(), t.fields[0].ty.clone()))
        .collect();

    if newtype_map.is_empty() {
        return;
    }

    let newtype_rust_paths: AHashMap<String, String> = surface
        .types
        .iter()
        .filter(|t| newtype_map.contains_key(&t.name))
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .collect();

    surface.types.retain(|t| !newtype_map.contains_key(&t.name));

    for typ in &mut surface.types {
        for field in &mut typ.fields {
            if let TypeRef::Named(name) = &field.ty
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            if let TypeRef::Optional(inner) = &field.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            if let TypeRef::Vec(inner) = &field.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                field.newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut field.ty);
        }
        for method in &mut typ.methods {
            for param in &mut method.params {
                if let TypeRef::Named(name) = &param.ty
                    && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
                {
                    param.newtype_wrapper = Some(rust_path.clone());
                }
                resolve_typeref(&newtype_map, &mut param.ty);
            }
            if let TypeRef::Named(name) = &method.return_type
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                method.return_newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut method.return_type);
        }
    }
    for func in &mut surface.functions {
        for param in &mut func.params {
            if let TypeRef::Named(name) = &param.ty
                && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
            {
                param.newtype_wrapper = Some(rust_path.clone());
            }
            resolve_typeref(&newtype_map, &mut param.ty);
        }
        if let TypeRef::Named(name) = &func.return_type
            && let Some(rust_path) = newtype_rust_paths.get(name.as_str())
        {
            func.return_newtype_wrapper = Some(rust_path.clone());
        }
        resolve_typeref(&newtype_map, &mut func.return_type);
    }
    for enum_def in &mut surface.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                resolve_typeref(&newtype_map, &mut field.ty);
            }
        }
    }
}

/// Recursively replace `TypeRef::Named(name)` with the newtype's inner type.
fn resolve_typeref(newtype_map: &AHashMap<String, TypeRef>, ty: &mut TypeRef) {
    match ty {
        TypeRef::Named(name) => {
            if let Some(inner) = newtype_map.get(name.as_str()) {
                *ty = inner.clone();
            }
        }
        TypeRef::Optional(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Vec(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Map(k, v) => {
            resolve_typeref(newtype_map, k);
            resolve_typeref(newtype_map, v);
        }
        _ => {}
    }
}

/// Resolve unresolved `trait_source` on methods after all source files have been processed.
///
/// When `impl Trait for Type` is encountered before the trait definition has been extracted
/// (e.g., `pub mod extractors` comes before `pub mod plugins` in lib.rs), the single-segment
/// trait name lookup fails because the trait `TypeDef` doesn't exist yet. This pass retroactively
/// resolves those methods by matching method names against trait types' method lists. ~keep
pub(super) fn resolve_trait_sources(surface: &mut ApiSurface) {
    let mut trait_method_map: AHashMap<String, Vec<(String, String)>> = AHashMap::new();
    let mut trait_methods_set: AHashMap<String, Vec<String>> = AHashMap::new();

    for typ in &surface.types {
        if !typ.is_trait {
            continue;
        }
        let method_names: Vec<String> = typ.methods.iter().map(|m| m.name.clone()).collect();
        trait_methods_set.insert(typ.name.clone(), method_names.clone());
        for method_name in &method_names {
            trait_method_map
                .entry(method_name.clone())
                .or_default()
                .push((typ.name.clone(), typ.rust_path.replace('-', "_")));
        }
    }

    if trait_method_map.is_empty() {
        return;
    }

    for typ in &mut surface.types {
        if typ.is_trait {
            continue;
        }

        let unresolved_names: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .map(|m| m.name.clone())
            .collect();

        for method in &mut typ.methods {
            if method.trait_source.is_some() {
                continue;
            }
            let Some(candidates) = trait_method_map.get(&method.name) else {
                continue;
            };

            if candidates.len() == 1 {
                method.trait_source = Some(candidates[0].1.clone());
            } else {
                let best = candidates.iter().max_by_key(|(trait_name, _)| {
                    trait_methods_set
                        .get(trait_name)
                        .map(|trait_methods| {
                            trait_methods
                                .iter()
                                .filter(|method_name| unresolved_names.contains(method_name))
                                .count()
                        })
                        .unwrap_or(0)
                });
                if let Some((_, rust_path)) = best {
                    method.trait_source = Some(rust_path.clone());
                }
            }
        }
    }
}

/// True when `value` carries no [`DefaultValue::Unresolved`] and no
/// [`DefaultValue::FunctionCall`]/[`DefaultValue::PublicFunctionCall`] anywhere within it
/// (including nested inside a `TupleVariant`/`StructVariant`/`ListLiteral` payload).
///
/// `Unresolved` is the documented "alef could not read this" marker and is never safe to
/// compare. `FunctionCall`/`PublicFunctionCall` get the same treatment though neither is
/// documented as unknown: each names a real zero-argument function whose return value alef
/// never evaluates, so two `FunctionCall`s naming different paths are exactly as undecidable as
/// one `Unresolved` value — asserting they disagree would be a guess about what the function
/// returns, not a reading. Only a value built entirely from literals, `Empty`, `None`, or
/// variants/lists of those is safe to compare. ~keep
fn is_fully_known(value: &DefaultValue) -> bool {
    match value {
        DefaultValue::Unresolved(_) | DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => false,
        DefaultValue::BoolLiteral(_)
        | DefaultValue::StringLiteral(_)
        | DefaultValue::IntLiteral(_)
        | DefaultValue::FloatLiteral(_)
        | DefaultValue::EnumVariant(_)
        | DefaultValue::Empty
        | DefaultValue::None => true,
        DefaultValue::TupleVariant(_, args) => args.iter().all(is_fully_known),
        DefaultValue::StructVariant(_, fields) => fields.iter().all(|(_, value)| is_fully_known(value)),
        DefaultValue::ListLiteral(items) => items.iter().all(is_fully_known),
    }
}

/// True when `value` is some spelling of "this type's zero" — [`DefaultValue::Empty`] itself,
/// or a literal that happens to equal the zero of its own literal kind (`IntLiteral(0)`,
/// `FloatLiteral(0.0)`, `BoolLiteral(false)`, `StringLiteral("")`, `DefaultValue::None`).
///
/// `Empty` is documented as *"the type's own zero"* (see [`DefaultValue::Empty`]), not as "no
/// value was recorded" — but the serde reader and the `impl Default` reader fold the same zero
/// value to different spellings. The serde reader can only ever write `Empty` for a bare
/// `#[serde(default)]` (`extract::extractor::helpers::fields::extract_field` never produces a
/// literal), while a hand-written `fn default() -> Self { Self { count: 0 } }` folds through
/// `extract::extractor::defaults::expr_to_default_value` to `IntLiteral(0)`, not `Empty`. Both
/// spellings name the same field default, so treating them as a disagreement here is the false
/// positive this function exists to avoid: it would fire on every zero-valued field a manual
/// `impl Default` writes out explicitly instead of leaving to the derive. ~keep
fn denotes_type_zero(value: &DefaultValue) -> bool {
    matches!(
        value,
        DefaultValue::Empty | DefaultValue::None | DefaultValue::BoolLiteral(false) | DefaultValue::IntLiteral(0)
    ) || matches!(value, DefaultValue::StringLiteral(s) if s.is_empty())
        || matches!(value, DefaultValue::FloatLiteral(f) if *f == 0.0)
}

/// True when `serde_default` and `actual_default` are `Empty` and an `EnumVariant` that name the
/// same value: an enum-typed field's "type zero" is not a literal `denotes_type_zero` can
/// recognize on its own, it is whichever variant carries `#[default]`.
///
/// `field_type` and `enum_default_variants` narrow this to only the case both sides can be
/// *proven* equal: `field_type` must be the field's own declared type (a struct-literal field
/// position can only ever be filled with a value of that type, so "the type's zero" and "this
/// field's zero" are the same question), and `enum_default_variants` must actually know that
/// type's `#[default]` variant. When the enum is not in the map — because it was not found, or
/// genuinely has no `#[default]` variant — this returns `false` and the caller falls back to
/// its ordinary (warn-on-mismatch) behavior rather than guessing agreement. ~keep
fn agrees_via_enum_default(
    serde_default: &DefaultValue,
    actual_default: &DefaultValue,
    field_type: &TypeRef,
    enum_default_variants: &AHashMap<String, String>,
) -> bool {
    let DefaultValue::EnumVariant(actual_variant) = actual_default else {
        return false;
    };
    if !matches!(serde_default, DefaultValue::Empty) {
        return false;
    }
    let TypeRef::Named(type_name) = field_type else {
        return false;
    };
    enum_default_variants
        .get(type_name)
        .is_some_and(|default_variant| default_variant == actual_variant)
}

/// Warn when a field's serde-reader default and its final `#[derive(Default)]`/manual
/// `impl Default` value are both fully known and disagree (issue #153).
///
/// `FieldDef::typed_default` is a single slot written up to three times: the serde reader sets
/// it first (`extract::extractor::helpers::fields::extract_field`), then either
/// `#[derive(Default)]` (`extract::extractor::types::extract_struct`) or a manual `impl Default`
/// (`extract::extractor::defaults::extract_default_values`) unconditionally overwrites it. By
/// the time extraction finishes, the serde value is gone — a binding generated from it would
/// silently disagree with the Rust core whenever the other path is taken instead. `serde_defaults`
/// is the serde reader's value, captured by the caller before it was overwritten (see
/// `extract::extractor::types::extract_struct` and the `pending_serde_defaults` threaded through
/// `extract::extractor::mod::extract_items` down to `extract::extractor::functions::impl_blocks`).
///
/// `#[derive(Default)]`'s blanket `Empty` write is treated as a genuinely known value here, not a
/// placeholder: `Empty` is documented (see `DefaultValue::Empty`, and the comment on the
/// `has_default` write in `extract_struct`) as an assertion that the derived default *is* the
/// field's type-zero, exactly as `Vec::new()` or `Default::default()` inside a manual `impl
/// Default` are. Refusing to compare it would silently exempt the most common case (a struct
/// that derives `Default` while also carrying `#[serde(default)]` fields) from this diagnostic.
///
/// A structural mismatch is still not always a disagreement: see [`denotes_type_zero`] for the
/// case where both sides name the same zero value under different `DefaultValue` spellings, and
/// [`agrees_via_enum_default`] for the enum-typed equivalent, where the type's zero is a named
/// variant rather than a literal.
///
/// `enum_default_variants` is whatever the caller's `EnumDef`s are known at the point this runs
/// (see the call sites for how much of the crate's enums that covers); an enum missing from it
/// is treated as "cannot prove agreement", not as "known to disagree" — see
/// [`agrees_via_enum_default`].
///
/// A disagreement matters because a binding generated from one of the two defaults silently
/// differs from the Rust core whenever the other path is taken. ~keep
pub(crate) fn warn_on_default_disagreement(
    rust_path: &str,
    fields: &[FieldDef],
    serde_defaults: &AHashMap<String, DefaultValue>,
    enum_default_variants: &AHashMap<String, String>,
) {
    for field in fields {
        let Some(serde_default) = serde_defaults.get(&field.name) else {
            continue;
        };
        let Some(actual_default) = &field.typed_default else {
            continue;
        };
        if !is_fully_known(serde_default) || !is_fully_known(actual_default) {
            tracing::debug!(
                target: "alef::extract::defaults",
                rust_type = rust_path,
                field = %field.name,
                "field default comparison skipped: serde default or resolved default is not fully known"
            );
            continue;
        }
        let agrees = serde_default == actual_default
            || (denotes_type_zero(serde_default) && denotes_type_zero(actual_default))
            || agrees_via_enum_default(serde_default, actual_default, &field.ty, enum_default_variants);
        if !agrees {
            tracing::warn!(
                target: "alef::extract::defaults",
                rust_type = rust_path,
                field = %field.name,
                serde_default = ?serde_default,
                resolved_default = ?actual_default,
                "field's `#[serde(default)]` value disagrees with its `#[derive(Default)]`/`impl Default` value"
            );
        }
    }
}

/// Run [`warn_on_default_disagreement`] for every struct with a recorded serde default, using
/// the *complete*, final crate surface rather than whatever was extracted so far.
///
/// Must run once, after every source file in the crate has been parsed (see
/// `extract::extractor::mod::extract`'s call site) — never inline while a single file is still
/// being walked. [`agrees_via_enum_default`] can only prove an `Empty`/`EnumVariant` pair agrees
/// when the field's enum type is already present in `enum_default_variants`; a struct whose
/// manual `impl Default` sets an enum field directly (`ocr_strategy: OcrStrategy::Auto`) is
/// resolved to a concrete `EnumVariant` the moment its own `impl Default` is read
/// (`extract::extractor::defaults::extract_default_values`), well before every other source file
/// — including the one declaring the enum itself — has necessarily been visited. Calling this
/// warning inline from that same per-file pass (as `extract::extractor::functions::impl_blocks`
/// used to) made the false-positive rate depend on `mod` declaration order: a crate that declares
/// `pub mod extraction;` before `pub mod ocr;` warned on every genuinely agreeing enum field in
/// `extraction`, purely because `ocr`'s enums were not yet in the map. Deferring to one pass over
/// the finished `surface` removes that dependency entirely, the same way
/// `resolve_enum_field_defaults` already defers its own enum-completeness requirement. ~keep
pub(super) fn warn_on_default_disagreements(surface: &ApiSurface, pending_serde_defaults: &SerdeDefaultsByType) {
    if pending_serde_defaults.is_empty() {
        return;
    }
    let enum_default_variants = enum_default_variant_names(&surface.enums);
    for typ in &surface.types {
        let Some(serde_defaults) = pending_serde_defaults.get(&typ.rust_path) else {
            continue;
        };
        warn_on_default_disagreement(&typ.rust_path, &typ.fields, serde_defaults, &enum_default_variants);
    }
}

#[cfg(test)]
mod default_disagreement_tests {
    use super::*;
    use tracing_test::traced_test;

    fn field(name: &str, typed_default: Option<DefaultValue>) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            typed_default,
            ..Default::default()
        }
    }

    fn serde_defaults(entries: &[(&str, DefaultValue)]) -> AHashMap<String, DefaultValue> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    /// (a) Two known values that genuinely disagree must be reported, naming the type, the
    /// field, and both values.
    #[test]
    #[traced_test]
    fn genuine_disagreement_between_two_known_values_is_reported() {
        let fields = vec![field("retries", Some(DefaultValue::IntLiteral(5)))];
        let defaults = serde_defaults(&[("retries", DefaultValue::IntLiteral(3))]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults, &AHashMap::new());

        assert!(logs_contain("my_crate::Config"));
        assert!(logs_contain("retries"));
        assert!(logs_contain("IntLiteral(3)"));
        assert!(logs_contain("IntLiteral(5)"));
    }

    /// (e) `#[derive(Default)]`'s blanket `Empty` write is a genuinely known value, not a
    /// placeholder: this is the realistic shape of (a) for a derived struct, where the serde
    /// reader recovered a concrete value but the derive gave every field the type's zero.
    #[test]
    #[traced_test]
    fn derived_defaults_empty_disagreeing_with_a_known_serde_default_is_reported() {
        let fields = vec![field("host", Some(DefaultValue::Empty))];
        let defaults = serde_defaults(&[("host", DefaultValue::StringLiteral("localhost".to_string()))]);

        warn_on_default_disagreement("my_crate::Client", &fields, &defaults, &AHashMap::new());

        assert!(logs_contain("my_crate::Client"));
        assert!(logs_contain("host"));
        assert!(logs_contain("StringLiteral(\"localhost\")"));
        assert!(logs_contain("Empty"));
    }

    /// (b) Agreement must not be reported.
    #[test]
    #[traced_test]
    fn agreeing_known_defaults_are_not_reported() {
        let fields = vec![field("retries", Some(DefaultValue::IntLiteral(3)))];
        let defaults = serde_defaults(&[("retries", DefaultValue::IntLiteral(3))]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (e) continued: `Empty` on both sides (bare `#[serde(default)]` alongside
    /// `#[derive(Default)]`) is agreement, not a placeholder collision, and must not be reported.
    #[test]
    #[traced_test]
    fn matching_empty_defaults_on_both_sides_are_not_reported() {
        let fields = vec![field("count", Some(DefaultValue::Empty))];
        let defaults = serde_defaults(&[("count", DefaultValue::Empty)]);

        warn_on_default_disagreement("my_crate::Config", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    fn enum_typed_field(name: &str, typed_default: Option<DefaultValue>, enum_type_name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            typed_default,
            ty: TypeRef::Named(enum_type_name.to_string()),
            ..Default::default()
        }
    }

    fn enum_default_variants(entries: &[(&str, &str)]) -> AHashMap<String, String> {
        entries
            .iter()
            .map(|(enum_name, variant_name)| ((*enum_name).to_string(), (*variant_name).to_string()))
            .collect()
    }

    /// (f) A bare `#[serde(default)]` (`Empty`) agrees with a manual `impl Default`'s
    /// `EnumVariant` when that variant is genuinely the field type's `#[default]` — both spell
    /// "this field's type-zero", just via different `DefaultValue` shapes. See
    /// `agrees_via_enum_default`.
    #[test]
    #[traced_test]
    fn empty_serde_default_agreeing_with_the_enum_default_variant_is_not_reported() {
        let fields = vec![enum_typed_field(
            "tier_strategy",
            Some(DefaultValue::EnumVariant("Auto".to_string())),
            "TierStrategy",
        )];
        let defaults = serde_defaults(&[("tier_strategy", DefaultValue::Empty)]);
        let enums = enum_default_variants(&[("TierStrategy", "Auto")]);

        warn_on_default_disagreement("my_crate::ConversionOptions", &fields, &defaults, &enums);

        assert!(!logs_contain("disagrees"));
    }

    /// (f) continued: a manual `impl Default` naming a variant *other than* the enum's
    /// `#[default]` is a genuine disagreement — serde would fall back to the `#[default]`
    /// variant, the manual impl would not — and must still be reported.
    #[test]
    #[traced_test]
    fn empty_serde_default_disagreeing_with_a_non_default_enum_variant_is_reported() {
        let fields = vec![enum_typed_field(
            "tier_strategy",
            Some(DefaultValue::EnumVariant("Tier2".to_string())),
            "TierStrategy",
        )];
        let defaults = serde_defaults(&[("tier_strategy", DefaultValue::Empty)]);
        let enums = enum_default_variants(&[("TierStrategy", "Auto")]);

        warn_on_default_disagreement("my_crate::ConversionOptions", &fields, &defaults, &enums);

        assert!(logs_contain("disagrees"));
        assert!(logs_contain("tier_strategy"));
    }

    /// (g) When the field's enum is absent from `enum_default_variants` — not yet extracted, or
    /// genuinely has no `#[default]` variant — agreement cannot be proven, so this falls back to
    /// the ordinary warn-on-mismatch behavior rather than assuming agreement.
    #[test]
    #[traced_test]
    fn empty_serde_default_against_an_unknown_enum_is_reported_conservatively() {
        let fields = vec![enum_typed_field(
            "tier_strategy",
            Some(DefaultValue::EnumVariant("Auto".to_string())),
            "TierStrategy",
        )];
        let defaults = serde_defaults(&[("tier_strategy", DefaultValue::Empty)]);

        warn_on_default_disagreement("my_crate::ConversionOptions", &fields, &defaults, &AHashMap::new());

        assert!(logs_contain("disagrees"));
    }

    /// (c) `Unresolved` on the resolved (`#[derive(Default)]`/`impl Default`) side is an
    /// unknown, not a disagreement, even though the serde side is a known, differing literal.
    #[test]
    #[traced_test]
    fn unresolved_resolved_default_is_not_reported() {
        let fields = vec![field(
            "level",
            Some(DefaultValue::Unresolved("Self::builder().level(9).build()".to_string())),
        )];
        let defaults = serde_defaults(&[("level", DefaultValue::IntLiteral(9))]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (c) continued: `Unresolved` on the serde side is likewise silent.
    #[test]
    #[traced_test]
    fn unresolved_serde_default_is_not_reported() {
        let fields = vec![field("level", Some(DefaultValue::IntLiteral(9)))];
        let defaults = serde_defaults(&[("level", DefaultValue::Unresolved("compute_default()".to_string()))]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (d) An `Unresolved` nested inside a `TupleVariant` payload makes the whole value
    /// unknown, even though the variant name itself matches on both sides.
    #[test]
    #[traced_test]
    fn nested_unresolved_inside_a_tuple_variant_is_not_reported() {
        let fields = vec![field(
            "mode",
            Some(DefaultValue::TupleVariant(
                "Custom".to_string(),
                vec![DefaultValue::Unresolved("compute()".to_string())],
            )),
        )];
        let defaults = serde_defaults(&[(
            "mode",
            DefaultValue::TupleVariant("Custom".to_string(), vec![DefaultValue::IntLiteral(5)]),
        )]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (d) continued: the same nested-unknown rule applies to a `StructVariant` payload.
    #[test]
    #[traced_test]
    fn nested_unresolved_inside_a_struct_variant_is_not_reported() {
        let fields = vec![field(
            "kind",
            Some(DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![("label".to_string(), DefaultValue::Unresolved("compute()".to_string()))],
            )),
        )];
        let defaults = serde_defaults(&[(
            "kind",
            DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![("label".to_string(), DefaultValue::StringLiteral("balanced".to_string()))],
            ),
        )]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// A `FunctionCall` names a real function whose return value alef never evaluates, so it is
    /// treated the same as `Unresolved`: comparing it against a differing literal would be a
    /// guess about what the function returns, not a reading, even when the paths look unrelated
    /// to the literal on the other side.
    #[test]
    #[traced_test]
    fn function_call_default_is_not_compared_even_against_a_differing_literal() {
        let fields = vec![field("token", Some(DefaultValue::StringLiteral("abc".to_string())))];
        let defaults = serde_defaults(&[("token", DefaultValue::FunctionCall("generate_token".to_string()))]);

        warn_on_default_disagreement("my_crate::Auth", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// A field with no serde default recorded (most fields) must not be touched at all.
    #[test]
    #[traced_test]
    fn field_absent_from_serde_defaults_is_skipped() {
        let fields = vec![field("untouched", Some(DefaultValue::IntLiteral(1)))];
        let defaults = serde_defaults(&[]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (f) The false-positive this fix targets: a bare `#[serde(default)]` folds to `Empty`,
    /// while a hand-written `impl Default` writing the field's zero out explicitly (`count: 0`)
    /// folds to `IntLiteral(0)`. Both name the same Rust default, so this must not warn.
    #[test]
    #[traced_test]
    fn a_zero_valued_serde_default_matching_the_rust_default_does_not_warn() {
        let fields = vec![field("count", Some(DefaultValue::IntLiteral(0)))];
        let defaults = serde_defaults(&[("count", DefaultValue::Empty)]);

        warn_on_default_disagreement("my_crate::Cfg", &fields, &defaults, &AHashMap::new());

        assert!(!logs_contain("disagrees"));
    }

    /// (f) Table-driven spread of the zero-equivalence fix across every `DefaultValue` spelling
    /// of "zero" (`Empty`, `IntLiteral(0)`, `FloatLiteral(0.0)`, `BoolLiteral(false)`,
    /// `StringLiteral("")`, `None`) in both comparison directions, plus the border cases that
    /// must keep warning: `Empty` against a non-zero literal, two differing non-zero literals,
    /// and `Empty` against a non-empty string.
    #[test]
    #[traced_test]
    fn zero_value_equivalence_across_default_value_spellings_is_evaluated_correctly() {
        struct Case {
            type_name: &'static str,
            serde_default: DefaultValue,
            actual_default: DefaultValue,
            should_warn: bool,
        }

        let cases = [
            Case {
                type_name: "case::EmptySerdeZeroIntActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::IntLiteral(0),
                should_warn: false,
            },
            Case {
                type_name: "case::EmptySerdeZeroFloatActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::FloatLiteral(0.0),
                should_warn: false,
            },
            Case {
                type_name: "case::EmptySerdeFalseActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::BoolLiteral(false),
                should_warn: false,
            },
            Case {
                type_name: "case::EmptySerdeEmptyStringActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::StringLiteral(String::new()),
                should_warn: false,
            },
            Case {
                type_name: "case::EmptySerdeNoneActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::None,
                should_warn: false,
            },
            Case {
                type_name: "case::IntZeroSerdeEmptyActual",
                serde_default: DefaultValue::IntLiteral(0),
                actual_default: DefaultValue::Empty,
                should_warn: false,
            },
            Case {
                // Borders the fix: `Empty` against a *non-zero* literal is a genuine
                // disagreement, not the zero-spelling case, and must still warn.
                type_name: "case::EmptySerdeNonZeroIntActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::IntLiteral(5),
                should_warn: true,
            },
            Case {
                // Borders the fix: two non-zero, non-`Empty` literals that genuinely differ
                // must still warn — zero-equivalence must not swallow real disagreements.
                type_name: "case::NonZeroIntSerdeNonZeroIntActual",
                serde_default: DefaultValue::IntLiteral(3),
                actual_default: DefaultValue::IntLiteral(5),
                should_warn: true,
            },
            Case {
                // Borders the fix: an empty string is a zero value, a non-empty string is not.
                type_name: "case::EmptySerdeNonEmptyStringActual",
                serde_default: DefaultValue::Empty,
                actual_default: DefaultValue::StringLiteral("localhost".to_string()),
                should_warn: true,
            },
        ];

        for case in cases {
            let fields = vec![field("value", Some(case.actual_default.clone()))];
            let defaults = serde_defaults(&[("value", case.serde_default.clone())]);

            warn_on_default_disagreement(case.type_name, &fields, &defaults, &AHashMap::new());

            let warned = logs_contain(case.type_name) && logs_contain("disagrees");
            assert_eq!(
                warned, case.should_warn,
                "case {} (serde={:?}, actual={:?}) expected should_warn={} but got {}",
                case.type_name, case.serde_default, case.actual_default, case.should_warn, warned
            );
        }
    }
}
