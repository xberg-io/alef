use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef, VersionAnnotation};
use crate::docs::descriptions::generate_param_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at, extract_param_docs};
#[cfg(test)]
use crate::docs::examples::render_function_example;
use crate::docs::examples::render_function_example_with_api;
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_error_phrase};
use crate::docs::naming::{field_name, func_name, lang_code_fence};
use crate::docs::rust_types::rust_param_type;
use crate::docs::signatures::render_function_signature;
use crate::docs::{clean_doc, doc_type, template_env, version_labels};

pub(super) fn push_version_annotation(out: &mut String, version: &VersionAnnotation) {
    if let Some(ref since) = version.since {
        let since = version_labels::major_minor(since);
        out.push_str(&template_env::render(
            "since_badge.jinja",
            minijinja::context! { since => since },
        ));
        out.push('\n');
        out.push('\n');
    }
    if let Some(ref dep) = version.deprecated {
        let since = dep
            .since
            .as_deref()
            .map(version_labels::major_minor)
            .unwrap_or_default();
        out.push_str(&template_env::render(
            "deprecated_notice.jinja",
            minijinja::context! {
                since => since,
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }
}

/// The function as the backends bind it: identical to `func` unless a `&mut T` DTO parameter
/// turns the unit return into `T`.
fn mut_writeback_return<'a>(func: &'a FunctionDef, api: &ApiSurface) -> std::borrow::Cow<'a, FunctionDef> {
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    match crate::codegen::mut_writeback::effective_return_type(&func.params, &func.return_type, &opaque_types) {
        Some(return_type) => {
            let mut bound = func.clone();
            bound.return_type = return_type;
            std::borrow::Cow::Owned(bound)
        }
        None => std::borrow::Cow::Borrowed(func),
    }
}

pub(super) fn render_function(
    func: &FunctionDef,
    lang: Language,
    _config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    // A `&mut T` DTO parameter makes the binding return the updated `T` in place of `()`
    // (see `codegen::mut_writeback`). Docs must document the signature the backends emit, not
    // the core Rust one, or the page tells the reader to drop a value the binding hands back. ~keep
    let with_writeback = mut_writeback_return(func, api);
    let func = with_writeback.as_ref();

    let mut out = String::new();
    let fn_name = func_name(&func.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => format!("{fn_name}()") },
    ));

    push_version_annotation(&mut out, &func.version);

    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        let doc = clean_doc(&func.doc, lang);
        // Nest under the `####` heading emitted just above, rather than shifting by a fixed
        // number of levels. A fixed `+2` assumes the doc comment starts at `#`, and a section that
        // starts anywhere else lands ABOVE its own parent: a rustdoc `# Observability` surfaced as
        // `###` under a `####` item, so it read as a sibling of the page's `### Functions` section and
        // took a bogus entry in the table of contents with it. ~keep
        let doc = demote_headings_to_start_at(&doc, 5);
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    out.push_str("**Signature:**\n\n");
    let lang_code = lang_code_fence(lang);
    let sig = render_function_signature(func, lang, ffi_prefix, &api.crate_name, api);
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));
    out.push('\n');

    out.push_str(&render_function_example_with_api(
        func,
        lang,
        ffi_prefix,
        &api.crate_name,
        api,
    ));

    push_parameters_table(&mut out, &func.params, &param_docs, lang, ffi_prefix, api);

    push_returns(
        &mut out,
        &func.return_type,
        func.error_type.as_deref(),
        lang,
        ffi_prefix,
        api,
    );
    push_errors(
        &mut out,
        func.error_type.as_deref(),
        &func.return_type,
        lang,
        &api.crate_name,
    );

    out
}

pub(super) fn push_parameters_table(
    out: &mut String,
    params: &[ParamDef],
    param_docs: &std::collections::HashMap<String, String>,
    lang: Language,
    ffi_prefix: &str,
    api: &ApiSurface,
) {
    if params.is_empty() {
        return;
    }
    out.push_str("**Parameters:**\n\n");
    out.push_str("| Name | Type | Required | Description |\n");
    out.push_str("|------|------|----------|-------------|\n");
    for param in params {
        let pname = field_name(&param.name, lang);
        let pty = if lang == Language::Rust {
            rust_param_type(param, ffi_prefix)
        } else if lang == Language::Zig {
            // ~keep See `render_zig_fn_sig`'s note: ask the backend's own boundary predicate
            // rather than re-deriving whether a `Named` param is an opaque handle or a struct
            // DTO serialised to bytes.
            crate::backends::zig::zig_boundary_param_type(&param.ty, param.optional, api)
        } else {
            doc_type_with_optional(&param.ty, lang, param.optional, ffi_prefix)
        };
        let required = if param.optional { "No" } else { "Yes" };
        let pdoc = param_docs
            .get(param.name.as_str())
            .map(|s| clean_doc_inline(s, lang))
            .unwrap_or_else(|| generate_param_description(&param.name, &param.ty));
        out.push_str(&template_env::render(
            "param_row.jinja",
            minijinja::context! {
                name => escape_table_cell(&pname),
                ty => escape_table_cell(&pty),
                required => required,
                doc => escape_table_cell(&pdoc),
            },
        ));
    }
    out.push('\n');
}

