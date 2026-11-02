//! SECURITY. `needs_omitempty_pointer` decides whether a Go struct field is emitted as a plain
//! value (whose Go zero is marshaled onto the wire as though the caller chose it) or as
//! pointer+`omitempty` (whose unset value drops the key, letting Rust's own serde default supply
//! it). A field carrying `#[serde(default = "path")]` reaches the IR as
//! `DefaultValue::FunctionCall` — alef records the function's *name*, never its return value — so
//! the Go zero is a claim about a value alef does not have, and the predicate must defer.
//!
//! It did not. `FunctionCall`/`PublicFunctionCall` were grouped with `Empty` in the `false` arm.
//! `Vec`/`Map` fields were saved by an unrelated rule (`go_struct_field_json_tag` tags every
//! collection `,omitempty`), but a scalar got neither the pointer nor the tag and shipped `""`,
//! `0` or `false` in place of what `path()` returns. Once extraction stopped letting a container's
//! `#[derive(Default)]` overwrite `FunctionCall` with `Empty`, the same `false` also routed the
//! `New()` constructor to `config_gen::default_value_for_field(field, "go")`, whose `FunctionCall`
//! arm answers `"nil"` — not assignable to a bare `string`, so the emitted Go stopped compiling.
//!
//! Lives in its own module because `tests.rs` next door is already over the repo's 1,000-line
//! file-size cap and must not grow. ~keep

use super::config_options::gen_config_options;
use super::structs::gen_struct_type;
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn named_default_field(name: &str, ty: TypeRef, path: &str, typed_default: DefaultValue) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        default: Some(format!("serde(default = \"{path}\")")),
        typed_default: Some(typed_default),
        ..Default::default()
    }
}

/// A `#[derive(Default)]` container: `has_default` is true and `serde_container_default` is false,
/// which is exactly the shape whose fields used to have their `FunctionCall` overwritten with
/// `Empty` during extraction. ~keep
fn derived_default_config(fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "FetchConfig".to_string(),
        fields,
        has_default: true,
        has_serde: true,
        ..Default::default()
    }
}

fn emit_struct(typ: &TypeDef) -> String {
    gen_struct_type(
        typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        (&std::collections::HashSet::new(), &[], &[]),
        &[],
    )
}

fn emit_config_options(typ: &TypeDef) -> String {
    gen_config_options(
        typ,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &[],
    )
}

#[test]
fn a_named_serde_default_on_a_string_becomes_pointer_omitempty_instead_of_the_go_zero() {
    let typ = derived_default_config(vec![named_default_field(
        "user_agent",
        TypeRef::String,
        "mylib::default_user_agent",
        DefaultValue::FunctionCall("mylib::default_user_agent".to_string()),
    )]);

    let out = emit_struct(&typ);

    assert!(
        out.contains("UserAgent *string `json:\"user_agent,omitempty\"`"),
        "a named serde default must defer via pointer+omitempty so the key is dropped and \
         `mylib::default_user_agent()` supplies the value; got:\n{out}"
    );
    assert!(
        !out.contains("UserAgent string `json:\"user_agent\"`"),
        "a plain `string` field marshals `\"\"` onto the wire as though the caller chose it, and \
         the real default never runs; got:\n{out}"
    );
}

#[test]
fn a_resolved_named_serde_default_on_a_number_also_defers() {
    let typ = derived_default_config(vec![named_default_field(
        "redirect_limit",
        TypeRef::Primitive(PrimitiveType::U32),
        "mylib::default_redirect_limit",
        DefaultValue::PublicFunctionCall("mylib::default_redirect_limit".to_string()),
    )]);

    let out = emit_struct(&typ);

    assert!(
        out.contains("RedirectLimit *uint32 `json:\"redirect_limit,omitempty\"`"),
        "`PublicFunctionCall` names a callable path but still carries no *value*, so it defers \
         exactly like `FunctionCall`; got:\n{out}"
    );
}

/// The constructor half of the same decision. Before the fix this field reached
/// `config_gen::default_value_for_field(field, "go")`, whose `FunctionCall` arm answers `"nil"` —
/// which is not assignable to a bare `string`. Pointer+omitempty makes `nil` the correct literal
/// for the same field, so the struct declaration and the constructor agree. ~keep
#[test]
fn the_new_constructor_seeds_nil_for_a_named_serde_default_matching_the_pointer_field() {
    let typ = derived_default_config(vec![named_default_field(
        "user_agent",
        TypeRef::String,
        "mylib::default_user_agent",
        DefaultValue::FunctionCall("mylib::default_user_agent".to_string()),
    )]);

    let out = emit_config_options(&typ);

    assert!(
        out.contains("UserAgent: nil"),
        "expected the New() constructor to seed `nil` so the key is omitted and Rust's serde \
         default applies; got:\n{out}"
    );
    assert!(
        !out.contains("UserAgent: \"\""),
        "seeding the Go zero ships `\"\"` in place of what `mylib::default_user_agent()` \
         returns; got:\n{out}"
    );
}

/// Discrimination control. `Empty` genuinely asserts "the Rust default IS this type's zero", so a
/// bare `#[serde(default)]` on the same container must stay a plain, non-pointer field. Without
/// this, a change that pointer-ized every default would satisfy the assertions above while
/// regressing every already-correct field. The two fields differ only in `typed_default`. ~keep
#[test]
fn a_bare_serde_default_still_stays_the_plain_go_zero() {
    let field = FieldDef {
        name: "user_agent".to_string(),
        ty: TypeRef::String,
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };
    let typ = derived_default_config(vec![field]);

    let out = emit_struct(&typ);

    assert!(
        out.contains("UserAgent string `json:\"user_agent\"`"),
        "an `Empty` default must stay a plain, non-pointer, non-omitempty field; got:\n{out}"
    );
    assert!(
        !out.contains("*string"),
        "an `Empty` default must not be pointer-ized; got:\n{out}"
    );
}
