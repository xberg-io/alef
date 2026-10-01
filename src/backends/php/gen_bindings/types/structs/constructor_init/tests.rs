//! Every test here is paired with its opposite control. The failure this module exists to
//! prevent is silent — a fabricated empty allow-list compiles, runs, and passes every smoke
//! test — so a suite that only exercised the refusal would stay green if generation started
//! failing for every input, and a suite that only exercised the happy path would stay green if
//! the refusal were deleted. The pairs below pin both directions. ~keep

use super::*;
use crate::core::ir::PrimitiveType;

const CORE_TYPE: &str = "sample_core::FetchPolicy";

fn names(values: &[&str]) -> AHashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn field(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        typed_default,
        ..Default::default()
    }
}

/// An `Option<T>` field, in the shape the extractor actually produces.
///
/// Verified against real IR (`alef extract`): `Option<Rule>` becomes `optional: true` with
/// `ty: Named("Rule")` — the `Option` is unwrapped, NOT kept as `TypeRef::Optional`. That variant
/// shows up only for the inner `Option` of an `Option<Option<T>>`. Modelling optionality with
/// `TypeRef::Optional` here would test a shape no crate produces. ~keep
fn optional_field(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        optional: true,
        ..field(name, ty, typed_default)
    }
}

/// A `Vec<SomeStruct>` whose element type is an untagged data enum: not representable as a PHP
/// constructor parameter, so the constructor omits it. This is the allow-list shape.
///
/// The element is modeled as an untagged data enum, not a plain struct, because
/// `php_field_can_be_constructor_param`'s `Vec` arm now accepts `Vec<Named>` for a plain
/// (non-opaque, non-enum) struct element too -- see `nested_struct_constructor_param_tests` in
/// `structs/tests.rs` for the positive coverage of that arm. Only an opaque type, an enum, or (as
/// used here) an untagged data enum keeps this collection axis genuinely non-representable.
fn rule_list() -> TypeRef {
    TypeRef::Vec(Box::new(TypeRef::Named("Rule".to_string())))
}

/// A single unrepresentable field — the scalar (non-collection) half of the shape axis. Modeled
/// as an untagged data enum (mapped to `serde_json::Value`, which has no ext-php-rs `FromZval`
/// impl) rather than a plain nested struct: `php_field_can_be_constructor_param`'s `Named` arm
/// (`structs.rs`) now accepts a bare nested `#[php_class]` struct as a by-reference constructor
/// param, so only an opaque type, an enum, or (as used here) an untagged data enum keeps this
/// axis genuinely non-representable.
fn nested_policy() -> TypeRef {
    TypeRef::Named("SsrfPolicy".to_string())
}

fn policy(has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "FetchPolicy".to_string(),
        rust_path: CORE_TYPE.to_string(),
        has_default,
        has_serde: true,
        fields,
        ..Default::default()
    }
}

fn build(typ: &TypeDef) -> anyhow::Result<ConstructorInit> {
    build_with_retained_cfg(typ, &[])
}

fn build_with_retained_cfg(typ: &TypeDef, never_skip_cfg_field_names: &[String]) -> anyhow::Result<ConstructorInit> {
    gen_constructor_field_inits(
        typ,
        &names(&["Mode"]),
        &names(&["Client"]),
        &names(&["SsrfPolicy", "Rule"]),
        never_skip_cfg_field_names,
    )
}

#[test]
fn should_omit_a_cfg_gated_field_that_the_binding_struct_strips() {
    let mut gated = field("feature_only", TypeRef::String, Some(DefaultValue::Empty));
    gated.cfg = Some("feature = \"enterprise\"".to_string());
    let typ = policy(
        true,
        vec![field("name", TypeRef::String, Some(DefaultValue::Empty)), gated],
    );

    let init = build(&typ).expect("a stripped cfg field needs no constructor initializer");

    assert_eq!(init.field_inits, "name");
}

#[test]
fn should_default_a_cfg_gated_field_explicitly_retained_by_the_binding() {
    let mut gated = field("feature_only", TypeRef::String, Some(DefaultValue::Empty));
    gated.cfg = Some("feature = \"enterprise\"".to_string());
    let typ = policy(
        true,
        vec![field("name", TypeRef::String, Some(DefaultValue::Empty)), gated],
    );

    let init = build_with_retained_cfg(&typ, &["feature_only".to_string()])
        .expect("a retained cfg field is initialized without becoming a PHP parameter");

    assert_eq!(init.field_inits, "name, feature_only: Default::default()");
}

