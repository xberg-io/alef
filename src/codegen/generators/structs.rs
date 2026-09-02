use crate::codegen::builder::StructBuilder;
use crate::codegen::doc_emission::{DocTarget, sanitize_rust_idioms};
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{CoreWrapper, TypeDef, TypeRef};

/// Whether every binding-visible field of `typ` round-trips losslessly through
/// `From<core::Type> for BindingType` (see `gen_from_core_to_binding_cfg`) -- the precondition
/// for a delegating `Deserialize` (deserialize the *core* type, then `.into()`) to be sound.
///
/// `gen_from_core_to_binding_cfg` fills an opaque field (not covered by
/// `serializable_opaque_type_names`) or a sanitized non-`Cow` field with `Default::default()`
/// instead of the real value. Delegating through it for such a field would silently discard
/// whatever the real core `Deserialize` produced there, which is exactly the "silently-wrong
/// guess" this change must never introduce. A cfg-gated field is excluded for a different
/// reason: alef cannot know at generation time whether the compiled core type carries it, so
/// any delegation decision here would itself be unverifiable. `has_stripped_cfg_fields` is the
/// same concern at the whole-type level. Both cases must keep falling back to the derived,
/// field-by-field `Deserialize` (and the existing `SerdeContainerConversionUnsupported`
/// diagnostic keeps naming the gap). ~keep
pub fn struct_deserialize_delegation_field_sound(
    typ: &TypeDef,
    opaque_type_names: &[String],
    serializable_opaque_type_names: &[String],
) -> bool {
    if typ.has_stripped_cfg_fields {
        return false;
    }
    binding_fields(&typ.fields).all(|field| {
        if field.cfg.is_some() {
            return false;
        }
        let is_unwrapped_opaque = field_references_opaque_type(&field.ty, opaque_type_names)
            && !field_references_opaque_type(&field.ty, serializable_opaque_type_names);
        let is_skipped_sanitized = field.sanitized && field.core_wrapper != CoreWrapper::Cow;
        !is_unwrapped_opaque && !is_skipped_sanitized
    })
}

/// Whether the derived, field-by-field `Deserialize` on the mirror struct provably disagrees
/// with the core type's own `Deserialize`.
///
/// The mirror reproduces only a narrow slice of the core type's serde surface: per-field
/// `#[serde(rename = "...")]` (re-emitted verbatim from [`FieldDef::serde_rename`]) and
/// `#[serde(skip)]` for fields the binding had to drop. Everything below is dropped on the
/// floor, so the derived impl reads a wire shape the core type never had:
///
/// * container `#[serde(from/into/try_from/transparent)]` -- the payload is not the derived
///   object at all, or is routed through a hand-written `From` alef cannot see.
/// * container `#[serde(default)]` ([`TypeDef::serde_container_default`]) and per-field
///   `#[serde(default)]` / `#[serde(default = "path")]` ([`FieldDef::default`]) -- every absent
///   key must be filled from the core's defaults, but the mirror's derive makes each one
///   *required*, so a partial payload the core accepts is rejected at the binding boundary.
/// * per-field `#[serde(with = "...")]` ([`FieldDef::serde_with`]) -- a hand-written codec whose
///   shape alef never derived and cannot restate.
/// * per-field `#[serde(flatten)]` -- the field's keys sit on the parent object, not nested
///   under the field name.
/// * per-field `#[serde(skip)]` ([`FieldDef::serde_skip`]) -- the key is absent from the payload
///   by construction, yet the mirror's derive demands it.
///
/// `#[serde(deny_unknown_fields)]` is deliberately not a trigger: alef does not carry it in the
/// IR, so it cannot be detected here. It is still *honoured* wherever delegation fires for one
/// of the reasons above, because the core type's own `Deserialize` is what runs.
///
/// `serde_skip_serializing_if` is likewise absent -- it only omits a key on the way out and
/// leaves `Deserialize` untouched, so it is not a disagreement this function is about. ~keep
fn struct_mirror_deserialize_disagrees_with_core(typ: &TypeDef) -> bool {
    typ.serde_container_conversion.is_present()
        || typ.serde_container_default
        || binding_fields(&typ.fields).any(|field| {
            field.default.is_some() || field.serde_with.is_some() || field.serde_flatten || field.serde_skip
        })
}

