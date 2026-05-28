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
        // This includes both sample-llm mock_response fixtures and sample_project http fixtures.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.needs_mock_server());

        // Check if any fixture uses the http integration test pattern (sample_project http fixtures).
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

        // anyhow is needed when any fixture uses a `test_backend` arg: the generated
        // Rust trait-bridge stubs reference `anyhow::Error` in their method signatures
        // because sample_core plugin traits declare `-> Result<T, anyhow::Error>`.
        // Without this direct dependency the stubs fail to compile with E0433.
        let all_call_args_for_anyhow = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .flat_map(|c| c.args.iter())
            .any(|a| a.arg_type == "test_backend");
        let any_fixture_test_backend = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .filter(|f| !is_skipped(f, "rust"))
            .any(|f| f.args.iter().any(|a| a.arg_type == "test_backend"));
        let needs_anyhow = all_call_args_for_anyhow || any_fixture_test_backend;

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
                needs_anyhow,
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
            let content = render_test_file(
                &group.category,
                &fixtures,
                e2e_config,
                config,
                _type_defs,
                &dep_name,
                needs_mock_server,
            );

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
/// Generates a minimal `struct TestStub<fixture_id_pascalcase>` with a `_name` field and
/// a concrete `impl <trait_name> for TestStub<fixture_id_pascalcase>` block where every
/// required method returns a language-default value. When the bridge config
/// declares a `super_trait`, a `name()` method is also emitted returning the
/// fixture's name string extracted from `fixture.input`.
///
/// The returned `arg_expr` wraps the stub in `std::sync::Arc::new(...)`, which
/// is the form expected by the generated `register_<trait>` function.
///
/// The `type_imports` field on the returned emission lists short symbol names
/// (trait name plus any named types referenced in method signatures) that the
/// caller must import from the crate under test so the stub compiles.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use std::fmt::Write as FmtWrite;

    let stub_name = format!("TestStub{}", fixture_id_to_pascal_case(&fixture.id));
    let trait_name = &trait_bridge.trait_name;
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let defaults = language_defaults("rust");

    // Collect named types that must be imported (trait name + return/param types).
    let mut type_imports: Vec<String> = Vec::new();
    // The trait itself must be in scope for `impl TraitName for ...` to resolve.
    type_imports.push(trait_name.clone());

    let mut setup = String::new();

    // Derive the crate module name from the super_trait path (e.g. "sample_core::plugins::Plugin"
    // → "sample_core"). Used to qualify single-arg `Result<T>` return types so that stub method
    // signatures match the trait declaration (which uses a crate-level `Result` alias).
    let crate_module: Option<&str> = trait_bridge
        .super_trait
        .as_deref()
        .and_then(|s| s.split("::").next())
        .filter(|s| !s.is_empty());

    // Struct definition with a cached name field.
    let _ = writeln!(setup, "struct {stub_name} {{ _name: &'static str }}");

    // When the trait has a super-trait (e.g. Plugin), emit a separate impl block
    // for it first.  Putting super-trait methods inside `impl TraitName for ...`
    // is a compile error (E0407).
    if let Some(super_trait) = &trait_bridge.super_trait {
        // Derive a short import alias: take the last path segment.
        let super_short = super_trait.split("::").last().unwrap_or(super_trait.as_str());
        // Use the fully-qualified path in the impl header so it resolves without
        // an extra `use` import.
        let _ = writeln!(setup, "impl {super_trait} for {stub_name} {{");
        let _ = writeln!(setup, "    fn name(&self) -> &str {{ self._name }}");
        let _ = writeln!(setup, "}}");
        // Track the short name for import, but only if it's not already qualified.
        if !super_short.is_empty() && !super_trait.contains("::") {
            type_imports.push(super_short.to_string());
        }
    }

    // Determine if any required method is async so we know whether to add #[async_trait].
    let has_async_methods = methods
        .iter()
        .any(|m| !(m.has_default_impl || trait_bridge.super_trait.is_some() && m.name == "name") && m.is_async);

    // When the trait has async methods (decorated with #[async_trait] in Rust), the
    // impl block must also carry `#[async_trait]`.  Without it the async fn signatures
    // won't match the trait's `BoxFuture`-transformed signatures and the compiler
    // emits E0195 (lifetime bounds mismatch).
    if has_async_methods {
        let _ = writeln!(setup, "#[async_trait::async_trait]");
    }

    // Impl block for the main trait.
    let _ = writeln!(setup, "impl {trait_name} for {stub_name} {{");

    // Required methods only (skip those with a default implementation).
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Skip Plugin supertrait methods — already emitted in the Plugin impl block above.
        if trait_bridge.super_trait.is_some() && method.name == "name" {
            continue;
        }
        emit_rust_stub_method(&mut setup, method, &*defaults, &mut type_imports, crate_module);
    }

    let _ = writeln!(setup, "}}");

    // Deduplicate imports (stable order for deterministic output).
    type_imports.sort();
    type_imports.dedup();

    // arg_expr: wrapped in Arc for the register call.
    let arg_expr = format!("std::sync::Arc::new({stub_name} {{ _name: \"{backend_name}\" }})");

    // Filter type_imports: skip Rust primitives and std types that are always in scope.
    let type_imports = type_imports
        .into_iter()
        .filter(|s| {
            !matches!(
                s.as_str(),
                "bool"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "f32"
                    | "f64"
                    | "usize"
                    | "isize"
                    | "String"
                    | "str"
                    | "Vec"
                    | "Option"
                    | "Result"
                    | "()"
            )
        })
        .collect();

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports,
        // Rust cargo integration tests run each `tests/*.rs` file in its own
        // process, so the global registry resets between files and no
        // teardown is required.
        teardown_block: String::new(),
    }
}

