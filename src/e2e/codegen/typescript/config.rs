//! Config file generators for TypeScript e2e tests (package.json, tsconfig.json, vitest.config.ts).

use crate::core::config::manifest_extras::ManifestExtras;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions as tv;
use minijinja::context;

pub(crate) fn render_package_json(
    pkg_name: &str,
    _pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    has_http_fixtures: bool,
    extras: Option<&ManifestExtras>,
) -> String {
    let dep_value = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // If alef.toml provides the version with a semver range operator
            // (`^`, `~`, `>=`, etc.), the caller has chosen the registry-conventional
            // form — use it verbatim. Otherwise prepend `^` for caret-range semver.
            let trimmed = pkg_version.trim_start();
            if trimmed.starts_with(['^', '~', '>', '<', '=']) {
                pkg_version.to_string()
            } else {
                format!("^{pkg_version}")
            }
        }
        crate::e2e::config::DependencyMode::Local => "workspace:*".to_string(),
    };
    let _ = has_http_fixtures; // HTTP test deps are added when http fixtures require them

    let rendered = crate::e2e::template_env::render(
        "typescript/package.json.jinja",
        context! {
            pkg_name => pkg_name,
            dep_value => dep_value,
            vitest => tv::npm::VITEST,
        },
    );

    match extras {
        Some(e) if !e.is_empty() => inject_package_json_extras(&rendered, e),
        _ => rendered,
    }
}

/// Splice `dependencies` / `dev_dependencies` from a [`ManifestExtras`] into the
/// `dependencies` / `devDependencies` JSON maps of a rendered `package.json`.
///
/// Idempotent: re-running with the same extras yields the same output. Existing
/// entries with the same key are overwritten by the extras (last-write-wins) so
/// project config can pin a version away from the default emitter.
pub(crate) fn inject_package_json_extras(manifest_json: &str, extras: &ManifestExtras) -> String {
    let mut root: serde_json::Value = match serde_json::from_str(manifest_json) {
        Ok(v) => v,
        Err(_) => return manifest_json.to_string(), // malformed input — leave untouched
    };
    let Some(obj) = root.as_object_mut() else {
        return manifest_json.to_string();
    };

    merge_into(obj, "dependencies", &extras.dependencies);
    merge_into(obj, "devDependencies", &extras.dev_dependencies);

    // Pretty-print with 2-space indent to match the template's style; append
    // trailing newline (template ends with one).
    let mut out = serde_json::to_string_pretty(&root).unwrap_or_else(|_| manifest_json.to_string());
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn merge_into(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    bucket: &str,
    deps: &std::collections::BTreeMap<String, crate::core::config::manifest_extras::ExtraDepSpec>,
) {
    if deps.is_empty() {
        return;
    }
    let bucket_obj = obj
        .entry(bucket.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(map) = bucket_obj.as_object_mut() {
        for (name, spec) in deps {
            if let Some(v) = spec.version() {
                map.insert(name.clone(), serde_json::Value::String(v.to_string()));
            }
        }
        // Keep keys in sorted order so re-renders are byte-stable.
        let sorted: serde_json::Map<String, serde_json::Value> = {
            let mut pairs: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs.into_iter().collect()
        };
        *map = sorted;
    }
}

pub(super) fn render_tsconfig() -> String {
    crate::e2e::template_env::render("typescript/tsconfig.jinja", context! {})
}

pub(super) fn render_vitest_config(with_global_setup: bool, with_file_setup: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    crate::e2e::template_env::render(
        "typescript/vitest.config.ts.jinja",
        context! {
            header => header,
            with_global_setup => with_global_setup,
            with_file_setup => with_file_setup,
        },
    )
}

pub(super) fn render_file_setup(test_documents_dir: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    crate::e2e::template_env::render(
        "typescript/setup.ts.jinja",
        context! {
            header => header,
            test_documents_dir => test_documents_dir,
        },
    )
}

pub fn render_app_harness(
    e2e_config: &crate::e2e::config::E2eConfig,
    groups: &[crate::e2e::fixture::FixtureGroup],
) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            // Convert the fixture to JSON for the harness to load.
            // We only need the http field, handler, request, and expected_response.
            let http_data = &fixture.http.as_ref().unwrap();
            let mut handler_obj = serde_json::json!({
                "route": &http_data.handler.route,
                "method": &http_data.handler.method,
                "body_schema": http_data.handler.body_schema.clone(),
            });
            // Include middleware if present for CORS preflight registration
            if let Some(middleware) = &http_data.handler.middleware {
                if let Ok(middleware_json) = serde_json::to_value(middleware) {
                    if let serde_json::Value::Object(ref obj) = handler_obj {
                        let mut handler_map = obj.clone();
                        handler_map.insert("middleware".to_string(), middleware_json);
                        handler_obj = serde_json::Value::Object(handler_map);
                    }
                }
            }
            let mut request_obj = serde_json::json!({
                "path": &http_data.request.path,
            });
            // Include content_type if present for multipart/form-encoded detection
            if let Some(ct) = &http_data.request.content_type {
                if let serde_json::Value::Object(ref obj) = request_obj {
                    let mut request_map = obj.clone();
                    request_map.insert("content_type".to_string(), serde_json::Value::String(ct.clone()));
                    request_obj = serde_json::Value::Object(request_map);
                }
            }
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": handler_obj,
                    "request": request_obj,
                    "expected_response": {
                        "status_code": http_data.expected_response.status_code,
                        "body": &http_data.expected_response.body,
                        "headers": &http_data.expected_response.headers,
                    }
                }
            });
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json = serde_json::to_string(&fixtures_map).unwrap_or_default();
    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;
    let header = hash::header(CommentStyle::DoubleSlash);

    let imports = e2e_config.harness.imports_for_lang("node");
    let app_class = e2e_config.harness.app_class_for_lang("node");
    let method_enum = &e2e_config.harness.method_enum;
    let run_method = e2e_config.harness.run_method_for_lang("node");
    // Node.js NAPI-RS binding has two route-registration forms:
    // - route() is a single-arg decorator returning a callable
    // - register_route() is a two-arg direct method
    // The harness uses two-arg registration, so always use registerRoute (camelCased).
    let register_method = "registerRoute".to_string();

    // For NAPI-RS bindings (Node.js/WASM), detect the constructor pattern.
    // If imports include "/node" or "wasm", use App.new() factory method.
    // Otherwise, use traditional new App() constructor.
    let constructor_method = if imports.iter().any(|imp| imp.contains("/node") || imp.contains("wasm")) {
        ".new()"
    } else {
        "new"
    };

    let route_builder_class = e2e_config.harness.route_builder.as_deref().unwrap_or("RouteBuilder");

    // Determine which ServerConfig factory expression to use (backend-specific defaults).
    // Node uses `serverConfigDefault()` factory; wasm-bindgen exposes the
    // `WasmServerConfig` class with a default constructor; generic TypeScript
    // bindings fall back to `new ServerConfig()`.
    let factory_lang = if imports.iter().any(|imp| imp.contains("/node")) {
        "node"
    } else if imports.iter().any(|imp| imp.contains("wasm")) {
        "wasm"
    } else {
        "typescript"
    };
    let server_config_factory = e2e_config.harness.server_config_factory_for_lang(factory_lang);
    // Companion import identifier: when the factory is a bare-identifier call,
    // the destructure import must include that identifier.
    let server_config_factory_import = e2e_config
        .harness
        .server_config_factory_import_for_lang(factory_lang)
        .unwrap_or_else(|| "ServerConfig".to_string());
    let import_style = e2e_config.harness.import_style_for_lang(factory_lang);

    crate::e2e::template_env::render(
        "typescript/app_harness.mjs.jinja",
        context! {
            header => header,
            host => host,
            port => port,
            response_body_field => e2e_config.harness.response_body_field.as_str(),
            fixtures_json => fixtures_json,
            imports => imports,
            app_class => app_class.as_deref().unwrap_or("App"),
            method_enum => method_enum.as_deref().unwrap_or("Method"),
            route_builder_class => route_builder_class,
            run_method => run_method.as_deref().unwrap_or("run"),
            register_route_method => register_method.as_str(),
            constructor_method => constructor_method,
            server_config_factory => server_config_factory,
            server_config_factory_import => server_config_factory_import,
            import_style => import_style,
        },
    )
}

