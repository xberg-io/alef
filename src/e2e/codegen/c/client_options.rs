use crate::e2e::fixture::Fixture;

pub(super) fn apply(
    source: &str,
    fixture: &Fixture,
    prefix: &str,
    factory: &str,
    json: &str,
) -> anyhow::Result<String> {
    let type_prefix = crate::codegen::c_consumer::export_type_prefix(prefix);
    let creation = format!("    {type_prefix}AlefHandle client = {prefix}_");
    let json_prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(json);
    let mut output = String::new();
    let mut count = 0;
    for line in source.lines() {
        if let Some(call) = line.strip_prefix(&creation) {
            let (_, args) = call
                .split_once('(')
                .ok_or_else(|| anyhow::anyhow!("malformed client factory call"))?;
            let mut args = args.splitn(3, ',');
            let api_key = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing client credential"))?;
            let base_url = args.next().ok_or_else(|| anyhow::anyhow!("missing client base URL"))?;
            output.push_str(&crate::e2e::template_env::render(
                "c/client_options.jinja",
                crate::alef_context! {
                    type_prefix => type_prefix, prefix => prefix, factory => factory,
                    json_prefix => crate::e2e::escape::escape_c(&json_prefix),
                    api_key => api_key.trim(), base_url => base_url.trim(),
                },
            ));
            count += 1;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    anyhow::ensure!(
        count > 0,
        "client_options fixture {} did not construct a C client",
        fixture.id
    );
    Ok(output)
}

pub(super) fn helpers() -> String {
    crate::e2e::template_env::render("c/client_options_helpers.jinja", crate::alef_context! {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_client_shapes_retain_runtime_credential_and_routing_expressions() {
        for (credential, url) in [
            ("api_key", "_base_url_arg"),
            ("\"test-key\"", "base_url"),
            ("\"test-key\"", "NULL"),
        ] {
            let source = format!(
                "    SAMPLEAlefHandle client = sample_create_client({credential}, {url}, (uint64_t)-1, (uint32_t)-1, NULL);\n"
            );
            let fixture = Fixture {
                id: "configured_client".into(),
                ..Fixture::default()
            };
            let rendered = apply(
                &source,
                &fixture,
                "sample",
                "configured_client_json",
                "{\"thresholds\":[18446744073709551615]}",
            )
            .unwrap();
            assert!(
                rendered.contains("sample_configured_client_json(_client_options_json)"),
                "{rendered}"
            );
            assert!(rendered.contains(&format!(", {credential}, {url});")), "{rendered}");
            assert!(rendered.contains("18446744073709551615"), "{rendered}");
            assert!(!rendered.contains("sample_create_client("), "{rendered}");
        }
    }

    #[test]
    fn absent_client_construction_is_an_error_instead_of_a_default_client_pass() {
        let fixture = Fixture {
            id: "configured_client".into(),
            ..Fixture::default()
        };
        let error = apply("void test_case(void) {}", &fixture, "sample", "from_json", "{}").unwrap_err();
        assert!(error.to_string().contains("did not construct a C client"));
    }
}
