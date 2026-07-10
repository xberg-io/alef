//! Generate Java visitor support: interface, result sealed interface,
//! VisitorBridge (upcall stubs), and IR-driven convert-with-visitor method fragments.
//!
//! # Panama FFM upcall strategy
//!
//! Java cannot expose method references as raw C function pointers. The generated
//! code uses Java 21+ Foreign Function & Memory API (Panama) upcall stubs:
//!
//! - the configured context type is emitted as a normal IR-derived Java record.
//! - the configured result type is emitted as a `sealed interface` from `EnumDef` metadata.
//! - the configured trait is emitted as an `interface` with default no-op methods.
//! - `VisitorBridge`: a package-private class that allocates one `MemorySegment`
//!   upcall stub per callback inside a `Arena.ofConfined()` scope, then writes
//!   all stubs into a flat `MemorySegment` matching the generated C callback table.
//! - convert-with-visitor methods drive the full lifecycle with caller-provided
//!   IR/config-derived method shape and FFI body fragments.

mod callbacks;
mod files;
mod helpers;

pub use callbacks::{CallbackSpec, ExtraParam};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use heck::ToSnakeCase;

/// Returns `(filename, content)` pairs for visitor-related Java files when metadata is complete.
///
/// Callers push these into the `files` vector in `generate_bindings`.
pub fn gen_visitor_files(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    class_name: &str,
) -> Option<Vec<(String, String)>> {
    let visitor = files::resolve_visitor_generation(api, config, class_name)?;
    Some(files::gen_visitor_files(package, &visitor))
}

/// Returns true when Java has enough metadata to emit visitor support safely.
pub fn has_visitor_generation_metadata(api: &ApiSurface, config: &ResolvedCrateConfig) -> bool {
    files::resolve_visitor_generation(api, config, "").is_some()
}

/// Generate NativeLib method handle declarations for visitor FFI functions.
///
/// These lines are injected into the `NativeLib` class body after the normal handles.
pub fn gen_native_lib_visitor_handles(prefix: &str, options_fields: &[String]) -> String {
    let pu = prefix.to_uppercase();
    let options_set_handles = options_fields
        .iter()
        .map(|field| {
            let field_snake = field.to_snake_case();
            crate::backends::java::template_env::render(
                "native_lib_options_set_visitor_handle.jinja",
                minijinja::context! {
                    handle_name => format!("{}_OPTIONS_SET_{}", pu, field_snake.to_uppercase()),
                    ffi_name => format!("{prefix}_options_set_{field_snake}"),
                },
            )
        })
        .collect::<Vec<_>>()
        .join("");
    crate::backends::java::template_env::render(
        "native_lib_visitor_handles.jinja",
        minijinja::context! {
            prefix => prefix,
            prefix_upper => pu,
            options_set_handles => options_set_handles,
        },
    )
}

/// Explicit inputs for rendering a convert-with-visitor method.
///
/// Returns the method source as a string (without surrounding class braces).
pub struct ConvertWithVisitorMethod<'a> {
    pub return_type: &'a str,
    pub method_name: &'a str,
    pub params: &'a str,
    pub exception_class: &'a str,
    pub prefix: &'a str,
    pub visitor_arg: &'a str,
    pub marshal_body: &'a str,
    pub invoke_body: &'a str,
}

