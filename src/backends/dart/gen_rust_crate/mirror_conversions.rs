use std::collections::HashSet;

use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};

pub(super) fn compute_types_containing_sanitized(
    api: &ApiSurface,
    direct_sanitized: &HashSet<String>,
    exclude_types: &HashSet<String>,
) -> HashSet<String> {
    let struct_by_name: std::collections::HashMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait && !t.is_opaque)
        .map(|t| (t.name.as_str(), t))
        .collect();
    let enum_by_name: std::collections::HashMap<&str, &EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Start with all directly-sanitized types and expand to any type that contains them.
    let mut result: HashSet<String> = direct_sanitized.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for ty in struct_by_name.values() {
            if result.contains(&ty.name) {
                continue;
            }
            let references_sanitized = ty
                .fields
                .iter()
                .any(|f| collect_named_types(&f.ty).iter().any(|n| result.contains(n)));
            if references_sanitized {
                result.insert(ty.name.clone());
                changed = true;
            }
        }
        // Enums also "contain" their variant field types — a Vec<EnumE> bridge argument
        // is layout-incompatible if any variant references a type whose mirror layout
        // differs from core.
        for en in enum_by_name.values() {
            if result.contains(&en.name) {
                continue;
            }
            let references_sanitized = en.variants.iter().any(|v| {
                v.fields
                    .iter()
                    .any(|f| collect_named_types(&f.ty).iter().any(|n| result.contains(n)))
            });
            if references_sanitized {
                result.insert(en.name.clone());
                changed = true;
            }
        }
    }

    result
}

/// Compute the transitive closure of all struct/enum types reachable from
/// `seed_types` (types with sanitized fields) via non-sanitized field references.
///
/// These are the types that need `From<MirrorT> for SourceT` impls so that
/// `.into()` calls in the generated From impls for sanitized-field types work.
/// Output-only types (e.g. result structs with sanitized fields) are excluded
/// from the seed set — they're never passed as function inputs.
pub(super) fn compute_types_needing_from_impl(
    api: &ApiSurface,
    seed_types: &HashSet<String>,
    exclude_types: &HashSet<String>,
) -> HashSet<String> {
    // Build lookup maps for quick access by name.
    let struct_by_name: std::collections::HashMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait && !t.is_opaque)
        .map(|t| (t.name.as_str(), t))
        .collect();
    let enum_by_name: std::collections::HashMap<&str, &EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| (e.name.as_str(), e))
        .collect();

    let mut result: HashSet<String> = seed_types.clone();
    let mut worklist: Vec<String> = seed_types.iter().cloned().collect();

    while let Some(type_name) = worklist.pop() {
        if let Some(ty) = struct_by_name.get(type_name.as_str()) {
            for field in binding_fields(&ty.fields) {
                if field.sanitized {
                    continue; // sanitized fields are not converted via From
                }
                // Collect all Named type references from this field.
                for named in collect_named_types(&field.ty) {
                    if !result.contains(&named)
                        && (struct_by_name.contains_key(named.as_str()) || enum_by_name.contains_key(named.as_str()))
                    {
                        result.insert(named.clone());
                        worklist.push(named);
                    }
                }
            }
        } else if let Some(en) = enum_by_name.get(type_name.as_str()) {
            // Process enum variant field types: the generated From<MirrorEnum> for CoreEnum
            // uses `.into()` for each variant field, so all variant field types also need
            // From<MirrorT> for CoreT impls.
            for variant in &en.variants {
                for field in &variant.fields {
                    if field.sanitized {
                        continue;
                    }
                    for named in collect_named_types(&field.ty) {
                        if !result.contains(&named)
                            && (struct_by_name.contains_key(named.as_str())
                                || enum_by_name.contains_key(named.as_str()))
                        {
                            result.insert(named.clone());
                            worklist.push(named);
                        }
                    }
                }
            }
        }
    }

    result
}

/// Collect all Named type names referenced (possibly nested) in a TypeRef.
fn collect_named_types(ty: &TypeRef) -> Vec<String> {
    collect_named_types_from_type_ref(ty)
}

/// Collect all Named type names referenced (possibly nested) in a TypeRef.
pub(super) fn collect_named_types_from_type_ref(ty: &TypeRef) -> Vec<String> {
    match ty {
        TypeRef::Named(name) => vec![name.clone()],
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => collect_named_types_from_type_ref(inner),
        TypeRef::Map(k, v) => {
            let mut names = collect_named_types_from_type_ref(k);
            names.extend(collect_named_types_from_type_ref(v));
            names
        }
        _ => vec![],
    }
}