/// Whether `typ` should get a delegating `Deserialize` -- reads the *core* type (honouring every
/// serde attribute it carries) and converts via `Into` -- instead of the derived, field-by-field
/// object `Deserialize` that disagrees with it.
///
/// Requires BOTH: the mirror's derive provably disagrees with the core type (see
/// [`struct_mirror_deserialize_disagrees_with_core`]), and delegation is field-sound (see
/// [`struct_deserialize_delegation_field_sound`]). Callers must separately confirm that
/// `From<core::Type> for BindingType` will actually be emitted for `typ` in this run --
/// typically by intersecting with the same convertible-type set that already gates that
/// backend's `gen_from_core_to_binding_cfg` call for the same type. This function only proves
/// delegation would be *sound*; it says nothing about whether the `Into` it depends on exists. ~keep
pub fn struct_wants_deserialize_delegation(
    typ: &TypeDef,
    opaque_type_names: &[String],
    serializable_opaque_type_names: &[String],
) -> bool {
    struct_mirror_deserialize_disagrees_with_core(typ)
        && struct_deserialize_delegation_field_sound(typ, opaque_type_names, serializable_opaque_type_names)
}

/// Render `field.default` (the core field's `#[serde(default)]` / `#[serde(default = "path")]`,
/// see [`FieldDef::default`]) back into the literal attribute text for the mirror field, for use
/// when whole-type `Deserialize` delegation does not fire (see [`struct_wants_deserialize_delegation`]
/// and [`should_delegate_deserialize`]).
///
/// Delegation is all-or-nothing per type: one unsound field (an unwrapped opaque, a
/// non-`Cow` sanitized field, a cfg-gated field) blocks it for the *entire* struct, including
/// every other field that has nothing to do with the unsound one. Without this fallback, such a
/// struct's derived, field-by-field `Deserialize` makes an absent-tolerant core field required on
/// the mirror, rejecting a partial JSON payload the core type itself accepts (the same failure
/// mode delegation exists to prevent, just not caught by it in this case). Mirroring the literal
/// attribute here keeps that one field absent-tolerant even though the struct as a whole still
/// uses the derived impl.
///
/// `extract_field` (see `extract::extractor::helpers::fields`) records a bare `#[serde(default)]`
/// as the `"/* serde(default) */"` sentinel (not valid attribute syntax on its own -- it is a
/// marker other call sites already match on verbatim, e.g. `backends::pyo3::gen_bindings::constructors`
/// and `backends::pyo3::gen_bindings::functions::converters`) and `#[serde(default = "path")]` as
/// the literal `"serde(default = \"path\")"` attribute text. Returns `None` when the core field
/// carries no serde default. ~keep
fn serde_default_field_attr(field: &crate::core::ir::FieldDef) -> Option<String> {
    match field.default.as_deref() {
        Some("/* serde(default) */") => Some("serde(default)".to_string()),
        Some(text) if text.starts_with("serde(default") => Some(text.to_string()),
        _ => None,
    }
}

/// Compose the fully-qualified core type path for `typ`: applies crate remaps when
/// `rust_path` already carries a module path, or falls back to `{core_import}::{name}` for a
/// bare (unqualified) path. Shared by [`gen_delegating_default_impl`] and
/// [`gen_delegating_deserialize_impl`] so the two delegating impls can never reference a
/// different core type for the same `typ`. ~keep
fn qualified_core_path(typ: &TypeDef, core_import: &str, source_crate_remaps: &[(&str, &str)]) -> String {
    let core_path = typ.rust_path.replace('-', "_");
    if core_path.contains("::") {
        crate::codegen::conversions::apply_crate_remaps(&core_path, source_crate_remaps)
    } else {
        format!("{core_import}::{}", typ.name)
    }
}

