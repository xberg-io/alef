//! Regression for the C free-function/raw-result path silently dropping the FFI generator's
//! `TypeRef::Bytes` -> `(const uint8_t *ptr, uintptr_t len)` pairing convention (see
//! `bytes_arg`'s module doc). Before the fix, a `type = "bytes"` arg reached `json_to_c` like any
//! other value, so a fixture-relative file path rendered as a bare `const char *` string literal
//! and the length parameter every other backend already emits was never written at all --
//! `xberg_pdf_page_count("pdf/sample_contract.pdf", NULL)` against a header declaring
//! `xberg_pdf_page_count(const uint8_t *pdf_bytes, uintptr_t pdf_bytes_len, const char *password)`.

use super::snippet_regressions::compile_snippet;
use super::*;

fn bytes_arg(name: &str, field: &str, optional: bool) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "bytes".into(),
        optional,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn string_arg(name: &str, field: &str, optional: bool) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "string".into(),
        optional,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn render_pdf_page_count_snippet(input: serde_json::Value) -> String {
    let fixture = Fixture {
        id: "pdf_page_count_valid".into(),
        description: "pdf_page_count: page count of a multi-page PDF".into(),
        input,
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "pdf_page_count".into();
    e2e.call.args = vec![
        bytes_arg("pdf_bytes", "input.pdf_bytes", false),
        string_arg("password", "input.password", true),
    ];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            raw_c_result_type: Some("uintptr_t".into()),
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("snippet renders")
}

/// The reproducer: a bare file-path string used to render as a single `const char *` argument,
/// which is one argument short of `xberg_pdf_page_count`'s real three-parameter C signature.
#[test]
fn a_bytes_typed_argument_emits_a_pointer_and_length_pair() {
    let rendered = render_pdf_page_count_snippet(serde_json::json!({ "pdf_bytes": "pdf/sample_contract.pdf" }));

    // ~keep The path legitimately appears inside the `fopen` the fix emits, so the assertion has
    // to name the CALL SITE: what the defect did was hand the path straight to the target function.
    assert!(
        !rendered.contains(r#"sample_pdf_page_count("pdf/sample_contract.pdf""#),
        "the file path must not be passed to the target function as a C string literal; got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"fopen("pdf/sample_contract.pdf", "rb")"#),
        "the file should be read into a byte buffer; got:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_pdf_page_count(pdf_bytes_buf, (uintptr_t)pdf_bytes_size, NULL)"),
        "expected a (ptr, len, password) call with 3 arguments; got:\n{rendered}"
    );

    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "uintptr_t sample_pdf_page_count(const uint8_t *pdf_bytes, uintptr_t pdf_bytes_len, const char *password);\n",
            "int sample_last_error_code(void);\n",
        ),
    );
}

/// The control: a required `bytes` arg alongside a *provided* optional `string` arg still emits
/// both the byte pair and the string literal, in the FFI's declared parameter order.
#[test]
fn a_provided_optional_password_still_renders_after_the_byte_pair() {
    let rendered = render_pdf_page_count_snippet(serde_json::json!({
        "pdf_bytes": "pdf/password_protected.pdf",
        "password": ""
    }));

    assert!(
        rendered.contains(r#"sample_pdf_page_count(pdf_bytes_buf, (uintptr_t)pdf_bytes_size, "")"#),
        "expected the byte pair followed by the literal password; got:\n{rendered}"
    );
}