pub(super) struct ReturnsRender<'a> {
    pub return_type: &'a TypeRef,
    pub return_type_override: Option<&'a str>,
    pub error_type: Option<&'a str>,
    pub lang: Language,
    pub ffi_prefix: &'a str,
    pub api: &'a ApiSurface,
}

pub(super) fn push_returns(
    out: &mut String,
    return_type: &TypeRef,
    error_type: Option<&str>,
    lang: Language,
    ffi_prefix: &str,
    api: &ApiSurface,
) {
    push_returns_with_override(
        out,
        &ReturnsRender {
            return_type,
            return_type_override: None,
            error_type,
            lang,
            ffi_prefix,
            api,
        },
    );
}

pub(super) fn push_returns_with_override(out: &mut String, render: &ReturnsRender<'_>) {
    let &ReturnsRender {
        return_type,
        return_type_override,
        error_type,
        lang,
        ffi_prefix,
        api,
    } = render;

    if matches!(return_type, TypeRef::Unit) {
        if let Some(override_ty) = return_type_override {
            out.push_str(&template_env::render(
                "returns.jinja",
                minijinja::context! { ty => override_ty },
            ));
            out.push('\n');
            return;
        }
        // ~keep A fallible Ffi/C function/method whose logical return is `()` reports
        // failure through the return itself: `signatures.rs`'s `render_c_fn_sig` /
        // `render_method_signature_with_override` document that return as `int32_t`
        // (see `backends/ffi/gen_bindings/functions/orchestration.rs`'s
        // `has_error && is_void_return -> i32`), so this prose must say the same thing.
        // Printing "No return value" here while the signature line says `int32_t` two
        // lines above is worse than not knowing the ABI at all -- the page contradicts
        // itself and a reader can't tell which half to trust. The emitter always returns
        // exactly `-1` on failure (`orchestration.rs:224`), never some other non-zero
        // value, so say `-1` here -- the same value `formatting.rs`'s
        // `ffi_error_return_phrase` names in the Errors section below.
        if matches!(lang, Language::Ffi | Language::C) && error_type.is_some() {
            out.push_str("**Returns:** `int32_t` status code -- `0` on success, `-1` on error.\n");
        } else {
            out.push_str("**Returns:** No return value.\n");
        }
        out.push('\n');
        return;
    }

    let ret_ty = return_type_override.map(str::to_string).unwrap_or_else(|| {
        if lang == Language::Zig {
            crate::backends::zig::zig_boundary_return_type(return_type, api)
        } else {
            doc_type(return_type, lang, ffi_prefix)
        }
    });
    if ret_ty.is_empty() {
        out.push_str("**Returns:** No return value.\n");
        out.push('\n');
    } else {
        out.push_str(&template_env::render(
            "returns.jinja",
            minijinja::context! { ty => ret_ty },
        ));
        out.push('\n');
    }
}