/// Collect all `Named` type identifiers referenced anywhere in `ty` into `out`.
///
/// Only the short identifier is collected (not the fully qualified path),
/// since stub method signatures use short names and callers emit the `use` import.
fn collect_named_types(ty: &crate::core::ir::TypeRef, out: &mut Vec<String>) {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => out.push(name.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Emit the Rust type name for a `TypeRef` using the standard identity mapping.
///
/// Produces valid Rust type syntax: `String`, `Vec<u8>`, `Option<T>`,
/// `HashMap<K, V>`, etc.  Named types pass through as-is — they must be in
/// scope at the call site (usually via a `use` import).
fn rust_type_name(ty: &crate::core::ir::TypeRef) -> String {
    use crate::codegen::type_mapper::{IdentityMapper, TypeMapper};
    IdentityMapper.map_type(ty)
}

/// Format a single Rust stub method with a correctly typed signature.
///
/// Emits `fn name(&self, _p0: T0, _p1: T1, ...) -> ReturnType { body }`.
/// Parameters use `_p{i}` names (underscore-prefixed to silence unused-variable
/// warnings) with explicit types so the impl block compiles.  The return type
/// arrow is emitted for every non-unit return type.  For `Result`-returning
/// methods (`error_type` is `Some`), the return type is `Result<T, error_type>`
/// and the body is `Ok(default_for_T)`.
///
/// For reference-returning methods (`returns_ref = true`), the IR collapses
/// `&[T]` into `Vec<T>` + flag.  A `Default::default()` value cannot be returned
/// as a reference without a named binding, so those methods fall back to
/// `unimplemented!()` which satisfies any return type via the never type `!`.
/// Emit a single method body inside a `impl Trait for Stub` block.
///
/// `crate_module`: when `Some`, used to qualify single-arg `Result<T>` return types
/// as `{crate_module}::Result<T>`.  Pass the crate root name (e.g. `"sample_core"`) when
/// the trait's return type uses a crate-level `Result` type alias rather than the
/// stdlib two-arg `Result<T, E>`.
fn emit_rust_stub_method(
    out: &mut String,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    type_imports: &mut Vec<String>,
    crate_module: Option<&str>,
) {
    use crate::core::ir::TypeRef;
    use std::fmt::Write as FmtWrite;

    // Build the parameter list: `_p0: TypeName, _p1: TypeName, ...`
    // Underscore-prefix silences unused-variable warnings; explicit types are
    // required by the Rust compiler in trait impl method signatures.
    let params_typed: Vec<String> = method
        .params
        .iter()
        .enumerate()
        .map(|(i, param)| {
            // Collect named types referenced in this parameter for import tracking.
            collect_named_types(&param.ty, type_imports);
            // Emit `&T` for reference params (is_ref = true) to match the trait signature.
            // The IR stores the inner type without the `&`; we re-add it here.
            // Use idiomatic Rust slice/str types for common reference forms:
            //   &Vec<u8>       → &[u8]      (byte slices)
            //   &String        → &str       (string slices)
            //   &mut Vec<u8>   → &mut [u8]
            //   &mut String    → &mut str
            //   &T / &mut T    → &T / &mut T
            if param.is_ref {
                use crate::core::ir::TypeRef;
                let mut_kw = if param.is_mut { "mut " } else { "" };
                let ref_str = match &param.ty {
                    TypeRef::Bytes => format!("&{mut_kw}[u8]"),
                    TypeRef::String => format!("&{mut_kw}str"),
                    other => format!("&{}{}", mut_kw, rust_type_name(other)),
                };
                format!("_p{i}: {ref_str}")
            } else {
                let ty_str = rust_type_name(&param.ty);
                format!("_p{i}: {ty_str}")
            }
        })
        .collect();
    let params_str = if params_typed.is_empty() {
        String::new()
    } else {
        format!(", {}", params_typed.join(", "))
    };

    // Build the return type annotation.  Unit returns need no arrow; all others
    // get `-> ReturnType` or `-> Result<ReturnType, ErrorType>`.
    let return_type_str = if method.returns_ref {
        // Reference-returning methods: derive the reference return type from the IR.
        // The IR stores the owned form; map common owned types to their reference forms.
        //   Vec<u8>      → &[u8]
        //   Vec<String>  → &[&str]
        //   String       → &str
        //   Vec<T>       → &[T]
        //   T            → &T
        // Emit with `unimplemented!()` body since returning a reference from a Default
        // value requires a named binding; the never type `!` satisfies any return type.
        use crate::core::ir::TypeRef;
        let ref_type = match &method.return_type {
            TypeRef::String => "&str".to_string(),
            TypeRef::Bytes => "&[u8]".to_string(),
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::String => "&[&str]".to_string(),
                TypeRef::Bytes => "&[u8]".to_string(),
                other => format!("&[{}]", rust_type_name(other)),
            },
            other => format!("&{}", rust_type_name(other)),
        };
        Some(ref_type)
    } else {
        match &method.return_type {
            TypeRef::Unit if method.error_type.is_none() => None,
            _ => {
                let base = rust_type_name(&method.return_type);
                collect_named_types(&method.return_type, type_imports);
                let full = if let Some(err) = &method.error_type {
                    // When `error_type` is `"anyhow::Error"` it signals the IR fallback
                    // for a single-arg `Result<T>` alias (like `sample_core::Result<T>`),
                    // not a literal `anyhow::Error` in the trait signature.
                    // Use `{crate}::Result<T>` so the stub method type matches the trait.
                    if err == "anyhow::Error" {
                        if let Some(module) = crate_module {
                            format!("{module}::Result<{base}>")
                        } else {
                            // No module context — keep the fallback form.
                            format!("Result<{base}, {err}>")
                        }
                    } else {
                        // Collect the error type name for import tracking only when it is a
                        // simple identifier (no path separators).  Fully-qualified names like
                        // `anyhow::Error` are already usable as-is in the signature without
                        // a `use` import; only bare names like `SampleCrateError` need one.
                        if !err.contains("::") {
                            type_imports.push(err.clone());
                        }
                        format!("Result<{base}, {err}>")
                    }
                } else {
                    base
                };
                Some(full)
            }
        }
    };

    // Build the method body.
    let body = if method.returns_ref {
        // Reference-returning methods: return the cheapest valid reference that
        // satisfies the return type without requiring a named binding.
        //   &str, &[u8], &[&str], &[T]  → use an empty literal (&[], "")
        //   &T (other)                   → fall back to unimplemented!()
        use crate::core::ir::TypeRef;
        match &method.return_type {
            TypeRef::String => "\"\"".to_string(),
            TypeRef::Bytes | TypeRef::Vec(_) => "&[]".to_string(),
            _ => "unimplemented!()".to_string(),
        }
    } else {
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
    let return_annotation = match &return_type_str {
        Some(rt) => format!(" -> {rt}"),
        None => String::new(),
    };
    let _ = writeln!(
        out,
        "    {async_kw}fn {name}(&self{params_str}){return_annotation} {{ {body} }}",
        name = method.name
    );
}

/// Convert a fixture ID (snake_case) to PascalCase for use in Rust struct names.
///
/// Transforms `register_embedding_backend_trait_bridge` → `RegisterEmbeddingBackendTraitBridge`.
fn fixture_id_to_pascal_case(id: &str) -> String {
    id.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
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
        let m2 = test_method(
            "process_async",
            TypeRef::Named("WorkResult".to_string()),
            true,
            Some("WorkError"),
        );
        let methods = [&m1, &m2];

        let fixture = make_fixture("my_test_fixture", serde_json::json!({ "name": "my-test-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // setup_block must contain the stub struct and impl.
        assert!(
            emission.setup_block.contains("TestStubMyTestFixture"),
            "setup_block should contain stub name, got: {}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("TestTrait"),
            "setup_block should reference trait by name, got: {}",
            emission.setup_block
        );
        // Must NOT hardcode any sample_core-domain trait name.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode OcrBackend"
        );
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
        assert!(
            emission.arg_expr.contains("Arc::new"),
            "arg_expr should use Arc::new, got: {}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.contains("TestStubMyTestFixture"),
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

        assert!(
            emission.setup_block.contains("fn required_method("),
            "required method should be emitted"
        );
        assert!(
            !emission.setup_block.contains("fn optional_method("),
            "method with default impl should be skipped"
        );
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
