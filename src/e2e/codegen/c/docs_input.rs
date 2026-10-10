use crate::e2e::escape::escape_c;

pub(super) fn render_c_docs_json(
    variable: &str,
    value: &serde_json::Value,
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    documentation_snippet: bool,
) -> (String, String, String) {
    if files.is_empty() {
        let json = serde_json::to_string(value).unwrap_or_default();
        return (String::new(), format!("\"{}\"", escape_c(&json)), String::new());
    }
    let mut value = value.clone();
    let mut reads = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let marker = format!("__ALEF_DOC_FILE_{index}__");
        let target = if file.field.is_empty() {
            Some(&mut value)
        } else {
            value.pointer_mut(&file.field)
        };
        let Some(target) = target else { continue };
        *target = serde_json::Value::String(marker.clone());
        reads.push((index, marker, file.path.clone()));
    }
    let base = serde_json::to_string(&value).unwrap_or_default();
    let mut setup = crate::e2e::template_env::render(
        "c/docs_json_base.jinja",
        crate::alef_context! { variable => variable, json => escape_c(&base) },
    );
    let mut source = format!("{variable}_json_base");
    for (index, marker, path) in reads {
        let output = format!("{variable}_json_{index}");
        setup.push_str(&crate::e2e::template_env::render(
            "c/docs_file_replace.jinja",
            crate::alef_context! {
                variable => variable,
                index => index,
                path => escape_c(&path),
                marker => escape_c(&format!("\"{marker}\"")),
                source => source,
                source_owned => index > 0,
                output => output,
                documentation_snippet => documentation_snippet,
            },
        ));
        source = output;
    }
    let cleanup =
        crate::e2e::template_env::render("c/docs_json_cleanup.jinja", crate::alef_context! { variable => source });
    (setup, source, cleanup)
}

#[cfg(test)]
mod tests {
    use super::render_c_docs_json;
    use crate::e2e::fixture::FixtureDocsFileInput;

    #[test]
    fn nested_typed_dto_files_become_runtime_json_byte_arrays() {
        let (setup, expression, cleanup) = render_c_docs_json(
            "request",
            &serde_json::json!({"content": "ignored"}),
            &[FixtureDocsFileInput {
                field: "/content".into(),
                path: "document.pdf".into(),
            }],
            true,
        );
        assert!(setup.contains("fopen(\"document.pdf\", \"rb\")"), "{setup}");
        assert!(setup.contains("snprintf"), "{setup}");
        assert_eq!(expression, "request_json_0");
        assert!(cleanup.contains("free(request_json_0)"));
    }
}