/// Generate a hand-written `impl<'de> serde::Deserialize<'de> for BindingType` that delegates
/// to the *core* type's own `Deserialize` and converts via `Into`, instead of a derived
/// field-by-field object `Deserialize`.
///
/// Only sound when [`struct_wants_deserialize_delegation`] is true for `typ` and the caller has
/// confirmed `From<core::Type> for BindingType` is actually emitted for this run -- see that
/// function's doc comment. Emitted in place of `#[derive(serde::Deserialize)]`; callers must
/// omit that derive when they emit this impl (both derive the same trait for the same type).
pub fn gen_delegating_deserialize_impl(
    typ: &TypeDef,
    core_import: &str,
    type_name_prefix: &str,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let binding_name = format!("{type_name_prefix}{}", typ.name);
    let core_qualified = qualified_core_path(typ, core_import, source_crate_remaps);
    crate::codegen::template_env::render(
        "structs/delegating_deserialize_impl.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_qualified,
        },
    )
}

/// Sanitize a struct-field docstring before propagating into a binding Rust
/// crate so explicit-link targets `[`X`](crate::X)` collapse to `` `X` ``.
/// The `crate::` path resolves in the originating crate but not here, and
/// without sanitization rustdoc raises `broken_intra_doc_links` /
/// `redundant_explicit_links` on the binding. `DocTarget::TsDoc` is used as
/// a target-agnostic sentinel — the prose pipeline only varies post-link
/// behaviour by target, and the link rewrite is identical for every target.
fn sanitize_field_doc(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    sanitize_rust_idioms(doc, DocTarget::TsDoc)
}

/// Check if a type's fields can all be safely defaulted.
/// Primitives, strings, collections, Options, and Duration all have Default impls.
/// Named types (custom structs) only have Default if explicitly marked with `has_default=true`.
/// If any field is a Named type without `has_default`, returning true would generate
/// code that calls `Default::default()` on a type that doesn't implement it.
pub fn can_generate_default_impl(typ: &TypeDef, known_default_types: &std::collections::HashSet<&str>) -> bool {
    for field in binding_fields(&typ.fields) {
        if field.cfg.is_some() {
            continue;
        }
        if !field_type_has_default(&field.ty, known_default_types) {
            return false;
        }
    }
    true
}

/// Check if a specific TypeRef can be safely defaulted.
fn field_type_has_default(ty: &TypeRef, known_default_types: &std::collections::HashSet<&str>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Optional(inner) => field_type_has_default(inner, known_default_types),
        TypeRef::Vec(inner) => field_type_has_default(inner, known_default_types),
        TypeRef::Map(k, v) => {
            field_type_has_default(k, known_default_types) && field_type_has_default(v, known_default_types)
        }
        TypeRef::Named(name) => known_default_types.contains(name.as_str()),
    }
}

/// Check if any two field names are similar enough to trigger clippy::similar_names.
/// This detects patterns like "sub_symbol" and "sup_symbol" (differ by 1-2 chars).
fn has_similar_names(names: &[&String]) -> bool {
    for (i, &name1) in names.iter().enumerate() {
        for &name2 in &names[i + 1..] {
            if name1.len() == name2.len() && diff_count(name1, name2) <= 2 {
                return true;
            }
        }
    }
    false
}

/// Count how many characters differ between two strings of equal length.
fn diff_count(s1: &str, s2: &str) -> usize {
    s1.chars().zip(s2.chars()).filter(|(c1, c2)| c1 != c2).count()
}

/// Check if a TypeRef references an opaque type, including through Optional and Vec wrappers.
/// Opaque types use `Arc<T>` which doesn't implement Serialize/Deserialize, so any struct with
/// such a field cannot derive those traits.
pub fn field_references_opaque_type(ty: &TypeRef, opaque_names: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => opaque_names.contains(name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_opaque_type(inner, opaque_names),
        TypeRef::Map(k, v) => {
            field_references_opaque_type(k, opaque_names) || field_references_opaque_type(v, opaque_names)
        }
        _ => false,
    }
}

