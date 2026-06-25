//! Python e2e test code generator.
//!
//! Generates `e2e/python/conftest.py` and `tests/test_{category}.py` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

mod assertions;
mod config;
mod helpers;
mod http;
mod json;
mod test_file;
mod test_function;
mod visitors;

use std::path::PathBuf;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;

use self::config::{render_conftest, render_pyproject};
use self::helpers::is_skipped;
use self::test_file::render_test_file;

/// Python e2e test code generator.
pub struct PythonE2eCodegen;

impl super::E2eCodegen for PythonE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("python");

        // NOTE: app_harness.py and the server-pattern conftest.py are emitted
        // by the spikard-e2e-http extension (Extension::emit_e2e "python" arm).
        // alef emits only the non-server-pattern conftest here.

        files.push(GeneratedFile {
            path: output_base.join("conftest.py"),
            content: render_conftest(e2e_config, groups),
            generated_header: true,
        });

        // NOTE: do NOT emit a root-level `__init__.py` in the test_app dir.
        // Marking `test_apps/python/` as a Python package interferes with
        // `uv sync` installing the published wheel — the editable install of
        // the e2e project picks up the root `__init__.py` and pytest then
        // resolves the generated package import against an empty local namespace,
        // missing the actual package `__init__.py` shipped in the wheel.
        // The conftest + tests/ subdirectory are sufficient for pytest.
        files.push(GeneratedFile {
            path: output_base.join("tests").join("__init__.py"),
            content: "\n".to_string(),
            generated_header: false,
        });

        let python_pkg = e2e_config.resolve_package("python");
        let default_pkg_name = e2e_config.call.module.replace('_', "-");
        let pkg_name = python_pkg
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .unwrap_or(default_pkg_name.as_str());
        let pkg_path = python_pkg
            .as_ref()
            .and_then(|p| p.path.as_deref())
            .unwrap_or("../../packages/python");
        // Resolve registry pin: explicit per-package override → workspace
        // version → 0.1.0 fallback. `render_pyproject` normalises bare
        // versions to `==<version>` so consumers can pin with bare strings
        // (`"1.4.0-rc.30"`) and still produce a valid PEP 508 requirement;
        // qualified specifiers (`">=1.2"`, `"~=2.0"`) pass through.
        let resolved = config.resolved_version();
        let owned_version: String = python_pkg
            .as_ref()
            .and_then(|p| p.version.as_deref())
            .map(str::to_owned)
            .or_else(|| resolved.as_ref().map(|v| v.to_string()))
            .unwrap_or_else(|| "0.1.0".to_string());
        files.push(GeneratedFile {
            path: output_base.join("pyproject.toml"),
            content: render_pyproject(pkg_name, pkg_path, &owned_version, e2e_config.dep_mode),
            generated_header: true,
        });

        for group in groups {
            let fixtures: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|fixture| is_python_fixture_runnable(fixture))
                .collect();
            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("test_{}.py", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config, config, _type_defs, _enums);
            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        // Registry-mode test_apps/ runners (e.g. a consumer's
        // `task smoke:python` step) invoke a fixed `uv run pytest
        // tests/test_smoke.py` smoke target by convention. Emit a minimal
        // smoke test whenever no `smoke` fixture category is present so the
        // runner does not error on a missing path.
        //
        // The emitted file just imports the published package — a true smoke
        // test that catches packaging regressions (missing wheels, broken
        // native extension, import-time errors) without depending on any
        // specific binding API.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            let smoke_path = output_base.join("tests").join("test_smoke.py");
            let has_smoke_emitted = files.iter().any(|f| f.path == smoke_path);
            if !has_smoke_emitted {
                files.push(GeneratedFile {
                    path: smoke_path,
                    content: render_python_smoke_test(pkg_name),
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "python"
    }
}

/// Render a minimal smoke test importing the published Python package.
///
/// The test asserts the module imports cleanly — a regression here points
/// at a packaging fault (missing wheel for platform, broken native
/// extension, import-time exception) rather than a binding-API issue.
fn render_python_smoke_test(pip_name: &str) -> String {
    use crate::core::hash::{self, CommentStyle};

    let header = hash::header(CommentStyle::Hash);

    let ctx = minijinja::context! {
        header => header,
        pip_name => pip_name,
    };

    crate::e2e::template_env::render("python/test_smoke.py.jinja", ctx)
}

fn is_python_fixture_runnable(fixture: &Fixture) -> bool {
    if is_skipped(fixture, "python") {
        return false;
    }

    if let Some(http) = &fixture.http {
        return http.expected_response.status_code != 101;
    }

    !fixture.assertions.is_empty()
}

/// Emit a Python test backend stub for a trait-bridge fixture.
///
/// Generates a duck-typed Python class `_TestStub_<fixture_id>` whose methods
/// return sensible default values. When `super_trait` is set, a `name()` method
/// is emitted returning the fixture's name string extracted from `fixture.input`.
///
/// Python trait bridges use duck typing — no base class or explicit interface
/// inheritance is required. The class just needs to provide the right method
/// signatures that the PyO3 bridge's `register_<trait>` function can call.
///
/// The returned `arg_expr` is `_TestStub_<fixture_id>()` (instantiation without
/// wrapping), which is the form expected by the generated `register_<trait>` call.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::{escape_python, sanitize_ident};
    use std::fmt::Write as FmtWrite;

    let stub_name = format!("_TestStub_{}", sanitize_ident(&fixture.id));
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let defaults = language_defaults("python");

    let mut setup = String::new();

    let _ = writeln!(setup, "class {stub_name}:");

    // Track whether we emitted any method (need `pass` if empty class).
    let mut method_count = 0usize;

    // name() from Plugin super-trait, if configured.
    if trait_bridge.super_trait.is_some() {
        let escaped = escape_python(&backend_name);
        let _ = writeln!(setup, "    def name(self):");
        let _ = writeln!(setup, "        return \"{escaped}\"");
        method_count += 1;
        // initialize() has a Rust default impl but PyO3 calls it unconditionally on
        // every registered plugin object — the Python stub must define it.
        let _ = writeln!(setup, "    def initialize(self):");
        let _ = writeln!(setup, "        pass");
        method_count += 1;
        // shutdown() also has a Rust default impl but PyO3 calls it unconditionally
        // on cleanup — the Python stub must define it.
        let _ = writeln!(setup, "    def shutdown(self):");
        let _ = writeln!(setup, "        pass");
        method_count += 1;
    }

    // Required methods only.
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Skip Plugin::name if we already emitted it.
        if trait_bridge.super_trait.is_some() && method.name == "name" {
            continue;
        }
        emit_python_stub_method(&mut setup, method, &*defaults);
        method_count += 1;
    }

    // Emit pass for an empty class body (unlikely but correct).
    if method_count == 0 {
        let _ = writeln!(setup, "    pass");
    }

    let arg_expr = format!("{stub_name}()");

    // Indent the entire class definition by 4 spaces so it sits at function-body
    // scope when the caller embeds it inside a `def test_*():` block.
    let indented_setup = indent_block(&setup, 4);

    // Pytest runs every test in a single python process, so registering a
    // test backend leaks into later tests in the suite. Emit
    // `unregister_<trait>("<backend_name>")` after the call+assertions so the
    // shared global registry is restored: the core's
    // `ensure_<trait>_initialized` self-heal triggers on the next access
    // (registry becomes empty after our unregister) and re-seeds defaults
    // like `tesseract` that smoke tests rely on. Without this teardown,
    // `test_register_ocr_backend_trait_bridge` leaves `test-backend` in the
    // registry and any later OCR fixture (e.g. `test_ocr_image_png`) fails
    // with `OCR backend 'tesseract' not registered`.
    let teardown_block = trait_bridge
        .unregister_fn
        .as_deref()
        .map(|unregister_fn| {
            let escaped = escape_python(&backend_name);
            format!("    {unregister_fn}(\"{escaped}\")\n")
        })
        .unwrap_or_default();

    super::TestBackendEmission {
        setup_block: indented_setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Indent every non-empty line of `block` by `spaces` spaces.
fn indent_block(block: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    block
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if block.ends_with('\n') { "\n" } else { "" }
}

/// Format a single Python stub method returning the language default for its return type.
fn emit_python_stub_method(
    out: &mut String,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) {
    use std::fmt::Write as FmtWrite;

    // Build parameter list: `self, _p0, _p1, ...` (unused, hence _ prefix).
    let mut param_parts = vec!["self".to_string()];
    for (i, _) in method.params.iter().enumerate() {
        param_parts.push(format!("_p{i}"));
    }
    let params_str = param_parts.join(", ");

    // Default return expression for the return type.
    // Named types in e2e stubs must return JSON-serialisable values: the PyO3
    // bridge calls the Python method and deserialises the return value from JSON.
    // Returning `TypeName()` would reference a type that is not imported/defined
    // in the generated test file and would cause a NameError at runtime. Return
    // an empty dict `{}` instead — it round-trips cleanly through serde_json.
    //
    // For numeric types in test backends, use a nonzero integer default.
    let default_val = match &method.return_type {
        crate::core::ir::TypeRef::Named(_) => "{}".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "False".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F32) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F64) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(_) => "1".to_string(),
        other => defaults.emit_default(other),
    };

    let async_kw = if method.is_async { "async " } else { "" };
    let _ = writeln!(out, "    {async_kw}def {name}({params_str}):", name = method.name);
    let _ = writeln!(out, "        return {default_val}");
}