fn emit_rust_struct_field(out: &mut String, cfg: Option<&str>, field_name: &str, expr: &str) {
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_struct_field_assignment.jinja",
        minijinja::context! {
            cfg => cfg,
            field_name => field_name,
            expr => expr,
        },
    ));
}

/// Emit a `From<SourceT> for T` implementation for a mirror struct.
///
/// Each field is converted using the appropriate strategy:
/// - `CoreWrapper::Cow` fields: `.into()` (Cow<'_, str> → String)
/// - `TypeRef::Json` fields: `serde_json::to_string(&v).unwrap_or_default()`
/// - `TypeRef::Named(n)` fields: `n::from(v.field)` (recursive)
/// - Other fields: `.into()` or direct copy
pub(super) fn emit_from_impl_for_struct(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    let name = &ty.name;
    let core_ty_base = if ty.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        ty.rust_path.replace('-', "_")
    };
    let core_ty = if ty.has_lifetime_params {
        format!("{core_ty_base}<'_>")
    } else {
        core_ty_base
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_core_struct_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));

    for field in binding_fields(&ty.fields) {
        if field.sanitized {
            // Sanitized fields (unknown types mapped to String/i64) can't be auto-converted.
            // Use a best-effort fallback.
            let fallback = sanitized_field_from_expr(field);
            // `cfg = None`: the dart bridge crate enables `features = ["full"]` on
            // the source dependency, so every core-side cfg-gated field is present
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            // crate's own (undefined) features, evaluating to false and leaving the
            // struct literal missing fields.
            emit_rust_struct_field(out, None, &field.name, &fallback);
        } else {
            let expr = field_from_expr(field, source_crate_name);
            // `cfg = None`: the dart bridge crate enables `features = ["full"]` on
            // the source dependency, so every core-side cfg-gated field is present
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            // crate's own (undefined) features, evaluating to false and leaving the
            // struct literal missing fields.
            emit_rust_struct_field(out, None, &field.name, &expr);
        }
    }

    // Note: no ..Default::default() here — the mirror struct has exactly the fields
    // known to the IR. has_stripped_cfg_fields only affects the CORE struct, not the mirror.
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Build the conversion expression for one struct field (core → mirror direction).
fn field_from_expr(field: &FieldDef, source_crate_name: &str) -> String {
    let name = &field.name;
    let _ = source_crate_name;
    match &field.ty {
        TypeRef::Json => {
            // Core has serde_json::Value or similar; mirror has String.
            if field.optional {
                format!("v.{name}.map(|j| serde_json::to_string(&j).unwrap_or_default())")
            } else {
                format!("serde_json::to_string(&v.{name}).unwrap_or_default()")
            }
        }
        TypeRef::String => {
            // The IR collapses `Cow<'_, str>` / `Box<str>` / `Arc<str>` into
            // `TypeRef::String` and only `Cow` is tracked on `core_wrapper`. Emit
            // `.into()` unconditionally so wrapped-string core fields convert
            // correctly (e.g. `Box<str> → String`); the crate-level
            // `#[allow(clippy::useless_conversion)]` absorbs the `String → String`
            // no-op case.
            if field.optional {
                format!("v.{name}.map(|s| s.into())")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Char => {
            // Core has char; mirror has String. Convert via to_string().
            if field.optional {
                format!("v.{name}.map(|c| c.to_string())")
            } else {
                format!("v.{name}.to_string()")
            }
        }
        TypeRef::Path => {
            // Core has PathBuf; mirror has String.
            // PathBuf does not implement Into<String>; use to_string_lossy().
            if field.optional {
                format!("v.{name}.map(|p| p.to_string_lossy().into_owned())")
            } else {
                format!("v.{name}.to_string_lossy().into_owned()")
            }
        }
        TypeRef::Bytes => {
            // bytes::Bytes or Vec<u8>; mirror uses Vec<u8>.
            match field.core_wrapper {
                CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                    if field.optional {
                        format!("v.{name}.map(|a| (*a).clone().into())")
                    } else {
                        format!("(*v.{name}).clone().into()")
                    }
                }
                _ => {
                    if field.optional {
                        format!("v.{name}.map(|b| b.into())")
                    } else {
                        format!("v.{name}.into()")
                    }
                }
            }
        }
        TypeRef::Named(inner_name) => {
            // Handle Arc wrapper on named types.
            match field.core_wrapper {
                CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                    // Core has Arc<T>; clone out of the Arc.
                    if field.optional {
                        format!("v.{name}.map(|a| {inner_name}::from((*a).clone()))")
                    } else {
                        format!("{inner_name}::from((*v.{name}).clone())")
                    }
                }
                _ => {
                    if field.optional && field.is_boxed {
                        format!("v.{name}.map(|b| {inner_name}::from(*b))")
                    } else if field.optional {
                        format!("v.{name}.map({inner_name}::from)")
                    } else if field.is_boxed {
                        format!("{inner_name}::from(*v.{name})")
                    } else {
                        format!("{inner_name}::from(v.{name})")
                    }
                }
            }
        }
        TypeRef::Vec(inner) => vec_inner_from_expr(
            inner,
            &field.vec_inner_core_wrapper,
            field.newtype_wrapper.as_deref(),
            name,
            field.optional,
        ),
        TypeRef::Optional(inner) => {
            // Nested-optional field. Core is `Option<Option<T>>` (the outer Option was
            // stripped into `field.optional`, leaving `field.ty = Optional(T)`). The
            // mirror flattens to a single `Option<T>` per `frb_rust_type`, so we must
            // flatten the core value before converting elements. When `!field.optional`
            // the existing direct map shape applies (no outer Option around v).
            let flatten = if field.optional { ".flatten()" } else { "" };
            match inner.as_ref() {
                TypeRef::Named(inner_name) => {
                    format!("v.{name}{flatten}.map({inner_name}::from)")
                }
                TypeRef::String => {
                    // The IR loses `Box<str>` / `Cow<'_, str>` / `Arc<str>` wrappers,
                    // so emit `.into()` to bridge them; absorbed by the crate-level
                    // `#[allow(clippy::useless_conversion)]` for plain `String`.
                    format!("v.{name}{flatten}.map(|s| s.into())")
                }
                TypeRef::Char => {
                    format!("v.{name}{flatten}.map(|s| s.into())")
                }
                TypeRef::Path => {
                    format!("v.{name}{flatten}.map(|p| p.to_string_lossy().into_owned())")
                }
                TypeRef::Primitive(_) => {
                    format!("v.{name}{flatten}.map(|x| x as _)")
                }
                _ => format!("v.{name}{flatten}"),
            }
        }
        TypeRef::Map(k, v_ty) => {
            // Maps: may need iter-collect to convert BTreeMap/AHashMap → HashMap,
            // and Value → String for value types.
            map_from_expr(name, k, v_ty, field.optional, field.core_wrapper.clone())
        }
        TypeRef::Duration => {
            // Duration: convert to i64 millis (FRB ABI). Duration is not a primitive
            // so `as _` casts do not compile; use `.as_millis() as i64` instead.
            if field.optional {
                format!("v.{name}.map(|d| d.as_millis() as i64)")
            } else {
                format!("v.{name}.as_millis() as i64")
            }
        }
        TypeRef::Primitive(_) | TypeRef::Unit => {
            // Primitives: alef widens to i64/f64/bool; core may use narrower types.
            // When newtype_wrapper is set, the core field is NewType(inner); unwrap with .0.
            if let Some(_nw) = &field.newtype_wrapper {
                // Newtype wrapper: unwrap .0 then cast.
                if field.optional {
                    format!("v.{name}.map(|x| x.0 as _)")
                } else {
                    format!("v.{name}.0 as _")
                }
            } else if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
    }
}

/// Build the Vec field conversion expression (core → mirror).
fn vec_inner_from_expr(
    inner: &TypeRef,
    vec_inner_core_wrapper: &CoreWrapper,
    field_newtype_wrapper: Option<&str>,
    name: &str,
    optional: bool,
) -> String {
    let item_conv = match (inner, vec_inner_core_wrapper) {
        (TypeRef::Named(inner_name), CoreWrapper::Arc | CoreWrapper::ArcMutex) => {
            // Vec<Arc<T>> — clone out of Arc then convert.
            format!("|a| {inner_name}::from((*a).clone())")
        }
        (TypeRef::Named(inner_name), _) => {
            format!("{inner_name}::from")
        }
        (TypeRef::String, _) => {
            // The IR collapses wrapped string types (`Box<str>`, `Cow<'_, str>`,
            // `Arc<str>`) into `TypeRef::String`, and `CoreWrapper` only tracks `Cow`.
            // Emit `.into()` unconditionally so `Vec<Box<str>>` → `Vec<String>` (and
            // friends) compile — the crate-level `#[allow(clippy::useless_conversion)]`
            // absorbs the `Vec<String> → Vec<String>` no-op case.
            "|s| s.into()".to_string()
        }
        (TypeRef::Char, _) => "|s| s.into()".to_string(),
        (TypeRef::Json, _) => "|j| serde_json::to_string(&j).unwrap_or_default()".to_string(),
        (TypeRef::Path, _) => "|p: std::path::PathBuf| p.to_string_lossy().into_owned()".to_string(),
        (TypeRef::Bytes, CoreWrapper::Arc | CoreWrapper::ArcMutex) => "|a| (*a).clone().into()".to_string(),
        (TypeRef::Bytes, _) => "|b| b.into()".to_string(),
        (TypeRef::Primitive(_), _) => {
            // When a newtype_wrapper is set on the field, the Vec elements are
            // newtypes (e.g. NodeIndex(usize)), not raw primitives. Unwrap with .0.
            if field_newtype_wrapper.is_some() {
                "|x| x.0 as _".to_string()
            } else {
                "|x| x as _".to_string()
            }
        }
        (TypeRef::Vec(inner2), _) => {
            // Vec<Vec<T>>
            match inner2.as_ref() {
                TypeRef::Primitive(_) => {
                    // Vec<Vec<primitive>>: inner cast needed.
                    return if optional {
                        format!(
                            "v.{name}.map(|vec| vec.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect::<Vec<_>>()).collect::<Vec<_>>())"
                        )
                    } else {
                        format!(
                            "v.{name}.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        )
                    };
                }
                _ => {
                    return format!("v.{name}");
                }
            }
        }
        _ => {
            return format!("v.{name}");
        }
    };

    if optional {
        format!("v.{name}.map(|vec| vec.into_iter().map({item_conv}).collect::<Vec<_>>())")
    } else {
        format!("v.{name}.into_iter().map({item_conv}).collect::<Vec<_>>()")
    }
}

/// Emit `From<MirrorT> for SourceT` for types with sanitized fields.
///
/// This is the mirror-to-core direction, required by bridge functions that accept a
/// `MirrorT` parameter and need to call the core function with SourceT.
/// Transmute is unsound for these types because sanitized fields (e.g. `Option<String>`
/// substituted for `Option<CancellationToken>`) have different memory sizes than the
/// corresponding core field, making the transmute layout assumption false.
///
/// Non-sanitized fields use field_from_expr_to_core (the inverse of field_from_expr).
/// Sanitized fields use `Default::default()` since they represent types that cannot
/// be meaningfully passed from Dart (e.g. CancellationToken, ConcurrencyConfig).
pub(super) fn emit_from_mirror_to_core_struct(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    let name = &ty.name;
    let core_ty = if ty.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        ty.rust_path.replace('-', "_")
    };

    // Core types with private (non-`pub`) fields cannot be built with struct-literal syntax
    // from the mirror crate. Seed the core `Default` (which fills the private fields) and
    // assign the public fields onto it — via the shared construction strategy used by every
    // backend. Excluded / lossily-sanitized fields are left at their default on the base.
    if ty.has_private_fields {
        let mut assignments = Vec::new();
        for field in &ty.fields {
            if field.binding_excluded {
                continue;
            }
            let safe_sanitized_string = matches!(field.ty, TypeRef::String) && field.core_wrapper == CoreWrapper::Cow;
            if field.sanitized && !safe_sanitized_string {
                continue;
            }
            assignments.push(crate::codegen::conversions::construction::FieldAssign {
                core_field: field.name.clone(),
                expr: field_from_expr_to_core(field, source_crate_name),
            });
        }
        out.push_str(&crate::codegen::conversions::construction::gen_private_field_from_impl(
            &crate::codegen::conversions::construction::PrivateFieldImpl {
                core_path: &core_ty,
                binding_name: name,
                param: "v",
                has_default: ty.has_default,
                assignments: &assignments,
                allow_attrs: &[
                    "clippy::field_reassign_with_default, clippy::let_and_return, clippy::useless_conversion",
                ],
            },
        ));
        return;
    }

    // The generated literal ends with `..Default::default()` whenever some core
    // fields are intentionally omitted from the explicit field list AND the core
    // type derives Default (the spread itself requires Default — otherwise E0277).
    // Fields are omitted in two cases, both of which need the spread to fill them:
    //   1. cfg-gated fields stripped from the IR (`has_stripped_cfg_fields`);
    //   2. binding-excluded (`alef(skip)`) fields, which are skipped from the
    //      literal in the `has_default` branch below so the core Default supplies
    //      them (e.g. `SsrfPolicy::scheme_allowlist`/`allowlist`).
    // Gating only on (1) left binding-excluded-only types (no cfg-stripped fields)
    // with neither the field nor a spread — a hard E0063. Cover both cases.
    let omits_core_fields = ty.has_stripped_cfg_fields || ty.fields.iter().any(|field| field.binding_excluded);
    let needs_default_spread = omits_core_fields && ty.has_default;
    // clippy flags the spread as `needless_update` when the field list looks
    // complete from the mirror's perspective; silence it only when the spread is
    // actually emitted, otherwise the annotation is dead (unused_attributes).
    if needs_default_spread {
        out.push_str("#[allow(clippy::needless_update)]\n");
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_mirror_struct_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));

    for field in &ty.fields {
        if field.binding_excluded {
            if !ty.has_default {
                // The core type does not derive Default, so the trailing
                // `..Default::default()` spread would fail with E0277. Emit
                // `<field>: Default::default()` explicitly for each binding-excluded
                // field. This loses any custom core-level Default behaviour for
                // these fields, but is the only way to construct the struct literal
                // when the core type lacks a Default impl.
                emit_rust_struct_field(out, None, &field.name, "Default::default()");
                continue;
            }
            // Skip binding_excluded fields entirely; the trailing `..Default::default()`
            // spread fills them with the CORE type's Default impl. Emitting
            // `<field>: Default::default()` would override that — and is wrong when
            // the core's Default calls a custom function (e.g. `CrawlConfig::default()`
            // sets `ssrf: SsrfPolicy::from_env()`, whereas `<SsrfPolicy as Default>`
            // is the static `deny_private = true` policy).
            continue;
        }
        // Sanitized String fields with a non-Cow core_wrapper indicate the core type
        // is something completely unrelated to a string (e.g. `Option<BoundingBox>`
        // sanitized down to `Option<String>` because BoundingBox isn't in the API
        // surface). Treat those like other sanitized fields and fall back to
        // Default::default(). Only the Cow case (core `Cow<'static, str>` extracted
        // as Named("str") and sanitized to String) safely roundtrips via .into().
        let safe_sanitized_string = matches!(field.ty, TypeRef::String) && field.core_wrapper == CoreWrapper::Cow;
        if field.sanitized && !safe_sanitized_string {
            // Sanitized fields have an unknown core type simplified in the IR.
            // Only types in the transitive closure from input-parameter types get this
            // impl generated, and those core types implement Default.
            // has cancel_token: Option<CancellationToken> which implements Default).
            //
            // `cfg = None`: the dart bridge crate enables `features = ["full"]` on
            // the source dependency, so every core-side cfg-gated field is present
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            // crate's own (undefined) features, evaluating to false and leaving the
            // struct literal missing fields.
            emit_rust_struct_field(out, None, &field.name, "Default::default()");
        } else {
            let expr = field_from_expr_to_core(field, source_crate_name);
            // `cfg = None`: the dart bridge crate enables `features = ["full"]` on
            // the source dependency, so every core-side cfg-gated field is present
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            // crate's own (undefined) features, evaluating to false and leaving the
            // struct literal missing fields.
            emit_rust_struct_field(out, None, &field.name, &expr);
        }
    }

    // Emit ..Default::default() when core fields were omitted from the literal
    // (cfg-stripped or binding-excluded) and the core type derives Default — see
    // `needs_default_spread` above. Without Default the spread would E0277; such a
    // type is unconstructible from the mirror and surfaces a diagnostic E0063.
    if needs_default_spread {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Emit a `From<MirrorEnum> for SourceEnum` implementation.
///
/// Unit-only enums: simple variant match. Data enums: reconstruct each variant.
/// Build the conversion expression for one struct field in the mirror-to-core direction.
/// This is the inverse of `field_from_expr` (which handles core-to-mirror).
fn field_from_expr_to_core(field: &FieldDef, _source_crate_name: &str) -> String {
    let name = &field.name;
    match &field.ty {
        TypeRef::String => {
            // The IR collapses `Cow<'_, str>` / `Box<str>` / `Arc<str>` into
            // `TypeRef::String` and only `Cow` is tracked on `core_wrapper`. Emit
            // `.into()` unconditionally so wrapped-string core fields receive the
            // right type (e.g. `String → Box<str>`); the crate-level
            // `#[allow(clippy::useless_conversion)]` absorbs the `String → String`
            // no-op case.
            if field.optional {
                format!("v.{name}.map(Into::into)")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Char => {
            // D5: `char: From<String>` does not exist in std. Use explicit extraction.
            // Mirror holds String; core holds char. Take the first character or the
            // default char ('\0') when the string is empty.
            if field.optional {
                format!("v.{name}.as_deref().and_then(|s| s.chars().next())")
            } else {
                format!("v.{name}.chars().next().unwrap_or_default()")
            }
        }
        TypeRef::Path => {
            if field.optional {
                format!("v.{name}.map(std::path::PathBuf::from)")
            } else {
                format!("std::path::PathBuf::from(v.{name})")
            }
        }
        TypeRef::Bytes => {
            if field.optional {
                format!("v.{name}.map(Into::into)")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Json => {
            // Mirror has String; core has serde_json::Value.
            if field.optional {
                format!("v.{name}.as_deref().and_then(|s| serde_json::from_str(s).ok())")
            } else {
                format!("serde_json::from_str(&v.{name}).unwrap_or_default()")
            }
        }
        TypeRef::Named(_) => {
            // Handle Arc core wrapper: mirror has bare T but core has Arc<T>.
            // Handle is_boxed: mirror has bare T but core has Box<T>.
            match field.core_wrapper {
                CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                    if field.optional {
                        format!("v.{name}.map(|x| std::sync::Arc::new(x.into()))")
                    } else {
                        format!("std::sync::Arc::new(v.{name}.into())")
                    }
                }
                _ if field.is_boxed => {
                    if field.optional {
                        format!("v.{name}.map(|x| Box::new(x.into()))")
                    } else {
                        format!("Box::new(v.{name}.into())")
                    }
                }
                _ => {
                    if field.optional {
                        format!("v.{name}.map(Into::into)")
                    } else {
                        format!("v.{name}.into()")
                    }
                }
            }
        }
        TypeRef::Vec(inner) => {
            match inner.as_ref() {
                TypeRef::Named(_) => {
                    // Handle Arc core wrapper on Vec element types.
                    match field.vec_inner_core_wrapper {
                        CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                            if field.optional {
                                format!(
                                    "v.{name}.map(|vec| vec.into_iter().map(|x| std::sync::Arc::new(x.into())).collect())"
                                )
                            } else {
                                format!("v.{name}.into_iter().map(|x| std::sync::Arc::new(x.into())).collect()")
                            }
                        }
                        _ => {
                            if field.optional {
                                format!("v.{name}.map(|vec| vec.into_iter().map(Into::into).collect())")
                            } else {
                                format!("v.{name}.into_iter().map(Into::into).collect()")
                            }
                        }
                    }
                }
                TypeRef::Vec(inner_inner) => {
                    // Vec<Vec<T>>: FRB uses f64 for f32 primitives — need explicit cast.
                    match inner_inner.as_ref() {
                        TypeRef::Primitive(_) => {
                            if field.optional {
                                format!(
                                    "v.{name}.map(|vv| vv.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect()).collect())"
                                )
                            } else {
                                format!(
                                    "v.{name}.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect()).collect()"
                                )
                            }
                        }
                        _ => {
                            if field.optional {
                                format!(
                                    "v.{name}.map(|vv| vv.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect())"
                                )
                            } else {
                                format!(
                                    "v.{name}.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect()"
                                )
                            }
                        }
                    }
                }
                TypeRef::Primitive(_) => {
                    // When the field has a newtype_wrapper, the Vec elements are newtypes
                    // (e.g. NodeIndex(usize)) on the core side. The mirror flattens to
                    // the raw primitive, so reverse wraps with the tuple constructor.
                    let elem_conv = if let Some(nw) = &field.newtype_wrapper {
                        format!("|x| {nw}(x as _)")
                    } else {
                        "|x| x as _".to_string()
                    };
                    if field.optional {
                        format!("v.{name}.map(|vec| vec.into_iter().map({elem_conv}).collect())")
                    } else {
                        format!("v.{name}.into_iter().map({elem_conv}).collect()")
                    }
                }
                _ => {
                    // Vec<String> too: the IR collapses `Box<str>` / `Cow<'_, str>` /
                    // `Arc<str>` to `TypeRef::String` and only the `Cow` shape is
                    // tracked on `vec_inner_core_wrapper`. Emit `Into::into`
                    // unconditionally so `Vec<String>` → `Vec<Box<str>>` (etc.)
                    // compiles; the crate-level `#[allow(clippy::useless_conversion)]`
                    // absorbs the `Vec<String> → Vec<String>` no-op case.
                    if field.optional {
                        format!("v.{name}.map(|vec| vec.into_iter().map(Into::into).collect())")
                    } else {
                        format!("v.{name}.into_iter().map(Into::into).collect()")
                    }
                }
            }
        }
        TypeRef::Optional(inner) => {
            // Inverse of `field_from_expr` Optional arm: mirror's flattened `Option<T>`
            // → core's nested `Option<Option<T>>`. Wrap the per-element conversion in
            // `Some(...)` so `Some(x_mirror) → Some(Some(x_core))` and `None → None`
            // (collapsing the "no change" vs "explicit clear" distinction; see the
            // `frb_rust_type` comment for the trade-off).
            let wrap_some = if field.optional { ".map(Some)" } else { "" };
            match inner.as_ref() {
                TypeRef::Named(_) => format!("v.{name}.map(Into::into){wrap_some}"),
                TypeRef::String | TypeRef::Char => format!("v.{name}.map(Into::into){wrap_some}"),
                TypeRef::Path => format!("v.{name}.map(std::path::PathBuf::from){wrap_some}"),
                TypeRef::Primitive(_) => format!("v.{name}.map(|x| x as _){wrap_some}"),
                _ => format!("v.{name}{wrap_some}"),
            }
        }
        TypeRef::Primitive(_) => {
            if let Some(nw) = &field.newtype_wrapper {
                if field.optional {
                    format!("v.{name}.map(|x| {nw}(x as _))")
                } else {
                    format!("{nw}(v.{name} as _)")
                }
            } else if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
        TypeRef::Duration => {
            // Mirror i64 → core Duration (stored as millis).
            if field.optional {
                format!("v.{name}.map(|ms| std::time::Duration::from_millis(ms as u64))")
            } else {
                format!("std::time::Duration::from_millis(v.{name} as u64)")
            }
        }
        TypeRef::Map(_, v_ty) => {
            // HashMap: convert via iterator. The IR collapses wrapped string types
            // (`Box<str>`, `Cow<'_, str>`, `Arc<str>`) into `TypeRef::String`, so emit
            // `.into()` on both keys and values — bridges `String → Box<str>` etc. at
            // the type level. Crate-level `#[allow(clippy::useless_conversion)]` absorbs
            // the `String → String` identity case.
            let val_conv = match v_ty.as_ref() {
                TypeRef::Primitive(_) => "v as _",
                TypeRef::Named(_) => "v.into()",
                // String / Path / etc.: `.into()` covers `String → Box<str>` and identity.
                _ => "v.into()",
            };
            if field.optional {
                format!("v.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), {val_conv})).collect())")
            } else {
                format!("v.{name}.into_iter().map(|(k, v)| (k.into(), {val_conv})).collect()")
            }
        }
        TypeRef::Unit => "()".to_string(),
    }
}

/// Build conversion expression for a Map field (core → mirror).
/// Mirror always uses HashMap<String, String> or HashMap<String, T>.
/// Core may use BTreeMap, AHashMap, HashMap with Value values, etc.
///
/// When `core_wrapper` is `CoreWrapper::Cow` the map itself is a
/// `Cow<'_, BTreeMap<...>>` — call `.into_owned()` before `.into_iter()` to
/// consume the borrow and produce an owned `BTreeMap` that can be iterated.
fn map_from_expr(name: &str, _k: &TypeRef, v_ty: &TypeRef, optional: bool, core_wrapper: CoreWrapper) -> String {
    // Determine value conversion strategy. The IR collapses wrapped string types
    // (`Box<str>`, `Cow<'_, str>`, `Arc<str>`) to `TypeRef::String`, so emit `.into()`
    // for the String case as well — it bridges `Box<str> → String` (which `(k, v)`
    // identity does NOT, and which trips `FromIterator` resolution at compile time)
    // while remaining a no-op for plain `String → String` under the crate-level
    // `#[allow(clippy::useless_conversion)]`.
    let value_conv = match v_ty {
        TypeRef::Json => {
            // serde_json serialize to String.
            "serde_json::to_string(&v).unwrap_or_default()"
        }
        TypeRef::Named(mirror_name) => return map_named_from_expr(name, mirror_name, optional, core_wrapper),
        TypeRef::Primitive(_) => {
            // Cast to target primitive (i64 for integers, f64 for floats).
            "v as _"
        }
        // String / Path / Bytes / etc.: rely on the appropriate `From` impl. For
        // String this is `From<Box<str>> for String` (or identity). For other types
        // `.into()` covers `Bytes → Vec<u8>`, `PathBuf → String` is NOT auto so the
        // explicit branches above must be kept exhaustive when new shapes appear.
        _ => "v.into()",
    };

    // Keys: same reasoning as values — the IR loses `Box<str>` / `Cow<'_, str>`
    // wrappers, so always emit `.into()` rather than the identity `k`. This bridges
    // `HashMap<Box<str>, _>` → `HashMap<String, _>` at the type level; the
    // crate-level `clippy::useless_conversion` allow absorbs the no-op String case.
    //
    // When the map itself is `Cow<'_, BTreeMap<...>>`, `.into_owned()` is needed first
    // to get an owned `BTreeMap` before calling `.into_iter()`.
    let iter_method = if core_wrapper == CoreWrapper::Cow {
        "into_owned().into_iter()"
    } else {
        "into_iter()"
    };
    let iter_expr = format!("{iter_method}.map(|(k, v)| (k.into(), {value_conv})).collect()");

    if optional {
        format!("v.{name}.map(|m| m.{iter_expr})")
    } else {
        format!("v.{name}.{iter_expr}")
    }
}

fn map_named_from_expr(field_name: &str, mirror_name: &str, optional: bool, core_wrapper: CoreWrapper) -> String {
    let iter_method = if core_wrapper == CoreWrapper::Cow {
        "into_owned().into_iter()"
    } else {
        "into_iter()"
    };
    let iter_expr = format!("{iter_method}.map(|(k, v)| (k.into(), {mirror_name}::from(v))).collect()");
    if optional {
        format!("v.{field_name}.map(|m| m.{iter_expr})")
    } else {
        format!("v.{field_name}.{iter_expr}")
    }
}

/// Fallback expression for sanitized fields (unknown core types mapped to String/i64).
///
/// Sanitized fields have an unknown or complex core type that was simplified in the IR.
/// We use Default::default() as a safe fallback — attempting serde_json::to_string
/// would require the type to implement Serialize, which is not guaranteed for all
/// sanitized or excluded types.
fn sanitized_field_from_expr(field: &FieldDef) -> String {
    let name = &field.name;
    match &field.ty {
        TypeRef::Primitive(_) => {
            // Sanitized primitive: try direct cast.
            if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
        // Cow<'_, str> fields are erroneously marked sanitized by the IR extractor
        // even though the underlying type is plainly `String`. Convert via `.into()` /
        // `.into_owned()` so the actual value reaches the mirror struct rather than an
        // empty `String::default()` placeholder (which silently broke `mime_type`,
        // `format`, and similar Cow-wrapped string fields).
        TypeRef::String | TypeRef::Char if field.core_wrapper == CoreWrapper::Cow => {
            if field.optional {
                format!("v.{name}.map(|s| s.into_owned())")
            } else {
                format!("v.{name}.into_owned()")
            }
        }
        _ => {
            // All other sanitized types: use Default.
            // We cannot safely serde-serialize unknown types.
            let _ = name;
            String::from("Default::default()")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, binding_excluded: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            binding_excluded,
            ..Default::default()
        }
    }

    fn typ(name: &str, has_default: bool, has_stripped_cfg_fields: bool, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("source::{name}"),
            fields,
            is_clone: true,
            has_default,
            has_stripped_cfg_fields,
            ..Default::default()
        }
    }

    #[test]
    fn mirror_to_core_binding_excluded_with_default_uses_spread() {
        let ty = typ(
            "DefaultedWithExcluded",
            true,
            true,
            vec![field("name", false), field("internal", true)],
        );
        let mut out = String::new();
        emit_from_mirror_to_core_struct(&mut out, &ty, "source");

        assert!(
            out.contains("..Default::default()"),
            "spread should be emitted when has_default && has_stripped_cfg_fields; got:\n{out}"
        );
        assert!(
            out.contains("#[allow(clippy::needless_update)]"),
            "needless_update allow should accompany the emitted spread; got:\n{out}"
        );
        // When has_default is true, binding_excluded fields are skipped; the spread
        // fills them with the core type's Default impl (which may have custom defaults).
        assert!(
            !out.contains("internal: Default::default()"),
            "binding-excluded field should be skipped when has_default is true; got:\n{out}"
        );
    }

    #[test]
    fn mirror_to_core_stripped_cfg_without_default_omits_spread() {
        let ty = typ("NoDefaultStripped", false, true, vec![field("name", false)]);
        let mut out = String::new();
        emit_from_mirror_to_core_struct(&mut out, &ty, "source");

        assert!(
            !out.contains("..Default::default()"),
            "spread must NOT be emitted when has_default is false; got:\n{out}"
        );
        assert!(
            !out.contains("#[allow(clippy::needless_update)]"),
            "needless_update allow must NOT be emitted when no spread; got:\n{out}"
        );
    }

    #[test]
    fn mirror_to_core_binding_excluded_without_default_emits_explicit_only() {
        let ty = typ(
            "NoDefaultExcluded",
            false,
            false,
            vec![field("name", false), field("internal", true)],
        );
        let mut out = String::new();
        emit_from_mirror_to_core_struct(&mut out, &ty, "source");

        assert!(
            !out.contains("..Default::default()"),
            "spread must NOT be emitted when has_default is false; got:\n{out}"
        );
        assert!(
            out.contains("internal: Default::default()"),
            "binding-excluded field must be explicitly defaulted; got:\n{out}"
        );
    }

    #[test]
    fn mirror_to_core_no_excluded_no_spread() {
        let ty = typ("Plain", true, false, vec![field("name", false), field("value", false)]);
        let mut out = String::new();
        emit_from_mirror_to_core_struct(&mut out, &ty, "source");

        assert!(
            !out.contains("..Default::default()"),
            "spread must not appear when there are no stripped cfg fields; got:\n{out}"
        );
    }
}