#[test]
fn should_apply_the_constructor_cfg_policy_to_the_binding_struct_field_set() {
    let mut gated = field("feature_only", TypeRef::String, Some(DefaultValue::Empty));
    gated.cfg = Some("feature = \"enterprise\"".to_string());
    let typ = policy(
        true,
        vec![field("name", TypeRef::String, Some(DefaultValue::Empty)), gated],
    );

    let stripped = php_binding_type(&typ, &[]);
    let retained = php_binding_type(&typ, &["feature_only".to_string()]);

    assert_eq!(
        stripped
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["name"]
    );
    assert_eq!(
        retained
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "feature_only"]
    );
}

#[test]
fn bare_serde_enum_default_is_used_without_defaulting_required_sibling() {
    let mut mode = field(
        "mode",
        TypeRef::Named("Mode".to_string()),
        Some(DefaultValue::EnumVariant("Preserve".to_string())),
    );
    mode.default = Some("/* serde(default) */".to_string());
    let typ = policy(false, vec![field("llm", TypeRef::String, None), mode]);

    let init = build(&typ).expect("representable fields form a constructor");

    assert!(init.field_inits.contains("llm"), "{}", init.field_inits);
    assert!(
        init.field_inits
            .contains("mode: mode.unwrap_or_else(crate::serde_defaults::fetch_policy_mode)"),
        "{}",
        init.field_inits
    );
    assert!(!init.field_inits.contains("llm.unwrap_or"), "{}", init.field_inits);
}

// ---------------------------------------------------------------------------
// Axis 1: derived `Default`. Every field the derive covers is `DefaultValue::Empty`, which the
// IR defines as "the default IS this type's own zero" — so the target-language zero is exact,
// not a guess, and must still be emitted.
// ---------------------------------------------------------------------------

