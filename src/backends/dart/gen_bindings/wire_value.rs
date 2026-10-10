//! `.wireValue` extension for FRB-generated flat Dart enums.
//!
//! flutter_rust_bridge derives a plain Dart `enum` for an all-unit Rust enum straight from the
//! generated Rust mirror (`gen_rust_crate::mirror`); alef has no template control over that
//! generated file. This module instead emits a Dart `extension` in the alef-owned bridge module
//! (`<module>.dart`) -- the one FRB-mode Dart file `flutter_rust_bridge_codegen` never
//! regenerates -- adding a `wireValue` getter that returns the exact serde wire string for the
//! variant (via [`wire_variant_value`]). This joins the wire-value accessor the go/java/csharp/
//! node/python backends already expose for the same purpose (typed string constants,
//! `getValue()`, ...). The idiomatic lowerCamelCase Dart member name FRB derives
//! (`DataNodeKind.keyValue`) and the wire value (`"KeyValue"`) are separate naming surfaces per
//! `centralized-naming` -- this getter is the only place the wire-value surface is reachable
//! from generated Dart. ~keep

use std::collections::HashSet;

use heck::ToLowerCamelCase;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::codegen::naming::wire_variant_value;
use crate::core::ir::EnumDef;

/// Enums flutter_rust_bridge maps to a plain Dart `enum`: non-empty, every variant a unit
/// variant, and not excluded from the binding.
///
/// Mirrors the filter `gen_rust_crate::mod::emit`'s mirror-enum loop applies
/// (`!exclude_types.contains(&e.name) && !e.binding_excluded`), plus the "all unit variants"
/// check `gen_ffi::types::emit_enum` uses to decide between a plain Dart `enum` and a sealed
/// class -- FRB makes that same decision from the Rust mirror's shape.
pub(super) fn flat_wire_enums<'a>(enums: &'a [EnumDef], exclude_types: &HashSet<&str>) -> Vec<&'a EnumDef> {
    enums
        .iter()
        .filter(|e| {
            !exclude_types.contains(e.name.as_str())
                && !e.binding_excluded
                && !e.variants.is_empty()
                && e.variants.iter().all(|v| v.fields.is_empty())
        })
        .collect()
}

/// Append a `{{Enum}}WireValue` extension exposing `.wireValue` for every enum in `enums`.
///
/// Must filter variants exactly like `gen_rust_crate::mirror::emit_mirror_enum`: this extension
/// is written against the plain Dart `enum` FRB derives FROM that mirror declaration, so a
/// foreign cfg-gated variant `configured_features` proves unreachable -- and that the mirror
/// therefore no longer declares -- has no corresponding Dart enum member here either. Emitting
/// `.wireValue` for it anyway referenced `SomeEnum.droppedVariant`, a member the FRB-derived Dart
/// enum no longer has: not merely a documentation gap but a Dart compile error. Asking the same
/// `enum_variant_declaration` authority the mirror now consults keeps the two in lockstep. ~keep
pub(super) fn emit_wire_value_extensions(
    enums: &[&EnumDef],
    source_crate_name: &str,
    configured_features: Option<&[String]>,
    out: &mut String,
) {
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    for en in enums {
        let is_host_enum = is_host_owned_rust_path(source_crate_name, &en.rust_path);
        let variants: Vec<minijinja::Value> = en
            .variants
            .iter()
            .filter(|v| {
                !matches!(
                    enum_variant_declaration(v, is_host_enum, configured_features_set.as_ref()),
                    VariantDeclaration::Drop
                )
            })
            .map(|v| {
                let vname = dart_safe_ident(&v.name.to_lower_camel_case());
                let wire = escape_dart_string(&wire_variant_value(
                    &v.name,
                    v.serde_rename.as_deref(),
                    en.serde_rename_all.as_deref(),
                ));
                crate::alef_context! { vname => vname, wire => wire }
            })
            .collect();
        out.push_str(&template_env::render(
            "enum_wire_value_extension.jinja",
            crate::alef_context! {
                name => en.name.as_str(),
                variants => variants,
            },
        ));
        out.push('\n');
    }
}