pub(super) fn push_errors(
    out: &mut String,
    error_type: Option<&str>,
    return_type: &TypeRef,
    lang: Language,
    crate_name: &str,
) {
    if let Some(err) = error_type {
        let error_phrase = format_error_phrase(err, return_type, lang, crate_name);
        out.push_str(&template_env::render(
            "errors_phrase.jinja",
            minijinja::context! { phrase => error_phrase },
        ));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::test_helpers::{TEST_CRATE_NAME, TEST_PREFIX, make_function, make_param};

    /// ~keep Reproduces alef #192: a `# Note` section that follows a `# Examples` section in a
    /// rustdoc comment was emitted twice on the generated function entry -- once correctly
    /// converted to a `**Note:**` bold label by `clean_doc`, and a second time raw and
    /// unconverted, leaked verbatim into the `**Example:**` code block. The leak happened
    /// because `codegen::doc_emission::parse_rustdoc_sections` (which powers the Example
    /// block) has its own, independent whitelist of recognised headings that does not include
    /// `note`, so an unrecognised `# Note` heading was folded into whatever section came
    /// before it -- `example`, if that's the last recognised heading in the doc. This doc
    /// mirrors the real consumer shape: Arguments, then an Example
    /// code fence, then a trailing Note section.
    #[test]
    fn test_note_section_after_examples_is_not_emitted_twice() {
        let mut func = make_function("count_tokens", vec![], TypeRef::String, false, None);
        func.doc = r#"Count the number of tokens in `text`.

# Arguments

* `text` - The text to tokenize.

# Example

```rust,no_run
let n = count_tokens("hello");
```

# Note

This function is intentionally excluded from language bindings."#
            .to_string();

        let doc_body = {
            let doc = clean_doc(&func.doc, Language::Rust);
            demote_headings_to_start_at(&doc, 5)
        };
        let example = render_function_example(&func, Language::Rust, TEST_PREFIX, TEST_CRATE_NAME);
        let rendered = format!("{doc_body}\n\n{example}");

        assert_eq!(
            rendered.matches("**Note:**").count(),
            1,
            "the Note section must be converted to a bold label exactly once: {rendered}"
        );
        assert!(
            !rendered
                .lines()
                .any(|line| line.trim_start().trim_start_matches('#').trim() == "Note"
                    && line.trim_start().starts_with('#')),
            "no raw, unconverted `# Note` heading (at any level) may survive in the rendered \
             output: {rendered}"
        );
    }

    /// ~keep The signature line and the "Returns:" prose are two independent renderers
    /// describing the same function on the same generated page. For a fallible Ffi/C
    /// function whose logical return is `()`, the ABI repurposes the return as an
    /// `int32_t` status code (see signatures.rs). If the two renderers drift -- one says
    /// `int32_t`, the other still says "No return value" -- the page contradicts itself,
    /// which is worse than the original bug because a reader can no longer tell which half
    /// to trust. This renders both from the same `FunctionDef` and asserts they agree,
    /// rather than asserting each against a literal that could independently go stale.
    #[test]
    fn test_c_signature_and_returns_prose_agree_on_fallible_void_status_code() {
        let func = make_function(
            "init",
            vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
            TypeRef::Unit,
            false,
            Some("InitError"),
        );

        let signature =
            render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME, &ApiSurface::default());

        let mut returns_prose = String::new();
        push_returns(
            &mut returns_prose,
            &func.return_type,
            func.error_type.as_deref(),
            Language::C,
            TEST_PREFIX,
            &ApiSurface::default(),
        );

        assert!(signature.starts_with("int32_t "), "signature: {signature}");
        assert!(
            returns_prose.contains("int32_t"),
            "returns prose must mention the same status-code type the signature declares: {returns_prose}"
        );
        assert!(
            !returns_prose.contains("No return value"),
            "must not still claim there's no return value once the signature says int32_t: {returns_prose}"
        );
    }

    #[test]
    fn test_c_signature_and_returns_prose_agree_infallible_void_stays_silent() {
        let func = make_function("touch", vec![], TypeRef::Unit, false, None);

        let signature =
            render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME, &ApiSurface::default());

        let mut returns_prose = String::new();
        push_returns(
            &mut returns_prose,
            &func.return_type,
            func.error_type.as_deref(),
            Language::C,
            TEST_PREFIX,
            &ApiSurface::default(),
        );

        assert!(signature.starts_with("void "), "signature: {signature}");
        assert!(returns_prose.contains("No return value"), "{returns_prose}");
        assert!(!returns_prose.contains("int32_t"), "{returns_prose}");
    }

    #[test]
    fn test_returns_with_override_wins_over_status_code_inference_for_unit_return() {
        // ~keep A curated return-type override (streaming's use case) is trusted verbatim even
        // when the logical return type is `()` -- the status-code inference must not
        // second-guess it.
        let mut out = String::new();
        push_returns_with_override(
            &mut out,
            &ReturnsRender {
                return_type: &TypeRef::Unit,
                return_type_override: Some("StreamHandle"),
                error_type: Some("StreamError"),
                lang: Language::C,
                ffi_prefix: TEST_PREFIX,
                api: &ApiSurface::default(),
            },
        );
        assert!(out.contains("StreamHandle"), "{out}");
        assert!(!out.contains("int32_t"), "{out}");
    }

    /// ~keep A core error type whose short name is `Error` is the case that let this bug hide:
    /// pascal-casing that short name yields `ErrorException`, which is a real class the domain
    /// error hierarchy declares (`codegen::error_gen::host_langs`), so the fabrication read as
    /// plausible even though no generated method throws it. Asserting against
    /// `exception_class_name` -- the same derivation the `throws` clause and the streaming
    /// override use -- rather than a hardcoded literal is what would actually have caught the
    /// original bug; a literal expectation would have pinned the wrong class just as easily.
    #[test]
    fn test_errors_prose_names_the_class_exception_class_name_derives() {
        let mut out = String::new();
        push_errors(
            &mut out,
            Some("Error"),
            &TypeRef::String,
            Language::Java,
            TEST_CRATE_NAME,
        );

        let expected_class = crate::backends::java::naming::exception_class_name(TEST_CRATE_NAME);
        assert_eq!(
            out.trim(),
            format!("**Errors:** Throws `{expected_class}`."),
            "the Errors: prose must name the class the Java binding actually declares, not a \
             pascal-cased spelling of the error type's own short name: {out}"
        );
    }
}
