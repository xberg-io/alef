use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

use super::http;
use super::test_method;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    lang: &str,
    namespace: &str,
    class_name: &str,
    test_class: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    php_enum_lowering: &super::enum_variant_access::PhpEnumLowering,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    php_lang_rename_all: String,
    config: &ResolvedCrateConfig,
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    let header = hash::e2e_header(CommentStyle::DoubleSlash);

    // Determine if any fixture is an HTTP test (needs GuzzleHttp).
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    // Collect options_type class names that need `use` imports (one import per unique name).
    // Also collects:
    //   - per-arg `element_type` for `Vec<T>` arguments (e.g. `PageAction`).
    //   - streaming-adapter `request_type` for fixtures that invoke a streaming
    //     adapter call (e.g. `StreamItemsRequest`). Without this import the
    //     generated `new StreamItemsRequest($url)` resolves to
    //     `DemoCrawler\E2e\StreamItemsRequest` (the test namespace) and PHPUnit
    //     errors with `Class "...\StreamItemsRequest" not found`.
    let mut options_type_imports: Vec<String> = fixtures
        .iter()
        .flat_map(|f| {
            let call = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            let php_override = call.overrides.get(lang);
            let opt_type = php_override
                .and_then(|o| o.options_type.as_deref())
                .or_else(|| {
                    e2e_config
                        .call
                        .overrides
                        .get(lang)
                        .and_then(|o| o.options_type.as_deref())
                })
                .or(call.options_type.as_deref());
            let element_types: Vec<String> = call
                .args
                .iter()
                .filter_map(|a| a.element_type.as_ref().map(|t| t.to_string()))
                .filter(|t| !super::values::is_php_reserved_type(t))
                .collect();
            let adapter_lookup_name = call.core_lookup_name(lang);
            let adapter_request_type: Option<String> = adapter_lookup_name
                .as_deref()
                .and_then(|name| adapters.iter().find(|a| a.name == name))
                .and_then(|a| a.request_type.as_deref())
                .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
            opt_type
                .map(|t| t.to_string())
                .into_iter()
                .chain(element_types)
                .chain(adapter_request_type)
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    options_type_imports.sort();

    // Build imports_use list
    let mut imports_use: Vec<String> = Vec::new();
    for type_name in &options_type_imports {
        if type_name != class_name {
            imports_use.push(format!("use {namespace}\\{type_name};"));
        }
    }

    // Render all test methods
    let mut fixtures_body = String::new();
    let mut trait_bridge_imports: Vec<String> = Vec::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            http::render_http_test_method(&mut fixtures_body, fixture, fixture.http.as_ref().unwrap());
        } else {
            test_method::render_test_method(
                &mut fixtures_body,
                fixture,
                e2e_config,
                lang,
                namespace,
                class_name,
                type_defs,
                enums,
                functions,
                php_enum_lowering,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
                adapters,
                &php_lang_rename_all,
                config,
                errors,
                &mut trait_bridge_imports,
            );
        }
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }
    // Merge trait-bridge imports into imports_use
    imports_use.extend(trait_bridge_imports);

    crate::e2e::template_env::render(
        "php/test_file.jinja",
        minijinja::context! {
            header => header,
            namespace => namespace,
            class_name => class_name,
            test_class => test_class,
            category => category,
            imports_use => imports_use,
            has_http_tests => has_http_tests,
            fixtures_body => fixtures_body,
        },
    )
}
