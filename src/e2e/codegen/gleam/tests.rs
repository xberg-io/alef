use super::args::build_args_and_setup;
use super::constructors::render_gleam_element_constructor;
use crate::core::config::{GleamElementConstructor, GleamElementField};

fn file_job_recipe() -> GleamElementConstructor {
    GleamElementConstructor {
        element_type: "FileJob".to_string(),
        constructor: "sample_crate.FileJob".to_string(),
        fields: vec![
            GleamElementField {
                gleam_field: "path".to_string(),
                kind: "file_path".to_string(),
                json_field: Some("path".to_string()),
                default: None,
                value: None,
            },
            GleamElementField {
                gleam_field: "config".to_string(),
                kind: "literal".to_string(),
                json_field: None,
                default: None,
                value: Some("option.None".to_string()),
            },
        ],
    }
}

#[test]
fn render_element_constructor_file_path_relative_path_gets_test_documents_prefix() {
    let item = serde_json::json!({ "path": "docx/fake.docx" });
    let out = render_gleam_element_constructor(&item, &file_job_recipe(), "../../test_documents");
    assert_eq!(
        out,
        "sample_crate.FileJob(path: \"../../test_documents/docx/fake.docx\", config: option.None)"
    );
}

#[test]
fn render_element_constructor_file_path_absolute_path_passes_through() {
    let item = serde_json::json!({ "path": "/etc/some/absolute" });
    let out = render_gleam_element_constructor(&item, &file_job_recipe(), "../../test_documents");
    assert!(
        out.contains("\"/etc/some/absolute\""),
        "absolute paths must NOT receive the test_documents prefix; got:\n{out}"
    );
}

#[test]
fn render_element_constructor_byte_array_emits_bitarray() {
    let recipe = GleamElementConstructor {
        element_type: "BytesJob".to_string(),
        constructor: "sample_crate.BytesJob".to_string(),
        fields: vec![
            GleamElementField {
                gleam_field: "content".to_string(),
                kind: "byte_array".to_string(),
                json_field: Some("content".to_string()),
                default: None,
                value: None,
            },
            GleamElementField {
                gleam_field: "mime_type".to_string(),
                kind: "string".to_string(),
                json_field: Some("mime_type".to_string()),
                default: Some("text/plain".to_string()),
                value: None,
            },
            GleamElementField {
                gleam_field: "config".to_string(),
                kind: "literal".to_string(),
                json_field: None,
                default: None,
                value: Some("option.None".to_string()),
            },
        ],
    };
    let item = serde_json::json!({ "content": [72, 105], "mime_type": "text/html" });
    let out = render_gleam_element_constructor(&item, &recipe, "../../test_documents");
    assert_eq!(
        out,
        "sample_crate.BytesJob(content: <<72, 105>>, mime_type: \"text/html\", config: option.None)"
    );
}

#[test]
fn build_args_with_json_object_wrapper_substitutes_placeholder() {
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "config".to_string(),
        field: "config".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let input = serde_json::json!({
        "config": { "use_cache": true, "force_ocr": false }
    });
    let Some((_setup, args_str)) = build_args_and_setup(
        &input,
        &[arg],
        "test_fixture",
        "../../test_documents",
        &[],
        Some("k.config_from_json_string({json})"),
        "sample_crate",
        &[],
        None,
        "default",
    ) else {
        panic!("expected Some result from build_args_and_setup");
    };
    assert!(
        args_str.starts_with("k.config_from_json_string("),
        "wrapper must envelop the JSON literal; got:\n{args_str}"
    );
    assert!(
        args_str.contains("use_cache"),
        "JSON payload must reach the wrapper; got:\n{args_str}"
    );
}

#[test]
fn build_args_without_json_object_wrapper_returns_none_for_skip() {
    use crate::e2e::config::ArgMapping;
    let arg = ArgMapping {
        name: "config".to_string(),
        field: "config".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    let input = serde_json::json!({ "config": { "x": 1 } });
    let result = build_args_and_setup(
        &input,
        &[arg],
        "test_fixture",
        "../../test_documents",
        &[],
        None,
        "sample_crate",
        &[],
        None,
        "default",
    );
    assert!(
        result.is_none(),
        "json_object without recipe/wrapper/from_json must return None for skip; got: {result:?}"
    );
}

#[test]
fn render_element_constructor_string_falls_back_to_default() {
    let recipe = GleamElementConstructor {
        element_type: "BytesJob".to_string(),
        constructor: "k.BytesJob".to_string(),
        fields: vec![GleamElementField {
            gleam_field: "mime_type".to_string(),
            kind: "string".to_string(),
            json_field: Some("mime_type".to_string()),
            default: Some("text/plain".to_string()),
            value: None,
        }],
    };
    let item = serde_json::json!({});
    let out = render_gleam_element_constructor(&item, &recipe, "../../test_documents");
    assert!(
        out.contains("mime_type: \"text/plain\""),
        "missing string field must fall back to default; got:\n{out}"
    );
}
