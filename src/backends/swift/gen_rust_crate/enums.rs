//! Emits the swift-bridge mirror enum wrapper and its `From` conversion.
//!
//! Every declared variant is mirrored as a FIELDLESS variant, data-carrying or not: the
//! `From<core>` conversion matches a data variant as `Variant(..)` / `Variant { .. }` and yields
//! the bare mirror variant, dropping its payload. There is no catch-all `Unknown` variant; a
//! variant the mirror does not declare is instead unreachable by construction (see
//! [`declared_variants`]), and the conversion gains a `_ => unreachable!()` arm only when some
//! core variant has no arm of its own.
//!
//! The reverse direction, `__alef_{enum}_from_swift_string`, is emitted only for enums whose
//! variants are all fieldless — a discriminant-only wire string cannot reconstruct field data.

use crate::backends::swift::gen_rust_crate::type_bridge::enum_from_string_fn_name;
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::{EnumDef, EnumVariant};
use std::collections::{HashMap, HashSet};

/// The variants a Swift-facing surface for `en` may advertise.
///
/// The swift-bridge mirror enum below is self-contained -- unlike the `From`-impl match arms in
/// [`emit_enum_wrapper`], its variant list never references the core type's path, so it compiles
/// regardless of cfg. That is exactly why it could (before alef #547) unconditionally advertise a
/// FOREIGN cfg-gated variant this binding's own configured feature set proves unreachable: a
/// Swift caller could construct `.extra` even though the `From`-impl already drops that variant's
/// arm unconditionally, so no conversion could ever produce it.
///
/// This is the SAME authority (`enum_variant_declaration`) napi's `gen_enum` consults for the
/// identical decision. It is `pub(crate)` rather than inlined into [`emit_enum_wrapper`] because
/// TWO surfaces must agree on this exact list, in two different modules: the mirror enum
/// declaration plus its to-string match here (both reference `Self::{variant}` against the mirror
/// type), and the public Swift `enum`'s `case` list emitted by
/// `gen_bindings::enums::emit_enum`. A Swift `case` for a variant the mirror dropped is a case no
/// conversion in either direction can ever produce or accept -- the facade advertises API that
/// does not exist. Both callers ask this one function instead of each re-deriving the rule. ~keep
///
/// `en.rust_path` (not the `type_paths`-remapped source path) is the same fact
/// `codegen::cfg::collect_cfg_gates` reads to decide whether a cfg is safe to forward as a Cargo
/// feature; a variant's cfg is only safe to re-emit verbatim when this enum is owned by the host
/// crate. See `is_host_owned_rust_path`'s doc for why both halves must agree. ~keep
pub(crate) fn declared_variants<'a>(
    en: &'a EnumDef,
    source_crate: &str,
    configured_features: Option<&[String]>,
) -> Vec<&'a EnumVariant> {
    let is_host_enum = is_host_owned_rust_path(source_crate, &en.rust_path);
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    en.variants
        .iter()
        .filter(|variant| {
            !matches!(
                enum_variant_declaration(variant, is_host_enum, configured_features_set.as_ref()),
                VariantDeclaration::Drop
            )
        })
        .collect()
}

