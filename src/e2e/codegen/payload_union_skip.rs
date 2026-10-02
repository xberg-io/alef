//! The one refusal for "this field is a payload-carrying union, and this binding's scalar wire
//! accessor does not exist on it".
//!
//! ~keep Four e2e assertion generators lower an enum-typed field by appending the accessor their
//! binding backend emits for the serde wire value: dart `.wireValue`
//! (`backends::dart::gen_bindings::wire_value::flat_wire_enums`), kotlin_android `.toWire()`
//! (`backends::kotlin::gen_bindings::object_wrapper::enums::emit_enum`), the kotlin/JVM target's
//! `.getValue()` on the Java facade (`backends::java::gen_bindings::types::enums::emits_get_value`)
//! and swift `.rawValue` (`backends::swift::gen_bindings::enums::emit_enum`). Every one of those
//! is emitted on a branch the backend takes only for some enum shapes; for a data-carrying union
//! the binding renders a freezed sealed class / Kotlin sealed class / Swift enum with associated
//! values instead, which declares no such member.
//!
//! Withholding the accessor is only half a fix, and the dangerous half on its own: the field then
//! falls through to the same generic string pipeline a `String` field takes, and is compared to
//! the fixture's wire literal. In Dart that compares freezed's diagnostic `toString()`, in Kotlin
//! a wrapper object against a `String` (a comparison that is simply false at runtime), and in
//! Swift it is a type mismatch that does not compile. The refusal below is what stops the
//! withheld accessor from turning into a wrong assertion.
//!
//! Refusing the whole assertion, not just its `equals` arm, is deliberate: every string-shaped arm
//! in these generators reads the same lowered expression, so `contains`/`starts_with`/`not_empty`
//! are wrong in exactly the same way `equals` is. The two targets do NOT agree on which enums lack
//! the accessor, so the predicate is per-target — see [`UnionLoweringTarget`].
//!
//! Comment syntax and indentation stay at the call site, matching
//! [`super::field_skip::nested_wildcard_skip_line`]. The returned line carries no trailing newline.

use super::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// Which binding's enum lowering a generated assertion is written against.
///
/// ~keep The four targets do not share one predicate. `Dart`, `KotlinAndroid` and `Swift` each get
/// their scalar accessor on the branch their backend takes when *every* variant is fieldless, so
/// the question is simply "does this enum carry data anywhere". `KotlinJvm` and `Java` follow
/// the Java facade's `emits_get_value` predicate, including its sealed-union path for externally
/// tagged single-`String` payload variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnionLoweringTarget {
    /// dart, whose `.wireValue` extension is emitted only for an all-fieldless-variants enum.
    Dart,
    /// kotlin_android, whose `enum class` branch (the only one declaring `toWire()`) is likewise
    /// reached only when every variant is fieldless.
    KotlinAndroid,
    /// kotlin/JVM, which asserts against the Java facade and therefore follows
    /// `backends::java::gen_bindings::types::enums::emits_get_value`.
    KotlinJvm,
    /// java, the facade [`Self::KotlinJvm`] borrows: same predicate, because it is the same
    /// `getValue()` on the same generated class. Named separately so the java e2e generator's
    /// call site reads as what it targets rather than as a kotlin variant it merely shares a
    /// predicate with, and so the two can diverge without one silently dragging the other. ~keep
    Java,
    /// swift, whose `: String` raw-value enum declaration (the only one with `.rawValue`) is
    /// reached only when every variant is fieldless.
    Swift,
}

/// Whether the binding for `target` lowers `field`'s enum to a shape carrying no scalar wire
/// accessor.
///
/// ~keep Answers `false` for anything the IR does not positively resolve to a concrete enum type
/// (an unresolved root type, or a field classified as enum only through the hand-maintained
/// `fields_enum` config): a resolver with no IR wired must keep its pre-existing behaviour rather
/// than have this guess and start refusing assertions it has no evidence about.
pub(crate) fn lacks_scalar_wire_accessor(
    field_resolver: &FieldResolver,
    field: &str,
    target: UnionLoweringTarget,
) -> bool {
    match target {
        UnionLoweringTarget::KotlinJvm | UnionLoweringTarget::Java => {
            field_resolver.java_enum_emits_get_value(field) == Some(false)
        }
        UnionLoweringTarget::Dart | UnionLoweringTarget::KotlinAndroid | UnionLoweringTarget::Swift => {
            field_resolver.ir_enum_is_data_carrying(field) == Some(true)
        }
    }
}

/// The `skipped:` line refusing an assertion whose field is a payload-carrying union in `target`'s
/// binding, or `None` when the field is absent, empty, or lowers to a scalar the assertion can be
/// compared against.
pub(crate) fn payload_union_skip_line(
    indent: &str,
    comment_open: &str,
    field_resolver: &FieldResolver,
    field: Option<&str>,
    target: UnionLoweringTarget,
) -> Option<String> {
    let field = field.filter(|f| !f.is_empty())?;
    if !lacks_scalar_wire_accessor(field_resolver, field, target) {
        return None;
    }
    Some(format!(
        "{indent}{comment_open} skipped: {}",
        FieldSkip::PayloadUnionHasNoScalarWireAccessor.message(field)
    ))
}

