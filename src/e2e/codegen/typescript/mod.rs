//! TypeScript e2e test generator using vitest.

mod assertions;
pub(crate) mod config;
mod json;
mod test_file;
mod visitors;

use crate::e2e::config::E2eConfig;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::path::PathBuf;

use super::E2eCodegen;
use crate::codegen::generators::trait_bridge::to_camel_case;
use config::{render_file_setup, render_global_setup, render_package_json, render_tsconfig, render_vitest_config};
pub use test_file::render_test_file;
use test_file::resolve_node_function_name;

/// TypeScript e2e code generator.
pub struct TypeScriptCodegen;

impl E2eCodegen for TypeScriptCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let output_base = PathBuf::from(e2e_config.effective_output()).join(self.language_name());
        let tests_base = output_base.join("tests");

        let mut files = Vec::new();

        // Resolve call config with overrides — use "node" key (Language::Node).
        let call = &e2e_config.call;
        let overrides = call.overrides.get("node");
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides.and_then(|o| o.function.as_ref()).cloned().unwrap_or_else(|| {
            let default_cc = e2e_config.resolve_call(None);
            resolve_node_function_name(default_cc)
        });
        let client_factory = overrides.and_then(|o| o.client_factory.as_deref());

        // Resolve package config.
        let node_pkg = e2e_config.resolve_package("node");
        let pkg_path = node_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/node".to_string());
        let pkg_name = node_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_version = node_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());

        // Server-pattern app_harness.mjs (spinning up the SUT as an HTTP server)
        // is emitted by a consumer extension via Extension::emit_e2e —
        // server-pattern e2e is not generic, so alef does not generate it.

        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(
                &pkg_name,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
                has_http_fixtures,
                // `harness_extras` deps support the alef-generated e2e harness code under
                // `e2e/{lang}/tests/` (Local dep mode). Registry mode emits the published-package
                // test_apps at `test_apps/{lang}/` whose tests only import the under-test package
                // and never need harness-specific dev deps. Injecting harness_extras here drags
                // unused native deps (e.g. upstream `tree-sitter`) into pnpm install, which can
                // break on newer Node SDKs that the unrelated native build doesn't support yet.
                match e2e_config.dep_mode {
                    crate::e2e::config::DependencyMode::Local => e2e_config.harness_extras.get(self.language_name()),
                    crate::e2e::config::DependencyMode::Registry => None,
                },
            ),
            generated_header: false,
        });

        files.push(GeneratedFile {
            path: output_base.join("tsconfig.json"),
            content: render_tsconfig(),
            generated_header: false,
        });

        // Emit an `.npmrc` that disables frozen-lockfile for the e2e install.
        // A napi binding declares its platform binaries as optionalDependencies
        // pinned to the exact (not-yet-published) release version, so before that
        // version is on the registry a frozen `pnpm install` cannot resolve them
        // and fails with ERR_PNPM_OUTDATED_LOCKFILE — and pnpm refuses to add the
        // unresolvable optional specifiers to the lockfile, so it can never be
        // pre-satisfied. pnpm defaults frozen-lockfile to true in CI, so opt out
        // here; the e2e project is a disposable test harness, not a
        // reproducibility gate. pnpm reads `.npmrc` from the install cwd
        // (`cd e2e/<lang> && pnpm install`), so this scopes the override to e2e.
        files.push(GeneratedFile {
            path: output_base.join(".npmrc"),
            content: "; alef-generated — frozen-lockfile is disabled because the under-test napi\n; package pins platform optionalDependencies to the unpublished release version.\nfrozen-lockfile=false\n".to_string(),
            generated_header: false,
        });

        // Emit a sealed `pnpm-workspace.yaml` ONLY in Registry mode to isolate
        // the test app's devDependencies (vitest, etc.) from the outer workspace.
        // Without it, pnpm hoists/dedupes transitive deps against any parent
        // workspace and `vitest` ends up uninstalled locally, breaking e2e tests.
        //
        // In Local mode, the test app depends on the binding via `workspace:*`,
        // which MUST resolve through the consumer's root pnpm workspace. Emitting
        // the sealed marker would shadow the root and cause `pnpm install` to fail:
        // `ERR_PNPM_NO_MATCHING_VERSION_INSIDE_WORKSPACE`.
        //
        // For cleanup: if in Local mode, explicitly tell cleanup to remove any
        // stale pnpm-workspace.yaml (with or without alef header). This handles
        // the migration from Registry to Local mode, even if prior Registry-mode
        // emissions lacked the generated header.
        let workspace_yaml_path = output_base.join("pnpm-workspace.yaml");
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            // Registry mode: emit with generated header for proper cleanup tracking
            files.push(GeneratedFile {
                path: workspace_yaml_path.clone(),
                content: "packages: []\nallowBuilds:\n  esbuild: true\n  tree-sitter: true\n".to_string(),
                generated_header: true,
            });
        } else {
            // Local mode: explicitly emit a cleanup marker to remove the file if
            // it exists, even without an alef header. This handles stale files
            // from prior Registry-mode runs or manual creation.
            if workspace_yaml_path.exists() {
                std::fs::remove_file(&workspace_yaml_path)
                    .map_err(|e| anyhow::anyhow!("Failed to remove stale pnpm-workspace.yaml: {}", e))?;
            }
        }

        // globalSetup spawns the mock-server binary and exposes its URL via
        // MOCK_SERVER_URL. Required whenever any fixture's call uses the mock
        // server — either via http blocks (real HTTP test fixtures), via
        // mock_response/mock_responses (function-call tests that build their
        // own URLs against MOCK_SERVER_URL), or because a client_factory is
        // wired to point at the mock server's `/fixtures/<id>` prefix.
        let any_needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());
        let needs_global_setup = client_factory.is_some() || has_http_fixtures || any_needs_mock_server;

        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(needs_global_setup, has_file_fixtures),
            generated_header: true,
        });

        // The server-pattern globalSetup (which spawns app_harness) is emitted by
        // a consumer extension. alef emits only the client/mock-server
        // globalSetup; when a server harness is configured the extension owns both
        // app_harness.mjs and its globalSetup.ts.
        let use_server_pattern = has_http_fixtures && !e2e_config.harness.imports.is_empty();
        if needs_global_setup && !use_server_pattern {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(false),
                generated_header: true,
            });
        }

        if has_file_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("setup.ts"),
                content: render_file_setup(&e2e_config.test_documents_dir),
                generated_header: true,
            });
        }

        let options_type = overrides.and_then(|o| o.options_type.clone());

        for group in groups {
            let active: Vec<_> = group
                .fixtures
                .iter()
                .filter(|fixture| is_node_fixture_runnable(fixture))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}.test.ts", crate::e2e::escape::sanitize_filename(&group.category));
            let content = render_test_file(
                "node",
                &group.category,
                &active,
                &module_path,
                &pkg_name,
                &function_name,
                &e2e_config.call.args,
                options_type.as_deref(),
                client_factory,
                e2e_config,
                type_defs,
                enums,
                "",
                config,
            );
            files.push(GeneratedFile {
                path: tests_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "node"
    }
}

