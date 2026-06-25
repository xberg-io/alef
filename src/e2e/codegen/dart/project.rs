use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

pub(super) fn render_pubspec(
    pkg_name: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
) -> String {
    let test_ver = crate::core::template_versions::pub_dev::TEST_PACKAGE;
    let http_ver = crate::core::template_versions::pub_dev::HTTP_PACKAGE;
    let ffi_ver = crate::core::template_versions::pub_dev::FFI_PACKAGE;

    let dep_block = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Only add ^ prefix if version doesn't already start with a constraint operator
            let constraint = if pkg_version.starts_with('^')
                || pkg_version.starts_with('~')
                || pkg_version.starts_with('>')
                || pkg_version.starts_with('<')
                || pkg_version.starts_with('=')
            {
                pkg_version.to_string()
            } else {
                format!("^{pkg_version}")
            };
            format!("  {pkg_name}: {constraint}")
        }
        crate::e2e::config::DependencyMode::Local => {
            format!("  {pkg_name}:\n    path: {pkg_path}")
        }
    };

    let sdk = crate::core::template_versions::toolchain::DART_SDK_CONSTRAINT;
    format!(
        r#"name: e2e_dart
version: 0.1.0
publish_to: none

environment:
  sdk: "{sdk}"

dependencies:
{dep_block}
  ffi: {ffi_ver}

dev_dependencies:
  test: {test_ver}
  http: {http_ver}
"#
    )
}

// The server-pattern `app_harness.dart` is now emitted by a consumer extension via
// `Extension::emit_e2e`; alef no longer emits it. Retained pending the dead-code
// sweep so the migration diff stays minimal.
#[allow(dead_code)]
pub(super) fn render_app_harness(groups: &[FixtureGroup], e2e_config: &E2eConfig, pkg_name: &str) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if let Some(http) = &fixture.http {
                let mut fixture_obj = serde_json::Map::new();

                let mut http_obj = serde_json::Map::new();

                // handler: route, method, body_schema
                let mut handler_obj = serde_json::Map::new();
                handler_obj.insert("route".to_string(), serde_json::json!(http.handler.route));
                handler_obj.insert("method".to_string(), serde_json::json!(http.handler.method.as_str()));
                if let Some(body_schema) = &http.handler.body_schema {
                    handler_obj.insert("body_schema".to_string(), body_schema.clone());
                } else {
                    handler_obj.insert("body_schema".to_string(), serde_json::Value::Null);
                }
                http_obj.insert("handler".to_string(), serde_json::Value::Object(handler_obj));

                // expected_response: status_code, body, headers
                let mut response_obj = serde_json::Map::new();
                response_obj.insert(
                    "status_code".to_string(),
                    serde_json::json!(http.expected_response.status_code),
                );
                if let Some(body) = &http.expected_response.body {
                    response_obj.insert("body".to_string(), body.clone());
                } else {
                    response_obj.insert("body".to_string(), serde_json::Value::Null);
                }

                let headers: serde_json::Map<String, serde_json::Value> = http
                    .expected_response
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                    .collect();
                response_obj.insert("headers".to_string(), serde_json::Value::Object(headers));

                http_obj.insert("expected_response".to_string(), serde_json::Value::Object(response_obj));

                fixture_obj.insert("http".to_string(), serde_json::Value::Object(http_obj));
                fixtures_map.insert(fixture.id.clone(), serde_json::Value::Object(fixture_obj));
            }
        }
    }

    let fixtures_json = serde_json::to_string(&fixtures_map).unwrap_or_else(|_| "{}".to_string());

    // Derive the bridge module name from the package name:
    // e.g. "my_pkg" → "my_pkg_bridge_generated"
    let bridge_module = format!("{pkg_name}_bridge_generated");

    // Render using the Jinja template.
    let ctx = minijinja::context! {
        fixtures_json => fixtures_json,
        pkg_name => pkg_name,
        bridge_module => bridge_module,
        host => &e2e_config.harness.host,
        port => e2e_config.harness.port,
    };
    crate::e2e::template_env::render("dart/app_harness.dart.jinja", ctx)
}