/// Whether `typ`'s binding struct should get a delegating `Deserialize` in this run: the
/// caller-provided `delegate_deserialize_to_core_for_types` set names it (guaranteeing the
/// `From<core::Type>` impl the delegation depends on will also be emitted) AND delegation is
/// independently sound for its fields. See [`struct_wants_deserialize_delegation`].
fn should_delegate_deserialize(typ: &TypeDef, cfg: &RustBindingConfig) -> bool {
    cfg.delegate_deserialize_to_core_for_types
        .is_some_and(|set| set.contains(&typ.name))
        && struct_wants_deserialize_delegation(typ, cfg.opaque_type_names, cfg.serializable_opaque_type_names)
}

/// Generate a struct definition using the builder, with a per-field attribute callback.
///
/// `extra_field_attrs` is called for each field and returns additional `#[...]` attributes to
/// prepend (beyond `cfg.field_attrs`). Pass `|_| vec![]` to use the default behaviour.
pub fn gen_struct_with_per_field_attrs(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    extra_field_attrs: impl Fn(&crate::core::ir::FieldDef) -> Vec<String>,
) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

    let field_names: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| f.cfg.is_none())
        .map(|f| &f.name)
        .collect();
    if has_similar_names(&field_names) {
        sb.add_attr("allow(clippy::similar_names)");
    }

    for d in cfg.struct_derives {
        sb.add_derive(d);
    }
    // Track which fields are opaque so we can conditionally skip derives and add #[serde(skip)].
    let opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| {
            f.cfg.is_none()
                && field_references_opaque_type(&f.ty, cfg.opaque_type_names)
                && !field_references_opaque_type(&f.ty, cfg.serializable_opaque_type_names)
        })
        .map(|f| f.name.as_str())
        .collect();
    // when set). Skipping the delegating impl for non-convertible types restores `#[derive(Default)]`
    let delegating_eligible = cfg.emit_delegating_default_impl
        && typ.has_default
        && cfg
            .emit_delegating_default_for_types
            .is_none_or(|s| s.contains(&typ.name));
    let suppress_default_derive = delegating_eligible;
    if !suppress_default_derive {
        sb.add_derive("Default");
    }
    sb.add_derive("serde::Serialize");
    let delegate_deserialize = should_delegate_deserialize(typ, cfg);
    if !delegate_deserialize {
        sb.add_derive("serde::Deserialize");
    }
    let has_serde = true;
    for field in binding_fields(&typ.fields) {
        let force_optional = cfg.option_duration_on_defaults
            && typ.has_default
            && !field.optional
            && matches!(field.ty, TypeRef::Duration);
        let ty = if (field.optional || force_optional) && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        let mut attrs: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
        attrs.extend(extra_field_attrs(field));
        // Add #[serde(skip)] for opaque fields or sanitized fields when the struct derives serde.
        let skip_sanitized_field = field.sanitized && field.core_wrapper != CoreWrapper::Cow;
        let skip_cfg_bridge_field = field.cfg.is_some()
            && cfg.never_skip_cfg_field_names.contains(&field.name)
            && field_references_opaque_type(&field.ty, cfg.opaque_type_names);
        if has_serde && (opaque_fields.contains(&field.name.as_str()) || skip_sanitized_field || skip_cfg_bridge_field)
        {
            attrs.push("serde(skip)".to_string());
        }
        // Whole-type delegation didn't fire for `typ` (or wasn't requested for it) -- mirror
        // this field's own `#[serde(default)]` directly so it stays absent-tolerant even under
        // the derived, field-by-field `Deserialize`. See `serde_default_field_attr`.
        if has_serde
            && !delegate_deserialize
            && !attrs.iter().any(|a| a == "serde(skip)")
            && let Some(default_attr) = serde_default_field_attr(field)
        {
            attrs.push(default_attr);
        }
        sb.add_field_with_doc(&field.name, &ty, attrs, &sanitize_field_doc(&field.doc));
    }
    let mut result = sb.build();
    if delegating_eligible {
        result.push_str(&gen_delegating_default_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    if delegate_deserialize {
        result.push_str(&gen_delegating_deserialize_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    result
}

/// Generate a struct definition using the builder, with per-field attribute and name override callbacks.
///
/// This is the most flexible variant.  Use it when the target language may need to escape
/// reserved keywords in field names (e.g. Python's `class` → `class_`).
///
/// * `extra_field_attrs` — called per field, returns additional `#[…]` attribute strings to
///   append **after** `cfg.field_attrs`.  Return an empty vec for the default behaviour.
/// * `field_name_override` — called per field, returns `Some(escaped_name)` when the Rust
///   binding struct field name should differ from `field.name` (e.g. for keyword escaping),
///   or `None` to keep the original name.
///
/// When a field name is overridden the caller is responsible for adding the appropriate
/// language attribute (e.g. `pyo3(get, name = "original")`) via `extra_field_attrs`.
/// `cfg.field_attrs` is **still** applied for non-renamed fields; for renamed fields the
/// caller should replace the default field attrs entirely by returning them from
/// `extra_field_attrs` and passing a modified `cfg` with empty `field_attrs`.
pub fn gen_struct_with_rename(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    extra_field_attrs: impl Fn(&crate::core::ir::FieldDef) -> Vec<String>,
    field_name_override: impl Fn(&crate::core::ir::FieldDef) -> Option<String>,
) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

    let field_names: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| f.cfg.is_none())
        .map(|f| &f.name)
        .collect();
    if has_similar_names(&field_names) {
        sb.add_attr("allow(clippy::similar_names)");
    }

    for d in cfg.struct_derives {
        sb.add_derive(d);
    }
    let opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| {
            f.cfg.is_none()
                && field_references_opaque_type(&f.ty, cfg.opaque_type_names)
                && !field_references_opaque_type(&f.ty, cfg.serializable_opaque_type_names)
        })
        .map(|f| f.name.as_str())
        .collect();
    let delegating_eligible = cfg.emit_delegating_default_impl
        && typ.has_default
        && cfg
            .emit_delegating_default_for_types
            .is_none_or(|s| s.contains(&typ.name));
    let suppress_default_derive = delegating_eligible;
    if !suppress_default_derive {
        sb.add_derive("Default");
    }
    sb.add_derive("serde::Serialize");
    let delegate_deserialize = should_delegate_deserialize(typ, cfg);
    if !delegate_deserialize {
        sb.add_derive("serde::Deserialize");
    }
    let has_serde = true;
    for field in binding_fields(&typ.fields) {
        let force_optional = cfg.option_duration_on_defaults
            && typ.has_default
            && !field.optional
            && matches!(field.ty, TypeRef::Duration);
        let ty = if (field.optional || force_optional) && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        let name_override = field_name_override(field);
        let extra_attrs = extra_field_attrs(field);
        let mut attrs: Vec<String> = if name_override.is_some() && !extra_attrs.is_empty() {
            extra_attrs
        } else {
            let mut a: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
            a.extend(extra_attrs);
            a
        };
        // Add #[serde(skip)] for opaque/sanitized fields and cfg-gated trait-bridge fields.
        let skip_sanitized_field = field.sanitized && field.core_wrapper != CoreWrapper::Cow;
        let skip_cfg_bridge_field = field.cfg.is_some()
            && cfg.never_skip_cfg_field_names.contains(&field.name)
            && field_references_opaque_type(&field.ty, cfg.opaque_type_names);
        if has_serde && (opaque_fields.contains(&field.name.as_str()) || skip_sanitized_field || skip_cfg_bridge_field)
        {
            attrs.push("serde(skip)".to_string());
        }
        // Mirror per-field `#[serde(rename = "...")]` from the core type so the binding
        if has_serde
            && !attrs.iter().any(|a| a.starts_with("serde(rename"))
            && !attrs.iter().any(|a| a == "serde(skip)")
            && let Some(rename) = &field.serde_rename
        {
            attrs.push(format!("serde(rename = \"{rename}\")"));
        }
        // Whole-type delegation didn't fire for `typ` (or wasn't requested for it) -- mirror
        // this field's own `#[serde(default)]` directly so it stays absent-tolerant even under
        // the derived, field-by-field `Deserialize`. See `serde_default_field_attr`.
        if has_serde
            && !delegate_deserialize
            && !attrs.iter().any(|a| a == "serde(skip)")
            && let Some(default_attr) = serde_default_field_attr(field)
        {
            attrs.push(default_attr);
        }
        let emit_name = name_override.unwrap_or_else(|| field.name.clone());
        sb.add_field_with_doc(&emit_name, &ty, attrs, &sanitize_field_doc(&field.doc));
    }
    let mut result = sb.build();
    if delegating_eligible {
        result.push_str(&gen_delegating_default_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    if delegate_deserialize {
        result.push_str(&gen_delegating_deserialize_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    result
}

/// Generate a struct definition using the builder.
pub fn gen_struct(typ: &TypeDef, mapper: &dyn TypeMapper, cfg: &RustBindingConfig) -> String {
    let mut sb = StructBuilder::new(&typ.name);
    for attr in cfg.struct_attrs {
        sb.add_attr(attr);
    }

    let field_names: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| f.cfg.is_none())
        .map(|f| &f.name)
        .collect();
    if has_similar_names(&field_names) {
        sb.add_attr("allow(clippy::similar_names)");
    }

    for d in cfg.struct_derives {
        sb.add_derive(d);
    }
    let _opaque_fields: Vec<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| {
            f.cfg.is_none()
                && field_references_opaque_type(&f.ty, cfg.opaque_type_names)
                && !field_references_opaque_type(&f.ty, cfg.serializable_opaque_type_names)
        })
        .map(|f| f.name.as_str())
        .collect();
    let delegating_eligible = cfg.emit_delegating_default_impl
        && typ.has_default
        && cfg
            .emit_delegating_default_for_types
            .is_none_or(|s| s.contains(&typ.name));
    let suppress_default_derive = delegating_eligible;
    if !suppress_default_derive {
        sb.add_derive("Default");
    }
    sb.add_derive("serde::Serialize");
    let delegate_deserialize = should_delegate_deserialize(typ, cfg);
    if !delegate_deserialize {
        sb.add_derive("serde::Deserialize");
    }
    let _has_serde = true;
    for field in binding_fields(&typ.fields) {
        if field.cfg.is_some() && !cfg.never_skip_cfg_field_names.contains(&field.name) {
            continue;
        }
        let force_optional = cfg.option_duration_on_defaults
            && typ.has_default
            && !field.optional
            && matches!(field.ty, TypeRef::Duration);
        let ty = if (field.optional || force_optional) && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        let mut attrs: Vec<String> = cfg.field_attrs.iter().map(|a| a.to_string()).collect();
        // Mirror per-field `#[serde(rename = "...")]` from the core type so the binding
        if let Some(rename) = &field.serde_rename
            && !attrs.iter().any(|a| a.starts_with("serde(rename"))
        {
            attrs.push(format!("serde(rename = \"{rename}\")"));
        }
        let emit_name = crate::core::keywords::rust_raw_ident(&field.name);
        if emit_name != field.name && !attrs.iter().any(|a| a.starts_with("serde(rename")) {
            attrs.push(format!("serde(rename = \"{}\")", field.name));
        }
        // Whole-type delegation didn't fire for `typ` (or wasn't requested for it) -- mirror
        // this field's own `#[serde(default)]` directly so it stays absent-tolerant even under
        // the derived, field-by-field `Deserialize`. See `serde_default_field_attr`.
        if !delegate_deserialize && let Some(default_attr) = serde_default_field_attr(field) {
            attrs.push(default_attr);
        }
        sb.add_field_with_doc(&emit_name, &ty, attrs, &sanitize_field_doc(&field.doc));
    }
    let mut result = sb.build();
    if delegating_eligible {
        result.push_str(&gen_delegating_default_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    if delegate_deserialize {
        result.push_str(&gen_delegating_deserialize_impl(
            typ,
            cfg.core_import,
            cfg.type_name_prefix,
            cfg.source_crate_remaps,
        ));
    }
    result
}

/// Generate an `impl Default for BindingType` that delegates to the core type's `Default`.
///
/// Emitted when the core struct has a custom `impl Default` (`typ.has_default == true`) and
/// the binding caller sets `cfg.emit_delegating_default_impl = true`. Without this delegation,
/// the binding's derived `Default` would use Rust's primitive zeros (e.g. `0` for `i64`,
/// `String::new()` for strings) and overwrite the core's semantic defaults when partial JSON
/// is deserialised via a struct-level `#[serde(default)]`.
///
/// The generated impl requires that `From<core::Type> for {type_name_prefix}{Type}` exists.
/// For PHP this is satisfied by `gen_from_core_to_binding_cfg`, which is emitted for every
/// non-opaque convertible type in the binding crate.
pub fn gen_delegating_default_impl(
    typ: &TypeDef,
    core_import: &str,
    type_name_prefix: &str,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let binding_name = format!("{type_name_prefix}{}", typ.name);
    let core_qualified = qualified_core_path(typ, core_import, source_crate_remaps);
    format!(
        "\n\nimpl Default for {binding_name} {{\n    fn default() -> Self {{\n        <{core_qualified} as Default>::default().into()\n    }}\n}}\n"
    )
}

/// Generate a `Default` impl for a non-opaque binding struct with `has_default`.
/// All fields use their type's Default::default().
/// Optional fields use None instead of Default::default().
/// This enables the struct to be used with `unwrap_or_default()` in config constructors.
///
/// WARNING: This assumes all field types implement Default. If a Named field type
/// doesn't implement Default, this impl will fail to compile. Callers should verify
/// that the struct's fields can be safely defaulted before calling this function.
pub fn gen_struct_default_impl(typ: &TypeDef, name_prefix: &str) -> String {
    let full_name = format!("{}{}", name_prefix, typ.name);
    let fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|field| !field.binding_excluded)
        .filter_map(|field| {
            if field.cfg.is_some() {
                return None;
            }
            let default_val = match &field.ty {
                TypeRef::Optional(_) => "None".to_string(),
                _ => "Default::default()".to_string(),
            };
            Some(minijinja::context! {
                name => field.name.clone(),
                default_val => default_val
            })
        })
        .collect();

    crate::codegen::template_env::render(
        "structs/default_impl.jinja",
        minijinja::context! {
            full_name => full_name,
            fields => fields
        },
    )
}

/// Check if any method on a type takes `&mut self`, meaning the opaque wrapper
/// must use `Arc<Mutex<T>>` instead of `Arc<T>` to allow interior mutability.
pub fn type_needs_mutex(typ: &TypeDef) -> bool {
    typ.methods
        .iter()
        .any(|m| m.receiver == Some(crate::core::ir::ReceiverKind::RefMut))
}

/// Check if a type wrapping `Arc<Mutex<T>>` should use `tokio::sync::Mutex` instead
/// of `std::sync::Mutex` because every `&mut self` method is `async`.
///
/// `std::sync::MutexGuard` is `!Send`, so holding a guard across `.await` makes the
/// surrounding future `!Send`, which fails to compile in PyO3 / NAPI-RS bindings that
/// require `Send` futures. `tokio::sync::MutexGuard` IS `Send`, so swapping the lock
/// type fixes the entire async-locking story for these structs.
///
/// The condition is tight: every method that takes `&mut self` MUST be async. If even
/// one sync method takes `&mut self`, switching to `tokio::sync::Mutex` would break
/// it (since `tokio::sync::Mutex::lock()` returns a `Future` and cannot be awaited
/// from sync context). In that mixed case we keep `std::sync::Mutex`.
pub fn type_needs_tokio_mutex(typ: &TypeDef) -> bool {
    use crate::core::ir::ReceiverKind;
    if !type_needs_mutex(typ) {
        return false;
    }
    let refmut_methods = typ.methods.iter().filter(|m| m.receiver == Some(ReceiverKind::RefMut));
    let mut any = false;
    for m in refmut_methods {
        any = true;
        if !m.is_async {
            return false;
        }
    }
    any
}

/// Generate an opaque wrapper struct with `inner: Arc<core::Type>`.
/// For trait types, uses `Arc<dyn Type + Send + Sync>`.
/// For types with `&mut self` methods, uses `Arc<Mutex<core::Type>>`.
///
/// Special case: if ALL methods on this type are sanitized, the type was created by the
/// impl-block fallback for a generic core type (e.g. `GraphQLExecutor<Q,M,S>`). Sanitized
/// methods never access `self.inner` (they emit `gen_unimplemented_body`), so we omit the
/// `inner` field entirely. This avoids generating `Arc<CoreType>` with missing generic
/// parameters, which would fail to compile.
pub fn gen_opaque_struct(typ: &TypeDef, cfg: &RustBindingConfig) -> String {
    let needs_mutex = type_needs_mutex(typ);
    let core_path = typ.rust_path.replace('-', "_");
    let has_unresolvable_generics = core_path.contains('<');
    let all_methods_sanitized = !typ.methods.is_empty() && typ.methods.iter().all(|m| m.sanitized);
    let omit_inner = all_methods_sanitized && has_unresolvable_generics;

    let struct_attrs: Vec<_> = cfg.struct_attrs.iter().map(|s| s.to_string()).collect();
    let has_derives = !cfg.struct_derives.is_empty();
    let inner_type = if typ.is_trait {
        format!("Arc<dyn {core_path} + Send + Sync>")
    } else if needs_mutex {
        format!("Arc<std::sync::Mutex<{core_path}>>")
    } else {
        format!("Arc<{core_path}>")
    };

    crate::codegen::template_env::render(
        "structs/opaque_struct.jinja",
        minijinja::context! {
            struct_name => typ.name.clone(),
            has_derives => has_derives,
            struct_attrs => struct_attrs,
            omit_inner => omit_inner,
            inner_type => inner_type,
        },
    )
}

/// Generate an opaque wrapper struct with `inner: Arc<core::Type>` and a name prefix.
/// For types with `&mut self` methods, uses `Arc<Mutex<core::Type>>`.
///
/// Special case: if ALL methods on this type are sanitized, omit the `inner` field.
/// See `gen_opaque_struct` for the rationale.
pub fn gen_opaque_struct_prefixed(typ: &TypeDef, cfg: &RustBindingConfig, prefix: &str) -> String {
    let needs_mutex = type_needs_mutex(typ);
    let core_path = typ.rust_path.replace('-', "_");
    let has_unresolvable_generics = core_path.contains('<');
    let all_methods_sanitized = !typ.methods.is_empty() && typ.methods.iter().all(|m| m.sanitized);
    let omit_inner = all_methods_sanitized && has_unresolvable_generics;

    let struct_attrs: Vec<_> = cfg.struct_attrs.iter().map(|s| s.to_string()).collect();
    let has_derives = !cfg.struct_derives.is_empty();
    let struct_name = format!("{prefix}{}", typ.name);
    let inner_type = if typ.is_trait {
        format!("Arc<dyn {core_path} + Send + Sync>")
    } else if needs_mutex {
        format!("Arc<std::sync::Mutex<{core_path}>>")
    } else {
        format!("Arc<{core_path}>")
    };

    crate::codegen::template_env::render(
        "structs/opaque_struct.jinja",
        minijinja::context! {
            struct_name => struct_name,
            has_derives => has_derives,
            struct_attrs => struct_attrs,
            omit_inner => omit_inner,
            inner_type => inner_type,
        },
    )
}

#[cfg(test)]
mod tests;