fn is_node_fixture_runnable(fixture: &Fixture) -> bool {
    if fixture.skip.as_ref().is_some_and(|skip| skip.should_skip("node")) {
        return false;
    }

    if let Some(http) = &fixture.http {
        return http.expected_response.status_code != 101;
    }

    !fixture.assertions.is_empty()
}

/// Emit a TypeScript/Node/WASM test backend stub for a trait-bridge fixture.
///
/// Generates a duck-typed TypeScript class `_TestStub_<fixture_id>` whose methods
/// return sensible default values via `Promise.resolve(...)` for async methods and
/// plain values for sync ones. When `super_trait` is set, a `name()` method is
/// emitted returning the fixture's name string extracted from `fixture.input`.
///
/// TypeScript/NAPI-RS bridges use duck typing — the class just needs to provide
/// the right methods that the NAPI-RS bridge's `register<Trait>` function can call.
///
/// The `language` parameter distinguishes `"node"` / `"typescript"` / `"wasm"` so
/// the emitter can use the correct `register` call convention (camelCase for JS).
///
/// The returned `arg_expr` is `new _TestStub_<fixture_id>()`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::{escape_js, sanitize_ident};
    use std::fmt::Write as FmtWrite;

    let stub_name = format!("_TestStub_{}", sanitize_ident(&fixture.id));
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let defaults = language_defaults("typescript");

    // Extract the backend input block if present (e.g., fixture.input.backend).
    // Used to populate method defaults like dimensions(), backend name, etc.
    let backend_input = fixture.input.get("backend").and_then(|v| v.as_object());

    let mut setup = String::new();

    let _ = writeln!(setup, "class {stub_name} {{");

    // name() from Plugin super-trait, if configured.
    if trait_bridge.super_trait.is_some() {
        let escaped = escape_js(&backend_name);
        let _ = writeln!(setup, "  name(): string {{ return \"{escaped}\"; }}");
    }

    // Emit all methods the high-level binding interface may call. Methods with
    // Rust default impls are optional in user code, but generated e2e stubs
    // provide no-op/default implementations for lifecycle probes.
    for method in methods {
        // Skip Plugin::name if we already emitted it.
        if trait_bridge.super_trait.is_some() && method.name == "name" {
            continue;
        }
        emit_ts_stub_method(&mut setup, method, &*defaults, backend_input);
    }

    // Emit dispose() method for cleanup. The test cleanup code calls dispose()
    // on trait bridge stubs to signal test teardown. This is a no-op in test stubs.
    let _ = writeln!(setup, "  async dispose(): Promise<void> {{ return undefined; }}");

    let _ = writeln!(setup, "}}");

    let arg_expr = format!("new {stub_name}()");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}

