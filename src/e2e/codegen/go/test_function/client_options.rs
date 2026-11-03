use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;
use std::fmt::Write as _;

pub(super) fn expression(
    out: &mut String,
    fixture: &Fixture,
    config: &E2eConfig,
    call: &CallConfig,
    alias: &str,
    (api_key, base_url): (&str, &str),
) -> Option<String> {
    let (factory, json) = crate::e2e::codegen::client_factory::client_options_json_call(fixture, config, call, "go")?;
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let prefix = crate::e2e::escape::go_string_literal(&prefix);
    let _ = writeln!(out, "\tclientAPIKeyJSON, clientAPIKeyErr := json.Marshal({api_key})");
    let _ = writeln!(out, "\tif clientAPIKeyErr != nil {{ t.Fatal(clientAPIKeyErr) }}");
    let _ = writeln!(out, "\tclientBaseURLJSON, clientBaseURLErr := json.Marshal({base_url})");
    let _ = writeln!(out, "\tif clientBaseURLErr != nil {{ t.Fatal(clientBaseURLErr) }}");
    let _ = writeln!(
        out,
        r#"	clientConfigJSON := {prefix} + string(clientAPIKeyJSON) + ",\"base_url\":" + string(clientBaseURLJSON) + "}}""#
    );
    let factory = crate::codegen::naming::to_go_name(factory);
    Some(format!("{alias}.{factory}(clientConfigJSON)"))
}
