//! Rust e2e test code generator.
//!
//! Generates `e2e/rust/Cargo.toml` and `tests/{category}_test.rs` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

pub mod assertions;
pub mod cargo_toml;
pub mod http;
pub mod mock_server;
pub mod test_file;

mod args;
mod assertion_helpers;
mod assertion_synthetic;

pub use cargo_toml::render_cargo_toml;
pub use mock_server::{render_common_module, render_mock_server_binary, render_mock_server_module};

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use anyhow::Result;
use std::path::PathBuf;

use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};

use super::E2eCodegen;
use test_file::{is_skipped, render_test_file};

/// Rust e2e test code generator.
pub struct RustE2eCodegen;

impl E2eCodegen for RustE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("rust");

        // Resolve crate name and path from config.
        let crate_name = resolve_crate_name(e2e_config, config);
        let crate_path = resolve_crate_path(e2e_config, &crate_name);
        let dep_name = crate_name.replace('-', "_");

        // Cargo.toml
        // Check if any call config (default or named) uses json_object/handle args (needs serde_json dep).
        let all_call_configs = std::iter::once(&e2e_config.call).chain(e2e_config.calls.values());
        let needs_serde_json = all_call_configs
            .flat_map(|c| c.args.iter())
            .any(|a| a.arg_type == "json_object" || a.arg_type == "handle");

        // Check if any fixture in any group requires a mock HTTP server.
        // This includes both liter-llm mock_response fixtures and spikard http fixtures.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.needs_mock_server());

        // Check if any fixture uses the http integration test pattern (spikard http fixtures).
        let needs_http_tests = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.http.is_some());

        // Check if any http fixture uses CORS or static-files middleware (needs tower-http).
        let needs_tower_http = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .filter(|f| !is_skipped(f, "rust"))
            .filter_map(|f| f.http.as_ref())
            .filter_map(|h| h.handler.middleware.as_ref())
            .any(|m| m.cors.is_some() || m.static_files.is_some());

        // Tokio is needed when any test is async (mock server, http tests, or async call config).
        let any_async_call = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .any(|c| c.r#async);
        let needs_tokio = needs_mock_server || needs_http_tests || any_async_call;

        let crate_version = resolve_crate_version(e2e_config).or_else(|| config.resolved_version());
        files.push(GeneratedFile {
            path: output_base.join("Cargo.toml"),
            content: render_cargo_toml(
                &crate_name,
                &dep_name,
                &crate_path,
                needs_serde_json,
                needs_mock_server,
                needs_http_tests,
                needs_tokio,
                needs_tower_http,
                e2e_config.dep_mode,
                crate_version.as_deref(),
                &config.features,
            ),
            generated_header: true,
        });

        // Generate mock_server.rs when at least one fixture uses mock_response.
        if needs_mock_server {
            files.push(GeneratedFile {
                path: output_base.join("tests").join("mock_server.rs"),
                content: render_mock_server_module(),
                generated_header: true,
            });
            // Generate common.rs module for spawning the standalone mock-server binary.
            files.push(GeneratedFile {
                path: output_base.join("tests").join("common.rs"),
                content: render_common_module(),
                generated_header: true,
            });
        }
        // Always generate standalone mock-server binary for cross-language e2e suites
        // when any fixture has http data (serves fixture responses for non-Rust tests).
        if needs_mock_server || needs_http_tests {
            files.push(GeneratedFile {
                path: output_base.join("src").join("main.rs"),
                content: render_mock_server_binary(),
                generated_header: true,
            });
        }

        // Per-category test files.
        for group in groups {
            let fixtures: Vec<&Fixture> = group.fixtures.iter().filter(|f| !is_skipped(f, "rust")).collect();

            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("{}_test.rs", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config, &dep_name, needs_mock_server);

            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "rust"
    }
}

// ---------------------------------------------------------------------------
// Config resolution helpers
// ---------------------------------------------------------------------------

fn resolve_crate_name(_e2e_config: &E2eConfig, config: &ResolvedCrateConfig) -> String {
    // Always use the Cargo package name (with hyphens) from alef.toml [crate].
    // The `crate_name` override in [e2e.call.overrides.rust] is for the Rust
    // import identifier, not the Cargo package name.
    config.name.clone()
}