pub fn render_global_setup(use_server_pattern: bool) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    let template = if use_server_pattern {
        "typescript/globalSetup_server.ts.jinja"
    } else {
        "typescript/globalSetup.ts.jinja"
    };

    crate::e2e::template_env::render(
        template,
        context! {
            header => header,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_package_json_local_uses_workspace_star() {
        let out = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, None);
        assert!(out.contains("workspace:*"), "got: {out}");
    }

    #[test]
    fn render_package_json_registry_uses_caret_version() {
        let out = render_package_json("my-pkg", "", "1.2.3", DependencyMode::Registry, false, None);
        assert!(out.contains("\"^1.2.3\""), "got: {out}");
    }

    #[test]
    fn render_package_json_registry_prerelease_uses_caret_semver() {
        let out = render_package_json("my-pkg", "", "3.6.0-rc.1", DependencyMode::Registry, false, None);
        assert!(
            out.contains("\"^3.6.0-rc.1\""),
            "pre-release npm pin must include caret, got: {out}"
        );
    }

    #[test]
    fn render_package_json_registry_already_prefixed_passes_through() {
        // When alef.toml's [crates.e2e.registry.packages.node] version field already
        // includes a semver range operator (`^3.6.0-rc.1`), the codegen must use it
        // verbatim — prepending another `^` produces a double-prefix bug.
        let out = render_package_json("my-pkg", "", "^3.6.0-rc.1", DependencyMode::Registry, false, None);
        assert!(
            out.contains("\"^3.6.0-rc.1\""),
            "already-prefixed input must pass through verbatim, got: {out}"
        );
        assert!(!out.contains("^^"), "must not double the `^` prefix, got: {out}");
    }

    #[test]
    fn render_vitest_config_with_global_setup_includes_global_setup_key() {
        let out = render_vitest_config(true, false);
        assert!(out.contains("globalSetup"), "got: {out}");
    }

    #[test]
    fn render_vitest_config_without_global_setup_omits_global_setup_key() {
        let out = render_vitest_config(false, false);
        assert!(!out.contains("globalSetup"), "got: {out}");
    }

    #[test]
    fn render_package_json_includes_harness_extras_dev_dependencies() {
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "tree-sitter".to_string(),
            crate::core::config::manifest_extras::ExtraDepSpec::Simple("^0.25.0".to_string()),
        );
        let out = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, Some(&extras));
        assert!(
            out.contains("\"tree-sitter\": \"^0.25.0\""),
            "tree-sitter dep missing from devDependencies. Got:\n{out}"
        );
        assert!(out.contains("\"vitest\""), "vitest baseline missing. Got:\n{out}");
    }

    #[test]
    fn render_package_json_with_extras_is_idempotent() {
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "tree-sitter".to_string(),
            crate::core::config::manifest_extras::ExtraDepSpec::Simple("^0.25.0".to_string()),
        );
        let first = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, Some(&extras));
        let second = inject_package_json_extras(&first, &extras);
        assert_eq!(first, second, "re-injection should be byte-stable");
    }

    #[test]
    fn render_package_json_extras_runtime_deps_land_in_dependencies() {
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "lodash".to_string(),
            crate::core::config::manifest_extras::ExtraDepSpec::Simple("^4.0.0".to_string()),
        );
        let out = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, Some(&extras));
        // Top-level "dependencies" block should now exist and contain lodash.
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        let lodash = v
            .get("dependencies")
            .and_then(|d| d.get("lodash"))
            .and_then(|s| s.as_str());
        assert_eq!(lodash, Some("^4.0.0"));
    }

    #[test]
    fn render_package_json_extras_overwrite_baseline_on_collision() {
        // If a user pins the same baseline key (e.g. "vitest") to a different version,
        // the harness_extras value wins (last-write-wins).
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "vitest".to_string(),
            crate::core::config::manifest_extras::ExtraDepSpec::Simple("^2.0.0".to_string()),
        );
        let out = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, Some(&extras));
        assert!(
            out.contains("\"vitest\": \"^2.0.0\""),
            "vitest pin not overridden. Got:\n{out}"
        );
        assert!(
            !out.contains(&format!("\"vitest\": \"{}\"", tv::npm::VITEST)),
            "default vitest leaked. Got:\n{out}"
        );
    }

    #[test]
    fn render_package_json_empty_extras_matches_no_extras() {
        let extras = ManifestExtras::default();
        let with_empty = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, Some(&extras));
        let without = render_package_json("my-pkg", "", "1.0.0", DependencyMode::Local, false, None);
        assert_eq!(with_empty, without);
    }

    #[test]
    fn render_global_setup_mock_server_waits_for_mock_server_shutdown() {
        let out = render_global_setup(false);
        assert!(out.contains("clearTimeout(startupTimeout)"), "got: {out}");
        assert!(out.contains("proc.stdout.off('data', onData)"), "got: {out}");
        assert!(out.contains("const proc = serverProcess;"), "got: {out}");
        assert!(out.contains("await new Promise<void>"), "got: {out}");
        assert!(out.contains("child.once('close'"), "got: {out}");
        assert!(out.contains("child.kill('SIGKILL')"), "got: {out}");
    }

    #[test]
    fn render_global_setup_mock_server_honors_preset_mock_server_url() {
        let out = render_global_setup(false);
        // When MOCK_SERVER_URL is pre-set by the test runner, reuse it and skip
        // spawning the local binary. The early return must precede the spawn().
        assert!(
            out.contains("const presetUrl = process.env.MOCK_SERVER_URL ?? process.env.SUT_URL;"),
            "globalSetup must short-circuit on a pre-set MOCK_SERVER_URL, got: {out}"
        );
        let guard = out.find("if (presetUrl)").expect("guard present");
        let spawn = out.find("spawn(").expect("spawn present");
        assert!(
            guard < spawn,
            "the pre-set MOCK_SERVER_URL guard must come before the spawn() call, got: {out}"
        );
    }

    #[test]
    fn render_global_setup_server_pattern_polls_for_tcp_readiness() {
        let out = render_global_setup(true);
        assert!(out.contains("SUT_URL"), "server-pattern should use SUT_URL");
        assert!(
            out.contains("app_harness.mjs"),
            "server-pattern should spawn app_harness.mjs"
        );
        assert!(
            out.contains("createConnection"),
            "server-pattern should check TCP readiness"
        );
    }
}
