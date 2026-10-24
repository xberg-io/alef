//! Assertions that step through a display-text union into one of its variants, such as
//! `choices[0].message.content.parts[0].text`.
//!
//! The getter of a display-text union returns the plain text of the value, so it cannot show the
//! parts of a list variant. The bridge type has a second, typed accessor for the same field. It
//! returns the union as a Swift value. A `#[serde(untagged)]` union encodes as the payload of its
//! variant, so the path below the union reads from the encoded value with the wire steps of the
//! variant, the way a JSON-bridged leaf is read.

use std::fmt::Write as FmtWrite;

use crate::backends::swift::gen_rust_crate::type_bridge::text_projected_value_names;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::{FieldResolver, JsonNavStep, WireAccessStep};
use crate::e2e::fixture::Assertion;

use super::accessors::materialise_vec_temporaries;
use super::json_bridged_navigation::{append_nav_steps, build_assertion_lines, json_nav_local_name};
use super::leaf_shape::mixed_map_then_vec_traversal_skip;

/// Render `assertion` over the typed value of a display-text union. Return `false`, and write
/// nothing, when the field does not cross an untagged display-text union read through a bridge
/// getter. A first-class struct property is not read this way: it holds the text variant only.
pub(super) fn try_render(out: &mut String, assertion: &Assertion, result_var: &str, resolver: &FieldResolver) -> bool {
    let Some(field) = assertion.field.as_deref().filter(|f| !f.is_empty()) else {
        return false;
    };
    let Some((prefix, owner, variant, suffix)) = resolver.ir_tagged_union_split(resolver.resolve(field)) else {
        return false;
    };
    if prefix.is_empty()
        || !resolver.is_text_projected_enum(&owner)
        || !resolver
            .enum_definition(&owner)
            .is_some_and(|definition| definition.serde_untagged)
    {
        return false;
    }
    let field_name = prefix.rsplit('.').next().unwrap_or(&prefix);
    if field_name.contains(['[', ']']) {
        return false;
    }
    let union_expr = resolver.accessor(&prefix, "swift", result_var);
    let Some((owner_expr, getter)) = union_expr.rsplit_once('.') else {
        return false;
    };
    if !getter.ends_with("()") {
        return false;
    }
    let Some(steps) = resolver.enum_payload_wire_steps(&owner, &variant, &suffix) else {
        return false;
    };
    let steps: Vec<JsonNavStep> = steps
        .into_iter()
        .map(|step| match step {
            WireAccessStep::Field(key) => JsonNavStep::Key(key),
            WireAccessStep::Index(index) => JsonNavStep::Index(index),
        })
        .collect();

    let local = json_nav_local_name(field, &assertion.assertion_type);
    let value = format!("_union_value{}", local.trim_start_matches("_json_nav"));
    let Some((setup, owner_expr, _)) = materialise_vec_temporaries(owner_expr, &local) else {
        out.push_str(&mixed_map_then_vec_traversal_skip(field));
        return true;
    };
    let Some(body) = build_assertion_lines(assertion, &local) else {
        let _ = writeln!(
            out,
            "        // skipped: {}; assertion type '{}' has no Swift renderer over decoded JSON yet",
            FieldSkip::NavigatedJsonBridgedAssertionTypeNotSupportedInSwift.message(field),
            assertion.assertion_type
        );
        return true;
    };
    for line in &setup {
        let _ = writeln!(out, "        {line}");
    }
    let accessor = text_projected_value_names(field_name).accessor;
    let encoded = format!(
        "(try? JSONSerialization.jsonObject(with: JSONEncoder().encode({value}), options: [.fragmentsAllowed]))"
    );
    let _ = writeln!(out, "        let {value} = try {owner_expr}.{accessor}()");
    let _ = writeln!(out, "        let {local}: Any? = {}", append_nav_steps(encoded, &steps));
    out.push_str(&body);
    true
}