fn resolve_crate_path(e2e_config: &E2eConfig, crate_name: &str) -> String {
    e2e_config
        .resolve_package("rust")
        .and_then(|p| p.path.clone())
        .unwrap_or_else(|| format!("../../crates/{crate_name}"))
}

fn resolve_crate_version(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config.resolve_package("rust").and_then(|p| p.version.clone())
}

/// Emit a Rust test backend stub for a trait-bridge fixture.
///
/// Generates a minimal `struct _TestStub_<fixture_id>` with a `_name` field and
/// a concrete `impl <trait_name> for _TestStub_<fixture_id>` block where every
/// required method returns a language-default value. When the bridge config
/// declares a `super_trait`, a `name()` method is also emitted returning the
/// fixture's name string extracted from `fixture.input`.
///
/// The returned `arg_expr` wraps the stub in `std::sync::Arc::new(...)`, which
/// is the form expected by the generated `register_<trait>` function.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::sanitize_ident;
    use std::fmt::Write as FmtWrite;

    let stub_name = format!("_TestStub_{}", sanitize_ident(&fixture.id));
    let trait_name = &trait_bridge.trait_name;
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let defaults = language_defaults("rust");

    let mut setup = String::new();

    // Struct definition with a cached name field.
    let _ = writeln!(setup, "struct {stub_name} {{ _name: &'static str }}");

    // Impl block.
    let _ = writeln!(setup, "impl {trait_name} for {stub_name} {{");

    // name() from Plugin super-trait, if configured.
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "    fn name(&self) -> &str {{ self._name }}");
    }

    // Required methods only (skip those with a default implementation).
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Skip the Plugin::name method if we already emitted it above.
        if trait_bridge.super_trait.is_some() && method.name == "name" {
            continue;
        }
        emit_rust_stub_method(&mut setup, method, &*defaults);
    }

    let _ = writeln!(setup, "}}");

    // arg_expr: wrapped in Arc for the register call.
    let arg_expr = format!("std::sync::Arc::new({stub_name} {{ _name: \"{backend_name}\" }})");

    super::TestBackendEmission { setup_block: setup, arg_expr }
}

/// Format a single Rust stub method.
///
/// Emits a method body using `LanguageDefaults` for the return value. The method
/// signature omits parameter names (uses `_` wildcards) since the stub never uses
/// them. For fallible methods the default is wrapped in `Ok(...)`.
///
/// The Rust compiler infers the concrete return type from the trait definition, so
/// `Default::default()` resolves correctly for value types. For reference-returning
/// methods (e.g. `-> &[&str]`), the emitted default may not be type-compatible —
/// in that case the body falls back to `unimplemented!()` which satisfies any return
/// type via the never type `!`. Phase 3 fixtures can provide richer stub bodies when
/// they actually call these methods.
fn emit_rust_stub_method(
    out: &mut String,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) {
    use crate::core::ir::TypeRef;
    use std::fmt::Write as FmtWrite;

    // Elide all parameters with `_` wildcards so the stub compiles regardless
    // of whether the parameter types implement Default/Clone.
    let param_wildcards: Vec<&str> = method.params.iter().map(|_| "_").collect();
    let params_str = if param_wildcards.is_empty() {
        String::new()
    } else {
        format!(", {}", param_wildcards.join(", "))
    };

    // For reference-returning methods (`returns_ref = true`), the IR collapses
    // `&[T]` into `Vec<T>` + flag. A `Default::default()` value can't be returned
    // as a reference without a named binding that outlives the method body, so we
    // emit `unimplemented!()` for those. All other return types get the LanguageDefaults
    // value (wrapped in Ok(...) for fallible methods).
    let body = if method.returns_ref {
        "unimplemented!()".to_string()
    } else {
        // For Unit / async with Unit, the Rust default "()" works.
        // For Named types, "TypeName::default()" works when Default is derived.
        // For primitives/String/Vec/etc., the defaults are concrete Rust expressions.
        let raw = match &method.return_type {
            TypeRef::Unit => "()".to_string(),
            _ => defaults.emit_default(&method.return_type),
        };
        if method.error_type.is_some() {
            format!("Ok({raw})")
        } else {
            raw
        }
    };

    let async_kw = if method.is_async { "async " } else { "" };
    let _ = writeln!(
        out,
        "    {async_kw}fn {name}(&self{params_str}) {{ {body} }}",
        name = method.name
    );
}