/// Escape a string for a single-quoted Dart string literal.
fn escape_dart_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'").replace('$', "\\$")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumVariant, FieldDef, PrimitiveType, TypeRef};

    fn unit_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            ..EnumVariant::default()
        }
    }

    fn flat_enum(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            variants,
            ..EnumDef::default()
        }
    }

    #[test]
    fn flat_wire_enums_excludes_data_variants() {
        let mut with_data = flat_enum("Shape", vec![unit_variant("Circle")]);
        with_data.variants.push(EnumVariant {
            name: "Rect".to_string(),
            fields: vec![FieldDef {
                name: "w".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..FieldDef::default()
            }],
            ..EnumVariant::default()
        });
        let enums = vec![with_data];
        let result = flat_wire_enums(&enums, &HashSet::new());
        assert!(
            result.is_empty(),
            "an enum with a data variant must not be treated as flat"
        );
    }

    #[test]
    fn flat_wire_enums_excludes_binding_excluded() {
        let mut excluded = flat_enum("Hidden", vec![unit_variant("A")]);
        excluded.binding_excluded = true;
        let enums = vec![excluded];
        assert!(flat_wire_enums(&enums, &HashSet::new()).is_empty());
    }

    #[test]
    fn flat_wire_enums_excludes_configured_exclude_types() {
        let enums = vec![flat_enum("Kind", vec![unit_variant("A")])];
        let exclude: HashSet<&str> = ["Kind"].into_iter().collect();
        assert!(flat_wire_enums(&enums, &exclude).is_empty());
    }

    #[test]
    fn flat_wire_enums_excludes_empty_enums() {
        let enums = vec![flat_enum("Empty", vec![])];
        assert!(flat_wire_enums(&enums, &HashSet::new()).is_empty());
    }

    /// The fixture shape from the reported defect: no `serde(rename_all)`, so the wire value
    /// is the exact Rust variant name -- PascalCase, not the lowerCamelCase Dart member name.
    #[test]
    fn emit_wire_value_extensions_uses_the_exact_wire_value_with_no_rename_all() {
        let enums = [flat_enum(
            "DataNodeKind",
            vec![unit_variant("KeyValue"), unit_variant("Sequence")],
        )];
        let refs: Vec<&EnumDef> = enums.iter().collect();
        let mut out = String::new();
        emit_wire_value_extensions(&refs, "mylib", Some(&[]), &mut out);
        assert!(
            out.contains("extension DataNodeKindWireValue on DataNodeKind {"),
            "got: {out}"
        );
        assert!(out.contains("case DataNodeKind.keyValue:"), "got: {out}");
        assert!(out.contains("return 'KeyValue';"), "got: {out}");
        assert!(out.contains("case DataNodeKind.sequence:"), "got: {out}");
        assert!(out.contains("return 'Sequence';"), "got: {out}");
    }

    /// `rename_all` and per-variant `serde_rename` must feed the same wire value the fixture
    /// tests will assert against -- no double translation through the Dart member name.
    #[test]
    fn emit_wire_value_extensions_honors_rename_all_and_serde_rename() {
        let mut en = flat_enum("Status", vec![unit_variant("InProgress"), unit_variant("Done")]);
        en.serde_rename_all = Some("kebab-case".to_string());
        en.variants[1].serde_rename = Some("finished".to_string());
        let enums = [en];
        let refs: Vec<&EnumDef> = enums.iter().collect();
        let mut out = String::new();
        emit_wire_value_extensions(&refs, "mylib", Some(&[]), &mut out);
        assert!(out.contains("return 'in-progress';"), "got: {out}");
        assert!(out.contains("return 'finished';"), "got: {out}");
    }

    fn unit_variant_with_cfg(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            cfg: cfg.map(str::to_string),
            ..unit_variant(name)
        }
    }

    /// The Dart compile-error consequence this module exists to prevent: once
    /// `gen_rust_crate::mirror::emit_mirror_enum` drops a foreign cfg-gated variant proven
    /// unreachable, this extension must drop it too -- an unfiltered `.wireValue` clause would
    /// reference a Dart enum member FRB no longer derives. The control half (feature configured)
    /// proves the drop is conditional on `configured_features`, not unconditional on being
    /// foreign-owned.
    #[test]
    fn emit_wire_value_extensions_drops_foreign_variant_proven_unreachable_keeps_when_active() {
        fn sync_mode() -> EnumDef {
            let mut en = flat_enum(
                "SyncMode",
                vec![
                    unit_variant_with_cfg("Manual", None),
                    unit_variant_with_cfg("Automatic", None),
                    unit_variant_with_cfg("Testkit", Some(r#"feature = "testkit""#)),
                ],
            );
            en.rust_path = "dep_crate::SyncMode".to_string();
            en
        }

        let excluded = [sync_mode()];
        let refs: Vec<&EnumDef> = excluded.iter().collect();
        let mut out = String::new();
        emit_wire_value_extensions(&refs, "mylib", Some(&[]), &mut out);
        assert!(
            out.contains("case SyncMode.manual:") && out.contains("case SyncMode.automatic:"),
            "the two always-present variants must still get wire-value clauses, got:\n{out}"
        );
        assert!(
            !out.contains("Testkit") && !out.contains("testkit"),
            "the excluded variant must not appear anywhere in the extension, got:\n{out}"
        );

        let active = [sync_mode()];
        let refs_active: Vec<&EnumDef> = active.iter().collect();
        let mut out_active = String::new();
        let active_features = vec!["testkit".to_string()];
        emit_wire_value_extensions(&refs_active, "mylib", Some(&active_features), &mut out_active);
        assert!(
            out_active.contains("case SyncMode.testkit:") && out_active.contains("return 'Testkit';"),
            "with \"testkit\" configured, the wire-value clause must be present, got:\n{out_active}"
        );
    }
}