pub(crate) fn emit_enum_wrapper(
    en: &EnumDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    configured_features: Option<&[String]>,
) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&en.name, source_crate, type_paths);
    let is_host_enum = is_host_owned_rust_path(source_crate, &en.rust_path);

    let declared_variants = declared_variants(en, source_crate, configured_features);

    out.push_str(&crate::backends::swift::template_env::render(
        "enum_unit_header.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    for variant in &declared_variants {
        out.push_str(&crate::backends::swift::template_env::render(
            "enum_unit_variant.jinja",
            minijinja::context! {
                variant_name => &variant.name,
            },
        ));
    }

    out.push_str("}\n\n");

    out.push_str(&crate::backends::swift::template_env::render(
        "enum_from_impl_header.jinja",
        minijinja::context! {
            source_path => &source_path,
            name => &en.name,
        },
    ));
    out.push_str("        match val {\n");

    for variant in &en.variants {
        // A variant merged in from a foreign `[[crates.source_crates]]` crate carries that
        // crate's own cfg gate; this swift-bridge crate never declares a Cargo feature for it
        // (see `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim onto the match arm
        // is an `unexpected cfg condition value` error. Drop the arm entirely instead -- named
        // and counted via `tracing::debug!`, not silently -- and fall through to the `_ =>
        // unreachable!()` catch-all below. ~keep
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %en.name,
                enum_rust_path = %en.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping Swift bridge From-impl arm for a foreign-crate enum variant behind a \
                 #[cfg(...)] this crate cannot declare as a Cargo feature; the variant is \
                 unreachable from this conversion"
            );
            continue;
        }

        let pattern = if variant.fields.is_empty() {
            variant.name.clone()
        } else if variant.is_tuple {
            format!("{}(..)", variant.name)
        } else {
            format!("{} {{ .. }}", variant.name)
        };

        // Mirror the dart enum_conversions emitter: variants gated by upstream `#[cfg(...)]`
        // (e.g. `Heif` under `#[cfg(feature = "heic")]`) must carry that same gate on the
        if let Some(condition) = variant.cfg.as_deref() {
            out.push_str("            #[cfg(");
            out.push_str(condition);
            out.push_str(")]\n");
        }

        out.push_str(&crate::backends::swift::template_env::render(
            "enum_from_variant.jinja",
            minijinja::context! {
                source_path => &source_path,
                variant_name => &variant.name,
                pattern => pattern,
            },
        ));
    }

    // A foreign cfg-gated variant's arm is dropped unconditionally above, so whether a catch-all
    // is still needed for it depends on whether this binding's own configured feature set proves
    // the variant unreachable -- delegated to
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features`, the same
    // resolver every other Rust-emitting backend's enum conversion uses, so this bespoke Swift
    // generator can't drift from that verdict (alef #547). `!en.excluded_variants.is_empty()`
    // covers the orthogonal gap this match alone can have: a core variant this binding never
    // generates an arm for at all, regardless of cfg. This match is over the real CORE type
    // (`source_path` above) -- swift-bridge only ever generates this one direction for enums, and
    // the bridge wrapper declared just above is never matched over -- so `configured_features`'
    // proof about the dependency is already the complete answer. `true` here. See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if crate::codegen::conversions::enum_conversion_needs_catch_all_for_features(
        en,
        is_host_enum,
        !en.excluded_variants.is_empty(),
        configured_features,
        true,
    ) {
        out.push_str(&format!(
            "            _ => unreachable!(\"bridge enum variant of {} not exposed in binding\"),\n",
            en.name
        ));
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // Filtered by `declared_variants`, not `en.variants`: this match arm names `Self::{variant}`
    // against the SAME mirror type declared above, so it must reference exactly the variants that
    // type actually has -- a proven-unreachable foreign variant dropped from the declaration would
    // otherwise still be matched here, an `E0599: no variant found` compile error. ~keep
    let mut variants = String::new();
    for variant in &declared_variants {
        let serde_name = serde_variant_wire_name(variant, en.serde_rename_all.as_deref());
        variants.push_str(&crate::backends::swift::template_env::render(
            "rust_enum_to_string_variant.rs.jinja",
            minijinja::context! {
                variant_name => &variant.name,
                serde_name => &serde_name,
            },
        ));
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "rust_enum_to_string_impl.rs.jinja",
        minijinja::context! {
            enum_name => &en.name,
            variants => variants,
        },
    ));

    // `__alef_{enum}_from_swift_string` reconstructs an enum variant from the wire string
    // swift-bridge hands it, which only carries a variant's discriminant -- never its field
    // data. That is fine for a fieldless (unit) variant: `EnumName::Variant` is a complete
    // value. It is not possible for a variant with fields: there is no field data in a `&str`
    // to reconstruct with. Every call site that would invoke this helper already knows this
    // and only does so when `unit_enum_names` (all variants fieldless) contains the enum --
    // see `gen_rust_crate::shims` and `gen_rust_crate::wrappers::methods`. A tagged enum's
    // parameters are routed through `serde_json::from_str` instead, never through this
    // function. So when any variant carries fields, this helper has no caller and emitting it
    // is emitting dead code that also happens to be broken (a bare `EnumName::StructVariant`
    // or `EnumName::TupleVariant` path does not type-check, E0533/E0308). Skipping emission
    // entirely -- rather than patching the arms to compile and silently panic at runtime --
    // keeps the absence of a string-based reconstruction honest: the function simply does not
    // exist for enums it cannot serve. ~keep
    let is_unit_enum = en.variants.iter().all(|v| v.fields.is_empty());
    if is_unit_enum {
        let mut from_string_variants = String::new();
        for variant in &en.variants {
            // This arm names `{{ source_path }}::{{ variant_name }}` directly, with no cfg
            // guard at all until this fix -- a bug independent of host-vs-foreign: a host-owned
            // cfg-gated variant (e.g. `Heif` under `#[cfg(feature = "heic")]`) referenced this
            // way is just as unguarded a reference to a possibly-nonexistent variant as a
            // foreign one is. A foreign cfg additionally cannot be forwarded as a Cargo feature
            // (see the From-impl loop above and `codegen::cfg::collect_cfg_gates`), so that case
            // drops the arm entirely instead of gating it. ~keep
            if variant.cfg.is_some() && !is_host_enum {
                tracing::debug!(
                    enum_name = %en.name,
                    enum_rust_path = %en.rust_path,
                    variant_name = %variant.name,
                    cfg = variant.cfg.as_deref().unwrap_or_default(),
                    "dropping Swift bridge from-string reconstruction arm for a foreign-crate \
                     enum variant behind a #[cfg(...)] this crate cannot declare as a Cargo \
                     feature; the variant is unreachable from this helper"
                );
                continue;
            }

            let serde_name = serde_variant_wire_name(variant, en.serde_rename_all.as_deref());
            if let Some(condition) = variant.cfg.as_deref() {
                from_string_variants.push_str("        #[cfg(");
                from_string_variants.push_str(condition);
                from_string_variants.push_str(")]\n");
            }
            from_string_variants.push_str(&crate::backends::swift::template_env::render(
                "rust_enum_from_string_variant.rs.jinja",
                minijinja::context! {
                    variant_name => &variant.name,
                    serde_name => &serde_name,
                    source_path => &source_path,
                },
            ));
        }

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_enum_from_string_impl.rs.jinja",
            minijinja::context! {
                fn_name => enum_from_string_fn_name(&en.name),
                enum_name => &en.name,
                source_path => &source_path,
                variants => from_string_variants,
            },
        ));
    }

    out
}