/// Extract a backend name string from the fixture input JSON.
///
/// Searches the top-level input object for the first string value at any depth
/// under keys commonly used for names (`name`, or the first string field found).
/// Falls back to the fixture id when no string is found.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    // Walk the top-level object, then one level deeper, looking for "name".
    if let Some(obj) = input.as_object() {
        // Direct "name" key.
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        // One level deeper in any nested object.
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        // First string value at the top level.
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Build a minimal `MethodDef` for use in unit tests.
#[cfg(test)]
fn test_method(
    name: &str,
    return_type: crate::core::ir::TypeRef,
    is_async: bool,
    error_type: Option<&str>,
) -> crate::core::ir::MethodDef {
    crate::core::ir::MethodDef {
        name: name.to_string(),
        params: Vec::new(),
        return_type,
        is_async,
        is_static: false,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
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
    fn resolve_crate_name_uses_config_name() {
        use crate::core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my_lib"
result_var = "result"
"#,
        )
        .unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let name = resolve_crate_name(&e2e, &resolved);
        assert_eq!(name, "my-lib");
    }

    #[test]
    fn emit_test_backend_rust_generates_struct_and_arc_expr() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };

        let m1 = test_method("do_work", TypeRef::String, false, None);
        let m2 = test_method("process_async", TypeRef::Named("WorkResult".to_string()), true, Some("WorkError"));
        let methods = [&m1, &m2];

        let fixture = make_fixture(
            "my_test_fixture",
            serde_json::json!({ "name": "my-test-backend" }),
        );

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // setup_block must contain the stub struct and impl.
        assert!(
            emission.setup_block.contains("_TestStub_my_test_fixture"),
            "setup_block should contain stub name, got: {}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("TestTrait"),
            "setup_block should reference trait by name, got: {}",
            emission.setup_block
        );
        // Must NOT hardcode any kreuzberg-domain trait name.
        assert!(!emission.setup_block.contains("OcrBackend"), "setup_block must not hardcode OcrBackend");
        assert!(
            !emission.setup_block.contains("DocumentExtractor"),
            "setup_block must not hardcode DocumentExtractor"
        );

        // name() emitted because super_trait is Some.
        assert!(
            emission.setup_block.contains("fn name("),
            "setup_block should emit name() when super_trait is set"
        );

        // Required methods emitted.
        assert!(
            emission.setup_block.contains("fn do_work("),
            "required method do_work should be in setup_block"
        );
        assert!(
            emission.setup_block.contains("fn process_async("),
            "required async method process_async should be in setup_block"
        );

        // arg_expr wraps in Arc::new.
        assert!(emission.arg_expr.contains("Arc::new"), "arg_expr should use Arc::new, got: {}", emission.arg_expr);
        assert!(
            emission.arg_expr.contains("_TestStub_my_test_fixture"),
            "arg_expr should reference stub struct, got: {}",
            emission.arg_expr
        );
    }

    #[test]
    fn emit_test_backend_rust_skips_default_impl_methods() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            ..Default::default()
        };

        let required = test_method("required_method", TypeRef::String, false, None);
        let mut optional = test_method("optional_method", TypeRef::String, false, None);
        optional.has_default_impl = true;
        let methods = [&required, &optional];

        let fixture = make_fixture("skip_defaults_fixture", serde_json::json!({}));
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(emission.setup_block.contains("fn required_method("), "required method should be emitted");
        assert!(!emission.setup_block.contains("fn optional_method("), "method with default impl should be skipped");
    }

    #[test]
    fn emit_test_backend_rust_name_extracted_from_input() {
        use crate::core::config::TraitBridgeConfig;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let fixture = make_fixture(
            "name_extraction_fixture",
            serde_json::json!({ "backend": { "name": "extracted-name" } }),
        );

        let emission = emit_test_backend(&bridge, &[], &fixture);

        assert!(
            emission.arg_expr.contains("extracted-name"),
            "arg_expr should contain the name from input.backend.name, got: {}",
            emission.arg_expr
        );
    }
}