/// Generate a convert-with-visitor method from IR/config-derived inputs.
///
/// Returns the method source as a string (without surrounding class braces).
pub fn gen_convert_with_visitor_method(method: &ConvertWithVisitorMethod<'_>) -> String {
    assert!(
        !method.return_type.is_empty(),
        "convert-with-visitor return type must be derived before rendering"
    );
    assert!(
        !method.method_name.is_empty(),
        "convert-with-visitor method name must be derived before rendering"
    );
    assert!(
        !method.params.is_empty(),
        "convert-with-visitor parameters must be derived before rendering"
    );
    assert!(
        !method.exception_class.is_empty(),
        "convert-with-visitor exception class must be derived before rendering"
    );
    assert!(
        !method.prefix.is_empty(),
        "convert-with-visitor native prefix must be derived before rendering"
    );
    assert!(
        !method.visitor_arg.is_empty(),
        "convert-with-visitor visitor argument must be derived before rendering"
    );
    assert!(
        !method.marshal_body.is_empty(),
        "convert-with-visitor marshal body must be derived before rendering"
    );
    assert!(
        !method.invoke_body.is_empty(),
        "convert-with-visitor invoke body must be derived before rendering"
    );

    let pu = method.prefix.to_uppercase();
    crate::backends::java::template_env::render(
        "convert_with_visitor.jinja",
        minijinja::context! {
            return_type => method.return_type,
            method_name => method.method_name,
            params => method.params,
            exception_class => method.exception_class,
            prefix_upper => pu,
            visitor_arg => method.visitor_arg,
            marshal_body => method.marshal_body,
            invoke_body => method.invoke_body,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_visitor_files_returns_four_files() {
        let api = super::files::tests::visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = super::files::tests::visitor_config("DemoVisitor", "VisitContext", "FlowDecision");
        let files = gen_visitor_files(&api, &config, "dev.sample_crate", "Demo").expect("metadata is complete");
        assert_eq!(files.len(), 3, "must return 3 files");
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            !names.contains(&"VisitContext.java"),
            "context is emitted by normal record generation"
        );
        assert!(names.contains(&"FlowDecision.java"), "must include result enum");
        assert!(names.contains(&"DemoVisitor.java"), "must include visitor interface");
        assert!(names.contains(&"VisitorBridge.java"), "must include VisitorBridge.java");
    }

    #[test]
    fn gen_visitor_files_skips_when_associated_metadata_is_absent() {
        let api = super::files::tests::visitor_api("DemoVisitor", "VisitContext", "FlowDecision");
        let config = super::files::tests::visitor_config_without_associated_types("DemoVisitor");
        assert!(gen_visitor_files(&api, &config, "dev.sample_crate", "Demo").is_none());
    }

    #[test]
    fn gen_native_lib_visitor_handles_includes_all_three_handles() {
        let out = gen_native_lib_visitor_handles("htm", &["visitor".to_string()]);
        assert!(out.contains("HTM_VISITOR_CREATE"), "must have visitor create handle");
        assert!(out.contains("HTM_VISITOR_FREE"), "must have visitor free handle");
        assert!(
            out.contains("HTM_OPTIONS_SET_VISITOR"),
            "must have options set visitor handle"
        );
    }

    #[test]
    fn gen_native_lib_visitor_handles_uses_configured_options_field() {
        let out = gen_native_lib_visitor_handles("syn", &["renderer".to_string()]);

        assert!(
            out.contains("SYN_OPTIONS_SET_RENDERER"),
            "must derive handle from renderer field"
        );
        assert!(
            out.contains("syn_options_set_renderer"),
            "must derive native symbol from renderer field"
        );
        assert!(
            !out.contains("SYN_OPTIONS_SET_VISITOR_HANDLE") && !out.contains("syn_options_set_visitor_handle"),
            "options-field mode must not bind the legacy visitor_handle setter"
        );
    }

    #[test]
    fn gen_convert_with_visitor_method_uses_explicit_method_shape() {
        let out = gen_convert_with_visitor_method(&ConvertWithVisitorMethod {
            return_type: "RenderResult",
            method_name: "renderWithVisitor",
            params: "String source, RenderOptions options, Visitor visitor",
            exception_class: "RendererException",
            prefix: "rnd",
            visitor_arg: "visitor",
            marshal_body: "var cSource = arena.allocateFrom(source);",
            invoke_body: "return invokeRender(cSource, visitorHandle);",
        });
        assert!(out.contains("renderWithVisitor"), "must define explicit method name");
        assert!(out.contains("RendererException"), "must use explicit exception type");
        assert!(out.contains("RenderResult"), "must use explicit IR-derived return type");
        assert!(
            out.contains("NativeLib.RND_VISITOR_CREATE"),
            "must call correct native handle"
        );
        assert!(
            !out.contains("ConversionResult") && !out.contains("ConversionOptions"),
            "must not synthesize legacy conversion DTO names"
        );
        assert!(
            !out.contains("CONVERSION_OPTIONS") && !out.contains("CONVERT_WITH_VISITOR"),
            "must not synthesize legacy conversion native handle names"
        );
    }
}