/// Extract a backend name string from the fixture input JSON.
///
/// See [`super::super::rust::extract_backend_name_from_input`] for the lookup strategy.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    use crate::e2e::codegen::E2eCodegen;

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
    fn language_name_is_python() {
        let codegen = PythonE2eCodegen;
        assert_eq!(codegen.language_name(), "python");
    }

    #[test]
    fn generate_empty_groups_produces_config_files_only() {
        use crate::core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

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
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let codegen = PythonE2eCodegen;
        let files = codegen.generate(&[], &e2e, &resolved, &[], &[]).unwrap();
        // conftest.py, tests/__init__.py, pyproject.toml
        // (NO root __init__.py — would shadow the published `demo_client` package
        // during `uv sync`'s editable install of the e2e project.)
        assert_eq!(files.len(), 3, "expected 3 config files, got: {}", files.len());
        let paths: Vec<_> = files
            .iter()
            .map(|f| f.path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("conftest.py")));
        assert!(paths.iter().any(|p| p.ends_with("pyproject.toml")));
        // tests/__init__.py is kept so pytest can resolve `from tests.X import …` if used
        assert!(
            paths.iter().any(|p| p.ends_with("tests/__init__.py")),
            "tests/__init__.py must still be emitted; got: {paths:?}"
        );
        assert!(
            !paths
                .iter()
                .any(|p| p.ends_with("python/__init__.py") && !p.ends_with("tests/__init__.py")),
            "no root-level __init__.py in test_apps/python/ — would shadow the published wheel \
             during uv sync's editable install. got: {paths:?}"
        );
    }

    #[test]
    fn emit_test_backend_python_generates_class_and_instance_expr() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let m1 = test_method("do_work", TypeRef::String, false, false);
        let m2 = test_method("async_op", TypeRef::Named("WorkResult".to_string()), true, false);
        let methods = [&m1, &m2];

        let fixture = make_fixture("py_test_fixture", serde_json::json!({ "name": "my-python-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // setup_block must define a Python class.
        assert!(
            emission.setup_block.contains("class _TestStub_py_test_fixture"),
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
            emission.setup_block.contains("def name("),
            "setup_block should emit name() when super_trait is set"
        );
        assert!(
            emission.setup_block.contains("my-python-backend"),
            "name() should return the backend name from input"
        );

        // Required methods emitted.
        assert!(
            emission.setup_block.contains("def do_work("),
            "required method do_work should be emitted"
        );
        assert!(
            emission.setup_block.contains("async def async_op("),
            "required async method should be emitted"
        );

        // arg_expr is a plain instantiation.
        assert_eq!(
            emission.arg_expr, "_TestStub_py_test_fixture()",
            "arg_expr should be a plain constructor call"
        );

        // setup_block must be indented 4 spaces for function-local scope.
        assert!(
            emission.setup_block.starts_with("    class "),
            "setup_block should be 4-space indented, got: {}",
            emission.setup_block
        );

        // Named return type must use {} not WorkResult().
        assert!(
            emission.setup_block.contains("return {}"),
            "Named return type should emit {{}} not a constructor call, got: {}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("WorkResult()"),
            "Named return type must not emit a constructor call, got: {}",
            emission.setup_block
        );
    }

    #[test]
    fn emit_test_backend_python_skips_default_impl_methods() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            ..Default::default()
        };

        let required = test_method("must_implement", TypeRef::String, false, false);
        let optional = test_method("may_implement", TypeRef::String, false, true);
        let methods = [&required, &optional];

        let fixture = make_fixture("py_skip_defaults", serde_json::json!({}));
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("def must_implement("),
            "required method should be emitted"
        );
        assert!(
            !emission.setup_block.contains("def may_implement("),
            "optional method should be skipped"
        );
    }

    #[test]
    fn emit_test_backend_python_includes_shutdown_with_super_trait() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let dimension_method = test_method("dimensions", TypeRef::Primitive(PrimitiveType::I32), false, false);
        let methods = [&dimension_method];

        let fixture = make_fixture(
            "py_embedding_backend",
            serde_json::json!({ "name": "test-embedding-backend" }),
        );
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // Verify name() is emitted
        assert!(
            emission.setup_block.contains("def name(self):"),
            "name() should be emitted with super_trait"
        );

        // Verify initialize() is emitted
        assert!(
            emission.setup_block.contains("def initialize(self):"),
            "initialize() should be emitted with super_trait"
        );

        // Verify shutdown() is emitted (the fix)
        assert!(
            emission.setup_block.contains("def shutdown(self):"),
            "shutdown() should be emitted with super_trait, got: {}",
            emission.setup_block
        );

        // All three should be present with correct bodies
        assert!(
            emission.setup_block.contains("def name(self):")
                && emission.setup_block.contains("return \"test-embedding-backend\""),
            "name() should return backend name"
        );
        assert!(
            emission.setup_block.contains("def initialize(self):") && emission.setup_block.contains("pass"),
            "initialize() should have pass body"
        );
        assert!(
            emission.setup_block.contains("def shutdown(self):") && emission.setup_block.contains("pass"),
            "shutdown() should have pass body"
        );
    }

    #[test]
    fn emit_test_backend_python_numeric_return_types_return_nonzero() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        // Test integer return types
        let dimensions_method = test_method("dimensions", TypeRef::Primitive(PrimitiveType::I32), false, false);
        let size_method = test_method("embedding_size", TypeRef::Primitive(PrimitiveType::U64), false, false);
        let float_method = test_method("similarity_score", TypeRef::Primitive(PrimitiveType::F64), false, false);
        let bool_method = test_method("is_valid", TypeRef::Primitive(PrimitiveType::Bool), false, false);
        let methods = [&dimensions_method, &size_method, &float_method, &bool_method];

        let fixture = make_fixture(
            "py_numeric_backend",
            serde_json::json!({ "name": "test-numeric-backend" }),
        );
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // Integer types should return 1 instead of 0 (for validation constraints)
        assert!(
            emission.setup_block.contains("def dimensions(") && emission.setup_block.contains("return 1"),
            "I32 should return 1, got: {}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("def embedding_size(") && emission.setup_block.contains("return 1"),
            "U64 should return 1, got: {}",
            emission.setup_block
        );

        // Float types should return 0.0
        assert!(
            emission.setup_block.contains("def similarity_score(") && emission.setup_block.contains("return 0.0"),
            "F64 should return 0.0, got: {}",
            emission.setup_block
        );

        // Bool should return False
        assert!(
            emission.setup_block.contains("def is_valid(") && emission.setup_block.contains("return False"),
            "Bool should return False, got: {}",
            emission.setup_block
        );
    }

    #[test]
    fn emit_test_backend_python_populates_teardown_when_unregister_fn_is_set() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_backend".to_string()),
            unregister_fn: Some("unregister_test_backend".to_string()),
            ..Default::default()
        };

        let m = test_method("do_work", TypeRef::String, false, false);
        let methods = [&m];
        let fixture = make_fixture("teardown_fixture", serde_json::json!({ "name": "my-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission
                .teardown_block
                .contains("unregister_test_backend(\"my-backend\")"),
            "teardown_block should call the unregister fn with the backend name, got: {:?}",
            emission.teardown_block
        );
    }

    #[test]
    fn emit_test_backend_python_omits_teardown_when_unregister_fn_is_unset() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        // No `unregister_fn` configured (e.g. per-call bridge with no global registry).
        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let m = test_method("do_work", TypeRef::String, false, false);
        let methods = [&m];
        let fixture = make_fixture("no_teardown_fixture", serde_json::json!({ "name": "x" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission.teardown_block.is_empty(),
            "teardown_block should be empty when no unregister_fn is configured, got: {:?}",
            emission.teardown_block
        );
    }
}