/// Compute the serde-serialized name for a unit enum variant.
///
/// Priority order:
/// 1. Explicit `#[serde(rename = "...")]` on the variant.
/// 2. `rename_all` transformation applied to the Rust identifier.
/// 3. Raw Rust identifier (no transformation).
fn serde_variant_wire_name(variant: &crate::core::ir::EnumVariant, rename_all: Option<&str>) -> String {
    crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    fn make_tuple_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: vec![crate::core::ir::FieldDef {
                name: "0".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            is_tuple: true,
            ..Default::default()
        }
    }

    fn make_struct_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: vec![crate::core::ir::FieldDef {
                name: "value".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            is_tuple: false,
            ..Default::default()
        }
    }

    /// A fieldless enum is exactly what `__alef_{enum}_from_swift_string` can reconstruct
    /// from a wire string, and every call site that would invoke it only does so once
    /// `unit_enum_names` (all variants fieldless) contains the enum. The helper must still
    /// be emitted for this shape.
    #[test]
    fn fieldless_enum_still_emits_from_string_helper() {
        let en = EnumDef {
            name: "Mode".to_string(),
            variants: vec![make_unit_variant("Fast", None), make_unit_variant("Thorough", None)],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            out.contains("fn __alef_mode_from_swift_string"),
            "expected the from-string helper for a fieldless enum, got:\n{out}"
        );
        assert!(
            out.contains("\"Fast\" => Ok(mylib::Mode::Fast),"),
            "expected an `Ok`-wrapped unit-variant arm now that the helper is fallible \
             (unknown wire values used to panic across the FFI boundary), got:\n{out}"
        );
        assert!(
            out.contains("Result<mylib::Mode, String>"),
            "the helper must return a Result so an unrecognised wire string can be reported \
             as a real error instead of unwinding a panic across the FFI boundary, got:\n{out}"
        );
        assert!(
            !out.contains("panic!"),
            "an unrecognised enum wire string must no longer panic across the FFI boundary, got:\n{out}"
        );
    }

    /// A variant with fields cannot be reconstructed from a wire string alone -- there is no
    /// field data in a `&str`. Before this fix, `emit_enum_wrapper` still emitted a bare
    /// `EnumName::Variant` path for every variant regardless of fields, which does not
    /// type-check against a tuple or struct variant (E0308 / E0533). No call site ever
    /// invokes this helper for an enum with any fielded variant (they route through
    /// `serde_json::from_str` instead), so the correct fix is to not emit the helper at all
    /// for this shape, rather than patch the arms to compile and panic at runtime.
    #[test]
    fn fielded_enum_omits_from_string_helper_entirely() {
        let en = EnumDef {
            name: "AuthHeaderFormat".to_string(),
            variants: vec![make_unit_variant("None", None), make_tuple_variant("ApiKey")],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            !out.contains("__alef_auth_header_format_from_swift_string"),
            "expected no from-string helper for an enum with a tuple variant, got:\n{out}"
        );
        assert!(
            !out.contains("fn __alef_"),
            "expected no from-string helper of any name for an enum with a tuple variant, got:\n{out}"
        );
    }

    /// Same as above but for a struct variant (named fields), the other data-carrying shape.
    #[test]
    fn struct_variant_enum_omits_from_string_helper_entirely() {
        let en = EnumDef {
            name: "CacheBackend".to_string(),
            variants: vec![make_unit_variant("Memory", None), make_struct_variant("OpenDal")],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            !out.contains("fn __alef_"),
            "expected no from-string helper for an enum with a struct variant, got:\n{out}"
        );
    }

    /// When a FOREIGN variant in the primary list carries a `#[cfg(...)]` gate not proven
    /// unreachable (`configured_features: None` is "unknown"), the From-impl match must emit a
    /// `_ => unreachable!()` catch-all arm so it remains exhaustive when that feature is inactive
    /// (E0004 guard). See `host_cfg_variant_keeps_its_arm_and_gains_a_cfg_guard_in_from_string_helper`
    /// below for the sibling host-owned case, which needs NO catch-all since its arm carries the
    /// identical `#[cfg(...)]` guard as the variant itself. ~keep
    #[test]
    fn cfg_gated_variant_emits_catch_all_in_from_impl() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            rust_path: "dep_crate::ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Jpeg", None),
                make_unit_variant("Heif", Some(r#"feature = "heic""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all `_ => unreachable!` arm when cfg-gated variant present, got:\n{out}"
        );
        assert!(
            out.contains("ImageOutputFormat"),
            "catch-all message must include the enum name, got:\n{out}"
        );
    }

    /// When no variant is cfg-gated and `excluded_variants` is empty, no catch-all
    /// should be emitted (the match is statically exhaustive without it).
    #[test]
    fn no_cfg_or_excluded_variants_does_not_emit_catch_all() {
        let en = EnumDef {
            name: "SimpleEnum".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            !out.contains("_ => unreachable!"),
            "unexpected catch-all arm in From impl for fully-covered enum:\n{out}"
        );
    }

    /// `excluded_variants` alone (no inline cfg gates) must still trigger the catch-all.
    #[test]
    fn excluded_variants_alone_emits_catch_all() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![make_unit_variant("Jpeg", None)],
            methods: vec![],
            excluded_variants: vec![make_unit_variant("ExcludedVariant", None)],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all arm when excluded_variants is non-empty, got:\n{out}"
        );
    }

    /// The regression this task fixes: a variant merged in from a foreign
    /// `[[crates.source_crates]]` crate (`rust_path` rooted in a crate other than the host)
    /// carries that crate's own cfg. Forwarding it verbatim onto the From-impl match arm names a
    /// feature this swift-bridge crate never declares -- an `unexpected cfg condition value`
    /// error -- so the arm must be dropped entirely instead of cfg-gated.
    #[test]
    fn foreign_cfg_variant_arm_is_dropped_not_gated_in_from_impl() {
        let en = EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "dep_crate::TierStrategy".to_string(),
            variants: vec![
                make_unit_variant("Auto", None),
                make_unit_variant("Tier1", Some(r#"feature = "testkit""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            !out.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{out}"
        );
        assert!(
            !out.contains("dep_crate::TierStrategy::Tier1 =>"),
            "a foreign-crate cfg-gated variant must not be referenced in the From-impl match, got:\n{out}"
        );
        assert!(
            out.contains("_ => unreachable!"),
            "dropping the arm must still leave the match exhaustive via the catch-all, got:\n{out}"
        );
    }

    /// Same regression, in the `__alef_{enum}_from_swift_string` reconstruction helper: before
    /// this fix that helper never gated a cfg'd variant's arm at all (host or foreign), so a
    /// foreign one is an outright compile error and even a host-owned one was an unguarded
    /// reference. The foreign case drops the arm.
    #[test]
    fn foreign_cfg_variant_arm_is_dropped_from_from_string_helper() {
        let en = EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "dep_crate::TierStrategy".to_string(),
            variants: vec![
                make_unit_variant("Auto", None),
                make_unit_variant("Tier1", Some(r#"feature = "testkit""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            out.contains("fn __alef_tier_strategy_from_swift_string"),
            "the helper is still emitted for the enum's remaining unit variants, got:\n{out}"
        );
        assert!(
            !out.contains("dep_crate::TierStrategy::Tier1"),
            "a foreign-crate cfg-gated variant must not be referenced in the from-string helper, got:\n{out}"
        );
    }

    /// A host-owned cfg-gated variant (`rust_path` rooted in the host crate) keeps its arm in
    /// both the From-impl match and the from-string helper, but the from-string helper's arm
    /// must now carry the same `#[cfg(...)]` guard the From-impl arm already carried -- omitting
    /// it is an unguarded reference to a variant that may not exist when the feature is off.
    #[test]
    fn host_cfg_variant_keeps_its_arm_and_gains_a_cfg_guard_in_from_string_helper() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            rust_path: "mylib::ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Jpeg", None),
                make_unit_variant("Heif", Some(r#"feature = "heic""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths, None);
        assert!(
            out.contains("mylib::ImageOutputFormat::Heif => Self::Heif,"),
            "the host-owned variant's From-impl arm must still be emitted, got:\n{out}"
        );
        assert!(
            out.contains("\"Heif\" => Ok(mylib::ImageOutputFormat::Heif),"),
            "the host-owned variant's from-string arm must still be emitted, got:\n{out}"
        );
        assert_eq!(
            out.matches("#[cfg(feature = \"heic\")]").count(),
            2,
            "both the From-impl arm and the from-string arm must carry the #[cfg] guard, got:\n{out}"
        );
        // alef #547: a host-owned cfg-gated variant's own arm carries the identical #[cfg(...)]
        // guard as the variant itself, so the two always compile in or out together and the match
        // stays exhaustive either way -- unlike the foreign case, no catch-all is ever needed
        // here, regardless of `configured_features`.
        assert!(
            !out.contains("_ => unreachable!"),
            "a host-owned cfg-gated variant alone must not trigger a catch-all, got:\n{out}"
        );
    }

    /// The regression this task fixes: the swift-bridge mirror enum's own variant list is a
    /// self-contained declaration -- unlike the From-impl arms above it, it never references
    /// `source_path`, so it compiled fine even while unconditionally listing a FOREIGN cfg-gated
    /// variant this binding's own configured feature set proves unreachable. A Swift caller could
    /// then construct `.extra` even though no From-impl arm (dropped unconditionally regardless
    /// of provenance, see the tests above) could ever produce it.
    #[test]
    fn foreign_variant_proven_unreachable_is_absent_from_mirror_declaration() {
        let en = EnumDef {
            name: "SampleMode".to_string(),
            rust_path: "dep_crate::SampleMode".to_string(),
            variants: vec![
                make_unit_variant("Base", None),
                make_unit_variant("Extra", Some(r#"feature = "testkit""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let configured_features: Vec<String> = vec![];
        let out = emit_enum_wrapper(&en, "hostlib", &type_paths, Some(&configured_features));

        assert!(
            out.contains("pub enum SampleMode {\n    Base,\n}\n\n"),
            "the mirror enum must declare only the reachable variant, got:\n{out}"
        );
        assert!(
            !out.contains("Extra,"),
            "a proven-unreachable foreign variant must not be declared on the mirror enum, got:\n{out}"
        );
        assert!(
            !out.contains("Self::Extra"),
            "the to-string match must not reference a variant the mirror enum no longer declares, got:\n{out}"
        );
    }

    /// Same foreign variant, but `configured_features` is `None` -- "unknown", not proven absent,
    /// since Cargo feature unification could still enable it -- so the mirror declaration must
    /// keep advertising it, unchanged from before this fix.
    #[test]
    fn foreign_variant_not_proven_unreachable_stays_on_mirror_declaration() {
        let en = EnumDef {
            name: "SampleMode".to_string(),
            rust_path: "dep_crate::SampleMode".to_string(),
            variants: vec![
                make_unit_variant("Base", None),
                make_unit_variant("Extra", Some(r#"feature = "testkit""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "hostlib", &type_paths, None);

        assert!(
            out.contains("pub enum SampleMode {\n    Base,\n    Extra,\n}\n\n"),
            "an unproven foreign variant must stay on the mirror declaration, got:\n{out}"
        );
        assert!(
            out.contains("Self::Extra => \"Extra\".to_string(),"),
            "the to-string match must still cover it, got:\n{out}"
        );
    }

    /// A host-owned cfg-gated variant must never be dropped from the mirror declaration --
    /// existing behavior, unchanged by this fix -- regardless of what `configured_features` says.
    #[test]
    fn host_owned_cfg_gated_variant_stays_on_mirror_declaration_regardless_of_configured_features() {
        let en = EnumDef {
            name: "SampleMode".to_string(),
            rust_path: "hostlib::SampleMode".to_string(),
            variants: vec![
                make_unit_variant("Base", None),
                make_unit_variant("Extra", Some(r#"feature = "extra_feature""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let configured_features: Vec<String> = vec![];
        let out = emit_enum_wrapper(&en, "hostlib", &type_paths, Some(&configured_features));

        assert!(
            out.contains("pub enum SampleMode {\n    Base,\n    Extra,\n}\n\n"),
            "a host-owned cfg-gated variant must stay declared, got:\n{out}"
        );
    }
}