#[test]
fn should_emit_type_zero_when_derived_default_marks_collection_field_empty() {
    let typ = policy(true, vec![field("allow_list", rule_list(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("a derived Default is knowable and must not fail generation");

    assert_eq!(init.field_inits, "allow_list: Default::default()");
    assert_eq!(init.prelude, "", "an exact type zero needs no delegating-Default local");
}

#[test]
fn should_emit_type_zero_when_derived_default_marks_scalar_field_empty() {
    let typ = policy(true, vec![field("ssrf", nested_policy(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("a derived Default is knowable and must not fail generation");

    assert_eq!(init.field_inits, "ssrf: Default::default()");
}

/// The `Empty` reading is a property of the recorded default, not of the owning type: a field
/// carrying a bare `#[serde(default)]` on a type with no `Default` impl is still exactly the
/// type zero. Without this arm the refusal below would swallow a case it has no business
/// refusing.
#[test]
fn should_emit_type_zero_for_empty_default_even_without_a_type_level_default_impl() {
    let typ = policy(false, vec![field("allow_list", rule_list(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("an explicitly-empty default is knowable regardless of the owning type");

    assert_eq!(init.field_inits, "allow_list: Default::default()");
}

// ---------------------------------------------------------------------------
// Axis 2: manual `impl Default`. The value is not a type zero, and for an unreadable body alef
// cannot spell it at all — but the impl is real, compiled Rust, so the binding reads it back
// off its own delegating `Default` rather than guessing.
// ---------------------------------------------------------------------------

#[test]
fn should_read_collection_field_off_core_default_when_manual_impl_is_unresolved() {
    let typ = policy(
        true,
        vec![field(
            "allow_list",
            rule_list(),
            Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "allow_list: __alef_core_defaults.allow_list");
    assert_eq!(
        init.prelude, "let __alef_core_defaults = <Self as ::core::default::Default>::default();\n",
        "the recovery must bind the delegating Default exactly once, before `Self {{ .. }}`"
    );
    assert!(
        !init.field_inits.contains("Default::default()"),
        "an unresolved manual Default must never fall back to the type zero, got: {}",
        init.field_inits
    );
}

#[test]
fn should_read_collection_field_off_core_default_when_manual_impl_holds_a_list_literal() {
    let typ = policy(
        true,
        vec![field(
            "allow_list",
            rule_list(),
            Some(DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral(
                "internal".to_string(),
            )])),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "allow_list: __alef_core_defaults.allow_list");
}

#[test]
fn should_read_scalar_field_off_core_default_when_manual_impl_names_an_enum_variant() {
    let typ = policy(
        true,
        vec![field(
            "ssrf",
            nested_policy(),
            Some(DefaultValue::EnumVariant("Strict".to_string())),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "ssrf: __alef_core_defaults.ssrf");
}

/// Two recovered fields must share one local. Binding it per field would move the same value
/// twice; binding it never would leave the initialisers referring to nothing.
#[test]
fn should_bind_the_core_default_local_once_for_several_recovered_fields() {
    let typ = policy(
        true,
        vec![
            field(
                "allow_list",
                rule_list(),
                Some(DefaultValue::Unresolved("x".to_string())),
            ),
            field(
                "deny_list",
                rule_list(),
                Some(DefaultValue::Unresolved("x".to_string())),
            ),
        ],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(
        init.field_inits,
        "allow_list: __alef_core_defaults.allow_list, deny_list: __alef_core_defaults.deny_list"
    );
    assert_eq!(init.prelude.matches("let __alef_core_defaults").count(), 1);
}

// ---------------------------------------------------------------------------
// Axis 3: no `Default` at all. There is no real value to recover, so generation fails rather
// than inventing one.
//
// Security direction, stated explicitly because the two lists fail in OPPOSITE directions and
// both directions are wrong:
//   - an allow-list fabricated empty admits nothing legitimately, so callers relax it or the
//     check is skipped — it fails OPEN in practice, and in the common "empty means unrestricted"
//     reading it fails open outright;
//   - a deny-list fabricated empty blocks nothing — it fails OPEN unambiguously.
// Neither is observable at runtime: the PHP caller cannot distinguish a value the Rust author
// chose from one alef invented. So the refusal is symmetric and does not try to guess a "safe"
// direction per field. ~keep
// ---------------------------------------------------------------------------

#[test]
fn should_fail_generation_for_an_allow_list_with_no_recoverable_default() {
    let typ = policy(false, vec![field("allow_list", rule_list(), None)]);

    let error = build(&typ).expect_err("an unknowable allow-list default must fail generation");
    let message = format!("{error}");

    assert!(
        message.contains("allow_list"),
        "error must name the field, got: {message}"
    );
    assert!(message.contains(CORE_TYPE), "error must name the type, got: {message}");
    // Not merely `contains("Default")` — the message mentions `Default::default()` while
    // explaining what it refuses to emit, so that would pass with the remedy list deleted. ~keep
    assert!(
        message.contains("#[derive(Default)]"),
        "error must name the remedy, got: {message}"
    );
}

#[test]
fn should_fail_generation_for_a_deny_list_with_no_recoverable_default() {
    let typ = policy(false, vec![field("deny_list", rule_list(), None)]);

    let error = build(&typ).expect_err("an unknowable deny-list default must fail generation");

    assert!(format!("{error}").contains("deny_list"));
}

#[test]
fn should_fail_generation_for_an_unknowable_scalar_field() {
    let typ = policy(false, vec![field("ssrf", nested_policy(), None)]);

    let error = build(&typ).expect_err("an unknowable nested-struct default must fail generation");

    assert!(format!("{error}").contains("ssrf"));
}

/// The refusal must be scoped to fields the constructor omits. A representable field with no
/// recorded default is handed to the caller as a parameter, so nothing is invented and nothing
/// should fail — this is the control that catches a refusal widened past its purpose.
#[test]
fn should_not_fail_when_the_field_with_no_default_is_a_real_constructor_parameter() {
    let typ = policy(
        false,
        vec![field("max_depth", TypeRef::Primitive(PrimitiveType::U32), None)],
    );

    let init = build(&typ).expect("a representable field takes its value from the caller");

    assert_eq!(init.field_inits, "max_depth: maxDepth");
}

/// Ordering control: one unknowable field must fail the whole constructor even when a knowable
/// sibling would otherwise render. A per-field fallback that "skipped" the bad field would pass
/// every test above and still ship the fabrication.
#[test]
fn should_fail_when_any_omitted_field_is_unknowable_even_beside_a_knowable_one() {
    let typ = policy(
        false,
        vec![
            field("allow_list", rule_list(), Some(DefaultValue::Empty)),
            field("deny_list", rule_list(), None),
        ],
    );

    let error = build(&typ).expect_err("one unknowable field must fail the constructor");

    assert!(format!("{error}").contains("deny_list"));
}

// ---------------------------------------------------------------------------
// Optionality is not an exemption.
//
// `None` for an omitted `Option` is the arm most likely to be right by accident: it type-checks,
// it looks like absence rather than invention, and it is what an empty `Option` looks like anyway.
// The generated PHPStan stub decides it. For every omitted field the stub says only:
//
//   @readonly Not settable via the constructor — this field's type has no ext-php-rs
//   #[php(prop)]/constructor-param support, so it can only be read via this getter.
//
// It promises the caller nothing about the VALUE. So absence is not defined at this boundary, and
// `None` is a claim nothing in the source crate made — the same fabrication as an empty
// allow-list. What lowers is absence the IR actually RECORDS, or a real `Default` to read. ~keep
// ---------------------------------------------------------------------------

/// Positive: absence the IR records. `DefaultValue::None` is the extractor stating the default is
/// null, which is an authored fact, so it lowers.
#[test]
fn should_lower_an_omitted_option_field_whose_recorded_default_is_none() {
    let typ = policy(
        false,
        vec![optional_field("ssrf", nested_policy(), Some(DefaultValue::None))],
    );

    let init = build(&typ).expect("a recorded null default is authored, not invented");

    assert_eq!(init.field_inits, "ssrf: Default::default()");
}

/// Positive, and the reason a blanket `None` would have been wrong even when it looked harmless:
/// an optional field on a type WITH a `Default` can default to `Some(..)`. The recovery reads
/// whatever the core impl says instead of assuming the empty `Option`.
#[test]
fn should_read_an_omitted_option_field_off_a_core_default_that_may_hold_some() {
    let typ = policy(
        true,
        vec![optional_field(
            "ssrf",
            nested_policy(),
            Some(DefaultValue::EnumVariant("Strict".to_string())),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "ssrf: __alef_core_defaults.ssrf");
    assert!(
        !init.field_inits.contains("None"),
        "an optional field must not be assumed empty when the core Default may hold Some(..), got: {}",
        init.field_inits
    );
}

/// Negative: no recorded default, no owning `Default`. Optionality alone must not license a value.
#[test]
fn should_fail_for_an_omitted_option_field_with_no_default_anywhere() {
    let typ = policy(false, vec![optional_field("ssrf", nested_policy(), None)]);

    let error = build(&typ).expect_err("optionality is not an exemption from the no-fabrication rule");

    assert!(format!("{error}").contains("ssrf"));
}

// ---------------------------------------------------------------------------
// Hostile identifiers.
//
// A consumer crate's field names are adversarial input. The `let` holding the core defaults must
// not be able to shadow a parameter or local, because a shadowed binding still type-checks
// whenever the types happen to agree — so the failure would be silent. ~keep
// ---------------------------------------------------------------------------

/// Control: an ordinary type keeps the plain base name, so the derivation is not gratuitously
/// renaming. Without this, a bug that always appended `_` would pass the hostile test below.
#[test]
fn should_use_the_plain_base_local_when_nothing_collides() {
    let typ = policy(true, vec![field("allow_list", rule_list(), Some(DefaultValue::Empty))]);

    assert_eq!(core_defaults_local(&typ), "__alef_core_defaults");
}

/// A field named exactly like the generated local must push the local aside, not the other way
/// round. `to_php_name("__alef_core_defaults")` is `alefCoreDefaults` (verified against real
/// generated output), so the parameter itself cannot collide — but the raw field name is also an
/// identifier this constructor emits, and nothing about `to_php_name` is guaranteed to keep
/// underscores out forever.
#[test]
fn should_pick_a_local_no_hostile_field_name_can_shadow() {
    let typ = policy(
        true,
        vec![
            field(
                "__alef_core_defaults",
                TypeRef::Primitive(PrimitiveType::U32),
                Some(DefaultValue::IntLiteral(7)),
            ),
            field(
                "allow_list",
                rule_list(),
                Some(DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral(
                    "internal".to_string(),
                )])),
            ),
        ],
    );

    let local = core_defaults_local(&typ);
    assert_ne!(
        local, "__alef_core_defaults",
        "the local must not reuse a real field name"
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert!(
        init.prelude.contains(&format!("let {local} =")),
        "the prelude must bind the derived local, got: {}",
        init.prelude
    );
    assert_eq!(
        init.field_inits,
        format!("__alef_core_defaults: alefCoreDefaults, allow_list: {local}.allow_list"),
        "the hostile field keeps its own parameter; only the core-defaults local moves"
    );
}

/// The derivation must survive the obvious second-order attack: occupying the base name AND the
/// name the first fallback would pick. One `_` is not a strategy.
#[test]
fn should_keep_lengthening_past_an_occupied_first_fallback() {
    let typ = policy(
        true,
        vec![
            field("__alef_core_defaults", TypeRef::String, Some(DefaultValue::Empty)),
            field("__alef_core_defaults_", TypeRef::String, Some(DefaultValue::Empty)),
            field("allow_list", rule_list(), Some(DefaultValue::Empty)),
        ],
    );

    assert_eq!(core_defaults_local(&typ), "__alef_core_defaults__");
}

// ---------------------------------------------------------------------------
// Untouched paths.
// ---------------------------------------------------------------------------

#[test]
fn should_keep_binding_excluded_fields_out_of_the_initialiser_entirely() {
    let mut excluded = field("allow_list", rule_list(), None);
    excluded.binding_excluded = true;
    let typ = policy(
        false,
        vec![
            excluded,
            field("max_depth", TypeRef::Primitive(PrimitiveType::U32), None),
        ],
    );

    let init = build(&typ).expect("an excluded field is not part of the binding struct");

    assert_eq!(init.field_inits, "max_depth: maxDepth");
}

#[test]
fn should_unwrap_the_bytes_newtype_for_a_representable_bytes_field() {
    let typ = policy(true, vec![field("payload", TypeRef::Bytes, Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("Bytes is a representable constructor parameter");

    assert_eq!(init.field_inits, "payload: payload.0");
}