/// Format a single TypeScript stub method.
///
/// Async methods return `Promise.resolve(<default>)`. Sync methods return the
/// default value directly. All parameters are elided with `_p0`, `_p1`, ...
/// prefixes (TypeScript allows unused parameters when prefixed with `_`).
///
/// Method defaults are extracted from fixture.input.backend first, then fall
/// back to language defaults. For numeric types that would emit 0, emit 1
/// instead (downstream validation rejects 0 for counts like dimensions()).
fn emit_ts_stub_method(
    out: &mut String,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    backend_input: Option<&serde_json::Map<String, serde_json::Value>>,
) {
    use std::fmt::Write as FmtWrite;

    // Build parameter list: `_p0?: any, _p1?: any, ...`
    let params: Vec<String> = method
        .params
        .iter()
        .enumerate()
        .map(|(i, _)| format!("_p{i}?: any"))
        .collect();
    let params_str = params.join(", ");

    // Named types in e2e stubs must return JSON-serialisable strings: the NAPI-RS
    // bridge calls the JS method, coerces the return value to a string via
    // `coerce_to_string()`, then parses it as JSON into the Rust type.  Returning
    // `new TypeName()` would produce a JS object whose `.toString()` yields
    // `"[object Object]"` — not valid JSON — causing a deserialization error in the
    // bridge.  Return the string literal `"{}"` instead; it round-trips cleanly
    // through `serde_json` as an empty object.
    //
    // For numeric types in test backends, use 1 instead of 0 to satisfy validation
    // constraints (e.g., EmbeddingBackend::dimensions() must return > 0).
    let default_val = match &method.return_type {
        crate::core::ir::TypeRef::Unit => "undefined".to_string(),
        crate::core::ir::TypeRef::Named(_) => "\"{}\"".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "false".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F32) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F64) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(_) => {
            // Try to extract fixture value first, then fall back to 1 instead of 0
            let fixture_val = backend_input
                .and_then(|b| b.get(&method.name.to_lowercase()))
                .or_else(|| backend_input.and_then(|b| b.get(&to_camel_case(&method.name))));

            if let Some(val) = fixture_val {
                // Emit the fixture value directly (primitives, numbers, etc.)
                match val {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => {
                        // Complex types: fall back to 1 for numeric
                        "1".to_string()
                    }
                }
            } else {
                // No fixture value: emit 1 instead of 0
                "1".to_string()
            }
        }
        other => defaults.emit_default(other),
    };
    let return_type = ts_stub_return_type(&method.return_type);

    if method.is_async {
        let _ = writeln!(
            out,
            "  async {name}({params_str}): Promise<{return_type}> {{ return {default_val}; }}",
            name = to_camel_case(&method.name)
        );
    } else {
        let _ = writeln!(
            out,
            "  {name}({params_str}): {return_type} {{ return {default_val}; }}",
            name = to_camel_case(&method.name)
        );
    }
}

