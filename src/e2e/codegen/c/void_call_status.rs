//! Whether a `returns_void`-configured C e2e call is genuinely void, or a fallible export
//! whose C ABI reports failure through an `i32` status code.
//!
//! `[crates.e2e.calls.*].returns_void` is an author-declared statement that there is no result
//! *value* to verify -- it says nothing about whether the underlying Rust call can fail.
//! `backends::ffi::orchestration::gen_free_function`/`gen_method_wrapper` give a `Result<(), E>`
//! export an `i32` C signature (`has_error && is_void_return` => `ret_type = "i32"`, `return -1;`
//! on the error path, `Ok(()) => 0` on success -- see `error_match_void.jinja`), so a `returns_void`
//! call whose Rust signature is fallible is not void at the ABI: it is a discarded status code.
//! `render_snippet_body`'s void branch (`c/test_function.rs`) used to call it and drop the return
//! value outright, which skipped `expects_error`/`not_error` checking entirely for every such
//! call -- see the module's history for the full defect writeup. ~keep

use super::CallIr;
use crate::e2e::config::CallConfig;

/// True when the core IR declares an `error_type` for this call's Rust signature -- i.e. the
/// call is `Result<_, E>`-returning and its C export therefore carries a status code even though
/// the fixture is configured `returns_void`.
///
/// Mirrors `TargetParams::resolve`'s lookup key (`CallConfig::core_lookup_name` for `"c"`): the
/// two must agree on which IR entry a call refers to, since both read the same `call`/`ir` pair
/// for the same fixture. An absent or unresolvable IR answers `false` -- nothing was learned, so
/// nothing is claimed, matching `TargetParams::IrAbsent`/`Unresolvable`'s "no claim" precedent
/// rather than asserting a status check on a signature that was never actually consulted. ~keep
pub(super) fn is_fallible(call: &CallConfig, ir: CallIr<'_>) -> bool {
    call.core_lookup_name("c")
        .as_deref()
        .and_then(|name| ir.signature(name))
        .and_then(|signature| signature.error_type)
        .is_some()
}

/// Render a `returns_void` snippet's call line: a captured, asserted `int32_t` status for a
/// fallible export, or a bare discarded call for a genuinely void one.
///
/// `expects_error` decides which polarity the assertion takes -- `!= 0` for a fixture declaring
/// `error`, `== 0` for everything else (including a bare `not_error` and no declared assertion at
/// all) -- the same convention `render_test_function_impl`'s `returns_status_code()` branch uses
/// for the trait-bridge registry status shape. ~keep
pub(super) fn render_call_line(
    function_name: &str,
    args: &str,
    result_var: &str,
    expects_error: bool,
    is_fallible: bool,
) -> String {
    if is_fallible {
        crate::e2e::template_env::render(
            "c/snippet_status_call.jinja",
            crate::alef_context! {
                result_var => result_var,
                function_name => function_name,
                args => args,
                expects_error => expects_error,
            },
        )
    } else {
        crate::e2e::template_env::render(
            "c/snippet_void_call.jinja",
            crate::alef_context! { function_name => function_name, args => args },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FunctionDef, TypeRef};

    fn call_named(function: &str) -> CallConfig {
        CallConfig {
            function: function.to_string(),
            ..CallConfig::default()
        }
    }

    /// The regression this module exists for: a `Result<(), E>` free function resolves as
    /// fallible even though its fixture is configured `returns_void`. ~keep
    #[test]
    fn a_result_unit_error_function_is_fallible() {
        let functions = vec![FunctionDef {
            name: "reset_cache".into(),
            return_type: TypeRef::Unit,
            error_type: Some("SampleError".into()),
            ..FunctionDef::default()
        }];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        assert!(is_fallible(&call_named("reset_cache"), ir));
    }

    /// A genuinely infallible `fn() -> ()` stays void -- no status to capture or assert.
    #[test]
    fn a_plain_unit_function_is_not_fallible() {
        let functions = vec![FunctionDef {
            name: "log_ping".into(),
            return_type: TypeRef::Unit,
            error_type: None,
            ..FunctionDef::default()
        }];
        let ir = CallIr {
            functions: &functions,
            type_defs: &[],
        };
        assert!(!is_fallible(&call_named("log_ping"), ir));
    }

    /// No IR in scope licenses no claim either way, matching `TargetParams::IrAbsent`.
    #[test]
    fn an_absent_ir_is_not_fallible() {
        assert!(!is_fallible(&call_named("reset_cache"), CallIr::default()));
    }

    #[test]
    fn fallible_call_line_captures_and_asserts_the_status() {
        let rendered = render_call_line("sample_reset_cache", "", "result", false, true);
        assert_eq!(
            rendered,
            "int32_t result = sample_reset_cache();\nassert(result == 0 && \"expected call to succeed\");\n"
        );
    }

    #[test]
    fn fallible_call_line_flips_polarity_when_the_fixture_expects_an_error() {
        let rendered = render_call_line("sample_reset_cache", "", "result", true, true);
        assert_eq!(
            rendered,
            "int32_t result = sample_reset_cache();\nassert(result != 0 && \"expected call to fail\");\n"
        );
    }

    /// A genuinely void call keeps the pre-existing bare-discard rendering untouched.
    #[test]
    fn non_fallible_call_line_discards_the_call_as_before() {
        let rendered = render_call_line("sample_log_ping", "", "result", false, false);
        assert_eq!(rendered, "sample_log_ping();\n");
    }

    /// End-to-end regression for issue #121: before this module's fix,
    /// `render_snippet_body`'s `returns_void` branch called `super::render_call_line`'s
    /// predecessor (a bare `snippet_void_call.jinja` render) unconditionally, discarding the
    /// `i32` status a fallible export returns and never emitting an assertion at all -- a
    /// `not_error` fixture on a `Result<(), E>` call rendered a snippet that checked nothing.
    /// This must go red against the pre-fix renderer and green once the status is captured
    /// and asserted. ~keep
    #[test]
    fn returns_void_fallible_call_captures_and_asserts_the_discarded_status() {
        use crate::core::config::ResolvedCrateConfig;
        use crate::core::config::e2e::E2eConfig;
        use crate::e2e::fixture::Fixture;

        let fixture = Fixture {
            id: "reset_cache".into(),
            description: "Reset the on-disk cache".into(),
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "reset_cache".into();
        e2e.call.returns_void = true;
        e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                header: Some("sample_ffi.h".into()),
                function: Some("sample_reset_cache".into()),
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "reset_cache".into(),
            return_type: TypeRef::Unit,
            error_type: Some("SampleError".into()),
            ..FunctionDef::default()
        }];
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = super::super::render_c_snippet(&fixture, &e2e, &config, &[], &functions)
            .expect("a fallible returns_void call must still render");

        assert!(
            rendered.contains("int32_t") && rendered.contains("sample_reset_cache()"),
            "the discarded FFI status must be captured into a typed variable:\n{rendered}"
        );
        assert!(
            rendered.contains("assert(") && rendered.contains("== 0"),
            "a not_error fixture on a fallible void call must assert the status reports \
             success, not render an unchecked call:\n{rendered}"
        );

        super::super::snippet_regressions::compile_snippet(
            &rendered,
            "sample_ffi.h",
            concat!("#include <stdint.h>\n", "int32_t sample_reset_cache(void);\n"),
        );
    }
}