#[cfg(test)]
mod tests {
    use super::{UnionLoweringTarget, lacks_scalar_wire_accessor, payload_union_skip_line};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::codegen::field_skip::{FieldSkip, SkipClass};
    use crate::e2e::field_access::FieldResolver;
    use std::collections::{HashMap, HashSet};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// `Envelope { unit: DataNodeKind, untagged: StageOutput, external: Payload }`.
    ///
    /// `DataNodeKind` is unit-only; `StageOutput` is `#[serde(untagged)]` with a data variant (so
    /// Java withholds `getValue()` too); `Payload` is an externally tagged data enum — the one
    /// shape where the JVM and Android predicates deliberately disagree. ~keep
    fn resolver() -> FieldResolver {
        let types = vec![TypeDef {
            name: "Envelope".to_string(),
            fields: vec![
                field("unit", TypeRef::Named("DataNodeKind".to_string())),
                field("untagged", TypeRef::Named("StageOutput".to_string())),
                field("external", TypeRef::Named("Payload".to_string())),
            ],
            ..TypeDef::default()
        }];
        let enums = vec![
            EnumDef {
                name: "DataNodeKind".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "KeyValue".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Sequence".to_string(),
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            },
            EnumDef {
                name: "StageOutput".to_string(),
                variants: vec![EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![field("_0", TypeRef::String)],
                    is_tuple: true,
                    ..EnumVariant::default()
                }],
                serde_untagged: true,
                ..EnumDef::default()
            },
            EnumDef {
                name: "Payload".to_string(),
                variants: vec![EnumVariant {
                    name: "Blob".to_string(),
                    fields: vec![field("_0", TypeRef::String)],
                    is_tuple: true,
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
        ];
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("Envelope".to_string()),
        )
        .with_java_wrapper_enum_names(
            enums
                .iter()
                .filter(|enum_def| !crate::backends::java::gen_bindings::emits_get_value(enum_def))
                .map(|enum_def| enum_def.name.clone())
                .collect(),
        )
    }

    /// The per-target predicate table checks every serde representation and target. ~keep
    #[test]
    fn the_predicate_answers_per_target() {
        let resolver = resolver();
        let expected = [
            ("unit", UnionLoweringTarget::Dart, false),
            ("unit", UnionLoweringTarget::KotlinAndroid, false),
            ("unit", UnionLoweringTarget::KotlinJvm, false),
            ("unit", UnionLoweringTarget::Java, false),
            ("unit", UnionLoweringTarget::Swift, false),
            ("untagged", UnionLoweringTarget::Dart, true),
            ("untagged", UnionLoweringTarget::KotlinAndroid, true),
            ("untagged", UnionLoweringTarget::KotlinJvm, true),
            ("untagged", UnionLoweringTarget::Java, true),
            ("untagged", UnionLoweringTarget::Swift, true),
            ("external", UnionLoweringTarget::Dart, true),
            ("external", UnionLoweringTarget::KotlinAndroid, true),
            ("external", UnionLoweringTarget::KotlinJvm, true),
            ("external", UnionLoweringTarget::Java, true),
            ("external", UnionLoweringTarget::Swift, true),
        ];
        for (field, target, want) in expected {
            assert_eq!(
                lacks_scalar_wire_accessor(&resolver, field, target),
                want,
                "field {field} on {target:?}"
            );
        }
    }

    /// A resolver with no IR wired must not start refusing assertions. ~keep
    #[test]
    fn an_unresolved_field_is_never_refused() {
        let bare = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        for target in [
            UnionLoweringTarget::Dart,
            UnionLoweringTarget::KotlinAndroid,
            UnionLoweringTarget::KotlinJvm,
            UnionLoweringTarget::Java,
            UnionLoweringTarget::Swift,
        ] {
            assert_eq!(
                payload_union_skip_line("    ", "//", &bare, Some("untagged"), target),
                None,
                "{target:?}"
            );
        }
    }

    #[test]
    fn a_missing_or_empty_field_yields_no_line() {
        let resolver = resolver();
        assert_eq!(
            payload_union_skip_line("    ", "//", &resolver, None, UnionLoweringTarget::Dart),
            None
        );
        assert_eq!(
            payload_union_skip_line("    ", "//", &resolver, Some(""), UnionLoweringTarget::Dart),
            None
        );
    }

    /// The exact rendered line, and the proof it is recognised as the variant it claims — a helper
    /// emitting an unregistered wording would still look right in a generated file, which is the
    /// bug the funnel exists to stop. ~keep
    #[test]
    fn the_rendered_line_is_exact_and_recognised() {
        let resolver = resolver();
        let line = payload_union_skip_line(
            "        ",
            "//",
            &resolver,
            Some("untagged"),
            UnionLoweringTarget::Swift,
        )
        .expect("a payload union must be refused");
        assert_eq!(
            line,
            "        // skipped: enum field 'untagged' is a payload-carrying union with no scalar \
             wire accessor in this binding"
        );
        assert_eq!(
            FieldSkip::extract_classified(&line),
            Some(("untagged", FieldSkip::PayloadUnionHasNoScalarWireAccessor)),
            "got: {line}"
        );
        assert_eq!(
            FieldSkip::PayloadUnionHasNoScalarWireAccessor.class(),
            SkipClass::GeneratorGap,
            "a consumer cannot close this from their own alef.toml, so it must never be fatal"
        );
    }
}