fn ts_stub_return_type(return_type: &crate::core::ir::TypeRef) -> &'static str {
    match return_type {
        crate::core::ir::TypeRef::Unit => "void",
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "boolean",
        crate::core::ir::TypeRef::Primitive(_) => "number",
        _ => "string",
    }
}

/// Extract a backend name string from the fixture input JSON.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Build a minimal `MethodDef` for tests.
#[cfg(test)]
fn test_method(
    name: &str,
    return_type: crate::core::ir::TypeRef,
    is_async: bool,
    has_default_impl: bool,
) -> crate::core::ir::MethodDef {
    crate::core::ir::MethodDef {
        name: name.to_string(),
        params: Vec::new(),
        return_type,
        is_async,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fixture(id: &str, input: serde_json::Value) -> crate::e2e::fixture::Fixture {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "description": "test fixture",
            "input": input,
            "assertions": []
        }))
        .expect("minimal fixture JSON must parse")
    }

    #[test]
    fn language_name_is_node() {
        let codegen = TypeScriptCodegen;
        assert_eq!(codegen.language_name(), "node");
    }

    #[test]
    fn generate_empty_groups_produces_config_files_only() {
        use crate::core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
        )
        .unwrap();
        let mut e2e = cfg.crates[0].e2e.clone().unwrap();
        e2e.dep_mode = crate::e2e::config::DependencyMode::Registry;
        let resolved = cfg.resolve().unwrap().remove(0);
        let codegen = TypeScriptCodegen;
        let files = codegen.generate(&[], &e2e, &resolved, &[], &[]).unwrap();
        // package.json, tsconfig.json, vitest.config.ts, pnpm-workspace.yaml
        assert!(files.len() >= 3, "got {} files", files.len());

        // The node test app must emit its own pnpm-workspace.yaml in Registry
        // mode so that `pnpm install` does not sweep it into an outer
        // workspace and skip installing its devDependencies (vitest) locally.
        let workspace = files
            .iter()
            .find(|f| f.path.ends_with("pnpm-workspace.yaml"))
            .expect("node codegen in Registry mode must emit pnpm-workspace.yaml for install isolation");
        assert!(
            workspace.content.contains("packages:"),
            "pnpm-workspace.yaml must declare an isolated workspace root, got: {}",
            workspace.content
        );

        // The node e2e app must disable frozen-lockfile: a napi binding pins its
        // platform binaries as optionalDependencies to the unpublished release
        // version, which a frozen `pnpm install` cannot resolve pre-publish.
        let npmrc = files
            .iter()
            .find(|f| f.path.ends_with(".npmrc"))
            .expect("node codegen must emit .npmrc to disable frozen-lockfile for the e2e install");
        assert!(
            npmrc.content.contains("frozen-lockfile=false"),
            "e2e .npmrc must set frozen-lockfile=false, got: {}",
            npmrc.content
        );
    }

    #[test]
    fn local_mode_does_not_emit_pnpm_workspace_yaml() {
        use crate::core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
        )
        .unwrap();
        // Default dep_mode is Local (not Registry)
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        assert_eq!(e2e.dep_mode, crate::e2e::config::DependencyMode::Local);
        let resolved = cfg.resolve().unwrap().remove(0);
        let codegen = TypeScriptCodegen;
        let files = codegen.generate(&[], &e2e, &resolved, &[], &[]).unwrap();

        // In Local mode, pnpm-workspace.yaml must NOT be emitted. The root
        // workspace will handle e2e/node naturally, and workspace:* deps will
        // resolve correctly through the parent workspace.
        let workspace = files.iter().find(|f| f.path.ends_with("pnpm-workspace.yaml"));
        assert!(
            workspace.is_none(),
            "Local mode must NOT emit pnpm-workspace.yaml; found: {:?}",
            workspace
        );
    }

    #[test]
    fn emit_test_backend_ts_generates_class_and_new_expr() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let m1 = test_method("syncOp", TypeRef::String, false, false);
        let m2 = test_method("asyncOp", TypeRef::Named("WorkResult".to_string()), true, false);
        let methods = [&m1, &m2];

        let fixture = make_fixture("ts_test_fixture", serde_json::json!({ "name": "my-ts-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // setup_block must define a TS class.
        assert!(
            emission.setup_block.contains("class _TestStub_ts_test_fixture"),
            "setup_block should define the stub class, got: {}",
            emission.setup_block
        );
        // Must NOT hardcode sample_core-domain trait names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode OcrBackend"
        );
        assert!(
            !emission.setup_block.contains("DocumentExtractor"),
            "setup_block must not hardcode DocumentExtractor"
        );

        // name() emitted because super_trait is set.
        assert!(
            emission.setup_block.contains("name()"),
            "setup_block should emit name() method"
        );
        assert!(
            emission.setup_block.contains("my-ts-backend"),
            "name() should return the backend name"
        );

        // Required methods emitted.
        assert!(
            emission.setup_block.contains("syncOp("),
            "required sync method should be emitted"
        );
        assert!(
            emission.setup_block.contains("async asyncOp("),
            "required async method should be emitted with async keyword"
        );
        assert!(
            emission.setup_block.contains("syncOp(): string"),
            "sync method should return the generated sync shape, got: {}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("async asyncOp(): Promise<string>"),
            "async method should return the generated async shape, got: {}",
            emission.setup_block
        );

        // arg_expr uses new keyword.
        assert_eq!(
            emission.arg_expr, "new _TestStub_ts_test_fixture()",
            "arg_expr should use new constructor"
        );

        // Named return type must use "{}" not new WorkResult().
        assert!(
            emission.setup_block.contains("return \"{}\";"),
            "Named return type should emit \"{{}}\" not a constructor call, got: {}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("new WorkResult()"),
            "Named return type must not emit a constructor call, got: {}",
            emission.setup_block
        );
    }

    #[test]
    fn emit_test_backend_ts_extracts_fixture_values_for_numeric_defaults() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("OcrBackend".to_string()),
            ..Default::default()
        };

        let m1 = test_method("dimensions", TypeRef::Primitive(PrimitiveType::U32), false, false);
        let m2 = test_method("model", TypeRef::String, false, false);
        let methods = [&m1, &m2];

        // Fixture with backend input containing dimensions value
        let fixture = make_fixture(
            "embedding_fixture",
            serde_json::json!({
                "name": "my-embedder",
                "backend": {
                    "dimensions": 768,
                    "model": "all-MiniLM-L6-v2"
                }
            }),
        );

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // Should extract dimensions: 768 from fixture.input.backend
        assert!(
            emission.setup_block.contains("dimensions(): number { return 768; }"),
            "numeric method should extract value from fixture.input.backend, got: {}",
            emission.setup_block
        );

        // TODO: extend extraction to string return types (currently only numeric values
        // are pulled from fixture.input.backend; string defaults still emit "").
    }

    #[test]
    fn emit_test_backend_ts_emits_default_impl_noops() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            ..Default::default()
        };

        let required = test_method("mustImplement", TypeRef::String, false, false);
        let optional = test_method("mayImplement", TypeRef::String, false, true);
        let methods = [&required, &optional];

        let fixture = make_fixture("ts_skip_defaults", serde_json::json!({}));
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("mustImplement("),
            "required method should be emitted"
        );
        assert!(
            emission.setup_block.contains("mayImplement("),
            "default-impl method should be emitted as a no-op stub"
        );
    }
}
