//! The C# backend's `error.<field>` equals checks.
//!
//! The base exception of an error type carries a get-only property for every whitelisted
//! introspection method, and each variant exception extends it. The check casts the caught
//! exception to the base class, so it holds whichever variant the call threw. ~keep

use crate::core::ir::ErrorDef;
use crate::e2e::fixture::Fixture;

/// One xUnit statement per `equals` assertion on a reachable `error.<method>` field, read off the
/// exception bound to `thrown`.
pub(super) fn render_checks(fixture: &Fixture, reachable: &[ErrorDef]) -> String {
    crate::e2e::codegen::error_field_checks::render(
        fixture,
        reachable,
        "        // ",
        "csharp",
        |error, method, expected| {
            let class = format!("{}Exception", error.name);
            let property = heck::ToUpperCamelCase::to_upper_camel_case(method);
            format!("        Assert.Equal({expected}, (int)Assert.IsAssignableFrom<{class}>(thrown).{property});\n")
        },
    )
}
