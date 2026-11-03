use crate::e2e::escape::escape_csharp;
use heck::ToUpperCamelCase;

pub(super) fn expression(
    fixture: &crate::e2e::fixture::Fixture,
    config: &crate::e2e::config::E2eConfig,
    call: &crate::e2e::config::CallConfig,
    class_name: &str,
    factory: &str,
    (api_key, base_url): (&str, &str),
) -> String {
    let Some((factory, json)) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, config, call, "csharp")
    else {
        return format!("{class_name}.{factory}({api_key}, {base_url}, null, null, null)");
    };
    let factory = factory.to_upper_camel_case();
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let prefix = escape_csharp(&prefix);
    let serializer = "global::System.Text.Json.JsonSerializer.Serialize";
    let key_json = format!("{serializer}({api_key})");
    let url_json = format!("{serializer}((object?)({base_url}))");
    format!(r#"{class_name}.{factory}("{prefix}" + {key_json} + ",\"base_url\":" + {url_json} + "}}")"#)
}
